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
    /// **As duas usam a variante básica, e isso foi medido, não escolhido no
    /// chute.** A variante `_20260209` da busca faz filtragem dinâmica — ela
    /// roda código do lado da Anthropic e traz conteúdo de página filtrado para
    /// dentro do contexto, em vez de só trechos. Serve para pesquisa difícil;
    /// aqui, para dizer se vai ter neblina na serra, ela custa caro à toa:
    ///
    /// | busca | entrada do turno | custo |
    /// |---|---|---|
    /// | `_20260209` | 139 mil tokens | ~US$ 0,65 |
    /// | `_20250305` | ~15 mil tokens | ~US$ 0,05 |
    ///
    /// A básica também não abre contêiner de execução de código, o que elimina
    /// junto toda a complicação de carregar `container` entre as requisições.
    ///
    /// O `web_fetch` fica só no Opus: no Haiku o suporte não é garantido, e a
    /// busca sozinha já resolve o caso rotineiro.
    pub fn ferramentas_de_servidor(self) -> Vec<Value> {
        let busca = json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 3,
        });

        match self {
            Self::Haiku => vec![busca],
            Self::Opus => vec![
                busca,
                json!({
                    "type": "web_fetch_20250910",
                    "name": "web_fetch",
                    "max_uses": 2,
                    // Uma página inteira no contexto custa mais do que vale:
                    // para saber o tempo e o trânsito num destino, o começo
                    // basta.
                    "max_content_tokens": 6000,
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
        assert_eq!(tipos, ["web_search_20250305", "web_fetch_20250910"]);

        // A filtragem dinâmica da `_20260209` custou 13x mais num turno medido.
        // Se alguém voltar a ligá-la, que seja de olho aberto.
        assert!(
            !tipos.iter().any(|t| t.ends_with("20260209")),
            "a variante com filtragem dinâmica traz conteúdo de página demais para este uso"
        );
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
