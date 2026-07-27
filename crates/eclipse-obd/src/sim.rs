//! A fonte simulada.
//!
//! O movimento do carro **não** mora aqui: vem do `eclipse-sim`, que é lido
//! também pelo GPS. Assim o mapa anda quando o motor acelera, em vez de cada
//! simulador contar a sua própria história.
//!
//! O que é do motor — temperatura e consumo — continua sendo acumulado aqui,
//! porque são coisas que o motor faz, não o trajeto.

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

fn marcha_para(speed: f32) -> usize {
    TROCAS.iter().filter(|&&troca| speed >= troca).count()
}

fn rpm_para(speed: f32) -> f32 {
    (speed * RELACAO[marcha_para(speed)]).max(MARCHA_LENTA_RPM)
}

#[derive(Clone)]
pub struct SimulatedSource {
    /// Segundos desde a partida. A velocidade sai daqui, não de estado próprio:
    /// é o que mantém o OBD e o GPS falando do mesmo carro.
    t: f32,
    speed: f32,
    /// Temperatura e combustível o motor acumula sozinho, então continuam aqui.
    coolant: f32,
    fuel: f32,
}

impl Default for SimulatedSource {
    fn default() -> Self {
        Self {
            t: 0.0,
            speed: 0.0,
            coolant: TEMPERATURA_AMBIENTE,
            fuel: COMBUSTIVEL_INICIAL,
        }
    }
}

impl SimulatedSource {
    fn avancar(&mut self) {
        self.t += DT;
        self.speed = eclipse_sim::velocidade_kmh(self.t);

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

        for fase in [
            eclipse_sim::Fase::Acelerando,
            eclipse_sim::Fase::Cruzeiro,
            eclipse_sim::Fase::Freando,
        ] {
            assert!(
                estados.iter().any(|e| eclipse_sim::fase_em(e.t) == fase),
                "a fase {fase:?} nunca aconteceu"
            );
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
