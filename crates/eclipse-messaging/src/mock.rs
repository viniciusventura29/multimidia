//! Fonte de mentira, para a interface de ler e responder ficar pronta antes da
//! head unit chegar.
//!
//! Quando o hardware chegar, entra no lugar dela um `AndroidNotificationSource`
//! com um `NotificationListenerService` em Kotlin. Só a implementação do trait
//! muda — tela, módulo e testes ficam.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::inbox::IncomingMessage;
use crate::source::{MessagingError, MessageSource};

const ROTEIRO: [(&str, &str, &str); 5] = [
    ("Ana", "Ana", "oi, você já saiu?"),
    ("Ana", "Ana", "tô te esperando aqui"),
    ("Churrasco sábado", "Bruno", "levo a carne"),
    ("Churrasco sábado", "Carla", "eu levo a bebida então"),
    ("Ana", "Ana", "avisa quando estiver chegando"),
];

pub struct MockMessageSource {
    proxima: usize,
    intervalo: Duration,
}

impl MockMessageSource {
    pub fn new(intervalo: Duration) -> Self {
        Self {
            proxima: 0,
            intervalo,
        }
    }
}

impl Default for MockMessageSource {
    fn default() -> Self {
        Self::new(Duration::from_secs(12))
    }
}

#[async_trait]
impl MessageSource for MockMessageSource {
    async fn next_message(&mut self) -> Option<IncomingMessage> {
        let (conversation, sender, body) = *ROTEIRO.get(self.proxima)?;
        self.proxima += 1;

        tokio::time::sleep(self.intervalo).await;

        Some(IncomingMessage {
            conversation: conversation.to_string(),
            sender: sender.to_string(),
            body: body.to_string(),
            at: Utc::now(),
            can_reply: true,
        })
    }

    async fn reply(&mut self, _conversa: &str, _texto: &str) -> Result<(), MessagingError> {
        Ok(())
    }
}
