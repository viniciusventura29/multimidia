use crate::pid::{Pid, Readings};
use crate::source::{ObdError, ObdSource};

/// A ordem de varredura.
///
/// RPM e velocidade aparecem em todo terço do ciclo porque mudam rápido; os
/// outros três se revezam, porque mudam devagar e cada leitura custa banda que
/// falta para os dois primeiros. Com ~300 ms por PID, dá RPM e velocidade a cada
/// ~0,9 s e o resto a cada ~2,7 s — que é o que o barramento do Eclipse entrega.
pub const SCHEDULE: [Pid; 9] = [
    Pid::Rpm,
    Pid::Speed,
    Pid::Coolant,
    Pid::Rpm,
    Pid::Speed,
    Pid::Fuel,
    Pid::Rpm,
    Pid::Speed,
    Pid::Voltage,
];

/// Varre os PIDs em ordem e acumula o que já foi lido.
///
/// Esta parte é a mesma para o simulador e para o ELM327 — só a fonte muda.
pub struct Poller<S> {
    source: S,
    readings: Readings,
    tick: usize,
}

impl<S: ObdSource> Poller<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            readings: Readings::default(),
            tick: 0,
        }
    }

    /// Qual PID a próxima chamada de [`Self::step`] vai ler.
    pub fn proximo(&self) -> Pid {
        SCHEDULE[self.tick % SCHEDULE.len()]
    }

    /// Lê um PID e devolve o conjunto atualizado.
    ///
    /// Um PID que o carro não suporta não derruba a varredura: ele fica vazio e
    /// a roda segue girando. Carro velho não responde tudo, e perder a
    /// temperatura não é motivo para perder o RPM junto.
    pub async fn step(&mut self) -> Result<&Readings, ObdError> {
        let pid = self.proximo();
        self.tick = self.tick.wrapping_add(1);

        match self.source.read(pid).await {
            Ok(valor) => self.readings.apply(pid, valor),
            Err(ObdError::Unsupported) => {}
            Err(err) => return Err(err),
        }

        Ok(&self.readings)
    }

    pub fn readings(&self) -> &Readings {
        &self.readings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Contadora {
        vistos: HashMap<Pid, usize>,
        nao_suportado: Option<Pid>,
    }

    #[async_trait]
    impl ObdSource for Contadora {
        async fn read(&mut self, pid: Pid) -> Result<f32, ObdError> {
            *self.vistos.entry(pid).or_default() += 1;
            if self.nao_suportado == Some(pid) {
                return Err(ObdError::Unsupported);
            }
            Ok(1.0)
        }
    }

    #[tokio::test]
    async fn pids_rapidos_sao_lidos_tres_vezes_mais() {
        let mut poller = Poller::new(Contadora::default());
        for _ in 0..SCHEDULE.len() * 4 {
            poller.step().await.unwrap();
        }

        let vistos = poller.source.vistos;
        assert_eq!(vistos[&Pid::Rpm], 12);
        assert_eq!(vistos[&Pid::Speed], 12);
        assert_eq!(vistos[&Pid::Coolant], 4);
        assert_eq!(vistos[&Pid::Fuel], 4);
        assert_eq!(vistos[&Pid::Voltage], 4);
    }

    /// Carro velho não responde todo PID. Perder a temperatura não pode custar o RPM.
    #[tokio::test]
    async fn pid_nao_suportado_nao_interrompe_a_varredura() {
        let mut poller = Poller::new(Contadora {
            nao_suportado: Some(Pid::Fuel),
            ..Default::default()
        });

        for _ in 0..SCHEDULE.len() {
            poller.step().await.expect("varredura não pode parar");
        }

        let leituras = poller.readings();
        assert!(leituras.fuel_pct.is_none(), "o PID sem suporte fica vazio");
        assert!(leituras.rpm.is_some(), "os outros continuam sendo lidos");
        assert!(leituras.coolant_c.is_some());
    }

    #[tokio::test]
    async fn falha_de_barramento_propaga() {
        struct Morta;

        #[async_trait]
        impl ObdSource for Morta {
            async fn read(&mut self, _pid: Pid) -> Result<f32, ObdError> {
                Err(ObdError::Timeout)
            }
        }

        let mut poller = Poller::new(Morta);
        assert!(poller.step().await.is_err());
    }
}
