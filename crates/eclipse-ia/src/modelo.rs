//! Qual modelo usar, e no que os dois diferem.
//!
//! Não é só trocar a string: Haiku e Opus aceitam **conjuntos diferentes de
//! parâmetros**, e mandar o do outro é 400 na cara — dentro do carro, longe do
//! log. Por isso a diferença vive num lugar só, com teste.

use serde_json::{json, Value};

/// O modelo que atende um gatilho.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modelo {
    /// Para o rotineiro: a saudação de quando o carro liga, o comentário
    /// periódico da viagem. Barato, e o trabalho é pequeno.
    Haiku,
    /// Para o que exige pesquisar e cruzar coisas: destino novo, alerta do
    /// carro. Aqui a qualidade da leitura importa mais que o custo.
    Opus,
}

impl Modelo {
    pub fn id(self) -> &'static str {
        match self {
            Self::Haiku => "claude-haiku-4-5",
            Self::Opus => "claude-opus-4-8",
        }
    }

    /// O `effort` existe no Opus 4.8 e **dá 400 no Haiku 4.5**.
    pub fn esforco(self) -> Option<&'static str> {
        match self {
            Self::Haiku => None,
            // `medium` e não `high`: aqui o trabalho é pesquisar duas ou três
            // coisas e resumir, não resolver um problema difícil. Esforço alto
            // gastaria token deliberando sobre um cartão de duas frases.
            Self::Opus => Some("medium"),
        }
    }

    /// Pensamento adaptativo. No Opus 4.8 é preciso pedir explicitamente —
    /// omitir o campo roda sem pensar nenhum.
    pub fn pensamento(self) -> Option<Value> {
        match self {
            Self::Haiku => None,
            Self::Opus => Some(json!({ "type": "adaptive" })),
        }
    }

    /// Teto de saída. Um quadro são seis cartões curtos; o resto do orçamento é
    /// pensamento e chamada de ferramenta.
    pub fn max_tokens(self) -> u32 {
        4096
    }

    /// As ferramentas de servidor que este modelo aceita.
    ///
    /// A variante `_20260209` (com filtragem dinâmica) roda no Opus 4.8, e o
    /// Haiku 4.5 só aceita a busca básica `_20250305`. Busca web errada para o
    /// modelo é 400. O `web_fetch` também fica só no Opus: no Haiku o suporte
    /// não é garantido, e busca sozinha já resolve o caso rotineiro.
    pub fn ferramentas_de_servidor(self) -> Vec<Value> {
        match self {
            Self::Haiku => vec![json!({
                "type": "web_search_20250305",
                "name": "web_search",
                "max_uses": 3,
            })],
            Self::Opus => vec![
                json!({
                    "type": "web_search_20260209",
                    "name": "web_search",
                    "max_uses": 5,
                }),
                json!({
                    "type": "web_fetch_20260209",
                    "name": "web_fetch",
                    "max_uses": 3,
                }),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haiku_nao_leva_effort_nem_thinking() {
        assert!(
            Modelo::Haiku.esforco().is_none(),
            "`effort` no Haiku 4.5 é 400"
        );
        assert!(Modelo::Haiku.pensamento().is_none());
    }

    #[test]
    fn cada_modelo_leva_a_variante_de_busca_que_aceita() {
        let haiku = Modelo::Haiku.ferramentas_de_servidor();
        assert_eq!(haiku.len(), 1);
        assert_eq!(haiku[0]["type"], "web_search_20250305");

        let opus = Modelo::Opus.ferramentas_de_servidor();
        let tipos: Vec<&str> = opus.iter().map(|f| f["type"].as_str().unwrap()).collect();
        assert_eq!(tipos, ["web_search_20260209", "web_fetch_20260209"]);
    }

    #[test]
    fn toda_ferramenta_de_servidor_tem_teto_de_uso() {
        // Sem `max_uses` uma pesquisa mal calibrada vira dez buscas num turno.
        for modelo in [Modelo::Haiku, Modelo::Opus] {
            for f in modelo.ferramentas_de_servidor() {
                assert!(
                    f["max_uses"].is_u64(),
                    "{} sem max_uses em {}",
                    modelo.id(),
                    f["type"]
                );
            }
        }
    }
}
