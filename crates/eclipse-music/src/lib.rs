//! Música.
//!
//! Mesma separação do `eclipse-obd`: o módulo conversa com o trait
//! [`MusicSource`], não com o Spotify. O que é específico do Spotify — e é
//! bastante — fica isolado.
//!
//! O ponto delicado desta crate não é tocar música, é [`tokens`]: manter um
//! perfil conectado sem perder a sessão por causa da rotação de refresh token,
//! e saber que os 6 meses de validade são inescapáveis.

pub mod source;
pub mod tokens;

pub use source::{MusicError, MusicSource, NowPlaying};
pub use tokens::{StoredToken, TokenError, TokenStore};
