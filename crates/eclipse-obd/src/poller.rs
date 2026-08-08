use std::collections::{HashMap, HashSet};

use crate::capacidades::Capacidades;
use crate::consumo::MetodoFluxo;
use crate::pid::{Pid, Readings};
use crate::source::{ObdError, ObdSource};

/// Quantos `NO DATA` seguidos bastam para tirar um PID da roda.
///
/// Três e não um: a ECU pode responder `NO DATA` nos primeiros instantes depois da
/// ignição, ainda acordando, e desistir na primeira negativa condenaria um sensor que
/// funciona. Três também não é caro — são ~1 s de barramento.
const FALTAS_PARA_DESISTIR: u8 = 3;

/// Os PIDs que sempre valem a pena, na ordem de prioridade.
///
/// A voltagem entra na lista dos lentos e **não** passa pela máscara: `ATRV` é uma
/// medida do próprio adaptador, não um PID do carro, e não aparece em máscara nenhuma.
const RAPIDOS_BASE: [Pid; 2] = [Pid::Rpm, Pid::Speed];
const LENTOS_BASE: [Pid; 3] = [Pid::Coolant, Pid::Fuel, Pid::Voltage];

/// A ordem em que os PIDs são varridos.
///
/// Um ciclo é: todos os rápidos, e **um** lento. Assim RPM, velocidade e a fonte de ar
/// aparecem em todo ciclo — porque mudam rápido, e porque a fonte de ar é integrada em
/// litros, então amostrá-la devagar não atrasa o número, erra a conta — enquanto
/// temperatura, nível e tensão se revezam num slot só.
///
/// A ~300 ms por leitura, com três rápidos: rápidos a cada ~1,2 s, lentos a cada
/// ~4,8 s. Sem a fonte de ar seriam ~0,9 s e ~2,7 s: é esse o preço do consumo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plano {
    rapidos: Vec<Pid>,
    lentos: Vec<Pid>,
}

impl Plano {
    /// Monta a varredura com o que este carro responde e o método de consumo escolhido.
    ///
    /// `sem_codigo_recusados` são os PIDs que não têm número no modo 01 (hoje só a
    /// voltagem, que é `ATRV` do adaptador) e que já se recusaram na prática. Eles
    /// não cabem na máscara de [`Capacidades`], então quem lembra deles é o poller.
    pub fn montar(
        capacidades: Capacidades,
        metodo: MetodoFluxo,
        sem_codigo_recusados: &HashSet<Pid>,
    ) -> Self {
        let vale = |pid: Pid| match pid.codigo() {
            Some(codigo) => capacidades.suporta(codigo),
            // Sem código não passa pela máscara — é do adaptador. Vale enquanto
            // ele responder: um `ATRV` que nunca volta custaria um slot lento por
            // ciclo, para sempre, e nunca sairia da roda pela máscara.
            None => !sem_codigo_recusados.contains(&pid),
        };

        let mut rapidos: Vec<Pid> = RAPIDOS_BASE.into_iter().filter(|p| vale(*p)).collect();
        if let Some(ar) = metodo.pid_de_ar().filter(|p| vale(*p)) {
            rapidos.push(ar);
        }

        let mut lentos: Vec<Pid> = LENTOS_BASE.into_iter().filter(|p| vale(*p)).collect();
        lentos.extend(metodo.pids_lentos().iter().copied().filter(|p| vale(*p)));

        // Um plano vazio faria o poller girar em falso sem nunca ler nada, e o
        // supervisor não teria erro nenhum para reagir. Se sobrou só o adaptador,
        // insiste no RPM: sem ele não há painel.
        if rapidos.is_empty() && lentos.is_empty() {
            rapidos.push(Pid::Rpm);
        }

        Self { rapidos, lentos }
    }

    /// Quantas leituras tem um ciclo completo.
    fn tamanho(&self) -> usize {
        self.rapidos.len() + usize::from(!self.lentos.is_empty())
    }

    /// Qual PID cai na posição `tick` da varredura.
    fn em(&self, tick: usize) -> Pid {
        let tamanho = self.tamanho();
        let passo = tick % tamanho;
        match self.rapidos.get(passo) {
            Some(pid) => *pid,
            // Passou dos rápidos: é o slot do lento, que gira a cada ciclo.
            None => self.lentos[(tick / tamanho) % self.lentos.len()],
        }
    }

    pub fn rapidos(&self) -> &[Pid] {
        &self.rapidos
    }

    pub fn lentos(&self) -> &[Pid] {
        &self.lentos
    }
}

/// Varre os PIDs em ordem e acumula o que já foi lido.
///
/// Esta parte é a mesma para o simulador e para o ELM327 — só a fonte muda.
pub struct Poller<S> {
    source: S,
    readings: Readings,
    tick: usize,
    capacidades: Capacidades,
    plano: Plano,
    faltas: HashMap<Pid, u8>,
    /// Quem desistiu de responder mas não tem lugar na máscara — ver
    /// [`Plano::montar`].
    sem_codigo_recusados: HashSet<Pid>,
    /// A varredura mudou desde a última vez que alguém perguntou.
    replanejou: bool,
}

impl<S: ObdSource> Poller<S> {
    /// Um poller que ainda não sabe o que o carro responde: pergunta tudo.
    pub fn new(source: S) -> Self {
        Self::com_capacidades(source, Capacidades::otimista())
    }

    pub fn com_capacidades(source: S, capacidades: Capacidades) -> Self {
        let sem_codigo_recusados = HashSet::new();
        let plano = Plano::montar(
            capacidades,
            MetodoFluxo::escolher(capacidades),
            &sem_codigo_recusados,
        );
        Self {
            source,
            readings: Readings::default(),
            tick: 0,
            capacidades,
            plano,
            faltas: HashMap::new(),
            sem_codigo_recusados,
            replanejou: true,
        }
    }

    /// Qual PID a próxima chamada de [`Self::step`] vai ler.
    pub fn proximo(&self) -> Pid {
        self.plano.em(self.tick)
    }

    /// Lê um PID e devolve o conjunto atualizado.
    ///
    /// Um PID que o carro não suporta não derruba a varredura: ele fica vazio e a
    /// roda segue girando. Carro velho não responde tudo, e perder a temperatura não
    /// é motivo para perder o RPM junto.
    ///
    /// Depois de algumas negativas seguidas o PID sai da roda de vez. Num barramento
    /// de 10.400 baud, insistir num PID que nunca responde é roubar uma leitura de
    /// RPM a cada ciclo, para sempre.
    pub async fn step(&mut self) -> Result<&Readings, ObdError> {
        let pid = self.proximo();
        self.tick = self.tick.wrapping_add(1);

        match self.source.read(pid).await {
            Ok(valor) => {
                self.readings.apply(pid, valor);
                self.faltas.remove(&pid);
                // Respondeu: tem. Vale mais que a máscara, que às vezes mente por
                // omissão — e é o que pode ser guardado em disco com segurança.
                if let Some(codigo) = pid.codigo() {
                    if !self.capacidades.suporta(codigo) {
                        self.capacidades.marcar(codigo);
                        self.replanejar();
                    }
                }
            }
            Err(ObdError::Unsupported) => {
                let faltas = self.faltas.entry(pid).or_default();
                *faltas += 1;
                if *faltas >= FALTAS_PARA_DESISTIR {
                    tracing::info!(?pid, "não responde este PID; saindo da roda");
                    match pid.codigo() {
                        Some(codigo) => self.capacidades.recusar(codigo),
                        // A voltagem não tem lugar na máscara, e sem esta lista
                        // ela ficava sendo pedida para sempre — um slot lento por
                        // ciclo gasto num `ATRV` que o adaptador não responde.
                        None => {
                            self.sem_codigo_recusados.insert(pid);
                        }
                    }
                    self.replanejar();
                }
            }
            Err(err) => return Err(err),
        }

        Ok(&self.readings)
    }

    fn replanejar(&mut self) {
        let novo = Plano::montar(
            self.capacidades,
            MetodoFluxo::escolher(self.capacidades),
            &self.sem_codigo_recusados,
        );
        if novo != self.plano {
            tracing::info!(rapidos = ?novo.rapidos, lentos = ?novo.lentos, "varredura remontada");
            self.plano = novo;
            // Recomeça o ciclo: continuar do tick antigo num plano de tamanho
            // diferente pularia PIDs de um jeito difícil de raciocinar.
            self.tick = 0;
            self.replanejou = true;
        }
    }

    pub fn readings(&self) -> &Readings {
        &self.readings
    }

    pub fn capacidades(&self) -> Capacidades {
        self.capacidades
    }

    pub fn plano(&self) -> &Plano {
        &self.plano
    }

    /// A varredura mudou desde a última pergunta? Consome o aviso.
    ///
    /// Quem cuida do consumo precisa saber, porque a fonte de vazão pode ter mudado
    /// junto — é o que faz a cascata descer sozinha na estrada.
    pub fn replanejou(&mut self) -> bool {
        std::mem::take(&mut self.replanejou)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Capacidades de um carro que respondeu a máscara e tem só estes PIDs.
    fn cap_com(pids: &[u8]) -> Capacidades {
        let mut c = Capacidades::otimista();
        for base in [0x00, 0x20, 0x40] {
            c.juntar(base, &[0, 0, 0, 0]);
        }
        for pid in pids {
            c.marcar(*pid);
        }
        c
    }

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

    /// Um carro sem MAF, sem coletor e sem vazão: sobra a carga calculada.
    fn carro_com_carga() -> Capacidades {
        cap_com(&[0x04, 0x05, 0x0C, 0x0D, 0x2F])
    }

    #[tokio::test]
    async fn pids_rapidos_sao_lidos_uma_vez_por_ciclo_e_os_lentos_se_revezam() {
        let cap = carro_com_carga();
        let mut poller = Poller::com_capacidades(Contadora::default(), cap);
        let ciclo = poller.plano().tamanho();
        // Rápidos: RPM, velocidade, carga. Lentos: temperatura, nível, tensão.
        assert_eq!(ciclo, 4);

        for _ in 0..ciclo * 3 {
            poller.step().await.unwrap();
        }

        let vistos = &poller.source.vistos;
        assert_eq!(vistos[&Pid::Rpm], 3);
        assert_eq!(vistos[&Pid::Speed], 3);
        assert_eq!(
            vistos[&Pid::Carga],
            3,
            "a fonte de ar é integrada: vai junto"
        );
        assert_eq!(vistos[&Pid::Coolant], 1);
        assert_eq!(vistos[&Pid::Fuel], 1);
        assert_eq!(vistos[&Pid::Voltage], 1);
    }

    #[tokio::test]
    async fn o_que_o_carro_nao_anuncia_nao_e_pedido() {
        let mut poller = Poller::com_capacidades(Contadora::default(), carro_com_carga());
        for _ in 0..40 {
            poller.step().await.unwrap();
        }

        // O MAF não está na máscara deste carro: nem uma leitura desperdiçada nele.
        assert!(!poller.source.vistos.contains_key(&Pid::Maf));
        assert!(!poller.source.vistos.contains_key(&Pid::Map));
    }

    /// Carro velho não responde todo PID. Perder a temperatura não pode custar o RPM.
    #[tokio::test]
    async fn pid_nao_suportado_nao_interrompe_a_varredura() {
        let mut poller = Poller::com_capacidades(
            Contadora {
                nao_suportado: Some(Pid::Fuel),
                ..Default::default()
            },
            carro_com_carga(),
        );

        for _ in 0..poller.plano().tamanho() * 3 {
            poller.step().await.expect("varredura não pode parar");
        }

        let leituras = poller.readings();
        assert!(leituras.fuel_pct.is_none(), "o PID sem suporte fica vazio");
        assert!(leituras.rpm.is_some(), "os outros continuam sendo lidos");
        assert!(leituras.coolant_c.is_some());
    }

    #[tokio::test]
    async fn pid_que_insiste_em_nao_responder_sai_da_roda() {
        let mut poller = Poller::com_capacidades(
            Contadora {
                nao_suportado: Some(Pid::Fuel),
                ..Default::default()
            },
            carro_com_carga(),
        );

        // Roda o bastante para as três faltas acontecerem e o plano ser remontado.
        for _ in 0..40 {
            poller.step().await.unwrap();
        }
        let tentativas_ate_desistir = poller.source.vistos[&Pid::Fuel];
        assert_eq!(
            tentativas_ate_desistir, FALTAS_PARA_DESISTIR as usize,
            "insistir num PID morto custa uma leitura de RPM por ciclo, para sempre"
        );

        for _ in 0..20 {
            poller.step().await.unwrap();
        }
        assert_eq!(
            poller.source.vistos[&Pid::Fuel],
            tentativas_ate_desistir,
            "e não volta a ser pedido"
        );
    }

    #[tokio::test]
    async fn quem_responde_sem_estar_na_mascara_entra_na_roda() {
        // Máscara mentirosa: não anuncia o MAF, mas a ECU responde.
        let mut poller = Poller::com_capacidades(Contadora::default(), Capacidades::otimista());
        poller.step().await.unwrap();

        assert!(
            poller.capacidades().suporta(Pid::Maf.codigo().unwrap()),
            "começou otimista, então o MAF é pedido e a resposta o confirma"
        );
    }

    #[tokio::test]
    async fn a_cascata_de_consumo_desce_sozinha_quando_o_maf_nao_responde() {
        // Carro que anuncia MAF e carga, mas na prática só entrega carga.
        let mut poller = Poller::com_capacidades(
            Contadora {
                nao_suportado: Some(Pid::Maf),
                ..Default::default()
            },
            cap_com(&[0x04, 0x05, 0x0C, 0x0D, 0x10]),
        );
        assert!(poller.plano().rapidos().contains(&Pid::Maf));

        for _ in 0..40 {
            poller.step().await.unwrap();
        }

        assert!(
            poller.plano().rapidos().contains(&Pid::Carga),
            "a fonte de ar virou a carga calculada sem ninguém mandar"
        );
        assert!(!poller.plano().rapidos().contains(&Pid::Maf));
        assert!(poller.replanejou(), "quem cuida do consumo precisa saber");
    }

    #[tokio::test]
    async fn a_tensao_da_bateria_nunca_sai_da_roda_por_causa_da_mascara() {
        // Máscara vazia: o carro não anuncia PID nenhum. A tensão vem do adaptador,
        // então continua legível — é o que mantém a bateria no header.
        let poller = Poller::com_capacidades(Contadora::default(), cap_com(&[]));
        assert!(poller.plano().lentos().contains(&Pid::Voltage));
    }

    /// ...mas sai da roda quando o adaptador não responde `ATRV` na prática.
    ///
    /// A voltagem não tem número de PID, então não cabe na máscara — e por isso
    /// era o único que insistia para sempre. Num barramento de 10.400 baud, isso
    /// é um slot lento por ciclo queimado até desligar o carro.
    #[tokio::test]
    async fn a_tensao_sai_da_roda_quando_o_adaptador_nao_responde() {
        let mut poller = Poller::com_capacidades(
            Contadora {
                nao_suportado: Some(Pid::Voltage),
                ..Default::default()
            },
            carro_com_carga(),
        );

        for _ in 0..60 {
            poller.step().await.unwrap();
        }

        assert_eq!(
            poller.source.vistos[&Pid::Voltage],
            FALTAS_PARA_DESISTIR as usize,
            "o ATRV mudo continuou sendo pedido"
        );
        assert!(!poller.plano().lentos().contains(&Pid::Voltage));
        // E os outros lentos continuam girando normalmente.
        assert!(poller.plano().lentos().contains(&Pid::Coolant));
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
