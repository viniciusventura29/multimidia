//! Guiagem: onde estou dentro de uma rota, e o que falar sobre isso.
//!
//! A rota vem de fora — quem a busca é o `DirectionsService`, que mora no lado
//! JavaScript junto do mapa. O raciocínio em cima dela é feito aqui, que é onde
//! a posição vive: quanto falta, qual a próxima manobra, se saímos do caminho e
//! quando abrir a boca.
//!
//! Isto **não** é o Navigation SDK: não há orientação de faixa, nem trânsito ao
//! vivo mudando a rota no meio do caminho. O trânsito entra só no tempo estimado,
//! porque a Directions API sabe informar isso na hora de calcular.

use serde::{Deserialize, Serialize};

use crate::fix::{distancia_m, Fix};

/// A partir de quantos metros de desvio considerar que saímos da rota.
///
/// Folgado de propósito: GPS erra 10–20 m em avenida com prédio alto, e anunciar
/// desvio a cada oscilação seria pior que não anunciar.
const TOLERANCIA_DESVIO_M: f64 = 45.0;

/// Quantas leituras seguidas fora da rota antes de mandar recalcular.
///
/// Uma só seria ruído. Três é cerca de três segundos — rápido para não deixar o
/// motorista perdido, lento para não recalcular por causa de um prédio
/// bloqueando o sinal.
const DESVIOS_PARA_RECALCULAR: u8 = 3;

/// Distâncias em que a próxima manobra é anunciada, em metros.
const MARCOS_DE_AVISO: [f64; 3] = [400.0, 150.0, 40.0];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passo {
    /// Já em texto puro: o Directions devolve HTML, e limpar isso é trabalho de
    /// quem recebe.
    pub instrucao: String,
    /// Referência visual de apoio, quando o Google manda uma.
    #[serde(default)]
    pub detalhe: Option<String>,
    pub distancia_m: f64,
    /// O código de manobra do Google (`turn-left`, `roundabout-right`…), que a
    /// tela usa para escolher a seta. Nem todo passo tem — o primeiro costuma
    /// vir sem, porque "siga em frente" não é manobra.
    #[serde(default)]
    pub manobra: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub destino: String,
    pub pontos: Vec<(f64, f64)>,
    pub passos: Vec<Passo>,
    pub distancia_total_m: f64,
    /// Já considerando trânsito, quando a Directions soube informar.
    pub duracao_total_s: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progresso {
    pub distancia_restante_m: f64,
    pub chegada_em_s: u32,
    pub passo_atual: usize,
    pub proxima_instrucao: String,
    pub proximo_detalhe: Option<String>,
    pub proxima_manobra: Option<String>,
    pub distancia_para_manobra_m: f64,
    pub desvio_m: f64,
    pub fora_da_rota: bool,
    /// Fora da rota tempo suficiente para valer a pena buscar outra.
    pub recalcular: bool,
    pub chegou: bool,
}

/// Distância de `p` ao segmento `a`–`b`, e a fração do segmento percorrida.
///
/// Trabalha em metros num plano local. A aproximação equiretangular erra frações
/// de metro em escala de quarteirão — bem menos que o próprio erro do GPS.
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

    /// Quanto da rota já foi percorrido, e a que distância do traçado estamos.
    fn situar(&self, aqui: (f64, f64)) -> (f64, f64) {
        let acumulado = self.acumulado();
        let mut melhor = (f64::MAX, 0.0);

        for (i, par) in self.pontos.windows(2).enumerate() {
            let (desvio, t) = projetar(aqui, par[0], par[1]);
            if desvio < melhor.0 {
                melhor = (desvio, acumulado[i] + (acumulado[i + 1] - acumulado[i]) * t);
            }
        }

        if melhor.0 == f64::MAX {
            return (0.0, 0.0);
        }
        melhor
    }
}

/// Uma rota sendo percorrida.
///
/// Guarda o que a `Route` sozinha não pode: quantas leituras seguidas estamos
/// fora do caminho, e o que já foi falado. Sem isso o painel recalcularia por
/// causa de uma oscilação de GPS e repetiria a mesma frase a cada segundo.
pub struct Guia {
    pub rota: Route,
    desvios_seguidos: u8,
    passo_falado: usize,
    marcos_ditos: [bool; MARCOS_DE_AVISO.len()],
    chegada_dita: bool,
}

impl Guia {
    pub fn nova(rota: Route) -> Self {
        Self {
            rota,
            desvios_seguidos: 0,
            passo_falado: usize::MAX,
            marcos_ditos: [false; MARCOS_DE_AVISO.len()],
            chegada_dita: false,
        }
    }

    /// Avalia uma posição: devolve onde estamos e o que falar, se for o caso.
    pub fn avaliar(&mut self, fix: &Fix) -> (Progresso, Option<String>) {
        let (desvio_m, ao_longo) = self.rota.situar((fix.lat, fix.lon));
        let total = self.rota.acumulado().last().copied().unwrap_or(0.0);
        let restante = (total - ao_longo).max(0.0);

        // Qual manobra vem a seguir: a primeira cujo fim ainda está à frente.
        let mut fim_do_passo = 0.0;
        let mut indice = self.rota.passos.len().saturating_sub(1);
        for (i, passo) in self.rota.passos.iter().enumerate() {
            fim_do_passo += passo.distancia_m;
            if fim_do_passo > ao_longo + 1.0 {
                indice = i;
                break;
            }
        }
        let passo = self.rota.passos.get(indice);

        let fora = desvio_m > TOLERANCIA_DESVIO_M;
        self.desvios_seguidos = if fora {
            self.desvios_seguidos.saturating_add(1)
        } else {
            0
        };

        let fracao = if total > 0.0 { restante / total } else { 0.0 };

        let progresso = Progresso {
            distancia_restante_m: restante,
            // Proporcional ao que falta. Usar a velocidade instantânea faria o
            // número dançar a cada semáforo.
            chegada_em_s: (self.rota.duracao_total_s as f64 * fracao).round() as u32,
            passo_atual: indice,
            proxima_instrucao: passo.map(|p| p.instrucao.clone()).unwrap_or_default(),
            proximo_detalhe: passo.and_then(|p| p.detalhe.clone()),
            proxima_manobra: passo.and_then(|p| p.manobra.clone()),
            distancia_para_manobra_m: (fim_do_passo - ao_longo).max(0.0),
            desvio_m,
            fora_da_rota: fora,
            recalcular: self.desvios_seguidos >= DESVIOS_PARA_RECALCULAR,
            chegou: restante < 30.0 && !self.rota.pontos.is_empty(),
        };

        let fala = self.locucao(&progresso);
        (progresso, fala)
    }

    /// O que falar agora, ou nada.
    ///
    /// Cada marco de distância é dito uma vez por manobra. Repetir a cada leitura
    /// seria insuportável — e ficar mudo até a esquina, inútil.
    fn locucao(&mut self, p: &Progresso) -> Option<String> {
        if p.chegou {
            return (!std::mem::replace(&mut self.chegada_dita, true))
                .then(|| "Você chegou.".to_string());
        }

        if p.recalcular {
            // Só na transição, senão repetiria enquanto durar o desvio.
            return (self.desvios_seguidos == DESVIOS_PARA_RECALCULAR)
                .then(|| "Recalculando a rota.".to_string());
        }

        if p.fora_da_rota {
            return None;
        }

        if p.passo_atual != self.passo_falado {
            self.passo_falado = p.passo_atual;
            self.marcos_ditos = [false; MARCOS_DE_AVISO.len()];
        }

        let instrucao = p.proxima_instrucao.trim();
        if instrucao.is_empty() {
            return None;
        }

        // O marco *mais próximo* que já foi ultrapassado, não o primeiro da
        // lista: entrando na rota a 83 m da esquina, o aviso é o de 40, não o de
        // 400. Como os marcos estão em ordem decrescente, é o último que casa.
        let (i, marco) = MARCOS_DE_AVISO
            .iter()
            .enumerate()
            .filter(|(_, &marco)| p.distancia_para_manobra_m <= marco)
            .next_back()?;

        if self.marcos_ditos[i] {
            return None;
        }

        // Marca os maiores junto — eles já não fazem sentido.
        for anterior in self.marcos_ditos.iter_mut().take(i + 1) {
            *anterior = true;
        }

        if *marco <= 40.0 {
            return Some(instrucao.to_string());
        }

        // A distância dita é a real, arredondada, não a do marco. Anunciar
        // "em 400 metros" estando a 83 seria mentira, e o motorista percebe.
        let metros = (p.distancia_para_manobra_m / 50.0).round() as u32 * 50;
        Some(format!(
            "Em {} metros, {}",
            metros.max(50),
            minuscula(instrucao)
        ))
    }
}

/// Deixa a primeira letra minúscula, para a frase emendar em "Em 400 metros, …".
fn minuscula(texto: &str) -> String {
    let mut chars = texto.chars();
    match chars.next() {
        Some(primeira) => primeira.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    detalhe: None,
                    distancia_m: 480.0,
                    manobra: None,
                },
                Passo {
                    instrucao: "Vire à direita na R. da Consolação".into(),
                    detalhe: Some("Você verá a padaria à direita".into()),
                    distancia_m: 460.0,
                    manobra: Some("turn-right".into()),
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
            accuracy_m: 10.0,
        }
    }

    #[test]
    fn no_comeco_falta_a_rota_inteira() {
        let mut g = Guia::nova(rota_reta());
        let (p, _) = g.avaliar(&em(-23.5713, -46.6443));

        assert!(p.distancia_restante_m > 900.0);
        assert!(!p.chegou && !p.fora_da_rota && !p.recalcular);
        assert_eq!(p.proxima_instrucao, "Siga pela Av. Paulista");
    }

    #[test]
    fn no_fim_anuncia_chegada_uma_vez_so() {
        let mut g = Guia::nova(rota_reta());
        let (p, fala) = g.avaliar(&em(-23.5650, -46.6515));

        assert!(p.chegou);
        assert_eq!(fala.as_deref(), Some("Você chegou."));
        assert_eq!(g.avaliar(&em(-23.5650, -46.6515)).1, None, "repetiu");
    }

    #[test]
    fn a_distancia_restante_so_diminui_andando_pela_rota() {
        let mut g = Guia::nova(rota_reta());
        let mut anterior = f64::MAX;

        for i in 0..=20 {
            let f = i as f64 / 20.0;
            let lat = -23.5713 + (-23.5650 + 23.5713) * f;
            let lon = -46.6443 + (-46.6515 + 46.6443) * f;
            let atual = g.avaliar(&em(lat, lon)).0.distancia_restante_m;

            assert!(atual <= anterior + 1.0, "aumentou: {anterior} -> {atual}");
            anterior = atual;
        }
    }

    #[test]
    fn a_manobra_e_o_indice_do_passo_mudam_quando_ele_termina() {
        let mut g = Guia::nova(rota_reta());

        let (cedo, _) = g.avaliar(&em(-23.5710, -46.6446));
        assert_eq!(cedo.passo_atual, 0);
        assert_eq!(cedo.proxima_manobra, None);

        let (tarde, _) = g.avaliar(&em(-23.5670, -46.6491));
        assert_eq!(tarde.passo_atual, 1);
        assert_eq!(tarde.proxima_manobra.as_deref(), Some("turn-right"));
    }

    /// Oscilação de GPS não pode virar recálculo; sair de verdade, sim.
    #[test]
    fn so_manda_recalcular_depois_de_desvios_seguidos() {
        let mut g = Guia::nova(rota_reta());
        let fora = em(-23.5695, -46.6440);

        assert!(!g.avaliar(&fora).0.recalcular, "recalculou na primeira");
        assert!(!g.avaliar(&fora).0.recalcular, "recalculou na segunda");

        let (p, fala) = g.avaliar(&fora);
        assert!(p.recalcular, "não recalculou na terceira");
        assert_eq!(fala.as_deref(), Some("Recalculando a rota."));

        assert_eq!(g.avaliar(&fora).1, None, "repetiu o aviso de recálculo");
    }

    #[test]
    fn voltar_para_a_rota_zera_a_contagem_de_desvio() {
        let mut g = Guia::nova(rota_reta());
        g.avaliar(&em(-23.5695, -46.6440));
        g.avaliar(&em(-23.5695, -46.6440));
        g.avaliar(&em(-23.5680, -46.6480)); // de volta ao traçado

        assert!(!g.avaliar(&em(-23.5695, -46.6440)).0.recalcular);
    }

    /// A mesma frase repetida a cada leitura seria insuportável.
    #[test]
    fn cada_frase_e_falada_uma_vez_so() {
        let mut g = Guia::nova(rota_reta());
        let mut falas = Vec::new();

        for i in 0..=40 {
            let f = i as f64 / 40.0;
            let lat = -23.5713 + (-23.5650 + 23.5713) * f;
            let lon = -46.6443 + (-46.6515 + 46.6443) * f;
            if let (_, Some(fala)) = g.avaliar(&em(lat, lon)) {
                falas.push(fala);
            }
        }

        for fala in &falas {
            assert_eq!(
                falas.iter().filter(|o| *o == fala).count(),
                1,
                "repetiu {fala:?} em {falas:?}"
            );
        }
        assert!(
            falas.iter().any(|f| f.contains("metros")),
            "nunca avisou distância: {falas:?}"
        );
    }

    /// Passo curto pode pular direto para o marco de perto; o painel não pode
    /// anunciar "em 400 metros" depois de já ter dito "em 150".
    #[test]
    fn nao_avisa_uma_distancia_maior_depois_de_uma_menor() {
        let mut g = Guia::nova(rota_reta());

        let (_, primeira) = g.avaliar(&em(-23.5688, -46.6471));
        assert!(primeira.is_some(), "devia ter avisado ao entrar perto");

        for _ in 0..5 {
            if let (_, Some(fala)) = g.avaliar(&em(-23.5688, -46.6471)) {
                panic!("falou de novo: {fala}");
            }
        }
    }

    #[test]
    fn a_frase_emenda_sem_maiuscula_no_meio() {
        assert_eq!(minuscula("Vire à direita"), "vire à direita");
        assert_eq!(minuscula(""), "");
    }

    #[test]
    fn rota_vazia_nao_quebra() {
        let mut g = Guia::nova(Route {
            destino: "lugar nenhum".into(),
            pontos: vec![],
            passos: vec![],
            distancia_total_m: 0.0,
            duracao_total_s: 0,
        });
        let (p, fala) = g.avaliar(&em(-23.57, -46.64));

        assert_eq!(p.distancia_restante_m, 0.0);
        assert!(!p.chegou, "rota vazia não é chegada");
        assert_eq!(fala, None);
    }
}
