use async_trait::async_trait;

use crate::pid::Pid;

#[derive(Debug, thiserror::Error)]
pub enum ObdError {
    #[error("o adaptador não respondeu")]
    Timeout,
    #[error("o carro não suporta este PID")]
    Unsupported,
    #[error("falha no barramento: {0}")]
    Bus(String),
    /// O canal com o adaptador caiu — não há resposta possível.
    ///
    /// Separado de [`ObdError::Timeout`] porque o tratamento é o oposto.
    /// Timeout é transitório: o ISO 9141-2 do Eclipse perde quadro, e repetir é
    /// o certo. Canal caído não tem repetição que resolva, e insistir custa um
    /// prazo cheio por PID antes de o poller desistir — era isso que derrubava
    /// o módulo de 32 em 32 segundos.
    #[error("o canal com o adaptador caiu: {0}")]
    LinkCaiu(String),
}

/// De onde vêm as leituras.
///
/// É este o trait que o ELM327 vai implementar. `read` **demora de propósito**:
/// no ISO 9141-2 do Eclipse cada PID é um round-trip de ~300 ms num barramento
/// de 10.400 baud, e essa lentidão faz parte do contrato — quem consome precisa
/// ser escrito para ela, não descobrir depois no carro.
#[async_trait]
pub trait ObdSource: Send {
    async fn read(&mut self, pid: Pid) -> Result<f32, ObdError>;
}
