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
