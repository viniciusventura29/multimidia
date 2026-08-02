//! O assistente: o que ele sabe do carro, quando ele resolve falar, e o quanto
//! pode gastar fazendo isso.
//!
//! O laço do agente em si mora no `eclipse-ia`, que não conhece Tauri nem os
//! módulos. O que está aqui é a fiação: transformar o barramento em ferramentas
//! ([`carro`]), decidir a hora de acionar, e segurar a conta.

pub mod carro;
pub mod gatilho;
pub mod imagem;
pub mod orcamento;
