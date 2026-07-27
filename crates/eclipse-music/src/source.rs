use async_trait::async_trait;
use serde::Serialize;

/// O que está tocando agora.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub track: String,
    pub artist: String,
    pub is_playing: bool,
    pub album_art: Option<String>,
    pub progress_ms: Option<u32>,
    pub duration_ms: Option<u32>,
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

    #[error("falha ao falar com o Spotify: {0}")]
    Network(String),
}

impl MusicError {
    /// Se reconectar é a única saída, a UI mostra um toque para reautenticar em
    /// vez de sugerir que foi um erro passageiro.
    pub fn exige_reautenticacao(&self) -> bool {
        matches!(self, Self::NeedsReauth | Self::NotConnected)
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
}
