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

use crate::fix::{diferenca_angular, distancia_m, interpolar, rumo, Fix};

/// A partir de quantos metros de desvio considerar que saímos da rota.
///
/// Folgado de propósito: GPS erra 10–20 m em avenida com prédio alto, e anunciar
/// desvio a cada oscilação seria pior que não anunciar.
const TOLERANCIA_DESVIO_M: f64 = 45.0;

/// Acima desta velocidade o rumo relatado pelo GPS é confiável o bastante para
/// vetar o encaixe na rua. Abaixo dela ele é ruído — ou nem existe.
const VELOCIDADE_CONFIAVEL_KMH: f32 = 15.0;

/// Discordância entre o rumo medido e o da rua que derruba o encaixe.
///
/// Mais que um ângulo reto não é imprecisão de GPS: é outra via, ou é o carro
/// indo no sentido contrário ao da rota.
const DISCORDANCIA_MAXIMA_GRAUS: f32 = 90.0;

/// Quantas leituras seguidas fora da rota antes de mandar recalcular.
///
/// Uma só seria ruído. Três é cerca de três segundos — rápido para não deixar o
/// motorista perdido, lento para não recalcular por causa de um prédio
/// bloqueando o sinal.
const DESVIOS_PARA_RECALCULAR: u8 = 3;

/// Distâncias em que a próxima manobra é anunciada, em metros.
const MARCOS_DE_AVISO: [f64; 3] = [400.0, 150.0, 40.0];

/// Quanto do traçado, para frente e para trás do último ponto conhecido, entra
/// na busca pelo trecho em que o carro está.
///
/// Sem janela, o trecho escolhido é o mais próximo da rota **inteira** — e numa
/// rota que passa duas vezes pela mesma via (ida e volta, trevo, retorno, laço
/// de quarteirão) isso põe o carro no trecho errado: a distância restante
/// despenca, o passo pula e a chegada é anunciada a quilômetros do destino.
///
/// Duzentos metros é sete segundos a 100 km/h — folga larga para um GPS de
/// 1 Hz. Perder o sinal por mais que isso cai na varredura global, que é o
/// comportamento certo: aí a âncora não vale mesmo.
const JANELA_DE_BUSCA_M: f64 = 200.0;

/// Quanto um trecho que corre na contramão do carro "afasta" na escolha.
///
/// Perto de um retorno, a mão contrária está a poucos metros — mais perto, em
/// linha reta, que o eixo da própria pista quando o GPS erra para aquele lado.
/// Distância lateral sozinha escolhe errado ali, e o painel passa a contar a
/// volta como se já estivesse acontecendo.
///
/// Penalidade, e não veto: um trecho na contramão a 4 m ainda ganha de um na
/// mão certa a 300 m — que é o caso de quem entrou na rua errada de verdade.
/// O valor é a própria [`TOLERANCIA_DESVIO_M`]: um trecho contramão só vence se
/// o alternativo já estiver fora da rota.
const PENALIDADE_CONTRA_MAO_M: f64 = TOLERANCIA_DESVIO_M;

/// O rumo relatado pelo GPS, quando dá para confiar nele.
///
/// Parado, ele é ruído — ou nem existe. Ver [`VELOCIDADE_CONFIAVEL_KMH`].
fn rumo_confiavel(fix: &Fix) -> Option<f32> {
    (fix.speed_kmh > VELOCIDADE_CONFIAVEL_KMH).then_some(fix.heading)
}

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
}

/// Onde a rua está, no trecho mais próximo do carro.
struct NaRua {
    /// A posição projetada sobre o traçado — o carro em cima da via, não ao lado.
    ponto: (f64, f64),
    /// Para onde a via aponta ali.
    rumo: f32,
}

/// Onde uma posição cai em relação ao traçado.
struct Situacao {
    desvio_m: f64,
    ao_longo_m: f64,
    /// `None` quando não há em que projetar: rota vazia, ou só com pontos
    /// repetidos — um traçado sem comprimento não tem direção nenhuma.
    na_rua: Option<NaRua>,
}

/// Uma rota sendo percorrida.
///
/// Guarda o que a `Route` sozinha não pode: quantas leituras seguidas estamos
/// fora do caminho, e o que já foi falado. Sem isso o painel recalcularia por
/// causa de uma oscilação de GPS e repetiria a mesma frase a cada segundo.
pub struct Guia {
    pub rota: Route,
    /// A distância acumulada até cada ponto do traçado.
    ///
    /// Calculada uma vez, na construção: o traçado não muda depois, e refazer
    /// esta soma a cada leitura de GPS era percorrer a rota inteira duas vezes
    /// por segundo para chegar sempre ao mesmo vetor.
    acumulado: Vec<f64>,
    /// Onde ao longo do traçado o carro estava na última avaliação.
    ///
    /// É a âncora da [`JANELA_DE_BUSCA_M`]: sem ela, a projeção escolhe o trecho
    /// mais próximo da rota inteira e uma rota que se cruza engana o painel.
    ao_longo_atual: Option<f64>,
    desvios_seguidos: u8,
    passo_falado: usize,
    marcos_ditos: [bool; MARCOS_DE_AVISO.len()],
    chegada_dita: bool,
}

impl Guia {
    pub fn nova(rota: Route) -> Self {
        Self {
            acumulado: rota.acumulado(),
            rota,
            // Começa no zero, e não em "não sei": a rota é sempre traçada **a
            // partir de onde o carro está** (ver `tracar` no módulo `nav`), então
            // o começo do traçado é onde ele está agora. Deixar sem âncora faria
            // a primeira avaliação varrer a rota inteira — e numa ida e volta é
            // justamente aí que o carro cai na perna de volta e o painel anuncia
            // chegada antes de sair do lugar.
            ao_longo_atual: Some(0.0),
            desvios_seguidos: 0,
            passo_falado: usize::MAX,
            marcos_ditos: [false; MARCOS_DE_AVISO.len()],
            chegada_dita: false,
        }
    }

    /// Quanto da rota já foi percorrido, a que distância do traçado estamos, e
    /// onde exatamente na via isso cai.
    ///
    /// Procura primeiro **perto de onde sabíamos estar**, e só abre para a rota
    /// inteira quando ali não cabe — que é o caso legítimo de ter saído do
    /// caminho, ou de o sinal ter sumido tempo demais. Ver [`JANELA_DE_BUSCA_M`].
    fn situar(&self, aqui: (f64, f64), rumo_do_carro: Option<f32>) -> Situacao {
        if let Some(ancora) = self.ao_longo_atual {
            if let Some(perto) = self.varrer(aqui, rumo_do_carro, Some(ancora)) {
                if perto.desvio_m <= TOLERANCIA_DESVIO_M {
                    return perto;
                }
            }
        }

        self.varrer(aqui, rumo_do_carro, None).unwrap_or(Situacao {
            desvio_m: 0.0,
            ao_longo_m: 0.0,
            na_rua: None,
        })
    }

    /// O trecho que melhor explica `aqui`, olhando só os que caem dentro da
    /// janela em torno de `ancora` (ou todos, quando ela é `None`).
    ///
    /// "Melhor" é a menor distância lateral **mais** a penalidade de contramão —
    /// ver [`PENALIDADE_CONTRA_MAO_M`]. O `desvio_m` devolvido é a distância de
    /// verdade, sem penalidade: é ela que decide se saímos da rota.
    ///
    /// `None` quando não sobrou trecho nenhum para olhar: rota vazia, ou uma
    /// âncora que não alcança segmento algum.
    fn varrer(
        &self,
        aqui: (f64, f64),
        rumo_do_carro: Option<f32>,
        ancora: Option<f64>,
    ) -> Option<Situacao> {
        let mut melhor = f64::MAX;
        let mut desvio_m = 0.0;
        let mut ao_longo_m = 0.0;
        let mut na_rua = None;

        for (i, par) in self.rota.pontos.windows(2).enumerate() {
            // Um segmento de comprimento zero (ponto repetido no traçado) não
            // aponta para lugar nenhum — dele não sai rumo de rua.
            let rumo_da_via = (par[0] != par[1]).then(|| rumo(par[0], par[1]));
            let (desvio, t) = projetar(aqui, par[0], par[1]);

            let contra_mao = match (rumo_do_carro, rumo_da_via) {
                (Some(carro), Some(via)) => {
                    diferenca_angular(carro, via) > DISCORDANCIA_MAXIMA_GRAUS
                }
                _ => false,
            };
            let nota = desvio
                + if contra_mao {
                    PENALIDADE_CONTRA_MAO_M
                } else {
                    0.0
                };
            if nota >= melhor {
                continue;
            }

            let (comeco, fim) = (self.acumulado[i], self.acumulado[i + 1]);
            let candidato = comeco + (fim - comeco) * t;

            // A janela é sobre o ponto **projetado**, não sobre o segmento: um
            // trecho longo (uma reta de rodovia) entraria inteiro só porque uma
            // ponta dele encosta na janela, e o encaixe cairia longe de novo.
            if ancora.is_some_and(|a| (candidato - a).abs() > JANELA_DE_BUSCA_M) {
                continue;
            }

            melhor = nota;
            desvio_m = desvio;
            ao_longo_m = candidato;
            na_rua = rumo_da_via.map(|r| NaRua {
                ponto: interpolar(par[0], par[1], t),
                rumo: r,
            });
        }

        (melhor != f64::MAX).then_some(Situacao {
            desvio_m,
            ao_longo_m,
            na_rua,
        })
    }

    /// A posição do carro **em cima da rua**, quando dá para afirmar isso.
    ///
    /// O GPS erra 10–30 m em rua com prédio alto, e o resultado é o carro
    /// desenhado ao lado da linha que ele está seguindo — o motorista vê o
    /// painel discordando da própria rota. Mas a rota é uma informação que o
    /// GPS não tem: se o carro está sobre ela, a rua é onde ele está, e a
    /// leitura crua é só o erro do sensor.
    ///
    /// Isto **não** é map matching: gruda na rota traçada, não em qualquer rua
    /// do mundo — para isso seria preciso a Roads API, que é paga por leitura.
    /// Sem rota, ou fora dela, a posição crua passa intacta.
    pub fn grudar(&self, fix: &Fix) -> Fix {
        let situacao = self.situar((fix.lat, fix.lon), rumo_confiavel(fix));

        // Longe do traçado é longe de verdade. Desenhar o carro na rota aqui
        // seria mentir justamente na hora em que o motorista precisa ver que
        // errou o caminho — é o mesmo limiar que dispara o recálculo.
        if situacao.desvio_m > TOLERANCIA_DESVIO_M {
            return *fix;
        }

        let Some(rua) = situacao.na_rua else {
            return *fix;
        };

        // Rumo medido brigando com o da via: é outra rua paralela, ou o carro
        // está indo no sentido contrário ao da rota. Nos dois casos o sensor
        // sabe mais que o traçado. Só vale enquanto o rumo do GPS é confiável,
        // ou seja, com o carro andando.
        //
        // A escolha do trecho já penaliza a contramão, mas isso é preferência,
        // não proibição: quando só há trecho contramão por perto, ele vence — e
        // é aqui que se recusa a desenhar o carro em cima dele.
        if rumo_confiavel(fix)
            .is_some_and(|r| diferenca_angular(r, rua.rumo) > DISCORDANCIA_MAXIMA_GRAUS)
        {
            return *fix;
        }

        // O rumo vem da via inclusive parado — é o que endireita a seta no
        // semáforo, onde o GPS não relata rumo nenhum e o carro apontaria para
        // o norte por falta de coisa melhor.
        Fix {
            lat: rua.ponto.0,
            lon: rua.ponto.1,
            heading: rua.rumo,
            ..*fix
        }
    }

    /// Avalia uma posição: devolve onde estamos e o que falar, se for o caso.
    pub fn avaliar(&mut self, fix: &Fix) -> (Progresso, Option<String>) {
        let Situacao {
            desvio_m,
            ao_longo_m: ao_longo,
            ..
        } = self.situar((fix.lat, fix.lon), rumo_confiavel(fix));
        let total = self.acumulado.last().copied().unwrap_or(0.0);
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

        // A âncora da próxima busca só avança com o carro **em cima** da rota.
        // Fora dela, o último ponto bom continua sendo onde deixamos o caminho —
        // é de lá que se volta, e é lá que a janela tem que continuar olhando.
        if !fora {
            self.ao_longo_atual = Some(ao_longo);
        }

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
            .rfind(|(_, &marco)| p.distancia_para_manobra_m <= marco)?;

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

    /* -------------------------------------------------------------- */
    /* Rota que passa duas vezes pelo mesmo lugar                      */
    /* -------------------------------------------------------------- */

    /// Metros ao norte e a leste de um ponto, em graus.
    fn deslocar(de: (f64, f64), norte_m: f64, leste_m: f64) -> (f64, f64) {
        (
            de.0 + norte_m / 110_540.0,
            de.1 + leste_m / (111_320.0 * de.0.to_radians().cos()),
        )
    }

    /// Um carro subindo a rua, a 40 km/h — rumo norte, como a perna de ida.
    fn subindo(onde: (f64, f64)) -> Fix {
        Fix {
            lat: onde.0,
            lon: onde.1,
            heading: 0.0,
            speed_kmh: 40.0,
            accuracy_m: 10.0,
        }
    }

    /// Sobe 500 m por uma rua e volta pela mão contrária, 8 m ao lado.
    ///
    /// É a forma de qualquer ida e volta, retorno ou laço de quarteirão — e o
    /// caso em que a projeção ingênua erra: perto da largada, o traçado de volta
    /// passa mais perto do carro que o de ida.
    fn rota_de_ida_e_volta() -> Route {
        let base = (-23.5700, -46.6500);
        let fim = deslocar(base, 500.0, 0.0);

        Route {
            destino: "de volta".into(),
            pontos: vec![base, fim, deslocar(fim, 0.0, 8.0), deslocar(base, 0.0, 8.0)],
            passos: vec![Passo {
                instrucao: "Siga e retorne".into(),
                detalhe: None,
                distancia_m: 1_008.0,
                manobra: None,
            }],
            distancia_total_m: 1_008.0,
            duracao_total_s: 180,
        }
    }

    /// O painel não pode anunciar chegada com o carro ainda na largada.
    ///
    /// Vinte metros depois de sair, com o GPS errando 12 m para o lado da mão
    /// contrária, o traçado de **volta** fica a 4 m do carro e o de ida a 12.
    /// Escolhendo o trecho mais próximo da rota inteira, o painel acha que
    /// faltam 20 metros para chegar — numa viagem de um quilômetro que acabou
    /// de começar.
    #[test]
    fn ida_e_volta_nao_faz_o_painel_chegar_na_largada() {
        let mut g = Guia::nova(rota_de_ida_e_volta());
        let base = (-23.5700, -46.6500);
        let saindo = deslocar(base, 20.0, 12.0);

        let (p, _) = g.avaliar(&subindo(saindo));

        assert!(
            !p.chegou,
            "anunciou chegada a {} m do fim",
            p.distancia_restante_m
        );
        assert!(
            p.distancia_restante_m > 900.0,
            "encaixou na perna de volta: faltariam {} m de 1008",
            p.distancia_restante_m
        );
        assert!(!p.fora_da_rota, "12 m de erro de GPS não é sair da rota");
    }

    /// E ao longo da ida inteira a conta continua andando para frente.
    #[test]
    fn ida_e_volta_conta_a_perna_de_ida_antes_da_de_volta() {
        let mut g = Guia::nova(rota_de_ida_e_volta());
        let base = (-23.5700, -46.6500);
        let mut anterior = f64::MAX;

        // Sobe os 500 m da ida, sempre com o erro de GPS puxando para a mão
        // contrária — que é o pior caso para a projeção.
        for passo in 0..=25 {
            let aqui = deslocar(base, f64::from(passo) * 20.0, 12.0);
            let atual = g.avaliar(&subindo(aqui)).0.distancia_restante_m;
            assert!(
                atual <= anterior + 1.0,
                "a {} m da largada a distância restante AUMENTOU: {anterior} -> {atual}",
                passo * 20
            );
            anterior = atual;
        }

        // Chegando no retorno, ainda falta a volta inteira.
        assert!(anterior > 450.0, "faltavam só {anterior} m no retorno");
    }

    /* -------------------------------------------------------------- */
    /* Grudar o carro na rua                                           */
    /* -------------------------------------------------------------- */

    /// Distância de um ponto ao traçado — o que o motorista enxerga como
    /// "o carro está ao lado da linha".
    fn distancia_ao_tracado(g: &Guia, ponto: (f64, f64)) -> f64 {
        g.situar(ponto, None).desvio_m
    }

    /// Um ponto a `metros` ao leste de `de` — deslocamento lateral limpo para
    /// simular o erro do GPS sem andar pela rota.
    fn ao_lado(de: (f64, f64), metros: f64) -> (f64, f64) {
        (de.0, de.1 + metros / (111_320.0 * de.0.to_radians().cos()))
    }

    #[test]
    fn o_carro_gruda_no_tracado_quando_esta_perto() {
        let g = Guia::nova(rota_reta());
        let torto = ao_lado((-23.5695, -46.6462), 20.0);

        assert!(
            distancia_ao_tracado(&g, torto) > 10.0,
            "o ponto de teste já nasceu em cima da linha"
        );

        let grudado = g.grudar(&em(torto.0, torto.1));
        assert!(
            distancia_ao_tracado(&g, (grudado.lat, grudado.lon)) < 0.5,
            "ficou a {} m da linha",
            distancia_ao_tracado(&g, (grudado.lat, grudado.lon))
        );
    }

    #[test]
    fn o_rumo_passa_a_ser_o_da_rua() {
        let g = Guia::nova(rota_reta());
        let torto = ao_lado((-23.5695, -46.6462), 20.0);

        let rua = crate::fix::rumo((-23.5713, -46.6443), (-23.5680, -46.6480));
        let grudado = g.grudar(&em(torto.0, torto.1));

        assert!(
            diferenca_angular(grudado.heading, rua) < 1.0,
            "seta em {}°, rua em {rua}°",
            grudado.heading
        );
    }

    /// Fora da rota o painel anuncia recálculo; desenhar o carro na linha ao
    /// mesmo tempo seria o painel discordando de si mesmo.
    #[test]
    fn longe_do_tracado_a_leitura_crua_passa_intacta() {
        let g = Guia::nova(rota_reta());
        let longe = ao_lado((-23.5695, -46.6462), 300.0);
        let crua = em(longe.0, longe.1);

        assert_eq!(g.grudar(&crua), crua);
    }

    /// Indo no sentido contrário ao da rota, quem sabe mais é o sensor.
    #[test]
    fn andando_contra_o_sentido_da_rota_o_gps_manda() {
        let g = Guia::nova(rota_reta());
        let torto = ao_lado((-23.5695, -46.6462), 20.0);

        // A rota reta corre para noroeste (~315°); este carro vai para sudeste.
        let crua = Fix {
            lat: torto.0,
            lon: torto.1,
            heading: 135.0,
            speed_kmh: 40.0,
            accuracy_m: 10.0,
        };
        assert_eq!(g.grudar(&crua), crua, "grudou o carro na contramão");
    }

    /// Parado, o mesmo desacordo de rumo é só ruído do GPS — aí a rua manda.
    #[test]
    fn parado_o_desacordo_de_rumo_nao_impede_o_encaixe() {
        let g = Guia::nova(rota_reta());
        let torto = ao_lado((-23.5695, -46.6462), 20.0);

        let crua = Fix {
            lat: torto.0,
            lon: torto.1,
            heading: 135.0,
            speed_kmh: 0.0,
            accuracy_m: 10.0,
        };
        let grudado = g.grudar(&crua);

        assert_ne!(grudado, crua, "deixou o carro torto no semáforo");
        assert!(
            diferenca_angular(grudado.heading, 135.0) > 90.0,
            "manteve o rumo ruidoso do GPS parado"
        );
    }

    /// O caso do notebook e do semáforo: o provedor nunca relatou rumo, então
    /// `heading` é zero. Sem a rua, a seta fica apontada para o norte.
    #[test]
    fn parado_sem_rumo_do_gps_a_seta_ainda_aponta_para_a_rua() {
        let g = Guia::nova(rota_reta());

        let crua = Fix {
            lat: -23.5695,
            lon: -46.6462,
            heading: 0.0,
            speed_kmh: 0.0,
            accuracy_m: 30.0,
        };
        let grudado = g.grudar(&crua);

        assert!(grudado.heading > 300.0, "seta em {}°", grudado.heading);
    }

    #[test]
    fn rota_vazia_nao_gruda_nem_quebra() {
        let g = Guia::nova(Route {
            destino: "lugar nenhum".into(),
            pontos: vec![],
            passos: vec![],
            distancia_total_m: 0.0,
            duracao_total_s: 0,
        });
        let crua = em(-23.57, -46.64);

        assert_eq!(g.grudar(&crua), crua);
    }

    /// Traçado degenerado (pontos repetidos) não tem direção — grudar nele
    /// apontaria a seta para o norte em nome de "corrigir" o GPS.
    #[test]
    fn tracado_sem_comprimento_nao_gruda() {
        let g = Guia::nova(Route {
            destino: "parado".into(),
            pontos: vec![(-23.57, -46.64), (-23.57, -46.64)],
            passos: vec![],
            distancia_total_m: 0.0,
            duracao_total_s: 0,
        });
        let crua = em(-23.57, -46.64);

        assert_eq!(g.grudar(&crua), crua);
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
