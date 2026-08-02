//! A superfície única de ferramentas do Eclipse, no formato do MCP.
//!
//! Tudo que o assistente sabe fazer — ler o carro, ver onde estamos, olhar o
//! relógio, buscar uma foto, gerar uma imagem, pintar o quadro — entra por aqui.
//! Um [`Provedor`] declara ferramentas e sabe executá-las; o [`Registro`] junta
//! todos num catálogo só e despacha pelo nome.
//!
//! **Por que MCP e não um enum de ações.** O formato do MCP (`tools/list`,
//! `tools/call`, JSON Schema por ferramenta) é o mesmo que a API da Anthropic
//! espera em `tools`, o mesmo que qualquer servidor MCP remoto fala, e o mesmo
//! que um cliente externo (Claude Desktop, por exemplo) saberia consumir. Falar
//! esse formato desde dentro significa que acrescentar uma capacidade nova é
//! sempre o mesmo gesto, venha ela do carro ou da internet.
//!
//! **Onde está o transporte.** Não está aqui, de propósito. O único consumidor
//! hoje é o agente, que roda no mesmo processo — abrir um socket no Android
//! seria atrito sem retorno. Mas [`protocolo::atender`] já responde JSON-RPC
//! 2.0, então expor isto como servidor MCP de verdade é escrever um transporte
//! por cima, não redesenhar nada.

mod erro;
mod ferramenta;
pub mod protocolo;
mod registro;

pub use erro::McpError;
pub use ferramenta::{campo, esquema_objeto, sem_argumentos, Ferramenta};
pub use registro::{Provedor, Registro, Resultado};
