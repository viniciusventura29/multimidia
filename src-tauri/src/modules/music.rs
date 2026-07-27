//! Música.
//!
//! Faixas de mentira, mas o caminho de **ação** é real: os botões da UI viram
//! `ModuleCommand::Action`, o Rust muda o estado e republica. A Fase 5 troca
//! este miolo pelo `rspotify` sem mexer na tela.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use serde::Serialize;

pub const MUSIC: ModuleId = ModuleId::new("music");

const PLAYLIST: [(&str, &str); 3] = [
    ("Weightless", "Marconi Union"),
    ("Nightcall", "Kavinsky"),
    ("Bloom", "ODESZA"),
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NowPlaying {
    track: String,
    artist: String,
    is_playing: bool,
}

#[derive(Default)]
pub struct PlaceholderMusic;

#[async_trait]
impl Module for PlaceholderMusic {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        let mut indice = 0usize;
        let mut tocando = false;

        let publica = |ctx: &ModuleCtx, indice: usize, tocando: bool| {
            let (track, artist) = PLAYLIST[indice];
            ctx.ready(&NowPlaying {
                track: track.to_string(),
                artist: artist.to_string(),
                is_playing: tocando,
            });
        };

        publica(&ctx, indice, tocando);

        while let Some(comando) = ctx.next_command().await {
            match comando {
                // Trocar de perfil é trocar de conta: a sessão atual cai e a
                // reprodução volta ao início, parada. Na Fase 5 isto vira
                // reautenticar com o refresh token do novo perfil — o efeito
                // visível na tela é exatamente este.
                ModuleCommand::ProfileChanged(_) => {
                    indice = 0;
                    tocando = false;
                }
                ModuleCommand::Action { payload, .. } => {
                    match payload.get("acao").and_then(|v| v.as_str()) {
                        Some("toggle") => tocando = !tocando,
                        Some("next") => {
                            indice = (indice + 1) % PLAYLIST.len();
                            tocando = true;
                        }
                        Some("prev") => {
                            indice = (indice + PLAYLIST.len() - 1) % PLAYLIST.len();
                            tocando = true;
                        }
                        _ => continue,
                    }
                }
            }

            publica(&ctx, indice, tocando);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::{factory, ModuleId, StateEnvelope, Status, Supervisor};
    use serde_json::{json, Value};
    use std::time::Duration;
    use tokio::sync::broadcast::Receiver;

    async fn proximo_ready(rx: &mut Receiver<StateEnvelope>) -> Option<Value> {
        tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                let envelope = rx.recv().await.expect("barramento fechou");
                if envelope.module == MUSIC && envelope.status == Status::Ready {
                    return envelope.data.expect("ready sem dado");
                }
            }
        })
        .await
        .ok()
    }

    fn acao(alvo: ModuleId, nome: &str) -> ModuleCommand {
        ModuleCommand::Action {
            target: alvo,
            payload: json!({ "acao": nome }),
        }
    }

    #[tokio::test]
    async fn toggle_alterna_reproducao_e_next_troca_faixa() {
        let mut supervisor = Supervisor::new();
        let mut rx = supervisor.subscribe();
        supervisor.spawn(factory(MUSIC, PlaceholderMusic::default));

        let inicial = proximo_ready(&mut rx).await.expect("estado inicial");
        assert_eq!(inicial["isPlaying"], false);
        let primeira_faixa = inicial["track"].clone();

        supervisor.dispatch(acao(MUSIC, "toggle"));
        let tocando = proximo_ready(&mut rx).await.expect("resposta do toggle");
        assert_eq!(tocando["isPlaying"], true);

        supervisor.dispatch(acao(MUSIC, "next"));
        let seguinte = proximo_ready(&mut rx).await.expect("resposta do next");
        assert_ne!(seguinte["track"], primeira_faixa);
    }

    /// Trocar de perfil derruba a sessão: outra conta não herda o que estava tocando.
    #[tokio::test]
    async fn troca_de_perfil_reinicia_a_reproducao() {
        use eclipse_core::Profile;
        use std::sync::Arc;

        let mut supervisor = Supervisor::new();
        let mut rx = supervisor.subscribe();
        supervisor.spawn(factory(MUSIC, PlaceholderMusic::default));

        let inicial = proximo_ready(&mut rx).await.expect("estado inicial");
        let primeira_faixa = inicial["track"].clone();

        // Avança e dá play, para haver o que perder.
        supervisor.dispatch(acao(MUSIC, "next"));
        let avancado = proximo_ready(&mut rx).await.expect("resposta do next");
        assert_ne!(avancado["track"], primeira_faixa);
        assert_eq!(avancado["isPlaying"], true);

        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(Profile::new(
            "Convidado",
            "#f5a524",
        ))));

        let depois = proximo_ready(&mut rx).await.expect("resposta da troca");
        assert_eq!(depois["track"], primeira_faixa);
        assert_eq!(depois["isPlaying"], false);
    }

    /// O barramento de comandos é compartilhado, então cada módulo precisa
    /// ignorar o que não é endereçado a ele.
    #[tokio::test]
    async fn ignora_acao_endereçada_a_outro_modulo() {
        let mut supervisor = Supervisor::new();
        let mut rx = supervisor.subscribe();
        supervisor.spawn(factory(MUSIC, PlaceholderMusic::default));

        proximo_ready(&mut rx).await.expect("estado inicial");

        supervisor.dispatch(acao(ModuleId::new("obd"), "toggle"));

        assert!(
            proximo_ready(&mut rx).await.is_none(),
            "música republicou por causa de uma ação do obd"
        );
    }
}
