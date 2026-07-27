//! O trajeto simulado.
//!
//! Determinístico de propósito: o estado é função das leituras já feitas, sem
//! aleatoriedade. Isso deixa o comportamento testável e faz o painel se
//! comportar igual em toda execução, o que ajuda a enxergar regressão de layout.

use std::f32::consts::TAU;
use std::time::Duration;

use async_trait::async_trait;

use crate::pid::Pid;
use crate::source::{ObdError, ObdSource};

/// Custo de um round-trip no barramento ISO 9141-2 do Eclipse.
pub const PID_ROUNDTRIP: Duration = Duration::from_millis(300);

/// Segundos de mundo que passam a cada leitura.
const DT: f32 = 0.3;

/// Relação rpm por km/h em cada marcha, do câmbio de cinco do GT.
///
/// É daqui que sai o dente de serra: dentro da marcha o RPM sobe com a
/// velocidade, e na troca ele despenca. Qualquer motorista reconhece o desenho,
/// e é o melhor teste visual que o painel tem.
const RELACAO: [f32; 5] = [135.0, 78.0, 52.0, 38.0, 28.0];
/// Velocidades em que o câmbio sobe de marcha.
const TROCAS: [f32; 4] = [25.0, 45.0, 70.0, 95.0];

const MARCHA_LENTA_RPM: f32 = 820.0;
const TEMPERATURA_AMBIENTE: f32 = 24.0;
const TEMPERATURA_OPERACAO: f32 = 88.0;
/// Aproximação exponencial do alvo: leva uns 90 s para o motor esquentar.
const AQUECIMENTO: f32 = 0.0116;
const TANQUE_LITROS: f32 = 60.0;
const COMBUSTIVEL_INICIAL: f32 = 62.0;
/// Acima da última troca, para o cruzeiro acontecer em quinta.
const VELOCIDADE_CRUZEIRO: f32 = 104.0;

const TICKS_PARADO: u64 = 27; // ~8 s
const TICKS_CRUZEIRO: u64 = 84; // ~25 s

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fase {
    MarchaLenta,
    Acelerando,
    Cruzeiro,
    Freando,
}

fn marcha_para(speed: f32) -> usize {
    TROCAS.iter().filter(|&&troca| speed >= troca).count()
}

fn rpm_para(speed: f32) -> f32 {
    (speed * RELACAO[marcha_para(speed)]).max(MARCHA_LENTA_RPM)
}

#[derive(Clone)]
pub struct SimulatedSource {
    fase: Fase,
    fase_tick: u64,
    speed: f32,
    coolant: f32,
    fuel: f32,
}

impl Default for SimulatedSource {
    fn default() -> Self {
        Self {
            fase: Fase::MarchaLenta,
            fase_tick: 0,
            speed: 0.0,
            coolant: TEMPERATURA_AMBIENTE,
            fuel: COMBUSTIVEL_INICIAL,
        }
    }
}

impl SimulatedSource {
    fn mudar(&mut self, fase: Fase) {
        self.fase = fase;
        self.fase_tick = 0;
    }

    fn avancar(&mut self) {
        self.fase_tick += 1;

        match self.fase {
            Fase::MarchaLenta => {
                self.speed = 0.0;
                if self.fase_tick >= TICKS_PARADO {
                    self.mudar(Fase::Acelerando);
                }
            }
            Fase::Acelerando => {
                // A aceleração cai conforme a velocidade sobe, como no carro real.
                let acel = (9.0 - self.speed * 0.06).max(2.0);
                self.speed = (self.speed + acel * DT).min(VELOCIDADE_CRUZEIRO);
                if self.speed >= VELOCIDADE_CRUZEIRO - 0.1 {
                    self.mudar(Fase::Cruzeiro);
                }
            }
            Fase::Cruzeiro => {
                // Variação leve de acelerador, para o painel não ficar estático.
                let onda = (self.fase_tick as f32 / 40.0 * TAU).sin();
                self.speed = VELOCIDADE_CRUZEIRO + onda * 3.0;
                if self.fase_tick >= TICKS_CRUZEIRO {
                    self.mudar(Fase::Freando);
                }
            }
            Fase::Freando => {
                self.speed = (self.speed - 12.0 * DT).max(0.0);
                if self.speed <= 0.0 {
                    self.mudar(Fase::MarchaLenta);
                }
            }
        }

        self.coolant += (TEMPERATURA_OPERACAO - self.coolant) * AQUECIMENTO;

        let litros_por_hora = 0.9 + rpm_para(self.speed) / 1000.0 * 2.2;
        let gasto_pct = litros_por_hora / 3600.0 * DT / TANQUE_LITROS * 100.0;
        self.fuel = (self.fuel - gasto_pct).max(0.0);
    }

    /// Alternador carregando: sobe um pouco com a rotação.
    fn voltagem(&self) -> f32 {
        13.8 + ((rpm_para(self.speed) - MARCHA_LENTA_RPM) / 4000.0 * 0.45).clamp(0.0, 0.45)
    }
}

#[async_trait]
impl ObdSource for SimulatedSource {
    async fn read(&mut self, pid: Pid) -> Result<f32, ObdError> {
        // A espera é parte do comportamento, não um detalhe de implementação:
        // é ela que faz a UI ser construída contra a lentidão do carro de verdade.
        tokio::time::sleep(PID_ROUNDTRIP).await;
        self.avancar();

        Ok(match pid {
            Pid::Rpm => rpm_para(self.speed),
            Pid::Speed => self.speed,
            Pid::Coolant => self.coolant,
            Pid::Fuel => self.fuel,
            Pid::Voltage => self.voltagem(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roda o trajeto por `n` leituras, devolvendo o estado a cada passo.
    fn rodar(n: usize) -> Vec<SimulatedSource> {
        let mut fonte = SimulatedSource::default();
        (0..n)
            .map(|_| {
                fonte.avancar();
                fonte.clone()
            })
            .collect()
    }

    #[test]
    fn motor_nunca_desce_da_marcha_lenta() {
        for estado in rodar(800) {
            assert!(
                rpm_para(estado.speed) >= MARCHA_LENTA_RPM,
                "rpm {} abaixo da marcha lenta a {} km/h",
                rpm_para(estado.speed),
                estado.speed
            );
        }
    }

    /// O dente de serra: subir de marcha tem que derrubar o RPM.
    #[test]
    fn trocar_de_marcha_derruba_o_rpm() {
        for &troca in &TROCAS {
            let antes = rpm_para(troca - 0.1);
            let depois = rpm_para(troca);

            assert!(
                depois < antes,
                "a {troca} km/h o rpm subiu de {antes} para {depois} — a troca não derrubou"
            );
        }
    }

    #[test]
    fn rpm_sobe_com_a_velocidade_dentro_da_mesma_marcha() {
        // Faixa inteira dentro da terceira marcha.
        assert!(rpm_para(60.0) > rpm_para(50.0));
        assert_eq!(marcha_para(50.0), marcha_para(60.0));
    }

    #[test]
    fn motor_esquenta_ate_a_temperatura_de_operacao_e_para_la() {
        let estados = rodar(1200);

        // ~90 s são 300 leituras. Aí já tem que estar quase no ponto.
        let aos_90s = estados[299].coolant;
        assert!(
            (80.0..=88.0).contains(&aos_90s),
            "aos 90 s o motor estava a {aos_90s} °C"
        );

        for estado in &estados {
            assert!(
                estado.coolant >= TEMPERATURA_AMBIENTE - 0.1
                    && estado.coolant <= TEMPERATURA_OPERACAO + 0.1,
                "temperatura fora da faixa: {}",
                estado.coolant
            );
        }
    }

    #[test]
    fn combustivel_so_cai() {
        let estados = rodar(600);
        for par in estados.windows(2) {
            assert!(par[1].fuel <= par[0].fuel, "o tanque encheu sozinho");
        }
        assert!(
            estados.last().unwrap().fuel < COMBUSTIVEL_INICIAL,
            "o consumo não apareceu"
        );
    }

    /// Marcha lenta → aceleração → cruzeiro → frenagem → marcha lenta, sem travar.
    #[test]
    fn o_trajeto_fecha_o_ciclo() {
        let estados = rodar(500);
        let velocidades: Vec<f32> = estados.iter().map(|e| e.speed).collect();

        assert!(
            velocidades.iter().any(|&v| v > 100.0),
            "o carro nunca chegou ao cruzeiro"
        );
        assert!(
            velocidades.iter().skip(200).any(|&v| v == 0.0),
            "o carro nunca voltou a parar"
        );
        assert!(
            velocidades.iter().all(|&v| v >= 0.0),
            "velocidade negativa"
        );

        let fases: Vec<Fase> = estados.iter().map(|e| e.fase).collect();
        for fase in [Fase::Acelerando, Fase::Cruzeiro, Fase::Freando] {
            assert!(fases.contains(&fase), "a fase {fase:?} nunca aconteceu");
        }
    }

    #[test]
    fn voltagem_fica_na_faixa_do_alternador() {
        for estado in rodar(600) {
            let v = estado.voltagem();
            assert!((13.8..=14.25).contains(&v), "voltagem estranha: {v}");
        }
    }
}
