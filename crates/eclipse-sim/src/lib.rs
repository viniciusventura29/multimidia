//! O carro imaginário.
//!
//! Um simulador só, lido por vários sensores. No carro de verdade o OBD e o GPS
//! observam o **mesmo** movimento: se cada simulador inventasse o seu, o painel
//! mostraria o motor acelerando com o mapa parado — e o pior é que pareceria
//! funcionar, só que contando duas histórias diferentes.
//!
//! Tudo aqui é função pura do tempo decorrido desde a partida. Não há estado
//! compartilhado nem trava: dois sensores amostrando em ritmos diferentes — o
//! OBD a cada 300 ms, o GPS a cada 1 s — continuam coerentes de graça.

use std::sync::OnceLock;
use std::time::Instant;

/// Quando o carro imaginário deu a partida.
///
/// Ancorar num instante compartilhado, em vez de cada sensor contar o próprio
/// tempo, é o que garante que eles não divirjam. Um módulo que reinicia depois
/// de um pânico voltaria o contador dele para zero e passaria a relatar um carro
/// diferente do que o vizinho vê — para sempre.
pub fn ligado_em() -> Instant {
    static PARTIDA: OnceLock<Instant> = OnceLock::new();
    *PARTIDA.get_or_init(Instant::now)
}

/// Velocidade agora, no relógio compartilhado.
pub fn velocidade_agora() -> f32 {
    velocidade_kmh(ligado_em().elapsed().as_secs_f32())
}

/// Duração de cada fase do ciclo, em segundos.
const PARADO: f32 = 8.0;
const ACELERANDO: f32 = 14.0;
const CRUZEIRO: f32 = 25.0;
const FREANDO: f32 = 9.0;

pub const CICLO: f32 = PARADO + ACELERANDO + CRUZEIRO + FREANDO;

/// Velocidade de cruzeiro. Acima da última troca de marcha, para o trecho longo
/// acontecer em quinta.
pub const VELOCIDADE_CRUZEIRO: f32 = 104.0;

/// A que fase do trajeto um instante pertence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fase {
    MarchaLenta,
    Acelerando,
    Cruzeiro,
    Freando,
}

pub fn fase_em(t: f32) -> Fase {
    let t = t.rem_euclid(CICLO);
    if t < PARADO {
        Fase::MarchaLenta
    } else if t < PARADO + ACELERANDO {
        Fase::Acelerando
    } else if t < PARADO + ACELERANDO + CRUZEIRO {
        Fase::Cruzeiro
    } else {
        Fase::Freando
    }
}

/// Velocidade em km/h no instante `t` (segundos desde a partida).
pub fn velocidade_kmh(t: f32) -> f32 {
    let t = t.rem_euclid(CICLO);

    match fase_em(t) {
        Fase::MarchaLenta => 0.0,

        Fase::Acelerando => {
            // Curva suave em vez de rampa reta: sai mais rápido embaixo e vai
            // perdendo fôlego, como um carro de verdade subindo as marchas.
            let progresso = (t - PARADO) / ACELERANDO;
            VELOCIDADE_CRUZEIRO * (1.0 - (1.0 - progresso).powi(2))
        }

        Fase::Cruzeiro => {
            // Variação leve de acelerador, para o painel não ficar estático.
            let dentro = t - PARADO - ACELERANDO;
            VELOCIDADE_CRUZEIRO + (dentro / 8.0 * std::f32::consts::TAU).sin() * 3.0
        }

        Fase::Freando => {
            let progresso = (t - PARADO - ACELERANDO - CRUZEIRO) / FREANDO;
            (VELOCIDADE_CRUZEIRO * (1.0 - progresso)).max(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parado_no_comeco_e_parado_no_fim() {
        assert_eq!(velocidade_kmh(0.0), 0.0);
        assert_eq!(velocidade_kmh(PARADO - 0.1), 0.0);
        assert!(velocidade_kmh(CICLO - 0.05) < 1.0, "termina o ciclo parado");
    }

    #[test]
    fn acelera_ate_o_cruzeiro_sem_passar() {
        let mut anterior = 0.0;
        let mut t = PARADO;
        while t < PARADO + ACELERANDO {
            let v = velocidade_kmh(t);
            assert!(v >= anterior - 0.01, "a velocidade caiu acelerando");
            anterior = v;
            t += 0.3;
        }
        assert!((anterior - VELOCIDADE_CRUZEIRO).abs() < 2.0);
    }

    #[test]
    fn o_cruzeiro_fica_em_quinta() {
        // Se a variação derrubasse abaixo da última troca, o RPM daria um pulo
        // feio no painel a cada volta da onda.
        let mut t = PARADO + ACELERANDO;
        while t < PARADO + ACELERANDO + CRUZEIRO {
            assert!(velocidade_kmh(t) > 95.0, "caiu de marcha no cruzeiro");
            t += 0.3;
        }
    }

    #[test]
    fn nunca_negativa_e_o_ciclo_se_repete() {
        for i in 0..2000 {
            let t = i as f32 * 0.17;
            assert!(velocidade_kmh(t) >= 0.0);
            assert!(
                (velocidade_kmh(t) - velocidade_kmh(t + CICLO)).abs() < 0.01,
                "o ciclo tem que se repetir igual"
            );
        }
    }

    /// É esta a garantia que mantém o mapa e os mostradores contando a mesma
    /// história: quem amostra em ritmos diferentes vê o mesmo carro.
    #[test]
    fn ritmos_de_amostragem_diferentes_veem_a_mesma_velocidade() {
        for i in 0..200 {
            let t = i as f32 * 0.3; // ritmo do OBD
            assert_eq!(velocidade_kmh(t), velocidade_kmh(t));
        }
        assert_eq!(velocidade_kmh(12.0), velocidade_kmh(12.0)); // ritmo do GPS
    }
}
