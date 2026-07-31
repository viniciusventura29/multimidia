//! OBD-II.
//!
//! Continua sendo fiação: a cadência, o protocolo e a conta de consumo moram no
//! `eclipse-obd`; o socket Bluetooth mora no `crate::obd_bt`. Aqui se conecta ao
//! adaptador, se roda a varredura publicando no barramento, se atendem os toques do
//! usuário e se grava em disco o que não pode morrer quando a ignição desligar.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_obd::{Acao, Arquivo, EstadoTanque, Painel, Poller, Veiculo};

pub const OBD: ModuleId = ModuleId::new("obd");

/// Nome dos arquivos no diretório de dados do app.
const VEICULO_JSON: &str = "veiculo.json";
const TANQUE_JSON: &str = "tanque.json";

/// Intervalo mínimo entre duas gravações do estado do tanque.
///
/// O estado muda a cada leitura, mas gravar a cada leitura seria escrever no flash da
/// head unit ~3 vezes por segundo para sempre. Um minuto de perda máxima num corte de
/// energia é ~0,1 L de gasolina — barato, comparado com a memória do aparelho.
const INTERVALO_GRAVACAO: Duration = Duration::from_secs(60);

/// Precisa do `AppHandle` para alcançar o plugin de Bluetooth. O supervisor
/// reconstrói o módulo a cada reconexão, então guardamos só o handle (que clona) e o
/// diretório de dados — o estado do tanque é relido do disco a cada tentativa.
pub struct ObdModule {
    app: tauri::AppHandle,
    dir: PathBuf,
}

impl ObdModule {
    pub fn new(app: tauri::AppHandle, dir: PathBuf) -> Self {
        Self { app, dir }
    }
}

#[async_trait]
impl Module for ObdModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        // O que é do carro, e não de quem dirige: sobrevive a trocar de perfil e a
        // reiniciar o módulo.
        let mut veiculo: Arquivo<Veiculo> = Arquivo::load(self.dir.join(VEICULO_JSON));
        let mut tanque: Arquivo<EstadoTanque> = Arquivo::load(self.dir.join(TANQUE_JSON));

        // Bluetooth clássico (SPP) só existe no Android. No desktop o carro é de
        // mentira, com a mesma lentidão de barramento — é o único jeito de mexer na
        // tela do carro sem estar dirigindo.
        #[cfg(not(mobile))]
        let (poller, protocolo) = {
            let _ = &self.app;
            tracing::info!("carro simulado: telemetria real só no Android (Bluetooth)");
            (
                Poller::new(eclipse_obd::SimulatedSource::default()),
                Some("simulado".to_string()),
            )
        };

        // Conecta e faz o handshake. Se falhar (adaptador não pareado, carro
        // desligado, permissão negada), o erro sobe: o supervisor degrada os
        // mostradores juntos e reconecta com backoff — o que se quer quando o
        // adaptador solta do conector no meio da estrada.
        #[cfg(mobile)]
        let (poller, protocolo) = {
            let source = crate::obd_bt::conectar(&self.app).await?;

            // O relatório que só o carro pode dar. É por ele que se descobre se o
            // consumo vai ser medido ou estimado, e é a primeira coisa a olhar com
            // `adb logcat -s EclipseObdBt` na primeira ignição.
            let capacidades = source.capacidades();
            let protocolo = source.protocolo().map(str::to_string);
            tracing::info!(
                protocolo = protocolo.as_deref().unwrap_or("?"),
                descoberto = capacidades.descoberto(),
                pids = ?capacidades.lista(),
                "capacidades do carro"
            );

            (Poller::com_capacidades(source, capacidades), protocolo)
        };

        tracing::info!(
            rapidos = ?poller.plano().rapidos(),
            lentos = ?poller.plano().lentos(),
            "varredura montada"
        );

        let mut painel = Painel::novo(poller, veiculo.dados, tanque.dados, protocolo);
        let mut ultima_gravacao = Instant::now();

        loop {
            painel.step(Instant::now()).await?;
            ctx.ready(&painel.telemetria());

            // Drena as ações **entre** leituras, e não num `select!`: a leitura de um
            // PID roda num `spawn_blocking` que espera o `>` do ELM327, e abandoná-la
            // no meio deixaria a resposta no buffer, desalinhando todas as leituras
            // seguintes. Um PID de latência (~300 ms) é preço barato por não
            // embaralhar o barramento.
            while let Some(comando) = ctx.try_next_command() {
                let ModuleCommand::Action { payload, .. } = comando else {
                    // Trocar de motorista não muda o tanque do carro.
                    continue;
                };

                match serde_json::from_value::<Acao>(payload.clone()) {
                    Ok(acao) => {
                        painel.aplicar(acao);
                        if acao.muda_o_veiculo() {
                            veiculo.dados = painel.veiculo();
                            if let Err(err) = veiculo.salvar() {
                                tracing::error!(%err, "não consegui gravar o veículo");
                            }
                        }
                        // Toque do usuário grava na hora: se a ignição desligar agora,
                        // "enchi o tanque" não pode se perder.
                        tanque.dados = painel.estado();
                        if let Err(err) = tanque.salvar() {
                            tracing::error!(%err, "não consegui gravar o tanque");
                        }
                        ultima_gravacao = Instant::now();
                        ctx.ready(&painel.telemetria());
                    }
                    Err(err) => {
                        tracing::warn!(%err, %payload, "ação desconhecida para o obd");
                    }
                }
            }

            if ultima_gravacao.elapsed() >= INTERVALO_GRAVACAO {
                ultima_gravacao = Instant::now();
                let estado = painel.estado();
                if estado != tanque.dados {
                    tanque.dados = estado;
                    if let Err(err) = tanque.salvar() {
                        tracing::error!(%err, "não consegui gravar o tanque");
                    }
                }
            }
        }
    }
}
