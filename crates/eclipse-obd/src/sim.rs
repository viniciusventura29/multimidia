//! A fonte simulada.
//!
//! O movimento do carro **não** mora aqui: vem do `eclipse-sim`, que é lido também
//! pelo GPS. Assim o mapa anda quando o motor acelera, em vez de cada simulador
//! contar a sua própria história.
//!
//! O que é do motor — temperatura, combustível, fluxo de ar — continua sendo
//! acumulado aqui, porque são coisas que o motor faz, não o trajeto.
//!
//! Existe para o painel poder ser visto e mexido no Mac. Bluetooth clássico só há no
//! Android, e conferir a tela do carro dirigindo com o laptop na mão não é opção.

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
/// É daqui que sai o dente de serra: dentro da marcha o RPM sobe com a velocidade, e
/// na troca ele despenca. Qualquer motorista reconhece o desenho, e é o melhor teste
/// visual que o painel tem.
const RELACAO: [f32; 5] = [135.0, 78.0, 52.0, 38.0, 28.0];
/// Velocidades em que o câmbio sobe de marcha.
const TROCAS: [f32; 4] = [25.0, 45.0, 70.0, 95.0];

const MARCHA_LENTA_RPM: f32 = 820.0;
const TEMPERATURA_AMBIENTE: f32 = 24.0;
const TEMPERATURA_OPERACAO: f32 = 88.0;
/// Aproximação exponencial do alvo: leva uns 90 s para o motor esquentar.
const AQUECIMENTO: f32 = 0.0116;
const TANQUE_LITROS: f32 = 61.0;
const COMBUSTIVEL_INICIAL_PCT: f32 = 62.0;
const AR_ADMITIDO_C: f32 = 28.0;

/// Massa de ar por litro de combustível queimado, em g — AFR × densidade.
///
/// Fecha o círculo com o [`crate::consumo`]: o simulador converte a vazão que
/// inventou em massa de ar, e a conta do painel a reconstrói. Assim um erro de
/// unidade em qualquer um dos dois lados aparece na tela em vez de passar batido.
const AR_POR_LITRO_G: f32 = 13.2 * 750.0;

fn marcha_para(speed: f32) -> usize {
    TROCAS.iter().filter(|&&troca| speed >= troca).count()
}

fn rpm_para(speed: f32) -> f32 {
    (speed * RELACAO[marcha_para(speed)]).max(MARCHA_LENTA_RPM)
}

/// Vazão de combustível de mentira, em L/h.
///
/// Marcha lenta perto de 1,3 L/h e cruzeiro a 104 km/h perto de 7 L/h — que dá uns
/// 15 km/l, otimista para um 3.0 V6 mas na ordem de grandeza certa.
fn vazao_lh(speed: f32) -> f32 {
    let rpm = rpm_para(speed);
    0.8 + rpm / 1000.0 * 0.6 + (speed / 100.0).powi(2) * 4.0
}

#[derive(Clone)]
pub struct SimulatedSource {
    /// Segundos desde a partida. A velocidade sai daqui, não de estado próprio: é o
    /// que mantém o OBD e o GPS falando do mesmo carro.
    t: f32,
    speed: f32,
    /// Temperatura e combustível o motor acumula sozinho, então continuam aqui.
    coolant: f32,
    fuel: f32,
    /// PIDs que este carro de mentira se recusa a responder.
    ///
    /// Serve para ver no Mac como o painel se comporta num carro que não tem MAF, ou
    /// que não informa o nível do tanque — que é o mais provável no Eclipse de 2000.
    /// Ligue com `ECLIPSE_SIM_SEM=maf,nivel`.
    sem: Vec<Pid>,
}

impl Default for SimulatedSource {
    fn default() -> Self {
        Self {
            t: 0.0,
            speed: 0.0,
            coolant: TEMPERATURA_AMBIENTE,
            fuel: COMBUSTIVEL_INICIAL_PCT,
            sem: sem_do_ambiente(),
        }
    }
}

/// Lê `ECLIPSE_SIM_SEM` — nomes separados por vírgula: `maf`, `nivel`, `carga`,
/// `coletor`, `vazao`.
fn sem_do_ambiente() -> Vec<Pid> {
    let Ok(lista) = std::env::var("ECLIPSE_SIM_SEM") else {
        // Por padrão o carro de mentira não informa vazão de combustível, como
        // nenhum carro de 2000 informa.
        return vec![Pid::VazaoComb];
    };

    let mut sem = Vec::new();
    for nome in lista.split(',') {
        match nome.trim().to_ascii_lowercase().as_str() {
            "maf" => sem.push(Pid::Maf),
            "nivel" | "fuel" => sem.push(Pid::Fuel),
            "carga" => sem.push(Pid::Carga),
            "coletor" | "map" => sem.push(Pid::Map),
            "iat" => sem.push(Pid::Iat),
            "vazao" => sem.push(Pid::VazaoComb),
            outro => tracing::warn!(outro, "ECLIPSE_SIM_SEM não conhece esse PID"),
        }
    }
    sem
}

impl SimulatedSource {
    /// Recebe a velocidade em vez de buscá-la: em produção vem do relógio
    /// compartilhado com o GPS, no teste vem de um tempo que o teste controla.
    fn avancar(&mut self, velocidade: f32) {
        self.t += DT;
        self.speed = velocidade;

        self.coolant += (TEMPERATURA_OPERACAO - self.coolant) * AQUECIMENTO;

        let gasto_pct = vazao_lh(self.speed) / 3600.0 * DT / TANQUE_LITROS * 100.0;
        self.fuel = (self.fuel - gasto_pct).max(0.0);
    }

    /// Alternador carregando: sobe um pouco com a rotação.
    fn voltagem(&self) -> f32 {
        13.8 + ((rpm_para(self.speed) - MARCHA_LENTA_RPM) / 4000.0 * 0.45).clamp(0.0, 0.45)
    }

    fn maf_gs(&self) -> f32 {
        vazao_lh(self.speed) * AR_POR_LITRO_G / 3600.0
    }

    /// Carga calculada: fluxo de ar atual sobre o máximo teórico desta rotação.
    fn carga_pct(&self) -> f32 {
        let maximo = rpm_para(self.speed) / 120.0 * 3.0 * 1.184;
        (self.maf_gs() / maximo * 100.0).clamp(2.0, 100.0)
    }
}

#[async_trait]
impl ObdSource for SimulatedSource {
    async fn read(&mut self, pid: Pid) -> Result<f32, ObdError> {
        // A espera é parte do comportamento, não um detalhe de implementação: é ela
        // que faz a UI ser construída contra a lentidão do carro de verdade.
        tokio::time::sleep(PID_ROUNDTRIP).await;
        // O relógio é compartilhado com o GPS: é o que mantém os dois falando do
        // mesmo carro mesmo que um deles tenha reiniciado no meio do caminho.
        self.avancar(eclipse_sim::velocidade_agora());

        if self.sem.contains(&pid) {
            return Err(ObdError::Unsupported);
        }

        Ok(match pid {
            Pid::Rpm => rpm_para(self.speed),
            Pid::Speed => self.speed,
            Pid::Coolant => self.coolant,
            Pid::Fuel => self.fuel,
            Pid::Voltage => self.voltagem(),
            Pid::Maf => self.maf_gs(),
            Pid::Carga => self.carga_pct(),
            // Coletor: fechado em marcha lenta, quase atmosférico em plena carga.
            Pid::Map => 25.0 + self.carga_pct() * 0.65,
            Pid::Iat => AR_ADMITIDO_C,
            Pid::VazaoComb => vazao_lh(self.speed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consumo::{vazao_lh as calcular, MetodoFluxo};
    use crate::pid::Readings;
    use crate::veiculo::Veiculo;

    /// Roda o trajeto por `n` leituras, devolvendo o estado a cada passo.
    fn rodar(n: usize) -> Vec<SimulatedSource> {
        let mut fonte = SimulatedSource {
            sem: Vec::new(),
            ..SimulatedSource::default()
        };
        (1..=n)
            .map(|i| {
                fonte.avancar(eclipse_sim::velocidade_kmh(i as f32 * DT));
                fonte.clone()
            })
            .collect()
    }

    #[test]
    fn o_motor_esquenta_e_o_tanque_baixa() {
        let passos = rodar(1_000);
        let fim = passos.last().unwrap();
        assert!(fim.coolant > 80.0, "temperatura {}", fim.coolant);
        assert!(fim.coolant <= TEMPERATURA_OPERACAO);
        assert!(fim.fuel < COMBUSTIVEL_INICIAL_PCT);
    }

    #[test]
    fn a_conta_do_painel_reconstroi_a_vazao_que_o_simulador_inventou() {
        // Se o simulador e o cálculo discordarem, um erro de unidade em qualquer um
        // dos dois aparece aqui em vez de virar um km/l esquisito na tela.
        let fonte = rodar(200).pop().unwrap();
        let r = Readings {
            rpm: Some(rpm_para(fonte.speed) as u32),
            speed_kmh: Some(fonte.speed as u32),
            maf_gs: Some(fonte.maf_gs()),
            ..Readings::default()
        };

        let calculado = calcular(MetodoFluxo::Maf, &r, &Veiculo::default()).unwrap();
        let simulado = vazao_lh(fonte.speed);
        assert!(
            (calculado - simulado).abs() < 0.05,
            "painel calculou {calculado} L/h e o simulador queimou {simulado}"
        );
    }

    #[test]
    fn a_carga_calculada_leva_a_mesma_vizinhanca_que_o_maf() {
        let fonte = rodar(200).pop().unwrap();
        let r = Readings {
            rpm: Some(rpm_para(fonte.speed) as u32),
            speed_kmh: Some(fonte.speed as u32),
            carga_pct: Some(fonte.carga_pct() as u8),
            ..Readings::default()
        };

        let por_carga = calcular(MetodoFluxo::Carga, &r, &Veiculo::default()).unwrap();
        let simulado = vazao_lh(fonte.speed);
        assert!(
            (por_carga - simulado).abs() < simulado * 0.25,
            "por carga {por_carga} L/h contra {simulado} do simulador"
        );
    }

    #[tokio::test]
    async fn o_carro_de_mentira_pode_esconder_pids() {
        let mut fonte = SimulatedSource {
            sem: vec![Pid::Maf, Pid::Fuel],
            ..SimulatedSource::default()
        };

        assert!(matches!(
            fonte.read(Pid::Maf).await,
            Err(ObdError::Unsupported)
        ));
        assert!(matches!(
            fonte.read(Pid::Fuel).await,
            Err(ObdError::Unsupported)
        ));
        assert!(fonte.read(Pid::Rpm).await.is_ok());
    }
}
