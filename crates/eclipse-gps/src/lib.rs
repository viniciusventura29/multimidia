//! Posição.
//!
//! Mesma separação dos outros sensores: o módulo fala com [`LocationSource`], e
//! quem fornece a posição é trocável. Hoje é [`PushedLocation`] — a
//! geolocalização do navegador, empurrada do JS — porque o Rust não tem como
//! chamar `navigator.geolocation` sozinho. No Android real pode continuar
//! sendo a mesma API do WebView, ou virar um plugin Kotlin sobre o
//! `LocationManager`, dependendo de como a WebView tratar a permissão.

pub mod fix;
pub mod guia;
pub mod parada;
pub mod pushed;
pub mod sol;
pub mod source;

pub use fix::Fix;
pub use guia::{Guia, Passo, Progresso, Route};
pub use parada::FiltroDeParada;
pub use pushed::{Emissor, PushedLocation, Receptor};
pub use source::{GpsError, LocationSource};
