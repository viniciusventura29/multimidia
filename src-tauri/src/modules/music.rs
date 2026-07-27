//! Música.
//!
//! O módulo não conhece o Spotify: ele fala com [`Conector`] e [`MusicSource`].
//! É isso que deixa a política de degradação testável sem rede e sem Client ID —
//! que é justamente a parte que eu consigo verificar.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_music::{DemoSource, MusicError, MusicSource, SpotifySource, TokenStore};
use uuid::Uuid;

pub const MUSIC: ModuleId = ModuleId::new("music");

/// De quanto em quanto tempo perguntar o que está tocando.
const INTERVALO: Duration = Duration::from_secs(3);
/// Degradado, tenta bem menos: sem device ativo ou sem token, insistir a cada
/// três segundos só gasta bateria e cota de API.
const INTERVALO_DEGRADADO: Duration = Duration::from_secs(20);
/// "Faça na próxima volta do laço", sem esperar o intervalo.
const AGORA: Duration = Duration::from_millis(1);

/// Abre uma sessão de música para um perfil.
#[async_trait]
pub trait Conector: Send + Sync + 'static {
    async fn conectar(&self, perfil: Uuid) -> Result<Box<dyn MusicSource>, MusicError>;
}

pub struct SpotifyConector {
    pub client_id: Option<String>,
    pub cofre: Arc<Mutex<TokenStore>>,
    /// `ECLIPSE_MUSIC_DEMO=1` troca o Spotify por faixas de mentira, para
    /// trabalhar no layout sem Client ID.
    pub demo: bool,
}

#[async_trait]
impl Conector for SpotifyConector {
    async fn conectar(&self, perfil: Uuid) -> Result<Box<dyn MusicSource>, MusicError> {
        if self.demo {
            return Ok(Box::new(DemoSource::default()));
        }

        let client_id = self
            .client_id
            .as_deref()
            .ok_or(MusicError::NotConfigured)?;

        Ok(Box::new(
            SpotifySource::conectar(client_id, perfil, Arc::clone(&self.cofre)).await?,
        ))
    }
}

/// O player que já está tocando no aparelho, via sessão de mídia do Android.
///
/// Ignora o perfil de propósito: o Android guarda uma conta só por app, ao
/// contrário do Spotify por token que cada perfil tinha antes. É a troca feita
/// ao escolher "o Spotify de verdade" em vez de embarcar um cliente próprio.
pub struct AndroidConector {
    pub app: tauri::AppHandle,
}

#[async_trait]
impl Conector for AndroidConector {
    async fn conectar(&self, _perfil: Uuid) -> Result<Box<dyn MusicSource>, MusicError> {
        Ok(Box::new(crate::modules::android_media::AndroidMediaSource::new(
            self.app.clone(),
        )))
    }
}

pub struct MusicModule {
    conector: Arc<dyn Conector>,
}

impl MusicModule {
    pub fn new(conector: Arc<dyn Conector>) -> Self {
        Self { conector }
    }
}

#[async_trait]
impl Module for MusicModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        let mut perfil: Option<Uuid> = None;
        let mut fonte: Option<Box<dyn MusicSource>> = None;
        let mut espera = INTERVALO_DEGRADADO;

        ctx.degraded("aguardando o perfil");

        loop {
            tokio::select! {
                comando = ctx.next_command() => match comando {
                    None => return Ok(()),

                    // Trocar de perfil é trocar de conta: a sessão atual cai e o
                    // módulo reconecta com o refresh token do novo perfil.
                    Some(ModuleCommand::ProfileChanged(novo)) => {
                        perfil = Some(novo.id);
                        fonte = None;
                        ctx.loading();
                        espera = AGORA;
                    }

                    Some(ModuleCommand::Action { payload, .. }) => {
                        let Some(atual) = fonte.as_mut() else { continue };

                        let resultado = match payload.get("acao").and_then(|v| v.as_str()) {
                            Some("toggle") => atual.toggle().await,
                            Some("next") => atual.next().await,
                            Some("prev") => atual.previous().await,
                            _ => continue,
                        };

                        match resultado {
                            // Não pinta o efeito do toque: relê o estado real,
                            // porque outro aparelho pode ter mexido na fila.
                            Ok(()) => espera = AGORA,
                            Err(err) => {
                                ctx.degraded(err.to_string());
                                fonte = None;
                                espera = INTERVALO_DEGRADADO;
                            }
                        }
                    }
                },

                _ = tokio::time::sleep(espera) => {
                    let Some(perfil) = perfil else {
                        espera = INTERVALO_DEGRADADO;
                        continue;
                    };

                    if fonte.is_none() {
                        match self.conector.conectar(perfil).await {
                            Ok(nova) => fonte = Some(nova),
                            Err(err) => {
                                ctx.degraded(err.to_string());
                                espera = INTERVALO_DEGRADADO;
                                continue;
                            }
                        }
                    }

                    match fonte.as_mut().expect("acabou de conectar").now_playing().await {
                        Ok(Some(tocando)) => {
                            ctx.ready(&tocando);
                            espera = INTERVALO;
                        }
                        // Conectado, mas não há device tocando em lugar nenhum.
                        // A Web API comanda um device; ela não cria um.
                        Ok(None) => {
                            ctx.degraded("nenhum dispositivo Spotify ativo");
                            espera = INTERVALO;
                        }
                        Err(err) => {
                            ctx.degraded(err.to_string());
                            fonte = None;
                            espera = INTERVALO_DEGRADADO;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::{factory, Profile, StateEnvelope, Status, Supervisor};
    use eclipse_music::NowPlaying;
    use serde_json::json;
    use tokio::sync::broadcast::Receiver;

    /// Conector de teste: devolve o que mandarem, sem rede.
    struct FakeConector {
        resultado: Box<dyn Fn() -> Result<Box<dyn MusicSource>, MusicError> + Send + Sync>,
    }

    #[async_trait]
    impl Conector for FakeConector {
        async fn conectar(&self, _perfil: Uuid) -> Result<Box<dyn MusicSource>, MusicError> {
            (self.resultado)()
        }
    }

    fn conector_que_falha(erro: fn() -> MusicError) -> Arc<dyn Conector> {
        Arc::new(FakeConector {
            resultado: Box::new(move || Err(erro())),
        })
    }

    async fn proximo(
        rx: &mut Receiver<StateEnvelope>,
        aceita: impl Fn(&StateEnvelope) -> bool,
    ) -> StateEnvelope {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let envelope = rx.recv().await.expect("barramento fechou");
                if envelope.module == MUSIC && aceita(&envelope) {
                    return envelope;
                }
            }
        })
        .await
        .expect("estado não chegou")
    }

    fn subir(conector: Arc<dyn Conector>) -> (Supervisor, Receiver<StateEnvelope>) {
        let mut supervisor = Supervisor::new();
        let rx = supervisor.subscribe();
        supervisor.spawn(factory(MUSIC, move || {
            MusicModule::new(Arc::clone(&conector))
        }));
        (supervisor, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn sem_client_id_o_tile_explica_o_que_falta() {
        let (supervisor, mut rx) = subir(conector_que_falha(|| MusicError::NotConfigured));
        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(Profile::new(
            "Vinicius", "#3ddc97",
        ))));

        let envelope = proximo(&mut rx, |e| {
            e.status == Status::Degraded && e.reason.as_deref() != Some("aguardando o perfil")
        })
        .await;

        assert!(
            envelope.reason.unwrap().contains("Client ID"),
            "o motivo precisa dizer o que fazer, não só que falhou"
        );
    }

    /// Token vencido não é erro de rede: a UI tem que oferecer reconectar.
    #[tokio::test(start_paused = true)]
    async fn token_vencido_pede_reconexao() {
        let (supervisor, mut rx) = subir(conector_que_falha(|| MusicError::NeedsReauth));
        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(Profile::new(
            "Vinicius", "#3ddc97",
        ))));

        let envelope = proximo(&mut rx, |e| {
            e.status == Status::Degraded && e.reason.as_deref() != Some("aguardando o perfil")
        })
        .await;

        assert!(envelope.reason.unwrap().contains("reconectar"));
    }

    #[tokio::test(start_paused = true)]
    async fn conectado_publica_o_que_esta_tocando_e_reage_ao_toque() {
        let conector: Arc<dyn Conector> = Arc::new(FakeConector {
            resultado: Box::new(|| Ok(Box::new(DemoSource::default()))),
        });
        let (supervisor, mut rx) = subir(conector);

        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(Profile::new(
            "Vinicius", "#3ddc97",
        ))));

        let parado = proximo(&mut rx, |e| e.status == Status::Ready).await;
        let tocando: NowPlaying = serde_json::from_value(parado.data.unwrap()).unwrap();
        assert!(!tocando.is_playing);

        supervisor.dispatch(ModuleCommand::Action {
            target: MUSIC,
            payload: json!({ "acao": "toggle" }),
        });

        let depois = proximo(&mut rx, |e| {
            e.status == Status::Ready
                && e.data
                    .as_ref()
                    .and_then(|d| d.get("isPlaying"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
        })
        .await;

        assert!(depois.seq > parado.seq);
    }

    /// Sem device ativo o módulo fica degradado, mas segue conectado — insistir
    /// aqui é normal, o usuário pode abrir o Spotify no celular a qualquer momento.
    #[tokio::test(start_paused = true)]
    async fn sem_dispositivo_ativo_o_tile_diz_isso() {
        struct SemDevice;

        #[async_trait]
        impl MusicSource for SemDevice {
            async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
                Ok(None)
            }
            async fn toggle(&mut self) -> Result<(), MusicError> {
                Ok(())
            }
            async fn next(&mut self) -> Result<(), MusicError> {
                Ok(())
            }
            async fn previous(&mut self) -> Result<(), MusicError> {
                Ok(())
            }
        }

        let conector: Arc<dyn Conector> = Arc::new(FakeConector {
            resultado: Box::new(|| Ok(Box::new(SemDevice))),
        });
        let (supervisor, mut rx) = subir(conector);

        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(Profile::new(
            "Vinicius", "#3ddc97",
        ))));

        let envelope = proximo(&mut rx, |e| {
            e.reason.as_deref() == Some("nenhum dispositivo Spotify ativo")
        })
        .await;

        assert_eq!(envelope.status, Status::Degraded);
    }
}
