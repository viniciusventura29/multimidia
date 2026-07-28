//! A ponte com o Spotify.
//!
//! ⚠️ **Este arquivo nunca foi exercitado contra a API real.** Ele precisa de um
//! Client ID de um app registrado no Spotify for Developers e de uma conta
//! Premium — sem isso não há como rodar o fluxo nem conferir o formato das
//! respostas. O que está coberto por teste é o [`crate::tokens`], que é onde
//! mora o risco de perder a sessão. O mapeamento de erro aqui é a melhor leitura
//! da documentação e deve ser apertado assim que der para rodar de verdade.
//!
//! Limites que valem lembrar:
//!
//! - A Web API **comanda** um device já ativo; ela não cria um. Sem nada tocando
//!   em lugar nenhum, os controles não têm o que controlar.
//! - Controle de playback exige Premium.
//! - Desde fev/2026 um app em Development Mode aceita no máximo 5 usuários
//!   autorizados — ou seja, no máximo 5 perfis.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{AdditionalType, PlayableItem};
use rspotify::{
    scopes, AuthCodePkceSpotify, ClientError, Config, Credentials, OAuth, Token, TokenCallback,
};
use uuid::Uuid;

use crate::source::{MusicError, MusicSource, NowPlaying};
use crate::tokens::TokenStore;

/// O Spotify exige a URI de redirect cadastrada no painel do app.
/// Esta precisa estar lá exatamente igual.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

pub fn escopos() -> HashSet<String> {
    scopes!(
        "user-read-playback-state",
        "user-modify-playback-state",
        "user-read-currently-playing",
        // Para listar e abrir as playlists do usuário dentro do Eclipse.
        "playlist-read-private",
        "playlist-read-collaborative"
    )
}

fn traduzir(err: ClientError) -> MusicError {
    use rspotify::http::HttpError;

    match err {
        ClientError::InvalidToken => MusicError::NeedsReauth,
        ClientError::Http(inner) => match *inner {
            HttpError::StatusCode(resposta) => match resposta.status().as_u16() {
                // Token recusado: não adianta repetir, só reconectando.
                400 | 401 => MusicError::NeedsReauth,
                // O Spotify devolve 403 quando a conta não é Premium.
                403 => MusicError::PremiumRequired,
                // E 404 quando não há device ativo para receber o comando.
                404 => MusicError::NoActiveDevice,
                outro => MusicError::Network(format!("HTTP {outro}")),
            },
            outro => MusicError::Network(outro.to_string()),
        },
        outro => MusicError::Network(outro.to_string()),
    }
}

pub struct SpotifySource {
    client: AuthCodePkceSpotify,
}

impl SpotifySource {
    /// Monta um cliente já autenticado a partir do refresh token guardado do perfil.
    pub async fn conectar(
        client_id: &str,
        perfil: Uuid,
        cofre: Arc<Mutex<TokenStore>>,
    ) -> Result<Self, MusicError> {
        let guardado = {
            let cofre = cofre.lock().unwrap_or_else(|e| e.into_inner());
            cofre.get(perfil).cloned().ok_or(MusicError::NotConnected)?
        };

        // Nem tenta a rede se o prazo de 6 meses já passou: a resposta seria um
        // invalid_grant, e dizer "reconecte" na hora é melhor que esperar falhar.
        if guardado.venceu(Utc::now()) {
            return Err(MusicError::NeedsReauth);
        }

        let cofre_callback = Arc::clone(&cofre);
        let config = Config {
            // O cache embutido do rspotify é um arquivo único; não serve para
            // vários perfis. Quem persiste aqui é o nosso cofre, pelo callback.
            token_cached: false,
            token_refreshing: true,
            token_callback_fn: Arc::new(Some(TokenCallback(Box::new(move |token: Token| {
                let mut cofre = cofre_callback.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(err) = cofre.renovou(perfil, token.refresh_token.as_deref()) {
                    tracing::error!(%err, "não consegui persistir a rotação do refresh token");
                }
                Ok(())
            })))),
            ..Default::default()
        };

        let client = AuthCodePkceSpotify::with_config(
            Credentials::new_pkce(client_id),
            OAuth {
                redirect_uri: REDIRECT_URI.to_string(),
                scopes: escopos(),
                ..Default::default()
            },
            config,
        );

        // Semeia um access token já vencido com o refresh token guardado: a
        // primeira chamada dispara a renovação, que por sua vez dispara o
        // callback e persiste a eventual rotação.
        *client.token.lock().await.unwrap() = Some(Token {
            access_token: String::new(),
            expires_in: Duration::seconds(0),
            expires_at: Some(Utc::now() - Duration::seconds(1)),
            refresh_token: Some(guardado.refresh_token),
            scopes: escopos(),
        });

        client.auto_reauth().await.map_err(traduzir)?;

        Ok(Self { client })
    }

    async fn tocando_agora(&self) -> Result<Option<NowPlaying>, MusicError> {
        let contexto = self
            .client
            .current_playing(None, None::<&[AdditionalType]>)
            .await
            .map_err(traduzir)?;

        let Some(contexto) = contexto else {
            return Ok(None);
        };

        let Some(PlayableItem::Track(faixa)) = contexto.item else {
            // Podcast ou nada: o painel não trata episódio ainda.
            return Ok(None);
        };

        Ok(Some(NowPlaying {
            track: faixa.name,
            artist: faixa
                .artists
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
                .join(", "),
            is_playing: contexto.is_playing,
            album_art: faixa.album.images.into_iter().next().map(|i| i.url),
            progress_ms: contexto.progress.map(|p| p.num_milliseconds() as u32),
            duration_ms: Some(faixa.duration.num_milliseconds() as u32),
        }))
    }
}

#[async_trait]
impl MusicSource for SpotifySource {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
        self.tocando_agora().await
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        // O estado real mora no Spotify, não aqui: perguntar antes evita mandar
        // "play" no que já está tocando quando outro aparelho mexeu na fila.
        let tocando = self
            .tocando_agora()
            .await?
            .map(|n| n.is_playing)
            .unwrap_or(false);

        if tocando {
            self.client.pause_playback(None).await
        } else {
            self.client.resume_playback(None, None).await
        }
        .map_err(traduzir)
    }

    async fn next(&mut self) -> Result<(), MusicError> {
        self.client.next_track(None).await.map_err(traduzir)
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        self.client.previous_track(None).await.map_err(traduzir)
    }
}

/// Resultado de uma autorização nova.
pub struct Autorizacao {
    pub refresh_token: String,
    pub quando: chrono::DateTime<Utc>,
}

/// Monta a URL de autorização e o cliente que vai trocar o código por token.
///
/// Devolve o cliente junto porque o PKCE exige que o mesmo `code_verifier`
/// gerado aqui seja usado na troca — um cliente novo não conseguiria completar.
pub fn iniciar_autorizacao(client_id: &str) -> Result<(AuthCodePkceSpotify, String), MusicError> {
    let mut client = AuthCodePkceSpotify::new(
        Credentials::new_pkce(client_id),
        OAuth {
            redirect_uri: REDIRECT_URI.to_string(),
            scopes: escopos(),
            ..Default::default()
        },
    );

    let url = client.get_authorize_url(None).map_err(traduzir)?;
    Ok((client, url))
}

/// Espera o Spotify redirecionar de volta e troca o código pelo refresh token.
///
/// Sobe um servidor de uma requisição só em `REDIRECT_URI`. É o caminho normal
/// para app desktop: o navegador abre, o usuário aprova, e o Spotify devolve o
/// código para o próprio aparelho — sem servidor na internet no meio.
pub async fn concluir_autorizacao(
    client: AuthCodePkceSpotify,
) -> Result<Autorizacao, MusicError> {
    let codigo = esperar_codigo().await?;

    client.request_token(&codigo).await.map_err(traduzir)?;

    let token = client
        .token
        .lock()
        .await
        .unwrap()
        .clone()
        .ok_or(MusicError::NeedsReauth)?;

    let refresh_token = token.refresh_token.ok_or_else(|| {
        MusicError::Network("o Spotify não devolveu refresh token na autorização".into())
    })?;

    Ok(Autorizacao {
        refresh_token,
        quando: Utc::now(),
    })
}

/// Servidor de uma requisição só, o suficiente para capturar o `code`.
async fn esperar_codigo() -> Result<String, MusicError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let porta = REDIRECT_URI
        .rsplit(':')
        .next()
        .and_then(|resto| resto.split('/').next())
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8888);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", porta))
        .await
        .map_err(|e| MusicError::Network(format!("não consegui escutar em {porta}: {e}")))?;

    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| MusicError::Network(e.to_string()))?;

    let mut buffer = [0u8; 2048];
    let lidos = stream
        .read(&mut buffer)
        .await
        .map_err(|e| MusicError::Network(e.to_string()))?;
    let requisicao = String::from_utf8_lossy(&buffer[..lidos]);

    let resposta = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
        <html><body style=\"background:#07090d;color:#e8edf5;font-family:system-ui;display:grid;place-items:center;height:100vh;margin:0\">\
        <p>Spotify conectado. Pode voltar para o Eclipse OS.</p></body></html>";
    let _ = stream.write_all(resposta.as_bytes()).await;
    let _ = stream.shutdown().await;

    extrair_codigo(&requisicao)
        .ok_or_else(|| MusicError::Network("o Spotify não devolveu um código".into()))
}

/// Tira o `code` da linha de requisição HTTP.
fn extrair_codigo(requisicao: &str) -> Option<String> {
    let linha = requisicao.lines().next()?;
    let alvo = linha.split_whitespace().nth(1)?;
    let query = alvo.split_once('?')?.1;

    query.split('&').find_map(|par| {
        let (chave, valor) = par.split_once('=')?;
        (chave == "code").then(|| valor.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrai_o_codigo_do_redirect() {
        let req = "GET /callback?code=AQD123abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1:8888\r\n\r\n";
        assert_eq!(extrair_codigo(req).as_deref(), Some("AQD123abc"));
    }

    /// O usuário pode recusar a autorização; aí vem `error` em vez de `code`.
    #[test]
    fn recusa_do_usuario_nao_vira_codigo() {
        let req = "GET /callback?error=access_denied&state=xyz HTTP/1.1\r\n\r\n";
        assert_eq!(extrair_codigo(req), None);
    }

    #[test]
    fn requisicao_sem_query_nao_quebra() {
        assert_eq!(extrair_codigo("GET /callback HTTP/1.1\r\n\r\n"), None);
        assert_eq!(extrair_codigo(""), None);
    }
}
