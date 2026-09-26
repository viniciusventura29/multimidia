//! Os módulos do painel.
//!
//! Cada um roda na própria task, publica o próprio estado e não sabe dos
//! vizinhos — com uma exceção declarada: o `assistente` lê o painel inteiro,
//! porque a graça dele é justamente cruzar carro, mapa e música numa frase só.

pub mod adaptador;
pub mod assistente;
pub mod messaging;
pub mod music;
// Só é USADO no Android (no desktop quem toca é o Web Playback SDK da
// WebView), mas segue sendo compilado no macOS para type-check — mesma razão
// do `obd_bt`. Sem isto, mexer aqui só quebraria no CI.
#[cfg_attr(not(mobile), allow(dead_code))]
pub mod musica_local;
pub mod nav;
pub mod obd;
