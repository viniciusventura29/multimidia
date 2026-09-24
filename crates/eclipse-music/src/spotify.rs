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
use rspotify::model::{AdditionalType, Market, PlayableItem};
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

/// O país em que este Spotify vive, tirado do próprio token.
///
/// Sem ele, a API devolve o conteúdo "como está guardado" — incluindo faixas que
/// NÃO tocam no país do dono. A tela então lista uma coisa e o Spotify toca
/// outra: ao mandar tocar com `offset` na URI de uma faixa que não existe no
/// contexto de verdade (o Spotify monta o contexto já filtrado pelo mercado, e
/// ainda religa faixas para as versões locais — é o "track relinking" deles), a
/// reprodução não acha aquela URI e começa em outro lugar. Era isso que fazia
/// tocar cinco faixas à frente da que se clicou.
///
/// `FromToken` e não um país fixo: o mercado sai do usuário dono do token, que
/// num carro é sempre a mesma pessoa.
const MERCADO: Market = Market::FromToken;

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
    /// Os nomes DESTE aparelho, para achar a central na lista do Connect.
    ///
    /// Plural porque o Android e o Spotify discordam: no carro do dono o
    /// sistema diz "K706" (nome de configuração) e o Spotify se anuncia como
    /// "HT-9960CA" (modelo de fábrica). Mandar um só fazia o app da central
    /// ser rejeitado — e a tela pedia para abrir o Spotify que já estava
    /// aberto.
    ///
    /// Vazio no desktop e quando o Android não soube responder. Nesse caso a
    /// escolha não arrisca adivinhar — ver `escolher_device`.
    nomes_do_aparelho: Vec<String>,
    /// O dispositivo que o Eclipse APRENDEU ser o da central.
    ///
    /// Aprendido observando quem aparece na lista quando o app local é
    /// acordado — ver `MusicSource::dispositivos`. Uma vez sabido, vence
    /// qualquer regra de nome: é conhecimento, não palpite.
    dispositivo_fixado: Option<String>,
}

/// O nome que o Spotify anuncia é o mesmo aparelho que o Android diz ser?
///
/// Comparação por continência, e não por igualdade, porque os dois lados nem
/// sempre escrevem igual: o Spotify às vezes prefixa ou sufixa o nome do
/// modelo, e o dono pode ter batizado a central em Configurações. Exigir
/// igualdade exata faria o casamento falhar por um espaço — e falhar aqui
/// manda o som para o celular no bolso de quem está dirigindo.
///
/// Vazio nunca casa: `"".contains("")` é verdadeiro, e isso faria QUALQUER
/// dispositivo passar por "a central" quando o Android não soube responder.
fn e_o_mesmo_aparelho(nome: &str, daqui: &str) -> bool {
    let nome = nome.trim().to_lowercase();
    let daqui = daqui.trim().to_lowercase();
    if nome.is_empty() || daqui.is_empty() {
        return false;
    }
    nome == daqui || nome.contains(&daqui) || daqui.contains(&nome)
}

/// Este dispositivo pode estar DENTRO do carro?
///
/// Existe por um defeito que chegou a acontecer: o dono deu play dirigindo e a
/// música começou a tocar no computador dele, em casa. Ele teve que abrir o
/// Spotify na central e trocar para "tocar neste dispositivo".
///
/// Como isso passou: antes, o Eclipse se anunciava pela WebView e estava
/// SEMPRE na lista, funcionando como piso — o computador, com a nota mais
/// baixa, nunca ganhava. Ao tirar a WebView do caminho do áudio (que é o
/// conserto do "pula cinco músicas"), o piso saiu junto. Sobrando só o
/// computador na lista, ele vence por ser o único.
///
/// A correção não é dar nota menor ao computador — ele já tinha a menor. É
/// dizer que certos dispositivos não são candidatos **em nenhuma hipótese**.
///
/// Só duas coisas contam como "dentro do carro":
/// - o aparelho cujo nome bate com o desta central;
/// - um que se anuncie como `Automobile` (nenhum computador ou celular faz isso).
///
/// O celular do dono fica de fora de propósito, mesmo que esteja no carro: o
/// som sairia pelo alto-falante dele, não pelo do veículo.
fn pode_ser_o_carro(nome: &str, tipo: &rspotify::model::DeviceType, daqui: &[String]) -> bool {
    use rspotify::model::DeviceType;
    if matches!(tipo, DeviceType::Automobile) {
        return true;
    }
    daqui.iter().any(|d| e_o_mesmo_aparelho(nome, d))
}

/// Nota de um dispositivo do Connect. Maior ganha — ver `escolher_device`.
///
/// Fora do método de propósito: é a regra que decide de onde sai o som, e
/// aninhada dentro de um `async fn` ela não podia ser testada.
fn pontos(nome: &str, tipo: &rspotify::model::DeviceType, ativo: bool, daqui: &[String]) -> i32 {
    use rspotify::model::DeviceType;

    // O app do Spotify DESTA central, achado por QUALQUER UM dos nomes que o
    // Android dá a si mesmo. É o único caso em que se tem certeza de onde o
    // som vai sair.
    if daqui.iter().any(|d| e_o_mesmo_aparelho(nome, d)) {
        return 100 + i32::from(ativo);
    }

    let base = match tipo {
        // Uma central que se anuncia como automóvel também é daqui — e nenhum
        // celular se anuncia assim.
        DeviceType::Automobile => 60,
        // O Eclipse pelo WebView: plano B. Continua melhor que mandar o som
        // para um aparelho que não está dentro do carro.
        _ if nome == NOME_DEVICE => 40,
        // Celular/tablet que NÃO é esta central: provavelmente o telefone no
        // bolso do dono. Só se não houver mais nada.
        DeviceType::Smartphone | DeviceType::Tablet => 10,
        // O PC é justamente o que se quer evitar aqui.
        DeviceType::Computer => 0,
        _ => 5,
    };
    base + i32::from(ativo)
}

impl SpotifySource {
    /// Monta um cliente já autenticado a partir do refresh token guardado do perfil.
    pub async fn conectar(
        client_id: &str,
        perfil: Uuid,
        cofre: Arc<Mutex<TokenStore>>,
        nomes_do_aparelho: Vec<String>,
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
            nomes_do_aparelho,
            dispositivo_fixado: None,
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

    /// Escolhe onde tocar.
    ///
    /// # Por que o WebView deixou de ser o preferido
    ///
    /// Até aqui a regra dava nota 100 para o próprio Eclipse — o Web Playback
    /// SDK, que decodifica o áudio DENTRO da WebView. No papel é elegante:
    /// dispensa o app do Spotify e responde na hora.
    ///
    /// No carro, não funciona. O relato do dono: "quando eu seleciono uma
    /// música ele começa a pular músicas até que ele para depois de pular 5, e
    /// aí toca um pouco e depois para de sair o som". Essa é a assinatura de
    /// um decodificador que não dá conta — a cada faixa que ele não consegue
    /// abrir, o Spotify avança para a próxima; quando enfim abre uma, o fôlego
    /// acaba no meio. Decodificar áudio com DRM numa WebView de SoC barata,
    /// disputando CPU com um mapa vetorial, é pedir demais.
    ///
    /// O app do Spotify instalado na central decodifica nativamente e não tem
    /// nada disso. Então ele passa na frente, e o WebView vira plano B.
    ///
    /// # O problema de saber QUAL é a central
    ///
    /// O app da central e o Spotify do celular do dono aparecem os dois como
    /// `Smartphone`. Escolher pelo tipo mandaria o som para o celular —
    /// trocando um defeito por outro pior, porque o carro ficaria mudo e o
    /// celular tocando no bolso.
    ///
    /// Por isso a identificação é pelo NOME: o app do Spotify se anuncia no
    /// Connect com o nome do aparelho, e o Android sabe dizer o nome deste
    /// aparelho (ver `nomeDoAparelho` no plugin). Sem esse nome — desktop, ou
    /// a chamada falhou — a regra não arrisca: cai no WebView, que é o
    /// comportamento de hoje.
    async fn escolher_device(&self) -> Result<String, MusicError> {
        let devices = self.client.device().await.map_err(traduzir)?;
        let daqui: &[String] = &self.nomes_do_aparelho;

        // O aprendido vence tudo. Se o Eclipse já descobriu qual é o Spotify
        // da central, não há regra de nome nem de tipo que deva discordar.
        if let Some(fixado) = &self.dispositivo_fixado {
            if let Some(d) = devices
                .iter()
                .find(|d| d.id.is_some() && d.name.eq_ignore_ascii_case(fixado))
            {
                tracing::info!(dispositivo = %d.name, "usando o Spotify aprendido da central");
                return d.id.clone().ok_or(MusicError::NoActiveDevice);
            }
        }

        // Filtra ANTES de pontuar. Pontuação escolhe o melhor entre candidatos
        // aceitáveis; ela não sabe recusar TODOS — e foi isso que deixou o som
        // sair no computador de casa quando ele era o único da lista.
        let escolhido = devices
            .iter()
            .filter(|d| d.id.is_some())
            .filter(|d| pode_ser_o_carro(&d.name, &d._type, daqui))
            .max_by_key(|d| pontos(&d.name, &d._type, d.is_active, daqui))
            .cloned();

        // O nível depende da RESPOSTA, e isso é de propósito.
        //
        // Esta é a decisão que define se o som sai pela WebView (que engasga,
        // pula faixa e emudece) ou pelo app nativo. Depois do primeiro teste no
        // carro ela era a única coisa que eu precisava saber — e estava em
        // `debug`, que nunca sai da memória. O diário voltou sem uma palavra
        // sobre o assunto.
        //
        // `info` não bastaria: linha de `info` só chega ao servidor de carona no
        // rastro de um aviso, e o rastro guarda 50 linhas. Numa sessão com o OBD
        // reiniciando em série, a linha que interessa é engolida antes de
        // alguém drená-la.
        //
        // Então: deu certo -> `info` (bom saber, dispensável). NÃO deu -> `warn`,
        // porque aí é degradação de verdade e precisa chegar.
        //
        // A lista inteira vai junto: "escolhi X" sem as alternativas não permite
        // julgar a escolha, e o caso de falha mais provável — a central nem
        // aparecer na lista — fica indistinguível de "escolhi errado".
        let candidatos: Vec<String> = devices
            .iter()
            .map(|d| format!("{} ({:?}, ativo={})", d.name, d._type, d.is_active))
            .collect();
        let nome_escolhido = escolhido
            .as_ref()
            .map(|d| format!("{} ({:?})", d.name, d._type))
            .unwrap_or_else(|| "nenhum".into());
        let e_daqui = escolhido
            .as_ref()
            .is_some_and(|d| daqui.iter().any(|n| e_o_mesmo_aparelho(&d.name, n)));

        if e_daqui {
            tracing::info!(
                aparelho = ?daqui,
                escolhido = nome_escolhido,
                ?candidatos,
                "o som vai pelo app do Spotify desta central"
            );
        } else {
            tracing::warn!(
                aparelho = ?daqui,
                escolhido = nome_escolhido,
                ?candidatos,
                "o som NÃO vai pelo app do Spotify desta central; \
                 ele é quem decodifica sem engasgar"
            );
        }

        match escolhido.and_then(|d| d.id) {
            Some(id) => Ok(id),
            // Lista vazia e "lista cheia de gente de fora" são problemas
            // diferentes, e só o segundo tem uma saída que o dono pode tomar.
            None if devices.is_empty() => Err(MusicError::NoActiveDevice),
            None => Err(MusicError::SoDispositivoDeFora),
        }
    }

    async fn tocando_agora(&self) -> Result<Option<NowPlaying>, MusicError> {
        let contexto = self
            .client
            .current_playing(Some(MERCADO), None::<&[AdditionalType]>)
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
            self.client.search(
                termo,
                SearchType::Track,
                Some(MERCADO),
                None,
                Some(20),
                None
            ),
            self.client.search(
                termo,
                SearchType::Album,
                Some(MERCADO),
                None,
                Some(12),
                None
            ),
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
            //
            // UMA chamada, e isso importa. A resposta de `album` JÁ TRAZ a
            // primeira página de faixas — pedi-las de novo com
            // `album_track_manual` baixava o mesmo JSON duas vezes, em sequência,
            // num carro que está quase sempre no hotspot do celular. O diário
            // mediu 6,5 s para abrir uma playlist por causa disso.
            let album = self
                .client
                .album(id.clone(), Some(MERCADO))
                .await
                .map_err(traduzir)?;
            let capa = album.images.into_iter().next().map(|i| i.url);
            let faixas = album
                .tracks
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
        // Também uma chamada só: `playlist` traz a primeira página de itens
        // junto, e o `playlist_items_manual` que vinha depois baixava exatamente
        // os mesmos cem itens de novo.
        let playlist = self
            .client
            .playlist(id, None, Some(MERCADO))
            .await
            .map_err(traduzir)?;
        let faixas = playlist
            .items
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

    async fn dispositivos(&mut self) -> Result<Vec<(String, bool)>, MusicError> {
        use rspotify::model::DeviceType;
        let devices = self.client.device().await.map_err(traduzir)?;
        Ok(devices
            .into_iter()
            .filter(|d| d.id.is_some())
            .map(|d| (d.name, matches!(d._type, DeviceType::Computer)))
            .collect())
    }

    fn fixar_dispositivo(&mut self, nome: Option<String>) {
        self.dispositivo_fixado = nome;
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

#[cfg(test)]
mod tests_escolha_de_device {
    use super::*;
    use rspotify::model::DeviceType;

    /// O nome que o Android deu a esta central.
    const CENTRAL: &str = "UIS7862";

    fn nomes(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn melhor<'a>(lista: &[(&'a str, DeviceType, bool)], daqui: &[String]) -> &'a str {
        lista
            .iter()
            .max_by_key(|(nome, tipo, ativo)| pontos(nome, tipo, *ativo, daqui))
            .map(|(nome, _, _)| *nome)
            .expect("lista não-vazia")
    }

    #[test]
    fn o_som_vai_para_a_central_e_nao_para_o_celular_do_dono() {
        // O risco que quase me fez não mexer nisto: o app da central e o
        // Spotify do celular aparecem os DOIS como `Smartphone`. Escolher pelo
        // tipo deixaria o carro mudo e o celular tocando no bolso.
        let lista = [
            ("iPhone do Vinicius", DeviceType::Smartphone, true),
            (CENTRAL, DeviceType::Smartphone, false),
            (NOME_DEVICE, DeviceType::Computer, false),
        ];
        assert_eq!(
            melhor(&lista, &nomes(&[CENTRAL])),
            CENTRAL,
            "com o nome do aparelho em mãos, não há empate a resolver"
        );
    }

    #[test]
    fn o_celular_do_dono_perde_ate_para_o_webview() {
        // Sem o app da central na lista, o WebView (que ao menos toca DENTRO
        // do carro) ganha do celular. Áudio ruim no carro é melhor que áudio
        // bom no bolso de quem está dirigindo.
        let lista = [
            ("iPhone do Vinicius", DeviceType::Smartphone, true),
            (NOME_DEVICE, DeviceType::Computer, false),
        ];
        assert_eq!(melhor(&lista, &nomes(&[CENTRAL])), NOME_DEVICE);
    }

    #[test]
    fn sem_saber_o_nome_daqui_nao_se_arrisca() {
        // Desktop, ou a pergunta ao Android falhou. Adivinhar entre dois
        // `Smartphone` seria pior que manter o comportamento antigo.
        let lista = [
            ("iPhone do Vinicius", DeviceType::Smartphone, true),
            (CENTRAL, DeviceType::Smartphone, false),
            (NOME_DEVICE, DeviceType::Computer, false),
        ];
        assert_eq!(melhor(&lista, &[]), NOME_DEVICE);
    }

    #[test]
    fn uma_central_que_se_diz_automovel_ganha_do_webview() {
        // Nenhum celular se anuncia como `Automobile`; quem se anuncia assim
        // está no carro, e decodifica nativamente.
        let lista = [
            ("Minha central", DeviceType::Automobile, false),
            (NOME_DEVICE, DeviceType::Computer, true),
        ];
        assert_eq!(melhor(&lista, &[]), "Minha central");
    }

    #[test]
    fn o_pc_e_o_ultimo_lugar_do_mundo() {
        // O caso que originou a regra: Spotify aberto no PC de casa levava o
        // som para lá, e o carro ficava mudo.
        let lista = [
            ("PC do escritório", DeviceType::Computer, true),
            (CENTRAL, DeviceType::Smartphone, false),
        ];
        assert_eq!(melhor(&lista, &nomes(&[CENTRAL])), CENTRAL);
    }

    #[test]
    fn o_nome_casa_sem_olhar_maiuscula() {
        let lista = [("uis7862", DeviceType::Smartphone, false)];
        assert_eq!(melhor(&lista, &nomes(&["UIS7862"])), "uis7862");
    }

    #[test]
    fn o_nome_casa_mesmo_com_prefixo_ou_sufixo() {
        // O Spotify nem sempre anuncia o nome exatamente como o Android o
        // escreve. Exigir igualdade faria o casamento falhar por um espaço — e
        // falhar aqui manda o som para o celular no bolso de quem dirige.
        for anunciado in ["Android UIS7862", "UIS7862 (Auto)", "uis7862"] {
            let lista = [
                ("iPhone do Vinicius", DeviceType::Smartphone, true),
                (anunciado, DeviceType::Smartphone, false),
            ];
            assert_eq!(
                melhor(&lista, &nomes(&[CENTRAL])),
                anunciado,
                "'{anunciado}' devia casar com '{CENTRAL}'"
            );
        }
    }

    fn aceitos<'a>(lista: &[(&'a str, DeviceType, bool)], daqui: &[String]) -> Vec<&'a str> {
        lista
            .iter()
            .filter(|(nome, tipo, _)| pode_ser_o_carro(nome, tipo, daqui))
            .map(|(nome, _, _)| *nome)
            .collect()
    }

    #[test]
    fn o_computador_de_casa_nunca_e_candidato() {
        // ACONTECEU DE VERDADE: o dono deu play dirigindo e a música começou a
        // tocar no computador dele, em casa.
        //
        // A pontuação sozinha não evitava isso. O computador já tinha a nota
        // mais baixa — mas nota mais baixa entre um candidato só ainda é o
        // vencedor. Recusar tem que ser categórico, não relativo.
        let lista = [("PC do escritório", DeviceType::Computer, true)];
        assert!(
            aceitos(&lista, &nomes(&[CENTRAL])).is_empty(),
            "sozinho na lista, o computador ainda assim não pode receber o som"
        );
    }

    #[test]
    fn o_celular_do_dono_tambem_nao_serve() {
        // Mesmo que o celular esteja DENTRO do carro, o som sairia pelo
        // alto-falante dele, não pelo do veículo.
        let lista = [("iPhone do Vinicius", DeviceType::Smartphone, true)];
        assert!(aceitos(&lista, &nomes(&[CENTRAL])).is_empty());
    }

    #[test]
    fn a_central_e_aceita_pelo_nome_ou_por_se_dizer_automovel() {
        let lista = [
            ("PC do escritório", DeviceType::Computer, true),
            ("iPhone do Vinicius", DeviceType::Smartphone, true),
            (CENTRAL, DeviceType::Smartphone, false),
            ("Minha central", DeviceType::Automobile, false),
        ];
        let ok = aceitos(&lista, &nomes(&[CENTRAL]));
        assert!(ok.contains(&CENTRAL), "casou pelo nome");
        assert!(ok.contains(&"Minha central"), "se anunciou como automóvel");
        assert_eq!(ok.len(), 2, "e mais ninguém: {ok:?}");
    }

    #[test]
    fn sem_o_nome_daqui_so_o_automovel_passa() {
        // Se o Android não soube dizer o nome do aparelho, resta o único sinal
        // que não depende dele. Melhor não tocar do que tocar longe do carro.
        let lista = [
            ("PC do escritório", DeviceType::Computer, true),
            ("Algum celular", DeviceType::Smartphone, true),
            ("Central", DeviceType::Automobile, false),
        ];
        assert_eq!(aceitos(&lista, &[]), vec!["Central"]);
    }

    #[test]
    fn o_caso_real_do_carro_k706_e_ht_9960ca() {
        // ACONTECEU: o dono abriu o Spotify na central e a tela continuou
        // dizendo para abrir o Spotify na central.
        //
        // O Android se apresenta como "K706" (o nome que ele deu nas
        // configurações) e o app do Spotify se anuncia como "HT-9960CA" (o
        // modelo de fábrica). Mandando um nome só, o dispositivo CERTO era
        // rejeitado — e a única coisa que sobrava na lista era o computador
        // de casa.
        let lista = [
            ("HT-9960CA", DeviceType::Tablet, false),
            ("DESKTOP-ATVE3IV", DeviceType::Computer, false),
        ];

        // Como era: só o nome de configuração.
        assert!(
            aceitos(&lista, &nomes(&["K706"])).is_empty(),
            "com um nome só, a central do dono não é reconhecida"
        );

        // Como fica: os dois nomes que o Android sabe dar a si mesmo.
        assert_eq!(
            aceitos(&lista, &nomes(&["K706", "HT-9960CA"])),
            vec!["HT-9960CA"],
            "com os dois, a central aparece — e o PC continua fora"
        );
    }

    #[test]
    fn nome_vazio_nao_casa_com_ninguem() {
        // `"".contains("")` é verdadeiro. Sem esta guarda, o Android não saber
        // responder faria o PRIMEIRO dispositivo da lista virar "a central".
        assert!(!e_o_mesmo_aparelho("", ""));
        assert!(!e_o_mesmo_aparelho("iPhone do Vinicius", ""));
        assert!(!e_o_mesmo_aparelho("", "UIS7862"));
    }
}
