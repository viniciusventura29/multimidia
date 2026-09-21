//! O transporte Bluetooth do ELM327.
//!
//! O `eclipse-obd` sabe o protocolo (o que mandar, como ler de volta) mas não
//! sabe carregar os bytes — isso é [`Elm327Transport`], e no Android quem carrega
//! é o plugin `tauri-plugin-obd-bt` (socket SPP nativo). Este arquivo é a cola:
//! escolhe o adaptador pareado, conecta, e adapta cada comando à ponte do plugin.
//!
//! As chamadas ao plugin (`run_mobile_plugin`) são **bloqueantes** — cada leitura
//! de PID espera o barramento do carro responder (centenas de ms). Por isso vão
//! em `spawn_blocking`, para não travar o executor async onde o poller vive.
//!
//! Aqui mora também o [`Radio`]: a mesma ponte vista de cima, como um trait. O
//! módulo `adaptador` fala com ele em vez de falar com o `AppHandle`, e é isso que
//! permite testar buscar/parear/gravar no Mac, sem carro e sem Android.

// Só é *usado* no Android (o desktop não chama `conectar`), mas segue sendo
// compilado no macOS para type-check. Sem isto, o desktop reclamaria de código
// morto em tudo aqui.
#![cfg_attr(not(mobile), allow(dead_code))]

use std::path::Path;

use async_trait::async_trait;
use eclipse_obd::{Arquivo, Elm327Source, Elm327Transport, ObdError};
use serde::{Deserialize, Serialize};
use tauri_plugin_obd_bt::{BtDevice, BtInfo, BtKind, ObdBtExt};

/// O adaptador escolhido pelo dono, no diretório de dados do app.
pub const ADAPTADOR_JSON: &str = "adaptador.json";

/// O adaptador que este carro usa.
///
/// Arquivo próprio, e não um campo no `Veiculo`: aquele é `Copy` e um `String`
/// dentro dele quebraria a conta de consumo inteira por nada. Some com ele e o
/// painel volta a adivinhar pelo nome — que é o que fazia antes desta tela existir.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptadorSalvo {
    /// O MAC. É por ele que se conecta, e é o único campo que precisa estar certo.
    pub mac: String,
    /// Como ele se anuncia, só para a tela ter o que mostrar.
    #[serde(default)]
    pub nome: String,
    /// Clássico ou BLE — sem isto a reconexão teria que adivinhar o transporte.
    #[serde(default)]
    pub tipo: BtKind,
}

/// Lê o adaptador salvo, se houver.
pub fn adaptador_salvo(dir: &Path) -> Option<AdaptadorSalvo> {
    Arquivo::<Option<AdaptadorSalvo>>::load(dir.join(ADAPTADOR_JSON)).dados
}

/// Nomes comuns de adaptadores ELM327/OBD, para achar o certo entre os pareados
/// quando o usuário não fixa um por `ECLIPSE_OBD_DEVICE`. Comparados contra o
/// nome já [normalizado](normalizar) — "V-LINK" vira "VLINK" e casa.
const PADROES_NOME: [&str; 7] = ["OBD", "ELM", "VLINK", "VIECAR", "KONNWEI", "VGATE", "ICAR"];

/// O nome reduzido ao que importa: só letras e dígitos, em maiúscula.
///
/// Foi um hífen que quebrou no carro de verdade: o adaptador se anuncia como
/// "V-LINK" e o padrão "VLINK" não era substring. Pontuação e espaço variam por
/// clone; letra e dígito não.
fn normalizar(nome: &str) -> String {
    nome.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_uppercase()
}

/// Um comando falhou na ponte do plugin. Qualquer falha de socket vira
/// [`ObdError::Bus`]: sobe pelo poller e faz o supervisor reconectar. (O carro
/// não responder um PID específico **não** passa por aqui — vem como texto
/// `NO DATA` e o parser do `eclipse-obd` trata como `Unsupported`.)
fn erro(e: tauri_plugin_obd_bt::Error) -> ObdError {
    ObdError::Bus(e.to_string())
}

/// O transporte de verdade: cada `command` vira uma chamada ao plugin Android.
pub struct AndroidBtTransport {
    app: tauri::AppHandle,
}

#[async_trait]
impl Elm327Transport for AndroidBtTransport {
    async fn command(&mut self, cmd: &str, timeout_ms: u32) -> Result<String, ObdError> {
        let app = self.app.clone();
        let cmd = cmd.to_string();
        tokio::task::spawn_blocking(move || app.obd_bt().command(&cmd, timeout_ms).map_err(erro))
            .await
            .map_err(|e| ObdError::Bus(format!("task de leitura falhou: {e}")))?
    }
}

/// O nome parece de um adaptador ELM327?
///
/// Serve para a tela destacar o candidato óbvio no meio dos fones de ouvido — e
/// para o palpite de quando não há nada escolhido. Nunca para esconder ninguém da
/// lista: clone não é obrigado a se chamar de nada.
pub fn parece_adaptador(nome: &str) -> bool {
    let nome = normalizar(nome);
    PADROES_NOME.iter().any(|p| nome.contains(p))
}

/// Escolhe qual adaptador pareado usar.
///
/// Com `ECLIPSE_OBD_DEVICE` (nome ou MAC), casa por ele; senão pega o primeiro
/// cujo nome pareça de um ELM327.
fn escolher<'a>(pareados: &'a [BtDevice], alvo: Option<&str>) -> Option<&'a BtDevice> {
    if let Some(alvo) = alvo {
        let alvo_norm = normalizar(alvo);
        return pareados.iter().find(|d| {
            d.address.eq_ignore_ascii_case(alvo)
                || (!alvo_norm.is_empty() && normalizar(&d.name).contains(&alvo_norm))
        });
    }
    pareados.iter().find(|d| parece_adaptador(&d.name))
}

/// Garante permissão, escolhe e abre o adaptador; devolve um rótulo para o log.
///
/// A ordem é **salvo → `ECLIPSE_OBD_DEVICE` → palpite pelo nome**. O salvo vem
/// primeiro porque é o único que o dono escolheu de verdade — e é o único caminho
/// que alcança um adaptador BLE, que nunca aparece na lista de pareados.
///
/// Tudo bloqueante num `spawn_blocking` só: pedir permissão espera o usuário
/// responder o diálogo, e listar/conectar falam com o rádio.
async fn preparar(app: &tauri::AppHandle, dir: &Path) -> Result<String, ObdError> {
    let app = app.clone();
    let alvo = std::env::var("ECLIPSE_OBD_DEVICE").ok();
    let salvo = adaptador_salvo(dir);

    tokio::task::spawn_blocking(move || -> Result<String, ObdError> {
        let bt = app.obd_bt();

        bt.ensure_permissions().map_err(erro)?;

        if let Some(a) = &salvo {
            match bt.connect(&a.mac, a.tipo) {
                Ok(()) => return Ok(format!("{} ({}, {})", a.nome, a.mac, a.tipo.como_texto())),
                // Não desiste: o dono pode ter trocado de adaptador sem mexer na
                // tela, e adivinhar pelo nome ainda acerta nesse caso.
                Err(err) => tracing::warn!(
                    mac = %a.mac,
                    %err,
                    "o adaptador salvo não atendeu; tentando os pareados"
                ),
            }
        }

        let pareados = bt.list_bonded().map_err(erro)?;
        for d in &pareados {
            tracing::info!(nome = %d.name, mac = %d.address, "adaptador Bluetooth pareado");
        }

        let escolhido = escolher(&pareados, alvo.as_deref()).ok_or_else(|| {
            ObdError::Bus(
                "nenhum adaptador escolhido; abra a tela do carro e toque em Adaptador OBD"
                    .to_string(),
            )
        })?;

        bt.connect(&escolhido.address, escolhido.kind)
            .map_err(erro)?;
        Ok(format!(
            "{} ({}, {})",
            escolhido.name,
            escolhido.address,
            escolhido.kind.como_texto()
        ))
    })
    .await
    .map_err(|e| ObdError::Bus(format!("task de conexão falhou: {e}")))?
}

/// Conecta ao adaptador e faz o handshake ELM327, devolvendo a fonte pronta.
pub async fn conectar(
    app: &tauri::AppHandle,
    dir: &Path,
) -> Result<Elm327Source<AndroidBtTransport>, ObdError> {
    let rotulo = preparar(app, dir).await?;
    tracing::info!(adaptador = %rotulo, "conectado; iniciando handshake do ELM327");
    Elm327Source::conectar(AndroidBtTransport { app: app.clone() }).await
}

/// Puxa a posição do Android e empurra para o módulo `nav`.
///
/// O `navigator.geolocation` nunca entregou nada nesta central: satélite e rede
/// falharam com o MESMO timeout, e esse empate é a assinatura de um pedido que
/// não chega ao sistema — a WebView do Android só libera geolocalização para a
/// página se o app responder o `onGeolocationPermissionsShowPrompt`, e o Tauri
/// não responde. Então a posição passa a vir por fora da WebView.
///
/// Sondagem em vez de callback: o plugin Android é pergunta-e-resposta, e 1 Hz
/// é de sobra para um mapa de carro. O ouvinte do lado Kotlin é quem acumula as
/// posições; aqui só se lê o que ele tem de mais fresco.
#[cfg(mobile)]
pub fn bombear_localizacao(app: tauri::AppHandle, emissor: eclipse_gps::Emissor) {
    use serde::Deserialize;

    /// O que o Kotlin devolve. Ver `Localizacao.ultima`.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Posicao {
        tem: bool,
        #[serde(default)]
        motivo: String,
        #[serde(default)]
        lat: f64,
        #[serde(default)]
        lon: f64,
        #[serde(default)]
        velocidade_ms: f32,
        #[serde(default)]
        rumo: f32,
        #[serde(default)]
        precisao_m: f32,
        #[serde(default)]
        provedor: String,
        #[serde(default)]
        idade_ms: i64,
        /// Por que não veio, quando não veio. Ver `Localizacao.diagnostico`.
        #[serde(default)]
        diagnostico: serde_json::Value,
    }

    tauri::async_runtime::spawn(async move {
        let mut relatou_fix = false;
        // Quando o último "ainda sem posição" foi ao diário. `None` = nenhum.
        let mut ultimo_relato: Option<std::time::Instant> = None;
        // Rumo da leitura anterior: parado, o Android devolve `-1` (não sei), e
        // sem guardar o último a seta do mapa voltaria ao norte a cada parada.
        let mut ultimo_rumo = 0.0_f32;

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

            let app2 = app.clone();
            let bruto = tokio::task::spawn_blocking(move || app2.obd_bt().ultima_posicao()).await;
            let Ok(Ok(bruto)) = bruto else { continue };
            let Ok(p) = serde_json::from_str::<Posicao>(&bruto) else {
                continue;
            };

            if !p.tem {
                // A cada 30 s, e não uma vez só. Contar uma vez foi um erro
                // meu: "ainda sem posição" um segundo depois do boot é o
                // esperado, e sem repetir não havia como saber se a posição
                // chegou depois, se piorou, ou se o carro ficou minutos assim.
                // Agora o diagnóstico vai junto, e é ele que aponta o dono do
                // problema: localização desligada, ROM sem provedor, antena, ou
                // céu.
                let agora = std::time::Instant::now();
                let na_hora = ultimo_relato
                    .map(|t: std::time::Instant| agora.duration_since(t).as_secs() >= 30)
                    .unwrap_or(true);
                if na_hora {
                    ultimo_relato = Some(agora);
                    tracing::warn!(
                        target: "nav",
                        motivo = %p.motivo,
                        diagnostico = %p.diagnostico,
                        "o Android ainda não tem posição"
                    );
                }
                let _ = emissor.send(Err(eclipse_gps::GpsError::SemSinal));
                continue;
            }

            if p.rumo >= 0.0 {
                ultimo_rumo = p.rumo;
            }

            if !relatou_fix {
                relatou_fix = true;
                // Marco, e não aviso: numa ignição em que o GPS PASSA a
                // funcionar nada mais subiria, e é justamente essa a notícia.
                if let Some(diario) = crate::diario::atual() {
                    let mut linha = crate::diario::Linha::nova(
                        crate::diario::Nivel::Info,
                        "nav",
                        "o Android entregou posição",
                    );
                    linha
                        .dados
                        .insert("provedor".into(), p.provedor.clone().into());
                    linha
                        .dados
                        .insert("precisao_m".into(), format!("{:.0}", p.precisao_m).into());
                    linha.dados.insert("idade_ms".into(), p.idade_ms.into());
                    diario.marco(linha);
                }
            }

            let _ = emissor.send(Ok(eclipse_gps::Fix {
                lat: p.lat,
                lon: p.lon,
                heading: ultimo_rumo,
                speed_kmh: p.velocidade_ms * 3.6,
                accuracy_m: if p.precisao_m < 0.0 {
                    0.0
                } else {
                    p.precisao_m
                },
            }));
        }
    });
}

/// Pergunta ao Android se o Spotify deixa o Eclipse navegar na biblioteca dele.
///
/// Temporária: existe para decidir se vale trocar o Web Playback SDK (que hoje
/// fica mudo, reinicia sozinho e erra a duração, porque decodifica áudio dentro
/// da WebView) por MediaController. Sai quando a resposta chegar.
///
/// Roda uma vez na subida, num `spawn_blocking` — conectar num serviço do
/// Android é bloqueante — e o que ela descobre vai para o diário de bordo como
/// marco, porque numa ignição em que nada dá errado nada subiria.
#[cfg(mobile)]
pub fn sondar_media(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let achado = tokio::task::spawn_blocking(move || {
            app.obd_bt().sondar_media().map_err(|e| e.to_string())
        })
        .await;

        let Some(diario) = crate::diario::atual() else {
            return;
        };
        let mut linha = crate::diario::Linha::nova(
            crate::diario::Nivel::Info,
            "sonda",
            "o que o MediaBrowser do aparelho respondeu",
        );
        let texto = match achado {
            Ok(Ok(json)) => json,
            Ok(Err(err)) => format!("{{\"erro\":\"{err}\"}}"),
            Err(err) => format!("{{\"erro\":\"a task falhou: {err}\"}}"),
        };
        // Cru, e não desserializado: a graça é ver exatamente o que veio,
        // inclusive campo que eu não previ.
        linha
            .dados
            .insert("resposta".into(), serde_json::Value::String(texto));
        diario.marco(linha);
    });
}

/// O rádio Bluetooth visto de cima: buscar, parear, listar.
///
/// Existe como trait por um motivo só, e é bom: a tela de escolha do adaptador
/// precisa de um estado com busca, pareamento que falha e arquivo que grava — e
/// nada disso se testa contra um `AppHandle`. Contra um rádio de mentira, se testa.
///
/// Só o que a **escolha** precisa está aqui. Falar com o ELM327 continua sendo
/// [`AndroidBtTransport`], que é outra conversa e tem outra dona (o módulo OBD).
pub trait Radio: Send + Sync {
    fn info(&self) -> Result<BtInfo, String>;
    fn permissoes(&self) -> Result<(), String>;
    fn pareados(&self) -> Result<Vec<BtDevice>, String>;
    fn buscar(&self) -> Result<(), String>;
    /// O que a busca achou até agora, e se ela ainda está correndo.
    fn achados(&self) -> Result<(Vec<BtDevice>, bool), String>;
    fn parar_busca(&self) -> Result<(), String>;
    fn parear(&self, mac: &str) -> Result<(), String>;
}

/// O rádio de verdade, o do aparelho.
pub struct RadioDoAparelho {
    app: tauri::AppHandle,
}

impl RadioDoAparelho {
    pub fn novo(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Radio for RadioDoAparelho {
    fn info(&self) -> Result<BtInfo, String> {
        self.app.obd_bt().info().map_err(|e| e.to_string())
    }
    fn permissoes(&self) -> Result<(), String> {
        self.app
            .obd_bt()
            .ensure_permissions()
            .map_err(|e| e.to_string())
    }
    fn pareados(&self) -> Result<Vec<BtDevice>, String> {
        self.app.obd_bt().list_bonded().map_err(|e| e.to_string())
    }
    fn buscar(&self) -> Result<(), String> {
        self.app.obd_bt().start_scan().map_err(|e| e.to_string())
    }
    fn achados(&self) -> Result<(Vec<BtDevice>, bool), String> {
        self.app.obd_bt().scan_results().map_err(|e| e.to_string())
    }
    fn parar_busca(&self) -> Result<(), String> {
        self.app.obd_bt().stop_scan().map_err(|e| e.to_string())
    }
    fn parear(&self, mac: &str) -> Result<(), String> {
        self.app.obd_bt().bond(mac).map_err(|e| e.to_string())
    }
}

/// Grava a escolha do dono (ou a apaga).
pub fn gravar_adaptador(dir: &Path, escolha: Option<AdaptadorSalvo>) -> std::io::Result<()> {
    let mut arquivo: Arquivo<Option<AdaptadorSalvo>> = Arquivo::load(dir.join(ADAPTADOR_JSON));
    arquivo.dados = escolha;
    arquivo.salvar()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> BtDevice {
        BtDevice {
            name: name.to_string(),
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            kind: tauri_plugin_obd_bt::BtKind::Spp,
            bonded: true,
            rssi: None,
        }
    }

    #[test]
    fn acha_o_adaptador_apesar_de_hifens_e_espacos_no_nome() {
        // O caso que falhou no carro: o adaptador se chama "V-LINK" e o padrão
        // "VLINK" não é substring por causa do hífen. O fone do usuário também
        // fica na lista — não pode ser escolhido no lugar.
        let pareados = [dev("Galaxy Buds"), dev("V-LINK")];
        let escolhido = escolher(&pareados, None).expect("tinha que achar o V-LINK");
        assert_eq!(escolhido.name, "V-LINK");

        // Variações reais dos clones: espaços e caixa baixa.
        for nome in ["v-link", "OBD II", "Vgate iCar Pro", "elm 327"] {
            let pareados = [dev("JBL Flip"), dev(nome)];
            assert!(
                escolher(&pareados, None).is_some(),
                "não achou o adaptador chamado {nome:?}"
            );
        }
    }

    #[test]
    fn alvo_explicito_vence_e_tambem_ignora_pontuacao() {
        let pareados = [dev("V-LINK"), dev("OBDII")];
        // Por MAC, ignorando caixa.
        let por_mac = escolher(&pareados, Some("aa:bb:cc:dd:ee:ff")).unwrap();
        assert_eq!(por_mac.name, "V-LINK");
        // Por nome, mesmo digitado sem o hífen.
        let por_nome = escolher(&pareados, Some("vlink")).unwrap();
        assert_eq!(por_nome.name, "V-LINK");
    }

    #[test]
    fn sem_adaptador_na_lista_nao_escolhe_nada() {
        let pareados = [dev("Galaxy Buds"), dev("JBL Flip"), dev("")];
        assert!(escolher(&pareados, None).is_none());
    }
}
