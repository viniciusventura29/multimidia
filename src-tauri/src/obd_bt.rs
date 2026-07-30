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

// Só é *usado* no Android (o desktop não chama `conectar`), mas segue sendo
// compilado no macOS para type-check. Sem isto, o desktop reclamaria de código
// morto em tudo aqui.
#![cfg_attr(not(mobile), allow(dead_code))]

use async_trait::async_trait;
use eclipse_obd::{Elm327Source, Elm327Transport, ObdError};
use tauri_plugin_obd_bt::{BtDevice, ObdBtExt};

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
        tokio::task::spawn_blocking(move || {
            app.obd_bt().command(&cmd, timeout_ms).map_err(erro)
        })
        .await
        .map_err(|e| ObdError::Bus(format!("task de leitura falhou: {e}")))?
    }
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
    pareados.iter().find(|d| {
        let nome = normalizar(&d.name);
        PADROES_NOME.iter().any(|p| nome.contains(p))
    })
}

/// Garante permissão, escolhe e abre o adaptador; devolve um rótulo para o log.
///
/// Tudo bloqueante num `spawn_blocking` só: pedir permissão espera o usuário
/// responder o diálogo, e listar/conectar falam com o rádio.
async fn preparar(app: &tauri::AppHandle) -> Result<String, ObdError> {
    let app = app.clone();
    let alvo = std::env::var("ECLIPSE_OBD_DEVICE").ok();

    tokio::task::spawn_blocking(move || -> Result<String, ObdError> {
        let bt = app.obd_bt();

        bt.ensure_permissions().map_err(erro)?;

        let pareados = bt.list_bonded().map_err(erro)?;
        for d in &pareados {
            tracing::info!(nome = %d.name, mac = %d.address, "adaptador Bluetooth pareado");
        }

        let escolhido = escolher(&pareados, alvo.as_deref()).ok_or_else(|| {
            ObdError::Bus(
                "nenhum adaptador OBD pareado; pareie o ELM327 nas configurações do Android \
                 (ou defina ECLIPSE_OBD_DEVICE com o nome/MAC)"
                    .to_string(),
            )
        })?;

        bt.connect(&escolhido.address).map_err(erro)?;
        Ok(format!("{} ({})", escolhido.name, escolhido.address))
    })
    .await
    .map_err(|e| ObdError::Bus(format!("task de conexão falhou: {e}")))?
}

/// Conecta ao adaptador e faz o handshake ELM327, devolvendo a fonte pronta.
pub async fn conectar(app: &tauri::AppHandle) -> Result<Elm327Source<AndroidBtTransport>, ObdError> {
    let rotulo = preparar(app).await?;
    tracing::info!(adaptador = %rotulo, "conectado; iniciando handshake do ELM327");
    Elm327Source::conectar(AndroidBtTransport { app: app.clone() }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> BtDevice {
        BtDevice {
            name: name.to_string(),
            address: "AA:BB:CC:DD:EE:FF".to_string(),
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
