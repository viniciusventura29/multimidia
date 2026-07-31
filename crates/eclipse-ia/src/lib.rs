//! O assistente do painel: o laço do agente e o quadro que ele pinta.
//!
//! Este crate não conhece Tauri nem os módulos do Eclipse. Ele recebe um
//! [`eclipse_mcp::Registro`] — de onde vêm as ferramentas, sejam elas do carro
//! ou da internet — e devolve um [`cartao::Quadro`]. Quem decide *quando*
//! acionar e *o que pedir* é o módulo `assistente` do `src-tauri`.
//!
//! O desenho em uma frase: o modelo pesquisa com as ferramentas e escreve
//! chamando `pintar_quadro`. Não existe outra saída — texto solto na resposta
//! final é ignorado, para não haver duas maneiras de dizer a mesma coisa.

pub mod agente;
pub mod cartao;
pub mod cliente;
pub mod modelo;
pub mod quadro;

pub use agente::{sistema_padrao, Agente, Config, McpRemoto, Turno, Uso};
pub use cartao::{Cartao, Ponto, Quadro, TipoGrafico, Tom, MAXIMO_CARTOES};
pub use cliente::{IaError, Transporte, TransporteHttp};
pub use modelo::Modelo;
pub use quadro::ProvedorQuadro;
