//! GPS simulado.
//!
//! A velocidade **não** é inventada aqui: vem do `eclipse-sim`, o mesmo carro
//! que o OBD está lendo. O resultado é que o mapa anda quando o ponteiro sobe e
//! para quando o carro freia — as duas metades do painel contando uma história só.

use std::time::Duration;

use async_trait::async_trait;

use crate::fix::{distancia_m, interpolar, rumo, Fix};
use crate::rota::TRACADO;
use crate::source::{GpsError, LocationSource};

/// Um GPS de consumo entrega cerca de uma posição por segundo.
pub const INTERVALO: Duration = Duration::from_secs(1);

pub struct SimulatedLocation {
    /// Por onde andar. Começa na Paulista e é trocado pela rota quando o
    /// motorista escolhe um destino.
    caminho: Vec<(f64, f64)>,
    /// Segundos desde a partida, no mesmo relógio do carro imaginário.
    t: f32,
    /// Em qual segmento do traçado o carro está.
    segmento: usize,
    /// Metros já andados dentro do segmento atual.
    andado_m: f64,
    /// Último rumo bom. Parado, o rumo não muda — senão o mapa giraria sozinho
    /// em cada semáforo, que é o defeito clássico de navegador ruim.
    ultimo_rumo: f32,
    /// +1 indo, -1 voltando.
    ///
    /// No fim do traçado o carro dá meia-volta em vez de reaparecer no começo.
    /// Emendar o último ponto no primeiro faria o carro cortar São Paulo na
    /// diagonal, por cima de quarteirões — e o mapa mostraria isso.
    sentido: i8,
}

impl Default for SimulatedLocation {
    fn default() -> Self {
        Self {
            caminho: TRACADO.to_vec(),
            t: 0.0,
            segmento: 0,
            andado_m: 0.0,
            ultimo_rumo: rumo(TRACADO[0], TRACADO[1]),
            sentido: 1,
        }
    }
}

impl SimulatedLocation {
    /// Os dois extremos do segmento atual, na ordem em que o carro os percorre.
    fn extremos(&self) -> ((f64, f64), (f64, f64)) {
        let i = self.segmento;
        if self.sentido > 0 {
            (self.caminho[i], self.caminho[i + 1])
        } else {
            (self.caminho[i + 1], self.caminho[i])
        }
    }

    fn proximo_segmento(&mut self) {
        let ultimo = self.caminho.len() - 2;

        if self.sentido > 0 && self.segmento == ultimo {
            self.sentido = -1;
        } else if self.sentido < 0 && self.segmento == 0 {
            self.sentido = 1;
        } else {
            self.segmento = (self.segmento as i32 + self.sentido as i32) as usize;
        }
    }

    /// Recebe a velocidade em vez de buscá-la: em produção vem do relógio
    /// compartilhado com o OBD, no teste vem de um tempo que o teste controla.
    /// Sem isso a simulação não seria reproduzível.
    fn avancar(&mut self, dt: f32, velocidade: f32) -> Fix {
        self.t += dt;
        let mut restante = velocidade as f64 / 3.6 * dt as f64;

        // Consome a distância podendo atravessar vários segmentos numa tacada:
        // a 104 km/h são ~29 m por leitura, e um segmento curto some rápido.
        while restante > 0.0 {
            let (a, b) = self.extremos();
            let comprimento = distancia_m(a, b);

            if self.andado_m + restante < comprimento {
                self.andado_m += restante;
                break;
            }

            restante -= comprimento - self.andado_m;
            self.andado_m = 0.0;
            self.proximo_segmento();
        }

        let (a, b) = self.extremos();
        let comprimento = distancia_m(a, b);
        let fracao = if comprimento > 0.0 {
            self.andado_m / comprimento
        } else {
            0.0
        };

        let (lat, lon) = interpolar(a, b, fracao);

        if velocidade > 1.0 {
            self.ultimo_rumo = rumo(a, b);
        }

        Fix {
            lat,
            lon,
            heading: self.ultimo_rumo,
            speed_kmh: velocidade,
        }
    }
}

#[async_trait]
impl LocationSource for SimulatedLocation {
    fn seguir(&mut self, caminho: &[(f64, f64)]) {
        // Menos de dois pontos não é caminho. Ignorar é melhor que entrar num
        // estado onde `extremos` estoura o índice.
        if caminho.len() < 2 {
            return;
        }

        self.caminho = caminho.to_vec();
        self.segmento = 0;
        self.andado_m = 0.0;
        self.sentido = 1;
        self.ultimo_rumo = rumo(self.caminho[0], self.caminho[1]);
    }

    async fn next_fix(&mut self) -> Result<Fix, GpsError> {
        tokio::time::sleep(INTERVALO).await;
        Ok(self.avancar(
            INTERVALO.as_secs_f32(),
            eclipse_sim::velocidade_agora(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rodar(segundos: usize) -> Vec<Fix> {
        let mut gps = SimulatedLocation::default();
        (1..=segundos)
            .map(|s| gps.avancar(1.0, eclipse_sim::velocidade_kmh(s as f32)))
            .collect()
    }

    #[test]
    fn parado_no_comeco_a_posicao_nao_muda() {
        let fixes = rodar(5); // o ciclo começa com 8 s de marcha lenta
        for fix in &fixes {
            assert_eq!(fix.speed_kmh, 0.0);
            assert!(
                distancia_m((fix.lat, fix.lon), TRACADO[0]) < 1.0,
                "andou parado"
            );
        }
    }

    /// A garantia central: o GPS anda exatamente na velocidade que o OBD reporta.
    #[test]
    fn a_distancia_percorrida_bate_com_a_velocidade_do_carro() {
        let mut gps = SimulatedLocation::default();
        let mut t = 1.0;
        let mut anterior = gps.avancar(1.0, eclipse_sim::velocidade_kmh(t));

        for _ in 0..60 {
            t += 1.0;
            let atual = gps.avancar(1.0, eclipse_sim::velocidade_kmh(t));
            let andou = distancia_m((anterior.lat, anterior.lon), (atual.lat, atual.lon));
            let esperado = atual.speed_kmh as f64 / 3.6;

            // Tolerância porque o traçado tem quinas: cortar uma curva encurta
            // um pouco o caminho contra a linha reta entre dois pontos.
            assert!(
                (andou - esperado).abs() < 3.0,
                "andou {andou:.1} m mas a {:.0} km/h deveria andar {esperado:.1} m",
                atual.speed_kmh
            );
            anterior = atual;
        }
    }

    #[test]
    fn o_rumo_nao_gira_com_o_carro_parado() {
        let mut gps = SimulatedLocation::default();
        let primeiro = gps.avancar(1.0, 0.0);
        for _ in 0..5 {
            assert_eq!(
                gps.avancar(1.0, 0.0).heading,
                primeiro.heading,
                "o mapa giraria sozinho no semáforo"
            );
        }
    }

    #[test]
    fn o_rumo_muda_ao_virar_e_fica_sempre_valido() {
        let fixes = rodar(400);
        for fix in &fixes {
            assert!((0.0..360.0).contains(&fix.heading), "rumo {} inválido", fix.heading);
        }
        let rumos: Vec<f32> = fixes.iter().map(|f| f.heading).collect();
        assert!(
            rumos.windows(2).any(|p| (p[0] - p[1]).abs() > 1.0),
            "o carro percorreu o traçado inteiro sem nunca virar"
        );
    }

    /// Sem isto, emendar o fim no começo faria o carro cortar a cidade na
    /// diagonal por cima de quarteirões — e o mapa mostraria o carro voando.
    #[test]
    fn nunca_se_afasta_do_tracado() {
        for fix in rodar(1200) {
            let perto = TRACADO
                .iter()
                .any(|&p| distancia_m((fix.lat, fix.lon), p) < 600.0);
            assert!(perto, "saiu do traçado: {}, {}", fix.lat, fix.lon);
        }
    }

    /// Sem isto, escolher um destino deixaria o carro andando na Paulista
    /// enquanto a rota aponta para outro lugar — e o painel acusaria "fora da
    /// rota" o tempo todo, o que é verdade mas é uma demonstração inútil.
    #[test]
    fn seguir_troca_o_caminho_e_recomeca_do_inicio() {
        let mut gps = SimulatedLocation::default();
        for _ in 0..20 {
            gps.avancar(1.0, 60.0);
        }

        let outro = vec![(-23.5400, -46.6300), (-23.5390, -46.6280), (-23.5380, -46.6260)];
        gps.seguir(&outro);

        let fix = gps.avancar(1.0, 0.0);
        assert!(
            distancia_m((fix.lat, fix.lon), outro[0]) < 5.0,
            "não recomeçou no início do caminho novo"
        );
    }

    #[test]
    fn seguir_ignora_caminho_curto_demais() {
        let mut gps = SimulatedLocation::default();
        let antes = gps.avancar(1.0, 0.0);

        gps.seguir(&[]);
        gps.seguir(&[(-23.54, -46.63)]);

        let depois = gps.avancar(1.0, 0.0);
        assert_eq!(
            (antes.lat, antes.lon),
            (depois.lat, depois.lon),
            "um caminho inválido não pode mexer no estado"
        );
    }

    #[test]
    fn ao_chegar_no_fim_o_carro_da_meia_volta() {
        let fixes = rodar(1200);
        let ida = fixes.iter().any(|f| (30.0..150.0).contains(&f.heading));
        let volta = fixes.iter().any(|f| (210.0..330.0).contains(&f.heading));
        assert!(ida && volta, "o carro nunca voltou pelo mesmo caminho");
    }
}
