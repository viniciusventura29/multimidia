//! Música.
//!
//! O módulo não conhece o Spotify: ele fala com [`Conector`] e [`MusicSource`].
//! É isso que deixa a política de degradação testável sem rede e sem Client ID —
//! que é justamente a parte que eu consigo verificar.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_music::{
    DemoSource, EmAndamento, MusicError, MusicSource, MusicState, SpotifySource, TokenStore,
};
use tauri_plugin_obd_bt::ObdBtExt;
use uuid::Uuid;

pub const MUSIC: ModuleId = ModuleId::new("music");

/// De quanto em quanto tempo perguntar o que está tocando.
const INTERVALO: Duration = Duration::from_secs(3);
/// Degradado, tenta bem menos: sem device ativo ou sem token, insistir a cada
/// três segundos só gasta bateria e cota de API.
const INTERVALO_DEGRADADO: Duration = Duration::from_secs(20);
/// "Faça na próxima volta do laço", sem esperar o intervalo.
const AGORA: Duration = Duration::from_millis(1);
/// Depois de mandar um comando, espera a propagação antes de reler.
///
/// O `currently-playing` do Spotify é eventualmente consistente: relendo 1ms
/// depois de pausar ele ainda devolve o estado ANTIGO, o painel republicava
/// "tocando" e o toque parecia não ter funcionado — o usuário tocava de novo e
/// a música voltava. Meio segundo é o suficiente para ler a verdade.
const PROPAGACAO: Duration = Duration::from_millis(600);

/// A partir de quanto uma ida ao Spotify conta como lenta.
///
/// Todo toque no transporte é uma ida e volta à API do Spotify, e a central
/// costuma estar no hotspot do celular. Um segundo e meio é o ponto em que o
/// dedo já desistiu e a pessoa toca de novo — que é como "travado" começa. Não
/// dá para consertar o que não está medido, e até aqui o módulo só logava erro
/// de cofre de token: sobre a lentidão, nada.
const LENTO: Duration = Duration::from_millis(1_500);

/// Espera antes de tentar reconectar, crescendo a cada falha seguida.
///
/// 20s fixos puniam um piscar de rede tanto quanto uma queda real; começar em 1s
/// faz o painel voltar sozinho quase na hora no caso comum.
fn recuo(falhas: u32) -> Duration {
    match falhas {
        0 | 1 => Duration::from_secs(1),
        2 => Duration::from_secs(3),
        3 => Duration::from_secs(8),
        _ => INTERVALO_DEGRADADO,
    }
}

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
    /// Para falar com a sessão de mídia do aparelho. `None` = só Web API.
    ///
    /// Opcional porque os testes deste módulo sobem o conector sem app do
    /// Tauri, e porque no desktop não existe sessão local nenhuma.
    pub app: Option<tauri::AppHandle>,
}

#[async_trait]
impl Conector for SpotifyConector {
    async fn conectar(&self, perfil: Uuid) -> Result<Box<dyn MusicSource>, MusicError> {
        if self.demo {
            return Ok(Box::new(DemoSource::default()));
        }

        let client_id = self.client_id.as_deref().ok_or(MusicError::NotConfigured)?;

        // O nome deste aparelho é o que separa "a central" de "o celular dele"
        // na lista do Spotify Connect — ver `escolher_device`. Falhar aqui não
        // é fatal: sem o nome a escolha cai no WebView, que é o de hoje.
        let nomes_do_aparelho = match &self.app {
            Some(app) => {
                let app = app.clone();
                tokio::task::spawn_blocking(move || app.obd_bt().nome_do_aparelho())
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or_default()
            }
            None => Vec::new(),
        };

        let nuvem: Box<dyn MusicSource> = Box::new(
            SpotifySource::conectar(
                client_id,
                perfil,
                Arc::clone(&self.cofre),
                nomes_do_aparelho,
            )
            .await?,
        );

        // A sessão local na FRENTE da nuvem, não no lugar dela: o que é rápido
        // e local (o que toca, a capa, o transporte) passa a não usar rede, e a
        // biblioteca continua na Web API. Ver `musica_local`.
        //
        // Se não houver sessão local — Spotify fechado, conexão recusada,
        // desktop — o decorador devolve tudo à nuvem e nada piora.
        match &self.app {
            Some(app) => Ok(Box::new(crate::modules::musica_local::SessaoLocal::nova(
                app.clone(),
                nuvem,
            ))),
            None => Ok(nuvem),
        }
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
        // Um PRAZO, não uma duração: `select!` reconstrói o futuro a cada volta,
        // então `sleep(espera)` era reiniciado por qualquer comando que chegasse.
        // Degradado, cada toque do usuário zerava a contagem e a reconexão nunca
        // acontecia — era o que obrigava a fechar e reabrir o app.
        let mut proximo = tokio::time::Instant::now() + INTERVALO_DEGRADADO;
        let mut falhas: u32 = 0;
        let agenda = |d: Duration| tokio::time::Instant::now() + d;

        // A primeira vez é notícia, o resto é ruído: uma rede ruim faria a mesma
        // linha subir a cada 3 s por toda a viagem.
        let mut avisou_da_lentidao = false;

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
                        proximo = agenda(AGORA);
                    }

                    Some(ModuleCommand::Action { payload, .. }) => {
                        // Sem fonte, o toque do usuário é o melhor sinal de
                        // "voltei do background": reconecta já, em vez de engolir
                        // a ação em silêncio e esperar o próximo tick.
                        let Some(atual) = fonte.as_mut() else {
                            proximo = agenda(AGORA);
                            continue;
                        };
                        let acao = payload.get("acao").and_then(|v| v.as_str());
                        let texto = |chave| payload.get(chave).and_then(|v| v.as_str());

                        // Avisa a tela ANTES de falar com o Spotify. A resposta
                        // deles leva segundos; o dedo precisa de retorno em
                        // milissegundos. Sem isto a tela ficava idêntica durante
                        // toda a espera e o motorista tocava de novo, achando
                        // que não tinha pegado.
                        estado.carregando = match acao {
                            Some("abrir") => Some(EmAndamento::Abrindo),
                            Some("buscar") => Some(EmAndamento::Buscando),
                            Some("playlists") => Some(EmAndamento::Playlists),
                            Some("toggle" | "next" | "prev" | "tocar" | "seek") => {
                                Some(EmAndamento::Transporte)
                            }
                            _ => None,
                        };
                        if estado.carregando.is_some() {
                            ctx.ready(&estado);
                        }

                        // Duas famílias de ação: as de transporte (tocar algo,
                        // pular) mandam reler o now_playing; busca/playlists
                        // devolvem listas que entram no estado publicado.
                        let comecou = Instant::now();
                        let resultado: Result<(), MusicError> = match acao {
                            Some("toggle") => atual.toggle().await.map(|_| proximo = agenda(PROPAGACAO)),
                            Some("next") => atual.next().await.map(|_| proximo = agenda(PROPAGACAO)),
                            Some("prev") => atual.previous().await.map(|_| proximo = agenda(PROPAGACAO)),
                            Some("tocar") => {
                                // `uri` é a faixa; `contexto` é a playlist/álbum de
                                // onde ela veio. Tocar dentro do contexto monta a
                                // fila — é o que faz "próxima/anterior" andarem.
                                let faixa = texto("uri");
                                let contexto = texto("contexto");
                                if faixa.is_none() && contexto.is_none() {
                                    continue;
                                }
                                atual
                                    .tocar(faixa, contexto)
                                    .await
                                    .map(|_| proximo = agenda(PROPAGACAO))
                            }
                            Some("seek") => match payload.get("posicaoMs").and_then(|v| v.as_u64()) {
                                Some(ms) => atual
                                    .seek(ms as u32)
                                    .await
                                    .map(|_| proximo = agenda(PROPAGACAO)),
                                None => continue,
                            },
                            Some("buscar") => match atual.buscar(texto("termo").unwrap_or("")).await {
                                Ok(busca) => {
                                    estado.busca = busca;
                                    // Sai do que estiver aberto: o resultado da
                                    // busca é a tela nova.
                                    estado.contexto = None;
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
                            // Abrir é entrar na playlist/álbum para escolher a
                            // faixa — antes tocar num deles já saía tocando tudo.
                            Some("abrir") => match texto("uri") {
                                Some(uri) => match atual.abrir(uri).await {
                                    Ok(contexto) => {
                                        estado.contexto = Some(contexto);
                                        ctx.ready(&estado);
                                        Ok(())
                                    }
                                    Err(err) => Err(err),
                                },
                                None => continue,
                            },
                            Some("fechar") => {
                                estado.contexto = None;
                                ctx.ready(&estado);
                                Ok(())
                            }
                            Some("limpar_busca") => {
                                estado.busca = Default::default();
                                ctx.ready(&estado);
                                Ok(())
                            }
                            _ => {
                                estado.carregando = None;
                                continue;
                            }
                        };

                        // Terminou — deu certo ou não, a espera acabou.
                        estado.carregando = None;

                        let levou = comecou.elapsed();
                        if levou >= LENTO {
                            tracing::warn!(
                                acao = acao.unwrap_or("?"),
                                ms = levou.as_millis() as u64,
                                "o Spotify demorou a responder o toque"
                            );
                        } else {
                            tracing::debug!(
                                acao = acao.unwrap_or("?"),
                                ms = levou.as_millis() as u64,
                                "toque atendido"
                            );
                        }

                        if let Err(err) = resultado {
                            tracing::warn!(
                                acao = acao.unwrap_or("?"),
                                ms = levou.as_millis() as u64,
                                %err,
                                "o toque no Spotify falhou"
                            );
                            // Publica `ready` com o problema em vez de `degraded`:
                            // degradar apagava a tela inteira (e o App desmontava
                            // o player justo quando o erro era "sem dispositivo",
                            // desligando o próprio device). Aqui a busca e as
                            // playlists continuam na mão do usuário.
                            estado.problema = Some(err.problema());
                            ctx.ready(&estado);
                            fonte = None;
                            falhas += 1;
                            proximo = agenda(recuo(falhas));
                        }
                    }
                },

                _ = tokio::time::sleep_until(proximo) => {
                    let Some(perfil) = perfil else {
                        proximo = agenda(INTERVALO_DEGRADADO);
                        continue;
                    };

                    if fonte.is_none() {
                        match self.conector.conectar(perfil).await {
                            Ok(nova) => {
                                fonte = Some(nova);
                                estado = MusicState::default();
                            }
                            Err(err) => {
                                estado.problema = Some(err.problema());
                                ctx.ready(&estado);
                                falhas += 1;
                                proximo = agenda(recuo(falhas));
                                continue;
                            }
                        }
                    }

                    let leitura = Instant::now();
                    match fonte.as_mut().expect("acabou de conectar").now_playing().await {
                        // Conectado é sempre `ready`, mesmo sem nada tocando: o
                        // painel continua útil (busca e playlists funcionam), e
                        // "nada tocando" é estado normal, não erro. Antes isto
                        // era `degraded`, o que apagava a tela de busca.
                        Ok(tocando) => {
                            // A leitura periódica é o que alimenta a capa, o
                            // título e a barra de progresso. Se ela mesma demora
                            // mais que o intervalo, o painel não anda a cada 3 s:
                            // anda quando o Spotify deixa, e é assim que uma tela
                            // de música parece engasgada sem nada estar quebrado.
                            let levou = leitura.elapsed();
                            if levou >= LENTO && !avisou_da_lentidao {
                                avisou_da_lentidao = true;
                                tracing::warn!(
                                    ms = levou.as_millis() as u64,
                                    intervalo_ms = INTERVALO.as_millis() as u64,
                                    "ler o que está tocando demora mais que o esperado"
                                );
                            }

                            estado.now_playing = tocando;
                            estado.problema = None;
                            falhas = 0;
                            ctx.ready(&estado);
                            proximo = agenda(INTERVALO);
                        }
                        Err(err) => {
                            tracing::warn!(%err, "perdi o Spotify; vou reconectar");
                            estado.problema = Some(err.problema());
                            ctx.ready(&estado);
                            fonte = None;
                            falhas += 1;
                            proximo = agenda(recuo(falhas));
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
    use eclipse_music::{MusicState, NowPlaying, TipoProblema};
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

        // Publica `ready` com o problema tipado, não `degraded`: a tela precisa
        // continuar utilizável (busca/playlists) e saber QUE ação oferecer.
        let envelope = proximo(&mut rx, |e| {
            e.data.as_ref().is_some_and(|d| !d["problema"].is_null())
        })
        .await;

        let estado: MusicState = serde_json::from_value((*envelope.data.unwrap()).clone()).unwrap();
        let problema = estado.problema.expect("problema publicado");
        assert_eq!(problema.tipo, TipoProblema::PrecisaLogin);
        assert!(
            problema.detalhe.contains("Client ID"),
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
            e.data.as_ref().is_some_and(|d| !d["problema"].is_null())
        })
        .await;

        let estado: MusicState = serde_json::from_value((*envelope.data.unwrap()).clone()).unwrap();
        let problema = estado.problema.expect("problema publicado");
        // Token vencido é login, não erro passageiro — a tela oferece reconectar.
        assert_eq!(problema.tipo, TipoProblema::PrecisaLogin);
        assert!(problema.detalhe.contains("reconectar"));
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
        let estado: MusicState = serde_json::from_value((*parado.data.unwrap()).clone()).unwrap();
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
        let estado: MusicState = serde_json::from_value((*envelope.data.unwrap()).clone()).unwrap();
        assert!(estado.now_playing.is_none());
    }
}
