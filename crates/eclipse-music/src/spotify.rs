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

/// A URI de redirect cadastrada no painel do Spotify. Difere por plataforma:
///
/// - No **mobile**, um servidor loopback não funciona: o Android congela o app
///   assim que ele vai pro fundo (enquanto o navegador aprova), então o
///   `127.0.0.1:8888` fica sem ninguém para responder e o navegador trava
///   carregando. O caminho robusto é um **deep link** de scheme próprio, que o
///   sistema entrega de volta ao app (e ainda o traz pra frente sozinho).
/// - No **desktop** o loopback funciona bem e não exige registrar scheme no SO.
///
/// AMBAS precisam estar cadastradas idênticas no painel do app Spotify.
///
/// ⚠️ Aqui é `target_os = "android"`, NÃO `mobile`: o cfg `mobile` é emitido
/// pelo `tauri_build` só para o crate `src-tauri`, não para dependências como
/// este `eclipse-music` — usar `mobile` aqui daria sempre o ramo desktop e a
/// URL sairia com `127.0.0.1` no Android (o bug que fazia o navegador travar).
#[cfg(target_os = "android")]
pub const REDIRECT_URI: &str = "eclipseos://callback";
#[cfg(not(target_os = "android"))]
pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

/// O nome com que o Eclipse se registra como device do Spotify (via Web Playback
/// SDK). Compartilhado entre o JS que cria o player e o Rust que escolhe onde
/// tocar — se divergirem, o Rust não acha o player e o som sai em outro aparelho.
pub const NOME_DEVICE: &str = "Eclipse OS";

pub fn escopos() -> HashSet<String> {
    scopes!(
        "user-read-playback-state",
        "user-modify-playback-state",
        "user-read-currently-playing",
        // Para listar e abrir as playlists do usuário dentro do Eclipse.
        "playlist-read-private",
        "playlist-read-collaborative",
        // `streaming` é o que permite o Web Playback SDK tocar o áudio DENTRO do
        // Eclipse — é o que dispensa o app oficial do Spotify no aparelho. Os
        // dois `user-read-*` são exigidos pelo SDK para identificar a conta.
        "streaming",
        "user-read-email",
        "user-read-private"
    )
}

/// "Artista A, Artista B" — o Spotify devolve lista em toda faixa e álbum.
fn nomes(artistas: Vec<rspotify::model::SimplifiedArtist>) -> String {
    artistas
        .into_iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(", ")
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
    /// Último estado conhecido de reprodução, atualizado a cada `now_playing`.
    /// Evita uma consulta de rede extra no `toggle` — ver o comentário lá.
    tocando: bool,
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

        Ok(Self {
            client,
            tocando: false,
        })
    }

    /// O access token vigente, para o Web Playback SDK usar no WebView.
    ///
    /// O SDK precisa do token cru (ele fala com o Spotify direto do JS para
    /// tocar o áudio aqui dentro, em vez de comandar outro aparelho). Curto —
    /// vence em ~1h — então o SDK pede de novo pelo callback dele.
    pub async fn access_token(&self) -> Result<String, MusicError> {
        self.client.auto_reauth().await.map_err(traduzir)?;
        self.client
            .token
            .lock()
            .await
            .unwrap()
            .as_ref()
            .map(|t| t.access_token.clone())
            .filter(|t| !t.is_empty())
            .ok_or(MusicError::NeedsReauth)
    }

    /// Escolhe onde tocar. Com o Spotify logado no PC E no celular ao mesmo
    /// tempo, pegar só "o device ativo" fazia o som sair no PC. Aqui a gente
    /// **prefere o próprio aparelho** (celular/tablet/automóvel = o head unit) e
    /// evita o PC; `is_active` só desempata. Vazio = ninguém para comandar (abrir
    /// o Spotify no aparelho uma vez para ele aparecer na lista).
    async fn escolher_device(&self) -> Result<String, MusicError> {
        use rspotify::model::DeviceType;

        fn pontos(nome: &str, tipo: &DeviceType, ativo: bool) -> i32 {
            // O próprio Eclipse, via Web Playback SDK, é o alvo preferido: o som
            // sai aqui dentro, sem depender do app oficial do Spotify.
            if nome == NOME_DEVICE {
                return 100 + if ativo { 1 } else { 0 };
            }
            let base = match tipo {
                // Depois dele, o aparelho onde o Eclipse roda (head unit/celular).
                DeviceType::Smartphone | DeviceType::Tablet | DeviceType::Automobile => 10,
                // O PC é justamente o que se quer evitar aqui.
                DeviceType::Computer => 0,
                _ => 5,
            };
            base + if ativo { 1 } else { 0 }
        }

        let devices = self.client.device().await.map_err(traduzir)?;
        println!(
            "[eclipse] devices Spotify: {:?}",
            devices
                .iter()
                .map(|d| format!("{} ({:?}, ativo={})", d.name, d._type, d.is_active))
                .collect::<Vec<_>>()
        );

        devices
            .into_iter()
            .filter(|d| d.id.is_some())
            .max_by_key(|d| pontos(&d.name, &d._type, d.is_active))
            .and_then(|d| d.id)
            .ok_or(MusicError::NoActiveDevice)
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
        let atual = self.tocando_agora().await?;
        self.tocando = atual.as_ref().is_some_and(|n| n.is_playing);
        Ok(atual)
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        // Usa o último estado conhecido em vez de perguntar ao Spotify antes:
        // aquela consulta extra dobrava a ida-e-volta de rede a cada toque, e era
        // parte do delay que se sentia no play/pause. Quem sabe o estado é o
        // módulo, que acabou de pollar — e no caminho normal o SDK do WebView
        // resolve isso localmente, sem passar por aqui (ver `spotifyPlayer.ts`).
        if self.tocando {
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

    async fn buscar(&mut self, termo: &str) -> Result<crate::source::Busca, MusicError> {
        use rspotify::model::{Id, SearchResult, SearchType};

        // Duas buscas em paralelo: faixa para tocar direto, álbum para abrir e
        // escolher a faixa dentro.
        let (faixas, albuns) = tokio::join!(
            self.client
                .search(termo, SearchType::Track, None, None, Some(20), None),
            self.client
                .search(termo, SearchType::Album, None, None, Some(12), None),
        );

        let faixas = match faixas.map_err(traduzir)? {
            SearchResult::Tracks(pagina) => pagina
                .items
                .into_iter()
                .filter_map(|faixa| {
                    Some(crate::source::Faixa {
                        uri: faixa.id?.uri(),
                        track: faixa.name,
                        artist: nomes(faixa.artists),
                        album_art: faixa.album.images.into_iter().next().map(|i| i.url),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };

        let albuns = match albuns.map_err(traduzir)? {
            SearchResult::Albums(pagina) => pagina
                .items
                .into_iter()
                .filter_map(|album| {
                    Some(crate::source::Album {
                        uri: album.id?.uri(),
                        nome: album.name,
                        artist: nomes(album.artists),
                        album_art: album.images.into_iter().next().map(|i| i.url),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };

        Ok(crate::source::Busca { faixas, albuns })
    }

    async fn abrir(&mut self, uri: &str) -> Result<crate::source::Contexto, MusicError> {
        use rspotify::model::{AlbumId, Id, PlayableItem, PlaylistId};

        if uri.contains(":album:") {
            let id = AlbumId::from_uri(uri)
                .map_err(|e| MusicError::Network(format!("URI de álbum inválida: {e}")))?
                .into_static();
            // O álbum inteiro numa chamada: nome/capa vêm do álbum, e as faixas
            // dele não repetem a capa (é a mesma), então herdam a do álbum.
            let album = self
                .client
                .album(id.clone(), None)
                .await
                .map_err(traduzir)?;
            let capa = album.images.into_iter().next().map(|i| i.url);
            let faixas = self
                .client
                .album_track_manual(id, None, Some(50), Some(0))
                .await
                .map_err(traduzir)?
                .items
                .into_iter()
                .filter_map(|f| {
                    Some(crate::source::Faixa {
                        uri: f.id?.uri(),
                        track: f.name,
                        artist: nomes(f.artists),
                        album_art: capa.clone(),
                    })
                })
                .collect();

            return Ok(crate::source::Contexto {
                uri: uri.to_string(),
                nome: album.name,
                subtitulo: nomes(album.artists),
                album_art: capa,
                faixas,
            });
        }

        let id = PlaylistId::from_uri(uri)
            .map_err(|e| MusicError::Network(format!("URI de playlist inválida: {e}")))?
            .into_static();
        let playlist = self
            .client
            .playlist(id.clone(), None, None)
            .await
            .map_err(traduzir)?;
        let faixas = self
            .client
            .playlist_items_manual(id, None, None, Some(100), Some(0))
            .await
            .map_err(traduzir)?
            .items
            .into_iter()
            // `item`, não `track`: o Spotify renomeou o campo (rspotify #550).
            .filter_map(|item| match item.item? {
                // Podcast numa playlist é ignorado: o painel só toca faixa.
                PlayableItem::Track(f) => Some(crate::source::Faixa {
                    uri: f.id?.uri(),
                    track: f.name,
                    artist: nomes(f.artists),
                    album_art: f.album.images.into_iter().next().map(|i| i.url),
                }),
                // Episódio de podcast ou item que a API introduziu depois: o
                // painel só sabe tocar faixa, então some da lista em vez de virar
                // uma linha que não faz nada ao ser tocada.
                _ => None,
            })
            .collect();

        Ok(crate::source::Contexto {
            uri: uri.to_string(),
            nome: playlist.name,
            subtitulo: "playlist".to_string(),
            album_art: playlist.images.into_iter().next().map(|i| i.url),
            faixas,
        })
    }

    async fn tocar(
        &mut self,
        faixa: Option<&str>,
        contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        use rspotify::model::{AlbumId, Offset, PlayContextId, PlayableId, PlaylistId, TrackId};

        let device = self.escolher_device().await?;

        // Com contexto: toca dentro da playlist/álbum, com a faixa como offset.
        // É isto que dá fila real — sem ela, "próxima" não tem para onde ir e a
        // reprodução simplesmente para (parecia pausar).
        if let Some(ctx) = contexto {
            // Mesma decisão de `abrir`: o tipo vem do próprio URI.
            let contexto_id = if ctx.contains(":album:") {
                PlayContextId::Album(
                    AlbumId::from_uri(ctx)
                        .map_err(|e| MusicError::Network(format!("URI de álbum inválida: {e}")))?
                        .into_static(),
                )
            } else {
                PlayContextId::Playlist(
                    PlaylistId::from_uri(ctx)
                        .map_err(|e| MusicError::Network(format!("URI de playlist inválida: {e}")))?
                        .into_static(),
                )
            };
            let offset = faixa.map(|u| Offset::Uri(u.to_string()));
            return self
                .client
                .start_context_playback(contexto_id, Some(&device), offset, None)
                .await
                .map_err(traduzir);
        }

        // Sem contexto: faixa avulsa (busca). Não há fila — é o comportamento
        // esperado de tocar um resultado solto.
        let Some(uri) = faixa else {
            return Err(MusicError::Network("nada para tocar".into()));
        };
        let faixa = TrackId::from_uri(uri)
            .map_err(|e| MusicError::Network(format!("URI de faixa inválida: {e}")))?
            .into_static();

        self.client
            .start_uris_playback(
                std::iter::once(PlayableId::Track(faixa)),
                Some(&device),
                None,
                None,
            )
            .await
            .map_err(traduzir)
    }

    async fn seek(&mut self, posicao_ms: u32) -> Result<(), MusicError> {
        self.client
            .seek_track(chrono::Duration::milliseconds(posicao_ms as i64), None)
            .await
            .map_err(traduzir)
    }

    async fn playlists(&mut self) -> Result<Vec<crate::source::Playlist>, MusicError> {
        use rspotify::model::Id;

        let pagina = self
            .client
            .current_user_playlists_manual(Some(50), Some(0))
            .await
            .map_err(traduzir)?;

        Ok(pagina
            .items
            .into_iter()
            .map(|p| crate::source::Playlist {
                uri: p.id.uri(),
                nome: p.name,
                album_art: p.images.into_iter().next().map(|i| i.url),
            })
            .collect())
    }
}

/// Resultado de uma autorização nova.
pub struct Autorizacao {
    pub refresh_token: String,
    pub quando: chrono::DateTime<Utc>,
}

/// Cliente PKCE no meio de uma autorização, carregando o `code_verifier`.
///
/// Opaco de propósito: o `src-tauri` precisa guardá-lo entre montar a URL e
/// receber o `code` (pelo deep link, no mobile) sem depender do rspotify.
pub struct PkcePendente(AuthCodePkceSpotify);

/// Monta a URL de autorização e o cliente que vai trocar o código por token.
///
/// Devolve o cliente junto porque o PKCE exige que o mesmo `code_verifier`
/// gerado aqui seja usado na troca — um cliente novo não conseguiria completar.
pub fn iniciar_autorizacao(client_id: &str) -> Result<(PkcePendente, String), MusicError> {
    let mut client = AuthCodePkceSpotify::new(
        Credentials::new_pkce(client_id),
        OAuth {
            redirect_uri: REDIRECT_URI.to_string(),
            scopes: escopos(),
            ..Default::default()
        },
    );

    let url = client.get_authorize_url(None).map_err(traduzir)?;
    Ok((PkcePendente(client), url))
}

/// Espera o Spotify redirecionar de volta e troca o código pelo refresh token.
///
/// Sobe um servidor de uma requisição só em `REDIRECT_URI`. É o caminho normal
/// para app desktop: o navegador abre, o usuário aprova, e o Spotify devolve o
/// código para o próprio aparelho — sem servidor na internet no meio.
pub async fn concluir_autorizacao(pendente: PkcePendente) -> Result<Autorizacao, MusicError> {
    let codigo = esperar_codigo().await?;
    trocar_codigo(pendente, &codigo).await
}

/// Troca o `code` pelo refresh token. É a metade final do PKCE, isolada porque
/// no mobile o `code` não vem de um servidor loopback e sim de um deep link —
/// o `connect_spotify` guarda o `client` (que carrega o `code_verifier`) e
/// chama isto quando o deep link chega.
pub async fn trocar_codigo(
    pendente: PkcePendente,
    codigo: &str,
) -> Result<Autorizacao, MusicError> {
    let client = pendente.0;
    client.request_token(codigo).await.map_err(traduzir)?;

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

/// Extrai o `code` de uma URL de callback (deep link `eclipseos://callback?...`
/// ou loopback). Reaproveita o parser da linha de requisição fingindo uma.
pub fn codigo_de_url(url: &str) -> Option<String> {
    extrair_codigo(&format!("GET {url} HTTP/1.1"))
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
