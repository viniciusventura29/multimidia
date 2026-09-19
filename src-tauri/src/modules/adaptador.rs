//! Qual adaptador OBD é o deste carro.
//!
//! É um módulo, e não um punhado de comandos Tauri, por um motivo prático: ele
//! precisa estar vivo **justamente quando o `obd` está caído** tentando reconectar
//! — que é o momento em que o dono abre esta tela. Como módulo ele tem estado
//! publicado, supervisor e reinício de graça, e a UI recebe a lista crescendo pelo
//! mesmo `module-state` de sempre.
//!
//! O que ele faz é curto: busca, pareia, grava um MAC em disco. Quem usa o que foi
//! gravado é o módulo `obd`, na próxima tentativa de conexão — que vem sozinha,
//! porque o supervisor já está reconectando com backoff.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use serde::{Deserialize, Serialize};
use tauri_plugin_obd_bt::{BtDevice, BtKind};

use crate::obd_bt::{adaptador_salvo, gravar_adaptador, parece_adaptador, AdaptadorSalvo, Radio};

pub const ADAPTADOR: ModuleId = ModuleId::new("adaptador");

/// De quanto em quanto tempo a lista de achados é relida do rádio.
///
/// Meio segundo é o ritmo de alguém olhando a tela esperando o nome aparecer. Mais
/// rápido não muda nada (o barramento de descoberta não é mais rápido que isso) e
/// gastaria IPC à toa; mais devagar faz a lista parecer travada.
const INTERVALO_BUSCA: Duration = Duration::from_millis(500);

/// Um aparelho visto na busca.
// `Deserialize` só no teste: é o que deixa os testes afirmarem sobre o mesmo
// JSON que a tela recebe, em vez de sobre a struct antes de serializar.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct Achado {
    pub nome: String,
    pub mac: String,
    pub tipo: BtKind,
    pub pareado: bool,
    pub rssi: Option<i32>,
    /// O nome parece de um ELM327? A tela usa para destacar, não para filtrar:
    /// clone nenhum é obrigado a se chamar de nada, e esconder seria pior.
    pub parece_obd: bool,
}

impl From<&BtDevice> for Achado {
    fn from(d: &BtDevice) -> Self {
        Self {
            nome: d.name.clone(),
            mac: d.address.clone(),
            tipo: d.kind,
            pareado: d.bonded,
            rssi: d.rssi,
            parece_obd: parece_adaptador(&d.name),
        }
    }
}

/// Em que pé está a escolha.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(
    tag = "fase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Fase {
    Ocioso,
    Buscando,
    Pareando { mac: String },
    Falhou { motivo: String },
}

/// O rádio do aparelho, como a tela precisa vê-lo.
///
/// "Não tem Bluetooth" e "está desligado" não são degradação do módulo — são a
/// resposta certa para a pergunta que o dono está fazendo, e ele precisa lê-la na
/// tela em vez de ver um quadro escuro.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct InfoRadio {
    pub existe: bool,
    pub ligado: bool,
    pub motivo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct Estado {
    pub salvo: Option<AdaptadorSalvo>,
    pub buscando: bool,
    pub encontrados: Vec<Achado>,
    pub fase: Fase,
    pub radio: InfoRadio,
}

/// O que o toque na tela manda fazer.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(
    tag = "acao",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Acao {
    Buscar,
    Parar,
    /// Este é o adaptador do carro: pareia se precisar, e grava.
    Escolher {
        mac: String,
    },
    /// Volta a adivinhar pelo nome, como era antes desta tela existir.
    Esquecer,
}

pub struct AdaptadorModule {
    radio: Arc<dyn Radio>,
    dir: PathBuf,
}

impl AdaptadorModule {
    pub fn new(radio: Arc<dyn Radio>, dir: PathBuf) -> Self {
        Self { radio, dir }
    }

    /// Roda no pool de bloqueantes: toda chamada ao rádio espera o Android.
    async fn no_radio<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&dyn Radio) -> Result<T, String> + Send + 'static,
    {
        let radio = Arc::clone(&self.radio);
        tokio::task::spawn_blocking(move || f(radio.as_ref()))
            .await
            .unwrap_or_else(|e| Err(format!("a task do rádio falhou: {e}")))
    }

    async fn info(&self) -> InfoRadio {
        match self.no_radio(|r| r.info()).await {
            Ok(i) => {
                // Em `info` e não `debug` porque é a primeira pergunta de toda
                // depuração de Bluetooth — em que Android este carro está? — e
                // como o diário sobe o `info` em volta de cada erro, a resposta
                // chega junto com o problema em vez de faltar justo nele.
                tracing::info!(
                    sdk = i.sdk_int,
                    existe = i.existe,
                    ligado = i.ligado,
                    "rádio do aparelho"
                );
                InfoRadio {
                    existe: i.existe,
                    ligado: i.ligado,
                    motivo: None,
                }
            }
            Err(motivo) => InfoRadio {
                existe: false,
                ligado: false,
                motivo: Some(motivo),
            },
        }
    }

    /// Relê a lista do rádio. A busca pode ter acabado sozinha (ela tem teto).
    async fn colher(&self, estado: &mut Estado) {
        match self.no_radio(|r| r.achados()).await {
            Ok((achados, ainda)) => {
                estado.encontrados = achados.iter().map(Achado::from).collect();
                if !ainda {
                    estado.buscando = false;
                    if estado.fase == Fase::Buscando {
                        estado.fase = Fase::Ocioso;
                    }
                }
            }
            Err(motivo) => {
                estado.buscando = false;
                estado.fase = Fase::Falhou { motivo };
            }
        }
    }

    async fn aplicar(&self, acao: Acao, estado: &mut Estado, ctx: &ModuleCtx) {
        match acao {
            Acao::Buscar => {
                estado.radio = self.info().await;
                let r = self
                    .no_radio(|r| r.permissoes().and_then(|()| r.buscar()))
                    .await;
                match r {
                    Ok(()) => {
                        estado.buscando = true;
                        estado.encontrados.clear();
                        estado.fase = Fase::Buscando;
                    }
                    Err(motivo) => {
                        estado.buscando = false;
                        estado.fase = Fase::Falhou { motivo };
                    }
                }
            }

            Acao::Parar => {
                let _ = self.no_radio(|r| r.parar_busca()).await;
                estado.buscando = false;
                estado.fase = Fase::Ocioso;
            }

            Acao::Escolher { mac } => self.escolher(mac, estado, ctx).await,

            Acao::Esquecer => {
                if let Err(err) = gravar_adaptador(&self.dir, None) {
                    tracing::error!(%err, "não consegui apagar o adaptador salvo");
                }
                estado.salvo = None;
                estado.fase = Fase::Ocioso;
            }
        }
    }

    async fn escolher(&self, mac: String, estado: &mut Estado, ctx: &ModuleCtx) {
        let Some(alvo) = estado.encontrados.iter().find(|a| a.mac == mac).cloned() else {
            estado.fase = Fase::Falhou {
                motivo: "esse aparelho sumiu da lista; busque de novo".to_string(),
            };
            return;
        };

        // BLE não pareia — conecta. Pedir vínculo a um adaptador Bluetooth 4.0 é o
        // erro que faz o dono achar que o aparelho está quebrado.
        if alvo.tipo == BtKind::Spp && !alvo.pareado {
            // Publicado ANTES de parear: o vínculo pode levar um minuto (são
            // quatro PINs tentados em sequência), e sem isto a tela ficaria parada
            // sem dizer o que está acontecendo.
            estado.fase = Fase::Pareando { mac: mac.clone() };
            ctx.ready(estado);

            let para_parear = mac.clone();
            if let Err(motivo) = self.no_radio(move |r| r.parear(&para_parear)).await {
                estado.fase = Fase::Falhou {
                    motivo: format!("não pareou: {motivo}"),
                };
                return;
            }
        }

        let salvo = AdaptadorSalvo {
            mac: alvo.mac.clone(),
            nome: alvo.nome.clone(),
            tipo: alvo.tipo,
        };
        if let Err(err) = gravar_adaptador(&self.dir, Some(salvo.clone())) {
            estado.fase = Fase::Falhou {
                motivo: format!("não consegui gravar a escolha: {err}"),
            };
            return;
        }

        // Parar a busca faz parte de escolher, e não é higiene: enquanto o rádio
        // está varrendo, o plugin recusa `connect` — o carro ficaria escolhido e
        // sem conectar, que é o pior dos dois mundos.
        let _ = self.no_radio(|r| r.parar_busca()).await;
        estado.buscando = false;
        estado.salvo = Some(salvo);
        estado.fase = Fase::Ocioso;
        tracing::info!(mac = %mac, "adaptador escolhido; o módulo obd pega na próxima tentativa");
    }
}

#[async_trait]
impl Module for AdaptadorModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        let mut estado = Estado {
            salvo: adaptador_salvo(&self.dir),
            buscando: false,
            encontrados: Vec::new(),
            fase: Fase::Ocioso,
            radio: self.info().await,
        };

        // Quem já está pareado aparece antes de qualquer busca: quase sempre o
        // adaptador certo já está ali, e obrigar a varrer para vê-lo seria mentir.
        if let Ok(pareados) = self.no_radio(|r| r.pareados()).await {
            estado.encontrados = pareados.iter().map(Achado::from).collect();
        }
        ctx.ready(&estado);

        loop {
            if estado.buscando {
                tokio::time::sleep(INTERVALO_BUSCA).await;
                self.colher(&mut estado).await;
                ctx.ready(&estado);

                // Sem bloquear: buscando, a tela precisa continuar recebendo a
                // lista mesmo que ninguém toque em nada.
                while let Some(comando) = ctx.try_next_command() {
                    self.atender(comando, &mut estado, &ctx).await;
                }
            } else {
                // Parado, o módulo dorme. Não há nada para fazer sem um toque.
                let Some(comando) = ctx.next_command().await else {
                    return Ok(());
                };
                self.atender(comando, &mut estado, &ctx).await;
            }
        }
    }
}

impl AdaptadorModule {
    /// Trata uma ordem e republica o estado.
    async fn atender(&self, comando: ModuleCommand, estado: &mut Estado, ctx: &ModuleCtx) {
        let ModuleCommand::Action { payload, .. } = comando else {
            // Trocar de motorista não troca o adaptador do carro.
            return;
        };

        match serde_json::from_value::<Acao>(payload.clone()) {
            Ok(acao) => {
                self.aplicar(acao, estado, ctx).await;
                ctx.ready(estado);
            }
            Err(err) => tracing::warn!(%err, %payload, "ação desconhecida para o adaptador"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::{factory, StateEnvelope, Supervisor};
    use serde_json::json;
    use std::sync::Mutex;
    use tauri_plugin_obd_bt::BtInfo;
    use tokio::sync::broadcast::Receiver;

    /// O MAC que nunca pareia — o clone que só aceita um PIN que ninguém sabe.
    const TEIMOSO: &str = "11:22:33:44:55:66";

    /// Um rádio de mentira: a busca revela os aparelhos de uma vez, e um deles
    /// recusa o vínculo.
    #[derive(Default)]
    struct RadioFalso {
        buscando: Mutex<bool>,
        pareados: Mutex<Vec<String>>,
    }

    fn dev(nome: &str, mac: &str, kind: BtKind, bonded: bool) -> BtDevice {
        BtDevice {
            name: nome.to_string(),
            address: mac.to_string(),
            kind,
            bonded,
            rssi: Some(-50),
        }
    }

    impl Radio for RadioFalso {
        fn info(&self) -> Result<BtInfo, String> {
            Ok(BtInfo {
                sdk_int: 30,
                existe: true,
                ligado: true,
            })
        }
        fn permissoes(&self) -> Result<(), String> {
            Ok(())
        }
        fn pareados(&self) -> Result<Vec<BtDevice>, String> {
            let vinculados = self.pareados.lock().unwrap();
            Ok(self
                .visiveis()
                .into_iter()
                .filter(|d| vinculados.contains(&d.address))
                .collect())
        }
        fn buscar(&self) -> Result<(), String> {
            *self.buscando.lock().unwrap() = true;
            Ok(())
        }
        fn achados(&self) -> Result<(Vec<BtDevice>, bool), String> {
            let buscando = *self.buscando.lock().unwrap();
            if !buscando {
                return Ok((Vec::new(), false));
            }
            let vinculados = self.pareados.lock().unwrap();
            let achados = self
                .visiveis()
                .into_iter()
                .map(|mut d| {
                    d.bonded = vinculados.contains(&d.address);
                    d
                })
                .collect();
            Ok((achados, true))
        }
        fn parar_busca(&self) -> Result<(), String> {
            *self.buscando.lock().unwrap() = false;
            Ok(())
        }
        fn parear(&self, mac: &str) -> Result<(), String> {
            if mac == TEIMOSO {
                return Err("não consegui parear".to_string());
            }
            self.pareados.lock().unwrap().push(mac.to_string());
            Ok(())
        }
    }

    impl RadioFalso {
        fn visiveis(&self) -> Vec<BtDevice> {
            vec![
                dev("Galaxy Buds", TEIMOSO, BtKind::Spp, false),
                dev("V-LINK", "AA:BB:CC:DD:EE:FF", BtKind::Spp, false),
                dev("iCar Pro BLE", "A0:E6:F8:11:22:33", BtKind::Ble, false),
            ]
        }
    }

    fn temp(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eclipse-adaptador-{nome}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar o diretório de teste");
        dir
    }

    fn subir(dir: PathBuf) -> (Supervisor, Receiver<StateEnvelope>) {
        let radio: Arc<dyn Radio> = Arc::new(RadioFalso::default());
        let mut supervisor = Supervisor::new();
        let rx = supervisor.subscribe();
        supervisor.spawn(factory(ADAPTADOR, move || {
            AdaptadorModule::new(Arc::clone(&radio), dir.clone())
        }));
        (supervisor, rx)
    }

    async fn proximo(rx: &mut Receiver<StateEnvelope>, aceita: impl Fn(&Estado) -> bool) -> Estado {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let envelope = rx.recv().await.expect("barramento fechou");
                if envelope.module != ADAPTADOR {
                    continue;
                }
                let Some(data) = envelope.data.as_ref() else {
                    continue;
                };
                let estado: Estado =
                    serde_json::from_value(serde_json::to_value(data.as_ref().clone()).unwrap())
                        .expect("o estado do adaptador tem que desserializar");
                if aceita(&estado) {
                    return estado;
                }
            }
        })
        .await
        .expect("o estado esperado não chegou")
    }

    /// Espera o módulo subir antes de mandar a primeira ordem.
    ///
    /// O barramento de comandos é um `broadcast`: ele não guarda o que passou. Uma
    /// ação despachada antes de o módulo assinar simplesmente não existe para ele —
    /// no app isso nunca acontece (o toque vem depois da tela pintar), mas no teste
    /// a corrida é garantida.
    async fn esperar_subir(rx: &mut Receiver<StateEnvelope>) -> Estado {
        proximo(rx, |_| true).await
    }

    fn acao(payload: serde_json::Value) -> ModuleCommand {
        ModuleCommand::Action {
            target: ADAPTADOR,
            payload,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn escolher_um_classico_pareia_e_grava() {
        let dir = temp("classico");
        let (supervisor, mut rx) = subir(dir.clone());
        esperar_subir(&mut rx).await;

        supervisor.dispatch(acao(json!({ "acao": "buscar" })));
        let buscando = proximo(&mut rx, |e| e.buscando && !e.encontrados.is_empty()).await;
        assert!(
            buscando
                .encontrados
                .iter()
                .any(|a| a.nome == "V-LINK" && a.parece_obd),
            "o V-LINK tem que aparecer marcado como candidato: {:?}",
            buscando.encontrados
        );
        assert!(
            buscando
                .encontrados
                .iter()
                .any(|a| a.nome == "Galaxy Buds" && !a.parece_obd),
            "o fone continua na lista, só não destacado — clone não é obrigado a ter nome"
        );

        supervisor.dispatch(acao(
            json!({ "acao": "escolher", "mac": "AA:BB:CC:DD:EE:FF" }),
        ));
        let pronto = proximo(&mut rx, |e| e.salvo.is_some()).await;

        assert_eq!(pronto.salvo.as_ref().unwrap().mac, "AA:BB:CC:DD:EE:FF");
        assert_eq!(pronto.salvo.as_ref().unwrap().tipo, BtKind::Spp);
        // Escolher para a busca: com o rádio varrendo, o plugin recusa `connect`,
        // e o carro ficaria escolhido e sem conectar.
        assert!(!pronto.buscando, "a busca tem que parar ao escolher");

        // E sobrevive ao reinício do módulo — que é o ponto do arquivo.
        assert_eq!(
            adaptador_salvo(&dir).unwrap().mac,
            "AA:BB:CC:DD:EE:FF",
            "a escolha tem que estar em disco"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn escolher_um_ble_nao_tenta_parear() {
        let dir = temp("ble");
        let (supervisor, mut rx) = subir(dir.clone());
        esperar_subir(&mut rx).await;

        supervisor.dispatch(acao(json!({ "acao": "buscar" })));
        proximo(&mut rx, |e| !e.encontrados.is_empty()).await;

        supervisor.dispatch(acao(
            json!({ "acao": "escolher", "mac": "A0:E6:F8:11:22:33" }),
        ));
        let pronto = proximo(&mut rx, |e| e.salvo.is_some()).await;

        // BLE não pareia — conecta. Pedir vínculo a um "Bluetooth 4.0" é o erro
        // que faz o dono achar que o adaptador está quebrado.
        assert_eq!(pronto.salvo.as_ref().unwrap().tipo, BtKind::Ble);
        assert!(!pronto.salvo.as_ref().unwrap().mac.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pareamento_que_falha_nao_grava_nada() {
        let dir = temp("teimoso");
        let (supervisor, mut rx) = subir(dir.clone());
        esperar_subir(&mut rx).await;

        supervisor.dispatch(acao(json!({ "acao": "buscar" })));
        proximo(&mut rx, |e| !e.encontrados.is_empty()).await;

        supervisor.dispatch(acao(json!({ "acao": "escolher", "mac": TEIMOSO })));
        let estado = proximo(&mut rx, |e| matches!(e.fase, Fase::Falhou { .. })).await;

        let Fase::Falhou { motivo } = &estado.fase else {
            panic!("tinha que ter falhado");
        };
        assert!(motivo.contains("não pareou"), "motivo legível: {motivo}");
        assert!(estado.salvo.is_none(), "não pode gravar o que não pareou");
        assert!(adaptador_salvo(&dir).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn esquecer_apaga_e_volta_a_adivinhar() {
        let dir = temp("esquecer");
        gravar_adaptador(
            &dir,
            Some(AdaptadorSalvo {
                mac: "AA:BB:CC:DD:EE:FF".to_string(),
                nome: "V-LINK".to_string(),
                tipo: BtKind::Spp,
            }),
        )
        .unwrap();

        let (supervisor, mut rx) = subir(dir.clone());
        proximo(&mut rx, |e| e.salvo.is_some()).await;

        supervisor.dispatch(acao(json!({ "acao": "esquecer" })));
        proximo(&mut rx, |e| e.salvo.is_none()).await;

        assert!(adaptador_salvo(&dir).is_none(), "o arquivo tem que sumir");
    }
}
