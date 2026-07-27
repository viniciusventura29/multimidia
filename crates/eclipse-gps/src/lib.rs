//! Posição.
//!
//! Mesma separação dos outros sensores: o módulo fala com [`LocationSource`], e
//! quem fornece a posição é trocável. Hoje é o trajeto simulado; no carro será
//! o `LocationManager` do Android atrás de um plugin Kotlin.
//!
//! A velocidade vem do `eclipse-sim`, compartilhada com o OBD, para o mapa e os
//! mostradores nunca discordarem sobre o que o carro está fazendo.

pub mod fix;
pub mod rota;
pub mod sim;
pub mod source;

pub use fix::Fix;
pub use rota::TRACADO;
pub use sim::SimulatedLocation;
pub use source::{GpsError, LocationSource};
