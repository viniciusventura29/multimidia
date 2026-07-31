//! Os módulos do painel.
//!
//! Cada um roda na própria task, publica o próprio estado e não sabe dos
//! vizinhos — com uma exceção declarada: o `assistente` lê o painel inteiro,
//! porque a graça dele é justamente cruzar carro, mapa e música numa frase só.

pub mod assistente;
pub mod messaging;
pub mod music;
pub mod nav;
pub mod obd;
