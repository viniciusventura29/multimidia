use async_trait::async_trait;

use crate::inbox::IncomingMessage;

#[derive(Debug, thiserror::Error)]
pub enum MessagingError {
    /// No Android, ler notificação exige uma permissão especial que o usuário
    /// concede à mão em Configurações — não é permissão de runtime comum.
    #[error("o acesso a notificações não foi concedido")]
    SemPermissao,

    /// A notificação foi dispensada e o `RemoteInput` foi junto.
    #[error("essa conversa não aceita mais resposta")]
    RespostaExpirada,

    #[error("falha ao enviar: {0}")]
    Envio(String),
}

/// De onde vêm as mensagens.
///
/// Hoje só existe `SemFonte`. A implementação real é um plugin Tauri com um
/// `NotificationListenerService` em Kotlin — por isso está atrás de um trait
/// desde já.
#[async_trait]
pub trait MessageSource: Send {
    /// Espera a próxima mensagem. `None` encerra a fonte.
    async fn next_message(&mut self) -> Option<IncomingMessage>;

    async fn reply(&mut self, conversa: &str, texto: &str) -> Result<(), MessagingError>;
}

/// Fonte nenhuma: a caixa fica vazia.
///
/// É a única enquanto a fonte de verdade (o `NotificationListenerService`)
/// não existe. Havia um mock com conversas inventadas a cada 12 s, e ele ia
/// para o APK também: na central, em 10/11/2026, apareceu "Ana: tô te
/// esperando aqui" por cima do painel, na tela de quem dirige. Saiu.
pub struct SemFonte;

#[async_trait]
impl MessageSource for SemFonte {
    async fn next_message(&mut self) -> Option<IncomingMessage> {
        None
    }

    async fn reply(&mut self, _conversa: &str, _texto: &str) -> Result<(), MessagingError> {
        Err(MessagingError::Envio(
            "ainda não há fonte de mensagens no carro".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sem_fonte_nao_inventa_mensagem() {
        let mut f = SemFonte;
        assert!(f.next_message().await.is_none());
        assert!(f.reply("Ana", "oi").await.is_err());
    }
}
