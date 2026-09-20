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

/// Diagnóstico: importa que exista, não que seja recente.
///
/// Estes NÃO entram na roda dos lentos, e o motivo é aritmético. A varredura lê
/// um lento por ciclo, então cada lento volta a cada `lentos.len()` ciclos —
/// somar cinco PIDs aos três de hoje faria a temperatura da água demorar 12 s
/// em vez de 4,5. Trocar a resposta dos mostradores que o motorista olha
/// dirigindo por um trim que muda de minuto em minuto seria um péssimo negócio.
///
/// Então eles têm um slot próprio, e raro: a cada [`CICLOS_POR_RARO`] ciclos, o
/// slot do lento é emprestado para um deles. Falha guardada, trim e sonda
/// respondem perguntas de oficina ("esse motor está saudável?"), e meio minuto
/// de atraso não muda nenhuma delas.
const RAROS_BASE: [Pid; 5] = [
    Pid::Falhas,
    Pid::TrimCurto,
    Pid::TrimLongo,
    Pid::Lambda1,
    Pid::Lambda2,
];

/// De quantos em quantos ciclos um PID raro rouba o slot do lento.
///
/// Oito ciclos é cerca de 12 s. Com cinco raros na roda, cada um volta a cada
/// minuto — e nenhum mostrador do painel perde mais que um oitavo da sua
/// cadência para isso.
const CICLOS_POR_RARO: usize = 8;

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
    raros: Vec<Pid>,
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

        let raros: Vec<Pid> = RAROS_BASE.into_iter().filter(|p| vale(*p)).collect();

        // Um plano vazio faria o poller girar em falso sem nunca ler nada, e o
        // supervisor não teria erro nenhum para reagir. Se sobrou só o adaptador,
        // insiste no RPM: sem ele não há painel.
        if rapidos.is_empty() && lentos.is_empty() {
            rapidos.push(Pid::Rpm);
        }

        Self {
            rapidos,
            lentos,
            raros,
        }
    }

    /// Quantas leituras tem um ciclo completo.
    fn tamanho(&self) -> usize {
        self.rapidos.len() + usize::from(!self.lentos.is_empty())
    }

    /// Qual PID cai na posição `tick` da varredura.
    fn em(&self, tick: usize) -> Pid {
        let tamanho = self.tamanho();
        let passo = tick % tamanho;
        let Some(pid) = self.rapidos.get(passo) else {
            // Passou dos rápidos: é o slot do lento, que gira a cada ciclo — e
            // que de vez em quando é emprestado para um raro.
            let ciclo = tick / tamanho;
            if !self.raros.is_empty() && ciclo % CICLOS_POR_RARO == CICLOS_POR_RARO - 1 {
                let volta = ciclo / CICLOS_POR_RARO;
                return self.raros[volta % self.raros.len()];
            }
            return self.lentos[ciclo % self.lentos.len()];
        };
        *pid
    }

    pub fn rapidos(&self) -> &[Pid] {
        &self.rapidos
    }

    pub fn lentos(&self) -> &[Pid] {
        &self.lentos
    }

    pub fn raros(&self) -> &[Pid] {
        &self.raros
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
    /// Quantas leituras seguidas falharam por barramento ou timeout.
    ///
    /// Zera a cada resposta — inclusive um `NO DATA`, que é uma resposta: o carro
    /// disse "não tenho esse sensor", e para isso ele precisou estar vivo.
    falhas_seguidas: u32,
    /// Já avisei nesta conexão que o barramento anda ruim?
    ///
    /// A primeira vez é notícia, o resto é ruído. Sem isto, um K-line barulhento
    /// encheria o diário com a mesma linha centenas de vezes por viagem.
    avisou_do_barramento: bool,
}

/// Quantas leituras seguidas podem falhar antes de desistir da conexão.
///
/// No ISO 9141-2 do Eclipse — 10.400 baud, meio-duplex, fiação de 26 anos —
/// `BUS ERROR` é TRANSITÓRIO: um quadro corrompido, um ruído, a ECU ocupada.
/// Tratar o primeiro como fatal custava a conexão inteira, e reconectar leva de
/// 10 a 30 segundos com o handshake do ELM327. Um quadro ruim passava a custar
/// meio minuto de telemetria.
///
/// Seis é um ciclo de varredura inteiro (~2 s): errar um quadro é o barramento
/// sendo o que ele é; errar seis seguidos, sem UMA resposta no meio, é o
/// adaptador ter soltado do conector — e aí reconectar é mesmo o certo.
const FALHAS_PARA_DESISTIR_DA_CONEXAO: u32 = 6;

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
            falhas_seguidas: 0,
            avisou_do_barramento: false,
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
                self.falhas_seguidas = 0;
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
                // `NO DATA` é uma RESPOSTA: para dizer "não tenho esse sensor" o
                // carro precisou estar vivo. Não conta como falha de conexão.
                self.falhas_seguidas = 0;
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
            // Barramento e timeout: transitórios até prova em contrário.
            Err(err) => {
                self.falhas_seguidas += 1;

                if self.falhas_seguidas >= FALHAS_PARA_DESISTIR_DA_CONEXAO {
                    tracing::warn!(
                        ?pid,
                        seguidas = self.falhas_seguidas,
                        %err,
                        "o barramento não responde há um ciclo inteiro; desistindo da conexão"
                    );
                    return Err(err);
                }

                // A primeira é notícia — quero saber que o K-line anda ruim. As
                // seguintes viram ruído, e um barramento barulhento encheria o
                // diário com a mesma linha centenas de vezes por viagem.
                if !self.avisou_do_barramento {
                    self.avisou_do_barramento = true;
                    tracing::warn!(
                        ?pid,
                        %err,
                        "quadro perdido no barramento; seguindo com a leitura anterior"
                    );
                } else {
                    tracing::debug!(?pid, %err, "mais um quadro perdido");
                }
            }
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

    /// Quantos ticks separam duas leituras do mesmo PID.
    fn cadencia(plano: &Plano, alvo: Pid, ticks: usize) -> Option<usize> {
        let mut vistos: Vec<usize> = (0..ticks).filter(|t| plano.em(*t) == alvo).collect();
        if vistos.len() < 2 {
            return None;
        }
        vistos.dedup();
        Some(vistos[1] - vistos[0])
    }

    /// Um carro como o do Eclipse: os 16 PIDs que ele respondeu de verdade.
    fn carro_do_eclipse() -> Capacidades {
        cap_com(&[
            0x01, 0x03, 0x04, 0x05, 0x06, 0x07, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x13, 0x14,
            0x15, 0x1C,
        ])
    }

    #[test]
    fn o_diagnostico_nao_rouba_a_cadencia_dos_mostradores() {
        let sem = HashSet::new();
        let cap = carro_do_eclipse();
        let plano = Plano::montar(cap, MetodoFluxo::escolher(cap), &sem);

        assert!(
            !plano.raros().is_empty(),
            "esse carro responde falha, trim e sonda"
        );

        // A pergunta que importa: a temperatura da água continua voltando na
        // mesma cadência de antes dos raros existirem? Se os cinco tivessem
        // entrado na roda dos lentos, ela cairia de 3 para 8 voltas.
        let tamanho = plano.rapidos().len() + 1;
        let agua = cadencia(&plano, Pid::Coolant, tamanho * CICLOS_POR_RARO * 4)
            .expect("a água tem que ser lida");
        assert_eq!(
            agua,
            tamanho * plano.lentos().len(),
            "a água volta a cada {} ciclos, como antes",
            plano.lentos().len()
        );
    }

    #[test]
    fn os_raros_entram_na_roda_mesmo_que_devagar() {
        let sem = HashSet::new();
        let cap = carro_do_eclipse();
        let plano = Plano::montar(cap, MetodoFluxo::escolher(cap), &sem);

        let tamanho = plano.rapidos().len() + 1;
        // Uma volta inteira dos raros: cada um precisa aparecer ao menos uma vez.
        let ticks = tamanho * CICLOS_POR_RARO * plano.raros().len();
        for raro in plano.raros() {
            assert!(
                (0..ticks).any(|t| plano.em(t) == *raro),
                "{raro:?} nunca foi lido em uma volta inteira"
            );
        }
    }

    #[test]
    fn carro_sem_diagnostico_nao_ganha_slot_vazio() {
        // Carro que não responde nenhum dos raros: o slot do lento não pode ser
        // emprestado para ninguém, senão vira uma leitura perdida por volta.
        let sem = HashSet::new();
        let cap = cap_com(&[0x04, 0x05, 0x0C, 0x0D, 0x10]);
        let plano = Plano::montar(cap, MetodoFluxo::escolher(cap), &sem);

        assert!(plano.raros().is_empty());
        let tamanho = plano.rapidos().len() + 1;
        for t in 0..tamanho * 20 {
            let pid = plano.em(t);
            assert!(
                plano.rapidos().contains(&pid) || plano.lentos().contains(&pid),
                "{pid:?} não devia estar na varredura"
            );
        }
    }

    /// Uma fonte que falha as `n` primeiras leituras e depois responde.
    struct Instavel {
        falhas: u32,
        erro: fn() -> ObdError,
    }

    #[async_trait]
    impl ObdSource for Instavel {
        async fn read(&mut self, _pid: Pid) -> Result<f32, ObdError> {
            if self.falhas > 0 {
                self.falhas -= 1;
                return Err((self.erro)());
            }
            Ok(1.0)
        }
    }

    #[tokio::test]
    async fn barramento_morto_derruba_a_conexao() {
        struct Morta;

        #[async_trait]
        impl ObdSource for Morta {
            async fn read(&mut self, _pid: Pid) -> Result<f32, ObdError> {
                Err(ObdError::Timeout)
            }
        }

        let mut poller = Poller::new(Morta);
        // As primeiras são absorvidas; o que derruba é o ciclo inteiro sem uma
        // resposta — aí o adaptador soltou mesmo e reconectar é o certo.
        for i in 1..FALHAS_PARA_DESISTIR_DA_CONEXAO {
            assert!(
                poller.step().await.is_ok(),
                "a falha {i} ainda não devia derrubar"
            );
        }
        assert!(
            poller.step().await.is_err(),
            "um ciclo inteiro morto derruba"
        );
    }

    #[tokio::test]
    async fn quadro_perdido_custa_uma_leitura_e_nao_a_conexao() {
        // O caso que tirava o carro do ar: UM `BUS ERROR` no meio de uma viagem
        // boa derrubava a conexão inteira, e reconectar leva de 10 a 30 s.
        let mut poller = Poller::new(Instavel {
            falhas: 1,
            erro: || ObdError::Bus("BUS ERROR".into()),
        });

        assert!(
            poller.step().await.is_ok(),
            "um quadro ruim não derruba nada"
        );
        assert!(poller.step().await.is_ok(), "e a leitura seguinte funciona");
    }

    #[tokio::test]
    async fn uma_resposta_no_meio_zera_a_contagem() {
        // Barramento barulhento que erra quase um ciclo, acerta, e erra de novo:
        // isso é um K-line de 26 anos em dia ruim, não um adaptador solto. Sem
        // zerar, duas rajadas separadas somariam e derrubariam uma conexão viva.
        let mut poller = Poller::new(Instavel {
            falhas: FALHAS_PARA_DESISTIR_DA_CONEXAO - 1,
            erro: || ObdError::Bus("BUS ERROR".into()),
        });

        for _ in 0..FALHAS_PARA_DESISTIR_DA_CONEXAO - 1 {
            assert!(poller.step().await.is_ok());
        }
        assert!(poller.step().await.is_ok(), "a boa que zera");

        poller.source.falhas = FALHAS_PARA_DESISTIR_DA_CONEXAO - 1;
        for _ in 0..FALHAS_PARA_DESISTIR_DA_CONEXAO - 1 {
            assert!(
                poller.step().await.is_ok(),
                "a segunda rajada recomeça do zero"
            );
        }
    }

    #[tokio::test]
    async fn no_data_nao_conta_como_falha_de_conexao() {
        // "Não tenho esse sensor" é uma RESPOSTA: para dizê-la o carro precisou
        // estar vivo. Um carro com vários PIDs ausentes não pode ser confundido
        // com um adaptador que soltou.
        let mut poller = Poller::new(Contadora {
            vistos: HashMap::new(),
            nao_suportado: Some(poller_pid_qualquer()),
        });
        for _ in 0..FALHAS_PARA_DESISTIR_DA_CONEXAO * 3 {
            assert!(poller.step().await.is_ok());
        }
    }

    fn poller_pid_qualquer() -> Pid {
        Pid::Rpm
    }
}
