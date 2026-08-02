//! O carro está parado — segure o mapa.
//!
//! Parado, o GPS não devolve o mesmo ponto duas vezes: cada leitura oscila
//! alguns metros (dezenas, em geolocalização por Wi-Fi), e repassar isso
//! adiante faz o carro "sambar" no mapa e o progresso da rota tremer no
//! semáforo. Este filtro ancora a posição na primeira leitura de uma parada e
//! devolve a âncora enquanto nada indicar movimento de verdade — o resto do
//! sistema simplesmente não vê o jitter.
//!
//! A zona morta não é fixa: ela cresce com a imprecisão relatada pelo próprio
//! provedor. Um raio que engolisse o espalhamento do Wi-Fi (~50 m) engoliria
//! também uma quadra andada com GPS bom.

use crate::fix::{distancia_m, Fix};

/// Abaixo disto o carro é considerado parado — o mesmo limiar do congelamento
/// de rumo no JS (1,4 m/s), para os dois lados discordarem o mínimo possível.
const VELOCIDADE_MINIMA_KMH: f32 = 5.0;

/// Piso do raio da âncora: um provedor otimista demais não pode encolher a
/// zona morta abaixo do espalhamento real de um GPS bom parado.
const RAIO_MINIMO_M: f64 = 10.0;

/// A precisão relatada é o círculo de 68% — uma leitura em três cai fora dele.
/// Sem a folga, o jitter normal do provedor estouraria a âncora toda hora.
const FATOR_PRECISAO: f64 = 1.5;

/// Congela a posição enquanto o carro está parado.
pub struct FiltroDeParada {
    ancora: Option<Fix>,
}

impl FiltroDeParada {
    pub fn novo() -> Self {
        Self { ancora: None }
    }

    /// Parado (velocidade baixa E deslocamento dentro do raio de incerteza),
    /// devolve a âncora — posição e rumo congelados. Qualquer sinal de
    /// movimento real re-ancora e deixa a leitura passar intacta; assim a
    /// próxima parada ancora exatamente onde o carro parou.
    pub fn filtrar(&mut self, bruto: Fix) -> Fix {
        let Some(ancora) = self.ancora else {
            self.ancora = Some(bruto);
            return bruto;
        };

        let raio = RAIO_MINIMO_M.max(f64::from(bruto.accuracy_m) * FATOR_PRECISAO);
        let andando = bruto.speed_kmh > VELOCIDADE_MINIMA_KMH
            || distancia_m((ancora.lat, ancora.lon), (bruto.lat, bruto.lon)) > raio;

        if andando {
            self.ancora = Some(bruto);
            return bruto;
        }
        ancora
    }
}

impl Default for FiltroDeParada {
    fn default() -> Self {
        Self::novo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAULISTA: (f64, f64) = (-23.5614, -46.6559);

    /// Um ponto a `metros` ao norte da Paulista — 1° de latitude ≈ 111 km.
    fn ao_norte(metros: f64) -> (f64, f64) {
        (PAULISTA.0 + metros / 111_000.0, PAULISTA.1)
    }

    fn fix(ponto: (f64, f64), speed_kmh: f32, accuracy_m: f32) -> Fix {
        Fix {
            lat: ponto.0,
            lon: ponto.1,
            heading: 90.0,
            speed_kmh,
            accuracy_m,
        }
    }

    #[test]
    fn parado_o_jitter_de_poucos_metros_sai_identico() {
        let mut filtro = FiltroDeParada::novo();
        let primeiro = filtro.filtrar(fix(PAULISTA, 0.0, 20.0));

        for metros in [3.0, 8.0, 5.0, 7.0] {
            let saida = filtro.filtrar(fix(ao_norte(metros), 0.0, 20.0));
            assert_eq!(saida, primeiro, "jitter de {metros} m vazou");
        }
    }

    #[test]
    fn em_movimento_a_leitura_passa_intacta() {
        let mut filtro = FiltroDeParada::novo();
        filtro.filtrar(fix(PAULISTA, 30.0, 10.0));

        let adiante = fix(ao_norte(8.0), 30.0, 10.0);
        assert_eq!(filtro.filtrar(adiante), adiante, "segurou o carro andando");
    }

    /// Recuperar o sinal noutro lugar (túnel, estacionamento coberto) não pode
    /// deixar o mapa preso onde o sinal caiu.
    #[test]
    fn um_salto_grande_solta_a_ancora_mesmo_parado() {
        let mut filtro = FiltroDeParada::novo();
        filtro.filtrar(fix(PAULISTA, 0.0, 20.0));

        let longe = fix(ao_norte(200.0), 0.0, 20.0);
        assert_eq!(filtro.filtrar(longe), longe, "ficou preso no ponto velho");
    }

    /// Wi-Fi: precisão de 50 m espalha as leituras por dezenas de metros — a
    /// zona morta tem que crescer junto (raio = 50 × 1,5 = 75 m).
    #[test]
    fn com_precisao_ruim_a_zona_morta_cresce_junto() {
        let mut filtro = FiltroDeParada::novo();
        let primeiro = filtro.filtrar(fix(PAULISTA, 0.0, 50.0));

        let espalhado = filtro.filtrar(fix(ao_norte(40.0), 0.0, 50.0));
        assert_eq!(espalhado, primeiro, "o espalhamento do Wi-Fi vazou");
    }

    /// Manobra de garagem: abaixo do limiar de velocidade, mas o deslocamento
    /// acumulado passa do raio — tem que soltar.
    #[test]
    fn arrancada_lenta_que_acumula_deslocamento_solta_a_ancora() {
        let mut filtro = FiltroDeParada::novo();
        filtro.filtrar(fix(PAULISTA, 0.0, 10.0));

        let adiante = fix(ao_norte(20.0), 3.0, 10.0);
        assert_eq!(filtro.filtrar(adiante), adiante, "engoliu movimento real");
    }
}
