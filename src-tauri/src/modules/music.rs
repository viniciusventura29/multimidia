//! Música.
//!
//! O módulo não conhece o Spotify: ele fala com [`Conector`] e [`MusicSource`].
//! É isso que deixa a política de degradação testável sem rede e sem Client ID —
//! que é justamente a parte que eu consigo verificar.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_music::{DemoSource, MusicError, MusicSource, MusicState, SpotifySource, TokenStore};
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
        // Acumula o que toca + resultados de busca + playlists. Persiste entre
        // os polls (o now_playing muda a cada 3s, mas busca/playlists ficam).
        let mut estado = MusicState::default();
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
                        estado = MusicState::default();
                        ctx.loading();
                        espera = AGORA;
                    }

                    Some(ModuleCommand::Action { payload, .. }) => {
                        let Some(atual) = fonte.as_mut() else { continue };
                        let acao = payload.get("acao").and_then(|v| v.as_str());
                        let texto = |chave| payload.get(chave).and_then(|v| v.as_str());

                        // Duas famílias de ação: as de transporte (tocar algo,
                        // pular) mandam reler o now_playing; busca/playlists
                        // devolvem listas que entram no estado publicado.
                        let resultado: Result<(), MusicError> = match acao {
                            Some("toggle") => atual.toggle().await.map(|_| espera = AGORA),
                            Some("next") => atual.next().await.map(|_| espera = AGORA),
                            Some("prev") => atual.previous().await.map(|_| espera = AGORA),
                            Some("tocar") => match texto("uri") {
                                Some(uri) => atual.tocar(uri).await.map(|_| espera = AGORA),
                                None => continue,
                            },
                            Some("tocar_playlist") => match texto("uri") {
                                Some(uri) => atual.tocar_playlist(uri).await.map(|_| espera = AGORA),
                                None => continue,
                            },
                            Some("buscar") => match atual.buscar(texto("termo").unwrap_or("")).await {
                                Ok(faixas) => {
                                    estado.resultados = faixas;
                                    ctx.ready(&estado);
                                    Ok(())
                                }
                                Err(err) => Err(err),
                            },
                            Some("playlists") => match atual.playlists().await {
                                Ok(pls) => {
                                    estado.playlists = pls;
                                    ctx.ready(&estado);
                                    Ok(())
                                }
                                Err(err) => Err(err),
                            },
                            _ => continue,
                        };

                        if let Err(err) = resultado {
                            ctx.degraded(err.to_string());
                            fonte = None;
                            espera = INTERVALO_DEGRADADO;
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
                            Ok(nova) => {
                                fonte = Some(nova);
                                estado = MusicState::default();
                            }
                            Err(err) => {
                                ctx.degraded(err.to_string());
                                espera = INTERVALO_DEGRADADO;
                                continue;
                            }
                        }
                    }

                    match fonte.as_mut().expect("acabou de conectar").now_playing().await {
                        // Conectado é sempre `ready`, mesmo sem nada tocando: o
                        // painel continua útil (busca e playlists funcionam), e
                        // "nada tocando" é estado normal, não erro. Antes isto
                        // era `degraded`, o que apagava a tela de busca.
                        Ok(tocando) => {
                            estado.now_playing = tocando;
                            ctx.ready(&estado);
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
    use eclipse_music::{MusicState, NowPlaying};
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
        let estado: MusicState = serde_json::from_value(parado.data.unwrap()).unwrap();
        assert!(!estado.now_playing.unwrap().is_playing);

        supervisor.dispatch(ModuleCommand::Action {
            target: MUSIC,
            payload: json!({ "acao": "toggle" }),
        });

        let depois = proximo(&mut rx, |e| {
            e.status == Status::Ready
                && e.data
                    .as_ref()
                    .and_then(|d| d.get("nowPlaying"))
                    .and_then(|n| n.get("isPlaying"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
        })
        .await;

        assert!(depois.seq > parado.seq);
    }

    /// Nada tocando NÃO é erro: conectado, o módulo publica `ready` com
    /// `nowPlaying` vazio — a tela de busca/playlists continua útil. (Antes isto
    /// era `degraded`, o que apagava a busca a cada 3s sem nada tocando.)
    #[tokio::test(start_paused = true)]
    async fn nada_tocando_segue_ready() {
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

        let envelope = proximo(&mut rx, |e| e.status == Status::Ready).await;
        let estado: MusicState = serde_json::from_value(envelope.data.unwrap()).unwrap();
        assert!(estado.now_playing.is_none());
    }
}
