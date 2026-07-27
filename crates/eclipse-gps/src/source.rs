use async_trait::async_trait;

use crate::fix::Fix;

#[derive(Clone, Debug, thiserror::Error)]
pub enum GpsError {
    /// No navegador, `navigator.geolocation` foi negada; no Android, seria a
    /// permissão de localização do sistema.
    #[error("permissão de localização não concedida")]
    SemPermissao,

    /// Céu bloqueado, garagem, túnel, ou o navegador simplesmente não sabe
    /// dizer onde o aparelho está. É estado normal de um GPS, não defeito.
    #[error("sem sinal de localização")]
    SemSinal,
}

/// De onde vem a posição.
///
/// Hoje é a geolocalização do navegador (`navigator.geolocation`, empurrada do
/// JS para o Rust); no Android real pode continuar sendo a mesma API do
/// WebView, ou o `LocationManager` atrás de um plugin Kotlin, se a WebView
/// bloquear a permissão como aconteceu com o Web Playback SDK do Spotify.
#[async_trait]
pub trait LocationSource: Send {
    /// Espera a próxima posição, ou o motivo de não haver uma agora.
    async fn next_fix(&mut self) -> Result<Fix, GpsError>;
}
