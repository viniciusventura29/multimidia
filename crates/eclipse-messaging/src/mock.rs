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
use crate::source::{MessageSource, MessagingError};

/// `(conversa, remetente, corpo, foto)`.
///
/// A foto é uma URL de placeholder: o mock só roda em dev, onde o WebView tem
/// internet. Na fonte real ela virá do ícone grande da notificação do Android —
/// e quando não vier (`None`), a tela cai numa inicial. Fixamos `?u=<nome>` para
/// cada pessoa ter sempre o mesmo rosto entre as mensagens.
const ROTEIRO: [(&str, &str, &str, Option<&str>); 5] = [
    (
        "Ana",
        "Ana",
        "oi, você já saiu?",
        Some("https://i.pravatar.cc/150?u=ana"),
    ),
    (
        "Ana",
        "Ana",
        "tô te esperando aqui",
        Some("https://i.pravatar.cc/150?u=ana"),
    ),
    (
        "Churrasco sábado",
        "Bruno",
        "levo a carne",
        Some("https://i.pravatar.cc/150?u=bruno"),
    ),
    (
        "Churrasco sábado",
        "Carla",
        "eu levo a bebida então",
        Some("https://i.pravatar.cc/150?u=carla"),
    ),
    (
        "Ana",
        "Ana",
        "avisa quando estiver chegando",
        Some("https://i.pravatar.cc/150?u=ana"),
    ),
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
        let (conversation, sender, body, avatar) = *ROTEIRO.get(self.proxima)?;
        self.proxima += 1;

        tokio::time::sleep(self.intervalo).await;

        Some(IncomingMessage {
            conversation: conversation.to_string(),
            sender: sender.to_string(),
            body: body.to_string(),
            at: Utc::now(),
            can_reply: true,
            avatar: avatar.map(|u| u.to_string()),
        })
    }

    async fn reply(&mut self, _conversa: &str, _texto: &str) -> Result<(), MessagingError> {
        Ok(())
    }
}
