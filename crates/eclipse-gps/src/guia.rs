//! Guiagem: onde estou dentro de uma rota.
//!
//! A rota vem de fora — quem a busca é o `DirectionsService`, que mora no lado
//! JavaScript junto do mapa. O que é feito aqui é o raciocínio em cima dela, que
//! é onde a posição vive: quanto falta, qual a próxima manobra, e se saímos do
//! caminho.
//!
//! Isto **não** é o Navigation SDK. Não há trânsito em tempo real influenciando
//! a rota, nem orientação de faixa. É geometria em cima de uma rota já calculada.

use serde::{Deserialize, Serialize};

use crate::fix::{distancia_m, Fix};

/// A partir de quantos metros de desvio considerar que saímos da rota.
///
/// Folgado de propósito: GPS de celular erra 10–20 m em avenida com prédio alto,
/// e anunciar "saiu da rota" a cada oscilação seria pior que não anunciar.
const TOLERANCIA_DESVIO_M: f64 = 45.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passo {
    /// Já em texto puro: o Directions devolve HTML, e limpar isso é trabalho de
    /// quem recebe, não do painel.
    pub instrucao: String,
    pub distancia_m: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub destino: String,
    /// O traçado inteiro, ponto a ponto.
    pub pontos: Vec<(f64, f64)>,
    pub passos: Vec<Passo>,
    pub distancia_total_m: f64,
    pub duracao_total_s: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progresso {
    pub distancia_restante_m: f64,
    pub chegada_em_s: u32,
    pub proxima_instrucao: String,
    pub distancia_para_manobra_m: f64,
    /// Quantos metros estamos afastados do traçado.
    pub desvio_m: f64,
    pub fora_da_rota: bool,
    pub chegou: bool,
}

/// Distância de `p` até o segmento `a`–`b`, e a fração do segmento já percorrida.
///
/// Trabalha em metros num plano local. A aproximação equiretangular erra frações
/// de metro em escala de quarteirão, que é bem menos que o próprio erro do GPS.
fn projetar(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let cos_lat = a.0.to_radians().cos();
    let m = |q: (f64, f64)| ((q.1 - a.1) * 111_320.0 * cos_lat, (q.0 - a.0) * 110_540.0);

    let (px, py) = m(p);
    let (bx, by) = m(b);

    let comprimento2 = bx * bx + by * by;
    if comprimento2 == 0.0 {
        return ((px * px + py * py).sqrt(), 0.0);
    }

    let t = ((px * bx + py * by) / comprimento2).clamp(0.0, 1.0);
    let (dx, dy) = (px - bx * t, py - by * t);

    ((dx * dx + dy * dy).sqrt(), t)
}

impl Route {
    /// Distância acumulada até cada ponto do traçado.
    fn acumulado(&self) -> Vec<f64> {
        let mut soma = 0.0;
        let mut saida = Vec::with_capacity(self.pontos.len());
        saida.push(0.0);

        for par in self.pontos.windows(2) {
            soma += distancia_m(par[0], par[1]);
            saida.push(soma);
        }
        saida
    }

    /// Onde estamos na rota, dado onde o carro está.
    pub fn progresso(&self, fix: &Fix) -> Progresso {
        let aqui = (fix.lat, fix.lon);
        let acumulado = self.acumulado();
        let percorrido_total = *acumulado.last().unwrap_or(&0.0);

        // Segmento mais próximo. Percorrer tudo é barato: uma rota de cidade tem
        // centenas de pontos, e isto roda uma vez por segundo.
        let mut melhor = (f64::MAX, 0.0);
        for (i, par) in self.pontos.windows(2).enumerate() {
            let (desvio, t) = projetar(aqui, par[0], par[1]);
            if desvio < melhor.0 {
                let ao_longo = acumulado[i] + (acumulado[i + 1] - acumulado[i]) * t;
                melhor = (desvio, ao_longo);
            }
        }

        let (desvio_m, ao_longo) = melhor;
        let restante = (percorrido_total - ao_longo).max(0.0);

        // Qual manobra vem a seguir: a primeira cujo fim ainda está à frente.
        let mut fim_do_passo = 0.0;
        let mut proxima = self.passos.last();
        for passo in &self.passos {
            fim_do_passo += passo.distancia_m;
            if fim_do_passo > ao_longo + 1.0 {
                proxima = Some(passo);
                break;
            }
        }

        // Tempo estimado proporcional ao que falta. Usar a velocidade
        // instantânea daria um número dançando a cada semáforo.
        let fracao = if percorrido_total > 0.0 {
            restante / percorrido_total
        } else {
            0.0
        };

        Progresso {
            distancia_restante_m: restante,
            chegada_em_s: (self.duracao_total_s as f64 * fracao).round() as u32,
            proxima_instrucao: proxima.map(|p| p.instrucao.clone()).unwrap_or_default(),
            distancia_para_manobra_m: (fim_do_passo - ao_longo).max(0.0),
            desvio_m,
            fora_da_rota: desvio_m > TOLERANCIA_DESVIO_M,
            chegou: restante < 30.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um trecho reto da Av. Paulista, ~900 m rumo noroeste.
    fn rota_reta() -> Route {
        Route {
            destino: "Consolação".into(),
            pontos: vec![
                (-23.5713, -46.6443),
                (-23.5680, -46.6480),
                (-23.5650, -46.6515),
            ],
            passos: vec![
                Passo {
                    instrucao: "Siga pela Av. Paulista".into(),
                    distancia_m: 480.0,
                },
                Passo {
                    instrucao: "Vire à direita na R. da Consolação".into(),
                    distancia_m: 460.0,
                },
            ],
            distancia_total_m: 940.0,
            duracao_total_s: 240,
        }
    }

    fn em(lat: f64, lon: f64) -> Fix {
        Fix {
            lat,
            lon,
            heading: 315.0,
            speed_kmh: 40.0,
        }
    }

    #[test]
    fn no_comeco_falta_a_rota_inteira() {
        let r = rota_reta();
        let p = r.progresso(&em(-23.5713, -46.6443));

        assert!(p.distancia_restante_m > 900.0);
        assert!(!p.chegou);
        assert!(!p.fora_da_rota);
        assert_eq!(p.proxima_instrucao, "Siga pela Av. Paulista");
    }

    #[test]
    fn no_fim_do_traçado_anuncia_chegada() {
        let r = rota_reta();
        let p = r.progresso(&em(-23.5650, -46.6515));

        assert!(p.distancia_restante_m < 30.0, "sobrou {}", p.distancia_restante_m);
        assert!(p.chegou);
        assert_eq!(p.chegada_em_s, 0);
    }

    #[test]
    fn a_distancia_restante_so_diminui_andando_pela_rota() {
        let r = rota_reta();
        let mut anterior = f64::MAX;

        for i in 0..=20 {
            let f = i as f64 / 20.0;
            let lat = -23.5713 + (-23.5650 + 23.5713) * f;
            let lon = -46.6443 + (-46.6515 + 46.6443) * f;
            let atual = r.progresso(&em(lat, lon)).distancia_restante_m;

            assert!(atual <= anterior + 1.0, "aumentou: {anterior} -> {atual}");
            anterior = atual;
        }
    }

    #[test]
    fn a_proxima_manobra_muda_quando_o_passo_termina() {
        let r = rota_reta();

        // Perto do começo, ainda no primeiro passo.
        let cedo = r.progresso(&em(-23.5710, -46.6446));
        assert_eq!(cedo.proxima_instrucao, "Siga pela Av. Paulista");

        // Passados uns 600 m, já é a segunda manobra que interessa.
        let tarde = r.progresso(&em(-23.5670, -46.6491));
        assert_eq!(
            tarde.proxima_instrucao,
            "Vire à direita na R. da Consolação"
        );
    }

    #[test]
    fn a_distancia_para_a_manobra_diminui_ao_se_aproximar_dela() {
        let r = rota_reta();
        let longe = r.progresso(&em(-23.5713, -46.6443)).distancia_para_manobra_m;
        let perto = r.progresso(&em(-23.5685, -46.6474)).distancia_para_manobra_m;
        assert!(perto < longe, "{perto} deveria ser menor que {longe}");
    }

    /// Duas quadras fora da avenida é desvio; oscilação de GPS não é.
    #[test]
    fn so_acusa_desvio_quando_realmente_saiu() {
        let r = rota_reta();

        let oscilando = r.progresso(&em(-23.56805, -46.64815));
        assert!(
            !oscilando.fora_da_rota,
            "acusou desvio com {} m, dentro do erro normal de GPS",
            oscilando.desvio_m
        );

        let outra_rua = r.progresso(&em(-23.5695, -46.6440));
        assert!(
            outra_rua.fora_da_rota,
            "não acusou desvio estando a {} m",
            outra_rua.desvio_m
        );
    }

    #[test]
    fn rota_vazia_nao_quebra() {
        let r = Route {
            destino: "lugar nenhum".into(),
            pontos: vec![],
            passos: vec![],
            distancia_total_m: 0.0,
            duracao_total_s: 0,
        };
        let p = r.progresso(&em(-23.57, -46.64));
        assert_eq!(p.distancia_restante_m, 0.0);
    }
}
