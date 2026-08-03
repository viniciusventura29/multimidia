//! De ar a litros.
//!
//! O carro não informa consumo. O que ele informa é quanto **ar** entra no motor, e
//! como a injeção mantém a mistura perto da estequiométrica, massa de combustível ≈
//! massa de ar ÷ AFR. Daí saem litros por hora, e de litros por hora sai todo o
//! resto: km/l, médias, litros gastos e autonomia.
//!
//! Como não se sabe de antemão o que um carro de 2000 responde, a vazão vem de uma
//! **cascata** de métodos ([`MetodoFluxo`]), do melhor para o pior. Cada número que
//! sai daqui carrega de onde veio, para o painel poder marcar estimativa em vez de
//! se passar por medição.
//!
//! Nada aqui faz I/O nem olha o relógio: o `Δt` entre amostras entra por parâmetro.
//! É o que permite provar a integração num teste em vez de dirigindo.

use std::time::Duration;

use serde::Serialize;

use crate::capacidades::Capacidades;
use crate::pid::{Pid, Readings};
use crate::veiculo::{EstadoTanque, Trecho, Veiculo};

/// Densidade do ar a 20 °C e 101,3 kPa, em g/L.
const AR_PADRAO_G_L: f32 = 1.184;
/// Constante do gás para o ar seco, em J/(kg·K).
const R_AR: f32 = 287.0;
/// Temperatura do ar assumida quando o carro não informa a do coletor.
const IAT_ASSUMIDA_C: f32 = 25.0;

/// Abaixo disto o carro está parado e km/l não existe — só L/h.
pub const PARADO_KMH: u32 = 5;

/// Constante de tempo da suavização da vazão.
///
/// Cinco segundos porque o barramento entrega ~1 leitura de ar por segundo: menos que
/// isso e o km/l tremeria a cada amostra; mais e ele demoraria a reagir a um pé fundo.
const TAU_S: f32 = 5.0;

/// Buraco de amostragem que não vale integrar.
///
/// Depois de uma reconexão do adaptador o intervalo entre amostras pode ser de
/// minutos. Multiplicar a última vazão conhecida por esse buraco inventaria litros
/// que não foram queimados — e o tanque na tela andaria para trás sozinho.
const DT_MAX: Duration = Duration::from_secs(5);

/// De onde sai a vazão de combustível.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MetodoFluxo {
    /// O próprio carro calcula a vazão (`015E`). Raro antes de 2010.
    Direto,
    /// Massa de ar medida (`0110`) dividida pela proporção ar/combustível.
    Maf,
    /// Ar estimado por rotação, pressão do coletor e temperatura do ar.
    Coletor,
    /// Ar estimado pela carga calculada (`0104`), que é "fluxo atual ÷ fluxo máximo".
    Carga,
    /// O carro não dá nenhuma das entradas: o painel diz que não sabe.
    Indisponivel,
}

impl MetodoFluxo {
    /// O melhor método que este carro permite.
    ///
    /// Enquanto a máscara não foi lida, [`Capacidades`] responde sim para tudo e a
    /// escolha começa otimista — o `NO DATA` vai empurrando a cascata para baixo
    /// sozinho nos primeiros segundos.
    pub fn escolher(capacidades: Capacidades) -> Self {
        let tem = |pid: Pid| capacidades.suporta(pid.codigo().unwrap());
        if tem(Pid::VazaoComb) {
            Self::Direto
        } else if tem(Pid::Maf) {
            Self::Maf
        } else if tem(Pid::Map) && tem(Pid::Rpm) {
            Self::Coletor
        } else if tem(Pid::Carga) && tem(Pid::Rpm) {
            Self::Carga
        } else {
            Self::Indisponivel
        }
    }

    /// O número é medido pelo carro, ou modelado por nós?
    ///
    /// `Coletor` e `Carga` são modelos com uma eficiência volumétrica constante
    /// chutada no meio — funcionam, mas o painel tem que dizer que são estimativa.
    pub fn medido(self) -> bool {
        matches!(self, Self::Direto | Self::Maf)
    }

    /// O PID que precisa ser lido rápido, porque é ele que é integrado em litros.
    pub fn pid_de_ar(self) -> Option<Pid> {
        match self {
            Self::Direto => Some(Pid::VazaoComb),
            Self::Maf => Some(Pid::Maf),
            Self::Coletor => Some(Pid::Map),
            Self::Carga => Some(Pid::Carga),
            Self::Indisponivel => None,
        }
    }

    /// PIDs que o método usa mas que mudam devagar.
    pub fn pids_lentos(self) -> &'static [Pid] {
        match self {
            // A temperatura do ar admitido entra na densidade, e muda em minutos.
            Self::Coletor => &[Pid::Iat],
            _ => &[],
        }
    }
}

/// Vazão de combustível em L/h para uma amostra, ou `None` se falta entrada.
///
/// Aferições (Eclipse GT 3.0 V6 em marcha lenta, ~750 rpm):
/// MAF 3 g/s → 1,1 L/h; coletor a 30 kPa → ~1,2 L/h; carga 22% → ~1,3 L/h. Consumo
/// real em marcha lenta fica entre 0,8 e 1,2 L/h, então a cascata inteira cai na
/// mesma vizinhança — que é o critério para o fator de calibração fazer sentido.
pub fn vazao_lh(metodo: MetodoFluxo, r: &Readings, v: &Veiculo) -> Option<f32> {
    let ar_g_s = match metodo {
        // Já vem em litros por hora: nem passa pela conta de ar.
        MetodoFluxo::Direto => return r.vazao_lh.map(|lh| lh * v.calibracao),
        MetodoFluxo::Maf => r.maf_gs?,
        MetodoFluxo::Coletor => {
            let rpm = r.rpm? as f32;
            let kpa = r.map_kpa? as f32;
            let t_k = r.iat_c.map(|c| c as f32).unwrap_or(IAT_ASSUMIDA_C) + 273.15;
            // Ciclos por segundo num quatro tempos são rpm/120; cada ciclo aspira a
            // cilindrada vezes a eficiência volumétrica, na densidade do coletor.
            (rpm / 120.0) * v.cilindrada_l * v.ve_media * (kpa * 1000.0) / (R_AR * t_k)
        }
        MetodoFluxo::Carga => {
            let rpm = r.rpm? as f32;
            let carga = r.carga_pct? as f32 / 100.0;
            // A carga calculada já é "fluxo atual ÷ fluxo máximo", então basta o
            // fluxo máximo teórico desta rotação.
            carga * (rpm / 120.0) * v.cilindrada_l * AR_PADRAO_G_L
        }
        MetodoFluxo::Indisponivel => return None,
    };

    if em_corte(r) {
        // Pé fora em marcha alta: a injeção corta e o ar continua passando. Sem isto
        // toda descida de serra apareceria como consumo.
        return Some(0.0);
    }

    Some(ar_g_s.max(0.0) * 3600.0 / (v.afr * v.densidade_g_l) * v.calibracao)
}

/// O motor está em corte de combustível na desaceleração?
///
/// Usa só o que já está sendo lido: se o método de vazão não trouxe carga nem
/// pressão do coletor, não há como saber, e aí é melhor não adivinhar do que
/// zerar a vazão por engano.
fn em_corte(r: &Readings) -> bool {
    let (Some(rpm), Some(kmh)) = (r.rpm, r.speed_kmh) else {
        return false;
    };
    if rpm < 1_300 || kmh < 20 {
        return false;
    }
    match (r.carga_pct, r.map_kpa) {
        (Some(carga), _) => carga < 15,
        (None, Some(kpa)) => kpa < 25,
        _ => false,
    }
}

/// O consumo como a tela recebe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Consumo {
    /// `None` com o carro parado: km/l não existe a 0 km/h.
    pub instantaneo_km_l: Option<f32>,
    pub litros_hora: Option<f32>,
    pub metodo: MetodoFluxo,
    pub medido: bool,
}

/// O tanque como a tela recebe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tanque {
    pub capacidade_l: f32,
    pub litros: Option<f32>,
    /// Porcentagem derivada dos litros estimados — combina com a barra do tanque.
    pub pct: Option<f32>,
    /// Quanto cabe até encher. A tela não faz essa subtração; ela só mostra.
    pub falta_para_encher_l: Option<f32>,
    pub autonomia_km: Option<f32>,
    pub media_tanque_km_l: Option<f32>,
    pub calibracao: f32,
    /// O nível vem do PID do carro **e** a estimativa já convergiu com ele.
    pub medido: bool,
}

/// A viagem como a tela recebe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viagem {
    pub distancia_km: f32,
    pub duracao_s: f32,
    pub litros: f32,
    pub media_km_l: Option<f32>,
}

/// Quantas amostras próximas da leitura do carro bastam para chamar o nível de medido.
const AMOSTRAS_PARA_CONVERGIR: u8 = 3;
/// Distância entre estimativa e leitura considerada "convergiu", em litros.
const CONVERGIU_L: f32 = 1.0;
/// Salto de nível que só um abastecimento explica, em litros.
const SALTO_DE_ABASTECIMENTO_L: f32 = 5.0;
/// Quanto da diferença com a leitura do carro é corrigido por amostra.
///
/// Devagar de propósito: o PID de nível chapinha em curva e vem quantizado em passos
/// de ~0,4%, então puxar tudo de uma vez faria o número dançar na tela.
const CORRECAO_POR_AMOSTRA: f32 = 0.05;

/// Acumula as amostras do barramento e responde o que a tela pergunta.
pub struct Medidor {
    veiculo: Veiculo,
    metodo: MetodoFluxo,
    estado: EstadoTanque,
    /// Vazão suavizada, para o número na tela não tremer.
    vazao_suave: Option<f32>,
    /// Última vazão crua, que é a que se integra.
    vazao_agora: Option<f32>,
    convergencias: u8,
}

impl Medidor {
    pub fn novo(veiculo: Veiculo, estado: EstadoTanque, capacidades: Capacidades) -> Self {
        Self {
            veiculo: veiculo.saneado(),
            metodo: MetodoFluxo::escolher(capacidades),
            estado,
            vazao_suave: None,
            vazao_agora: None,
            convergencias: 0,
        }
    }

    /// O método pode mudar em pleno funcionamento: o poller descobre em runtime que
    /// um PID não responde, e a cascata desce um degrau.
    pub fn recalcular_metodo(&mut self, capacidades: Capacidades) {
        let novo = MetodoFluxo::escolher(capacidades);
        if novo != self.metodo {
            tracing::info!(?novo, antes = ?self.metodo, "fonte de consumo mudou");
            self.metodo = novo;
            self.vazao_suave = None;
            self.vazao_agora = None;
        }
    }

    pub fn metodo(&self) -> MetodoFluxo {
        self.metodo
    }

    pub fn veiculo(&self) -> Veiculo {
        self.veiculo
    }

    pub fn estado(&self) -> EstadoTanque {
        self.estado
    }

    /// Uma amostra nova do barramento, `dt` depois da anterior.
    pub fn amostrar(&mut self, r: &Readings, dt: Duration) {
        self.vazao_agora = vazao_lh(self.metodo, r, &self.veiculo);

        if let Some(vazao) = self.vazao_agora {
            let s = dt.as_secs_f32().clamp(0.0, DT_MAX.as_secs_f32());
            // Alfa por tempo, e não por amostra: a cadência do barramento é irregular
            // (um PID lento no meio do ciclo atrasa o próximo), e um alfa fixo faria
            // a suavização mudar de força junto com o ritmo da varredura.
            let alfa = 1.0 - (-s / TAU_S).exp();
            self.vazao_suave = Some(match self.vazao_suave {
                Some(antes) => antes + (vazao - antes) * alfa,
                None => vazao,
            });
        }

        if dt <= DT_MAX {
            let h = dt.as_secs_f32() / 3600.0;
            let litros = self.vazao_agora.unwrap_or(0.0) * h;
            let km = r.speed_kmh.unwrap_or(0) as f32 * h;
            // Só conta tempo com o motor girando: parado no posto não é viagem.
            let segundos = if r.rpm.is_some_and(|rpm| rpm > 0) {
                dt.as_secs_f32()
            } else {
                0.0
            };

            self.estado.tanque.somar(km, litros, segundos);
            self.estado.viagem.somar(km, litros, segundos);
            if let Some(restantes) = self.estado.litros.as_mut() {
                *restantes = (*restantes - litros).max(0.0);
            }
        }

        self.fundir_nivel(r);
    }

    /// Casa a estimativa integrada com o nível que o carro informa.
    ///
    /// A integração é precisa no curto prazo e escorrega no longo; o PID de nível é o
    /// contrário. Então o PID entra como âncora: puxa a estimativa devagar, e só um
    /// salto grande é lido como "abasteceram".
    fn fundir_nivel(&mut self, r: &Readings) {
        let Some(pct) = r.fuel_pct else { return };
        let lido = pct as f32 * self.veiculo.capacidade_l / 100.0;

        let Some(estimado) = self.estado.litros else {
            // Primeira leitura da vida: não há nada com que casar, adota-se.
            self.estado.litros = Some(lido);
            return;
        };

        if lido > estimado + SALTO_DE_ABASTECIMENTO_L {
            tracing::info!(lido, estimado, "nível subiu: tanque abastecido");
            self.encheu_ate(lido);
            return;
        }

        if (lido - estimado).abs() < CONVERGIU_L {
            self.convergencias = self.convergencias.saturating_add(1);
        } else {
            self.convergencias = 0;
        }
        self.estado.litros = Some(estimado + (lido - estimado) * CORRECAO_POR_AMOSTRA);
    }

    /// "Enchi o tanque": o nível vira a capacidade e o trecho do tanque zera.
    pub fn encheu(&mut self) {
        self.encheu_ate(self.veiculo.capacidade_l);
    }

    /// "Coloquei N litros": soma ao que havia, limitado pela capacidade.
    pub fn abasteceu(&mut self, litros: f32) {
        if !litros.is_finite() || litros <= 0.0 {
            return;
        }
        let antes = self.estado.litros.unwrap_or(0.0);
        self.encheu_ate((antes + litros).min(self.veiculo.capacidade_l));
    }

    /// O dono informou o nível na mão, sem abastecer — não zera a média do tanque.
    pub fn corrigiu_nivel(&mut self, litros: f32) {
        if litros.is_finite() {
            self.estado.litros = Some(litros.clamp(0.0, self.veiculo.capacidade_l));
            self.convergencias = 0;
        }
    }

    fn encheu_ate(&mut self, litros: f32) {
        self.estado.litros = Some(litros.clamp(0.0, self.veiculo.capacidade_l));
        // A média do tanque é "deste tanque": com gasolina nova, ela recomeça.
        self.estado.tanque = Trecho::default();
        self.convergencias = 0;
    }

    pub fn zerar_viagem(&mut self) {
        self.estado.viagem = Trecho::default();
    }

    /// Ajuste de configuração do carro.
    ///
    /// Mudar a capacidade não mexe nos litros estimados de propósito: quem corrigiu o
    /// tamanho do tanque não abasteceu nada — só o teto mudou.
    pub fn ajustar(&mut self, veiculo: Veiculo) {
        self.veiculo = veiculo.saneado();
        if let Some(litros) = self.estado.litros.as_mut() {
            *litros = litros.min(self.veiculo.capacidade_l);
        }
    }

    pub fn consumo(&self, r: &Readings) -> Consumo {
        let vazao = self.vazao_suave;
        let km_l = match (r.speed_kmh, vazao) {
            // Vazão perto de zero (motor cortando, ou parado) faria a divisão
            // explodir para centenas de km/l — é ausência de informação, não recorde.
            (Some(kmh), Some(lh)) if kmh >= PARADO_KMH && lh > 0.1 => Some(kmh as f32 / lh),
            _ => None,
        };

        Consumo {
            instantaneo_km_l: km_l.map(uma_casa),
            litros_hora: vazao.map(uma_casa),
            metodo: self.metodo,
            medido: self.metodo.medido(),
        }
    }

    pub fn tanque(&self) -> Tanque {
        let cap = self.veiculo.capacidade_l;
        let litros = self.estado.litros;
        let media = self.media_para_autonomia();

        Tanque {
            capacidade_l: cap,
            litros: litros.map(uma_casa),
            pct: litros.map(|l| uma_casa(l / cap * 100.0)),
            falta_para_encher_l: litros.map(|l| uma_casa((cap - l).max(0.0))),
            autonomia_km: litros.map(|l| (l * media).round()),
            media_tanque_km_l: self.estado.tanque.km_por_litro().map(uma_casa),
            calibracao: self.veiculo.calibracao,
            medido: self.convergencias >= AMOSTRAS_PARA_CONVERGIR,
        }
    }

    pub fn viagem(&self) -> Viagem {
        let t = self.estado.viagem;
        Viagem {
            distancia_km: uma_casa(t.km),
            duracao_s: t.segundos.round(),
            litros: uma_casa(t.litros),
            media_km_l: t.km_por_litro().map(uma_casa),
        }
    }

    /// O km/l com que se estima autonomia.
    ///
    /// A média do tanque primeiro, porque é a mais estável e a que casa com "quanto
    /// esta gasolina rende"; a da viagem como reserva; e só então o padrão do carro,
    /// para o painel não ficar sem responder "chego?" no primeiro dia.
    fn media_para_autonomia(&self) -> f32 {
        self.estado
            .tanque
            .km_por_litro()
            .or_else(|| self.estado.viagem.km_por_litro())
            .unwrap_or(self.veiculo.km_por_litro_padrao)
    }
}

fn uma_casa(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap_com(pids: &[u8]) -> Capacidades {
        let mut c = Capacidades::otimista();
        // Zera os três blocos e liga só o que o teste quer: é o carro que respondeu
        // a máscara, ao contrário do otimista que ainda não sabe nada.
        c.juntar(0x00, &[0, 0, 0, 0]);
        c.juntar(0x20, &[0, 0, 0, 0]);
        c.juntar(0x40, &[0, 0, 0, 0]);
        for pid in pids {
            c.marcar(*pid);
        }
        c
    }

    fn leitura() -> Readings {
        Readings {
            rpm: Some(750),
            speed_kmh: Some(0),
            ..Readings::default()
        }
    }

    #[test]
    fn a_cascata_desce_um_degrau_por_vez() {
        assert_eq!(
            MetodoFluxo::escolher(cap_com(&[0x5E, 0x10, 0x0B, 0x04, 0x0C])),
            MetodoFluxo::Direto
        );
        assert_eq!(
            MetodoFluxo::escolher(cap_com(&[0x10, 0x0B, 0x04, 0x0C])),
            MetodoFluxo::Maf
        );
        assert_eq!(
            MetodoFluxo::escolher(cap_com(&[0x0B, 0x04, 0x0C])),
            MetodoFluxo::Coletor
        );
        assert_eq!(
            MetodoFluxo::escolher(cap_com(&[0x04, 0x0C])),
            MetodoFluxo::Carga
        );
        assert_eq!(
            MetodoFluxo::escolher(cap_com(&[0x0D])),
            MetodoFluxo::Indisponivel
        );
    }

    #[test]
    fn so_maf_e_vazao_direta_contam_como_medidos() {
        assert!(MetodoFluxo::Maf.medido());
        assert!(MetodoFluxo::Direto.medido());
        assert!(
            !MetodoFluxo::Coletor.medido(),
            "modelo com VE chutada é estimativa"
        );
        assert!(!MetodoFluxo::Carga.medido());
    }

    #[test]
    fn as_quatro_fontes_batem_na_marcha_lenta() {
        let v = Veiculo::default();

        // MAF de 3 g/s: 3 * 3600 / (13,2 * 750) = 1,09 L/h.
        let maf = Readings {
            maf_gs: Some(3.0),
            ..leitura()
        };
        assert!(
            (vazao_lh(MetodoFluxo::Maf, &maf, &v).unwrap() - 1.09).abs() < 0.02,
            "{:?}",
            vazao_lh(MetodoFluxo::Maf, &maf, &v)
        );

        // Coletor a 30 kPa, ar a 40 °C, 750 rpm: ~5,0 g/s → ~1,8 L/h.
        let coletor = Readings {
            map_kpa: Some(30),
            iat_c: Some(40),
            ..leitura()
        };
        let lh = vazao_lh(MetodoFluxo::Coletor, &coletor, &v).unwrap();
        assert!((0.8..2.5).contains(&lh), "coletor deu {lh} L/h");

        // Carga de 22% a 750 rpm: 0,22 * 6,25 * 3,0 * 1,184 = 4,9 g/s → ~1,8 L/h.
        let carga = Readings {
            carga_pct: Some(22),
            ..leitura()
        };
        let lh = vazao_lh(MetodoFluxo::Carga, &carga, &v).unwrap();
        assert!((0.8..2.5).contains(&lh), "carga deu {lh} L/h");

        // Vazão direta passa reto, só com a calibração.
        let direto = Readings {
            vazao_lh: Some(2.0),
            ..leitura()
        };
        assert_eq!(vazao_lh(MetodoFluxo::Direto, &direto, &v), Some(2.0));
    }

    #[test]
    fn a_calibracao_multiplica_o_resultado() {
        let v = Veiculo {
            calibracao: 1.1,
            ..Veiculo::default()
        };
        let r = Readings {
            maf_gs: Some(3.0),
            ..leitura()
        };
        let com = vazao_lh(MetodoFluxo::Maf, &r, &v).unwrap();
        let sem = vazao_lh(MetodoFluxo::Maf, &r, &Veiculo::default()).unwrap();
        assert!((com / sem - 1.1).abs() < 0.001);
    }

    #[test]
    fn descida_de_serra_com_pe_fora_nao_gasta_gasolina() {
        let v = Veiculo::default();
        // 2500 rpm a 80 km/h com carga no mínimo: a injeção corta.
        let corte = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(80),
            maf_gs: Some(4.0),
            carga_pct: Some(8),
            ..Readings::default()
        };
        assert_eq!(vazao_lh(MetodoFluxo::Maf, &corte, &v), Some(0.0));

        // A mesma rotação com o pé na tábua não é corte.
        let acelerando = Readings {
            carga_pct: Some(70),
            ..corte
        };
        assert!(vazao_lh(MetodoFluxo::Maf, &acelerando, &v).unwrap() > 1.0);
    }

    #[test]
    fn sem_carga_nem_coletor_nao_se_adivinha_corte() {
        let v = Veiculo::default();
        // Só MAF: não há como distinguir corte de marcha lenta em movimento, então
        // não se zera nada — errar por baixo o consumo seria pior que não detectar.
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(80),
            maf_gs: Some(4.0),
            ..Readings::default()
        };
        assert!(vazao_lh(MetodoFluxo::Maf, &r, &v).unwrap() > 1.0);
    }

    /// Roda `voltas` amostras de `dt` com a mesma leitura.
    fn rodar(m: &mut Medidor, r: &Readings, dt: Duration, voltas: usize) {
        for _ in 0..voltas {
            m.amostrar(r, dt);
        }
    }

    fn medidor() -> Medidor {
        Medidor::novo(
            Veiculo::default(),
            EstadoTanque::default(),
            cap_com(&[0x10, 0x0C, 0x0D]),
        )
    }

    #[test]
    fn dez_minutos_a_oito_litros_por_hora_gastam_um_e_um_terco() {
        let mut m = medidor();
        m.encheu();
        // MAF de 22 g/s → 8 L/h. 10 min a 100 km/h.
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 600);

        let v = m.viagem();
        assert!((v.litros - 1.33).abs() < 0.05, "litros {}", v.litros);
        assert!((v.distancia_km - 16.7).abs() < 0.2, "km {}", v.distancia_km);
        assert!(
            (v.media_km_l.unwrap() - 12.5).abs() < 0.3,
            "média {:?}",
            v.media_km_l
        );

        let t = m.tanque();
        assert!(
            (t.litros.unwrap() - (61.0 - 1.33)).abs() < 0.1,
            "tanque {:?}",
            t.litros
        );
    }

    #[test]
    fn buraco_de_reconexao_nao_inventa_litros() {
        let mut m = medidor();
        m.encheu();
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };

        // Meia hora de adaptador solto entre duas amostras. Se isso fosse integrado,
        // o tanque na tela cairia 4 L parado no acostamento.
        m.amostrar(&r, Duration::from_secs(1));
        m.amostrar(&r, Duration::from_secs(1_800));

        assert!(m.viagem().litros < 0.01, "litros {}", m.viagem().litros);
    }

    #[test]
    fn parado_nao_tem_km_por_litro_mas_tem_litros_por_hora() {
        let mut m = medidor();
        let r = Readings {
            rpm: Some(750),
            speed_kmh: Some(0),
            maf_gs: Some(3.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 30);

        let c = m.consumo(&r);
        assert_eq!(c.instantaneo_km_l, None, "km/l a 0 km/h seria invenção");
        assert!(c.litros_hora.unwrap() > 0.9);
        assert!(c.medido, "MAF é medida do carro");
    }

    #[test]
    fn a_vazao_suavizada_persegue_o_valor_novo_sem_pular() {
        let mut m = medidor();
        let calmo = Readings {
            rpm: Some(750),
            speed_kmh: Some(0),
            maf_gs: Some(3.0),
            ..Readings::default()
        };
        rodar(&mut m, &calmo, Duration::from_secs(1), 60);
        let antes = m.consumo(&calmo).litros_hora.unwrap();

        let pe_fundo = Readings {
            maf_gs: Some(30.0),
            ..calmo
        };
        m.amostrar(&pe_fundo, Duration::from_secs(1));
        let depois = m.consumo(&pe_fundo).litros_hora.unwrap();

        let cru = vazao_lh(MetodoFluxo::Maf, &pe_fundo, &Veiculo::default()).unwrap();
        assert!(depois > antes, "tem que reagir");
        assert!(depois < cru, "mas não pular direto para o valor cru");
    }

    #[test]
    fn encher_o_tanque_zera_a_media_do_tanque_e_nao_a_da_viagem() {
        let mut m = medidor();
        m.encheu();
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 600);
        assert!(m.tanque().media_tanque_km_l.is_some());

        m.encheu();
        assert_eq!(m.tanque().litros, Some(61.0));
        assert_eq!(
            m.tanque().media_tanque_km_l,
            None,
            "gasolina nova, média do tanque nova"
        );
        assert!(
            m.viagem().media_km_l.is_some(),
            "a viagem continua: quem abastece no meio da estrada não recomeçou a viagem"
        );
    }

    #[test]
    fn abastecer_parcial_soma_sem_passar_da_capacidade() {
        let mut m = medidor();
        m.corrigiu_nivel(10.0);
        m.abasteceu(20.0);
        assert_eq!(m.tanque().litros, Some(30.0));

        m.abasteceu(100.0);
        assert_eq!(m.tanque().litros, Some(61.0), "não cabe mais que o tanque");
    }

    #[test]
    fn a_autonomia_usa_a_media_do_tanque_e_cai_para_o_padrao() {
        let mut m = medidor();
        m.encheu();
        // Sem histórico: 61 L * 9 km/l padrão = 549 km.
        assert_eq!(m.tanque().autonomia_km, Some(549.0));

        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 600);

        // Agora a média real (~12,5 km/l) manda, e a autonomia sobe.
        let t = m.tanque();
        assert!(
            t.autonomia_km.unwrap() > 700.0,
            "autonomia {:?}",
            t.autonomia_km
        );
    }

    #[test]
    fn o_nivel_do_carro_ancora_a_estimativa_sem_fazer_o_numero_pular() {
        let mut m = medidor();
        // Primeira leitura vira a âncora: 50% de 61 L.
        let meio = Readings {
            fuel_pct: Some(50),
            ..Readings::default()
        };
        m.amostrar(&meio, Duration::from_secs(1));
        assert_eq!(m.tanque().litros, Some(30.5));

        // O carro passa a dizer 40% (24,4 L). A estimativa tem que caminhar para lá
        // devagar, não teleportar — o PID chapinha em curva.
        let menos = Readings {
            fuel_pct: Some(40),
            ..Readings::default()
        };
        m.amostrar(&menos, Duration::from_secs(1));
        let depois = m.tanque().litros.unwrap();
        assert!(depois < 30.5 && depois > 30.0, "andou demais: {depois}");

        rodar(&mut m, &menos, Duration::from_secs(1), 300);
        assert!(
            (m.tanque().litros.unwrap() - 24.4).abs() < 0.5,
            "devia convergir: {:?}",
            m.tanque().litros
        );
    }

    #[test]
    fn nivel_so_e_medido_depois_de_convergir() {
        let mut m = medidor();
        let meio = Readings {
            fuel_pct: Some(50),
            ..Readings::default()
        };

        m.amostrar(&meio, Duration::from_secs(1));
        assert!(
            !m.tanque().medido,
            "na primeira amostra o número é a leitura, mas o filtro não convergiu"
        );

        rodar(&mut m, &meio, Duration::from_secs(1), 5);
        assert!(m.tanque().medido, "leitura estável: agora é medição");
    }

    #[test]
    fn salto_grande_de_nivel_e_lido_como_abastecimento() {
        let mut m = medidor();
        let quase_seco = Readings {
            fuel_pct: Some(10),
            ..Readings::default()
        };
        rodar(&mut m, &quase_seco, Duration::from_secs(1), 10);

        let cheio = Readings {
            fuel_pct: Some(95),
            ..Readings::default()
        };
        m.amostrar(&cheio, Duration::from_secs(1));

        // Sem esta regra, a correção lenta levaria uma hora de estrada para admitir
        // que o carro foi abastecido — com a autonomia errada esse tempo todo.
        let t = m.tanque();
        assert!(t.litros.unwrap() > 55.0, "litros {:?}", t.litros);
        assert_eq!(t.media_tanque_km_l, None, "tanque novo, média nova");
    }

    #[test]
    fn mudar_a_capacidade_nao_e_abastecer() {
        let mut m = medidor();
        m.encheu();
        assert_eq!(m.tanque().litros, Some(61.0));

        m.ajustar(Veiculo {
            capacidade_l: 45.0,
            ..Veiculo::default()
        });
        assert_eq!(
            m.tanque().litros,
            Some(45.0),
            "o teto baixou, então o conteúdo é limitado por ele"
        );

        m.ajustar(Veiculo {
            capacidade_l: 61.0,
            ..Veiculo::default()
        });
        assert_eq!(
            m.tanque().litros,
            Some(45.0),
            "corrigir o tamanho do tanque não coloca gasolina dentro dele"
        );
    }

    #[test]
    fn zerar_viagem_nao_mexe_no_tanque() {
        let mut m = medidor();
        m.encheu();
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 600);

        m.zerar_viagem();
        assert_eq!(m.viagem().distancia_km, 0.0);
        assert!(m.tanque().media_tanque_km_l.is_some());
        assert!(m.tanque().litros.unwrap() < 61.0);
    }

    #[test]
    fn perder_o_pid_de_ar_troca_o_metodo_e_esquece_a_vazao_velha() {
        let mut m = medidor();
        let r = Readings {
            rpm: Some(2_500),
            speed_kmh: Some(100),
            maf_gs: Some(22.0),
            ..Readings::default()
        };
        rodar(&mut m, &r, Duration::from_secs(1), 30);
        assert!(m.consumo(&r).litros_hora.is_some());

        // O carro parou de responder o MAF: a cascata desce, e a vazão suavizada do
        // método antigo não pode contaminar a do novo.
        let mut cap = cap_com(&[0x10, 0x0C, 0x0D, 0x04]);
        cap.recusar(0x10);
        m.recalcular_metodo(cap);

        assert_eq!(m.metodo(), MetodoFluxo::Carga);
        assert_eq!(
            m.consumo(&r).litros_hora,
            None,
            "sem carga lida ainda, sem vazão"
        );
        assert!(!m.consumo(&r).medido, "agora é estimativa");
    }

    #[test]
    fn tempo_de_viagem_so_conta_com_o_motor_girando() {
        let mut m = medidor();
        let desligado = Readings {
            rpm: None,
            speed_kmh: Some(0),
            ..Readings::default()
        };
        rodar(&mut m, &desligado, Duration::from_secs(1), 60);
        assert_eq!(m.viagem().duracao_s, 0.0);

        let ligado = Readings {
            rpm: Some(750),
            speed_kmh: Some(0),
            maf_gs: Some(3.0),
            ..Readings::default()
        };
        rodar(&mut m, &ligado, Duration::from_secs(1), 60);
        assert_eq!(m.viagem().duracao_s, 60.0);
    }
}
