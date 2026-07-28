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

    /// O Android exige a permissão manual "acesso a notificações" para ler a
    /// sessão de mídia — não é login, é um toggle em Ajustes. A UI oferece um
    /// botão que abre a tela certa em vez de repetir o erro do sistema.
    #[error("falta conceder acesso a notificações ao Eclipse OS")]
    PermissionRequired,
}

impl MusicError {
    /// Se reconectar é a única saída, a UI mostra um toque para reautenticar em
    /// vez de sugerir que foi um erro passageiro.
    pub fn exige_reautenticacao(&self) -> bool {
        matches!(self, Self::NeedsReauth | Self::NotConnected)
    }

    /// Se o caminho é abrir uma tela de permissão do sistema, não refazer login.
    pub fn exige_permissao(&self) -> bool {
        matches!(self, Self::PermissionRequired)
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

    /// Busca faixas por texto. Default vazio: fontes que não são a Web API
    /// (demo, sessão de mídia) não têm busca.
    async fn buscar(&mut self, _termo: &str) -> Result<Vec<Faixa>, MusicError> {
        Ok(Vec::new())
    }

    /// Toca uma faixa (URI do Spotify) no device ativo — o app do Spotify em
    /// segundo plano. Default: não suportado.
    async fn tocar(&mut self, _uri: &str) -> Result<(), MusicError> {
        Err(MusicError::NotConfigured)
    }
}
