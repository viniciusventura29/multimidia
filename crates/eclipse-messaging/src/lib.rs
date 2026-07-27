//! Mensagens.
//!
//! Vale ser franco sobre o teto desta funcionalidade: **não existe API oficial
//! do WhatsApp para conta pessoal**. No Android o caminho é o
//! `NotificationListenerService` — API pública, é o mesmo mecanismo que o
//! Android Auto usa — e ele só enxerga o que virou notificação. Sem histórico,
//! sem mensagem já lida no celular, e responder só enquanto a notificação viver.
//!
//! Além disso, WhatsApp é uma conta por aparelho: isto **não** troca junto com o
//! perfil. Por perfil dá para filtrar contatos e escolher se as notificações
//! aparecem, e é só.

pub mod inbox;
pub mod mock;
pub mod source;

pub use inbox::{Autor, Conversation, Inbox, IncomingMessage, Message};
pub use mock::MockMessageSource;
pub use source::{MessageSource, MessagingError};
