use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// O que está tocando agora.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub track: String,
    pub artist: String,
    pub is_playing: bool,
    pub album_art: Option<String>,
    pub progress_ms: Option<u32>,
    pub duration_ms: Option<u32>,
}

/// Uma faixa achada na busca (ou item de playlist) — o suficiente para listar
/// e mandar tocar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Faixa {
    /// URI do Spotify (`spotify:track:...`) — é o que se manda para tocar.
    pub uri: String,
    pub track: String,
    pub artist: String,
    pub album_art: Option<String>,
}

/// Uma playlist do usuário.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// URI do Spotify (`spotify:playlist:...`) — o que se manda para tocar.
    pub uri: String,
    pub nome: String,
    pub album_art: Option<String>,
}

/// Um álbum, para poder abrir e escolher a faixa dentro dele.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub uri: String,
    pub nome: String,
    pub artist: String,
    pub album_art: Option<String>,
}

/// O que a busca devolve: faixas e álbuns, para o painel oferecer os dois.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Busca {
    pub faixas: Vec<Faixa>,
    pub albuns: Vec<Album>,
}

/// Uma playlist ou álbum **aberto**, com as faixas dentro.
///
/// É o que faltava para escolher a música: antes tocar numa playlist já mandava
/// tocar a playlist inteira, sem deixar ver o que tinha dentro.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contexto {
    /// URI da playlist/álbum — serve para o botão "tocar tudo".
    pub uri: String,
    pub nome: String,
    /// "playlist" ou o nome do artista, dependendo do que foi aberto.
    pub subtitulo: String,
    pub album_art: Option<String>,
    pub faixas: Vec<Faixa>,
}

/// O que está errado, de forma que a tela saiba o que oferecer.
///
/// Antes a tela adivinhava isso por **regex no texto do erro** — e dois casos
/// não casavam ("nenhum dispositivo ativo" e "exige Premium"), então o painel
/// mostrava "sem sinal" e não oferecia saída nenhuma. Tipado, não tem como
/// esquecer um caso.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipoProblema {
    /// Precisa (re)fazer o login do Spotify.
    PrecisaLogin,
    /// Conta sem Premium — login não resolve, o Spotify não permite.
    PrecisaPremium,
    /// Ninguém para tocar. Normalmente o player do Eclipse ainda está subindo.
    SemDispositivo,
    /// Falha transitória: vale tentar de novo, sozinho.
    Rede,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problema {
    pub tipo: TipoProblema,
    /// Texto para mostrar ao usuário.
    pub detalhe: String,
}

/// Estado publicado do módulo de música: o que toca agora, a última busca, as
/// playlists do usuário, o que estiver aberto e o que estiver errado.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicState {
    pub now_playing: Option<NowPlaying>,
    pub busca: Busca,
    pub playlists: Vec<Playlist>,
    pub contexto: Option<Contexto>,
    /// `None` = tudo bem.
    pub problema: Option<Problema>,
}

#[derive(Debug, thiserror::Error)]
pub enum MusicError {
    /// O refresh token venceu (o Spotify expira em 6 meses) ou foi revogado.
    /// Não adianta tentar de novo: só reautenticando.
    #[error("o Spotify precisa reconectar")]
    NeedsReauth,

    /// A Web API comanda um device que já esteja ativo — ela não cria um.
    /// Sem nada tocando em lugar nenhum, não há o que controlar.
    #[error("nenhum dispositivo Spotify ativo")]
    NoActiveDevice,

    /// Controlar playback pela API exige Premium; conta free devolve 403.
    #[error("o controle de playback exige Spotify Premium")]
    PremiumRequired,

    #[error("o perfil ainda não conectou o Spotify")]
    NotConnected,

    /// Falta o Client ID no `eclipse.toml`. Não é falha de rede nem de conta:
    /// o app nunca foi configurado.
    #[error("falta configurar o Client ID do Spotify")]
    NotConfigured,

    #[error("falha ao falar com o Spotify: {0}")]
    Network(String),
}

impl MusicError {
    /// Traduz o erro no que a tela deve oferecer. Substitui a regex que a UI
    /// fazia sobre o texto — e que deixava dois casos sem saída.
    pub fn problema(&self) -> Problema {
        let tipo = match self {
            Self::NeedsReauth | Self::NotConnected | Self::NotConfigured => {
                TipoProblema::PrecisaLogin
            }
            Self::PremiumRequired => TipoProblema::PrecisaPremium,
            Self::NoActiveDevice => TipoProblema::SemDispositivo,
            Self::Network(_) => TipoProblema::Rede,
        };
        Problema {
            tipo,
            detalhe: self.to_string(),
        }
    }
}

/// De onde vem a música.
///
/// Mesma ideia do `ObdSource`: o módulo fala com o trait, não com o Spotify.
/// Trocar por outro serviço, ou por um mock nos testes, não mexe na UI.
#[async_trait]
pub trait MusicSource: Send {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError>;
    async fn toggle(&mut self) -> Result<(), MusicError>;
    async fn next(&mut self) -> Result<(), MusicError>;
    async fn previous(&mut self) -> Result<(), MusicError>;

    /// Busca faixas e álbuns por texto. Default vazio: fontes que não são a Web
    /// API (demo) não têm busca.
    async fn buscar(&mut self, _termo: &str) -> Result<Busca, MusicError> {
        Ok(Busca::default())
    }

    /// Abre uma playlist ou álbum e devolve as faixas de dentro. Decide pelo
    /// próprio URI (`spotify:playlist:...` vs `spotify:album:...`).
    async fn abrir(&mut self, _uri: &str) -> Result<Contexto, MusicError> {
        Err(MusicError::NotConfigured)
    }

    /// Toca música no device ativo.
    ///
    /// - Com `contexto` (URI de playlist/álbum): toca **dentro** dele, montando a
    ///   fila — é o que faz "próxima/anterior" funcionarem. `faixa` (URI de faixa)
    ///   vira o ponto de partida (offset); sem ela, começa do início.
    /// - Sem `contexto`, só com `faixa`: toca a faixa avulsa (sem fila — o caso da
    ///   busca, onde não há um contexto para navegar).
    ///
    /// Default: não suportado.
    async fn tocar(
        &mut self,
        _faixa: Option<&str>,
        _contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        Err(MusicError::NotConfigured)
    }

    /// Salta para uma posição da faixa atual (em milissegundos). O caminho rápido
    /// é o SDK do WebView; aqui é o fallback pela Web API. Default: não suportado.
    async fn seek(&mut self, _posicao_ms: u32) -> Result<(), MusicError> {
        Err(MusicError::NotConfigured)
    }

    /// As playlists do usuário. Default vazio.
    async fn playlists(&mut self) -> Result<Vec<Playlist>, MusicError> {
        Ok(Vec::new())
    }
}
