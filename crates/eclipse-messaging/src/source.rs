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
/// Hoje só existe o mock. A implementação real é um plugin Tauri com um
/// `NotificationListenerService` em Kotlin, e depende da head unit comprada —
/// por isso está atrás de um trait desde já.
#[async_trait]
pub trait MessageSource: Send {
    /// Espera a próxima mensagem. `None` encerra a fonte.
    async fn next_message(&mut self) -> Option<IncomingMessage>;

    async fn reply(&mut self, conversa: &str, texto: &str) -> Result<(), MessagingError>;
}
