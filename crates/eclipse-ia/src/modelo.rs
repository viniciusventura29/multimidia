//! Qual modelo usar.
//!
//! Só um: Haiku 4.5. O Opus 4.8 já rodava aqui para os gatilhos que pesquisam,
//! mas custava caro demais para um computador de bordo — num dia medido ~98% do
//! gasto veio dele — e ainda era mais lento. Ficou o Haiku para tudo.
//!
//! Mesmo com um modelo só, os parâmetros que a API aceita não são óbvios (o
//! `effort`, por exemplo, dá 400 no Haiku). Por isso o que o modelo aceita vive
//! num lugar só, com teste.

use serde_json::{json, Value};

/// O modelo que atende os gatilhos. Hoje só existe o Haiku.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modelo {
    /// Barato e rápido. Dá conta do trabalho do painel: resumir o que as
    /// ferramentas já entregam prontas.
    Haiku,
}

impl Modelo {
    pub fn id(self) -> &'static str {
        match self {
            Self::Haiku => "claude-haiku-4-5",
        }
    }

    /// Teto de saída. Um quadro são seis cartões curtos; o resto do orçamento é
    /// pensamento e chamada de ferramenta.
    pub fn max_tokens(self) -> u32 {
        4096
    }

    /// As ferramentas de servidor que este modelo aceita.
    ///
    /// **Usa a variante básica da busca, e isso foi medido, não escolhido no
    /// chute.** A variante `_20260209` faz filtragem dinâmica — ela roda código
    /// do lado da Anthropic e traz conteúdo de página filtrado para dentro do
    /// contexto, em vez de só trechos. Serve para pesquisa difícil; aqui, para
    /// dizer se vai ter neblina na serra, ela custa caro à toa:
    ///
    /// | busca | entrada do turno | custo |
    /// |---|---|---|
    /// | `_20260209` | 139 mil tokens | ~US$ 0,65 |
    /// | `_20250305` | ~15 mil tokens | ~US$ 0,05 |
    ///
    /// A básica também não abre contêiner de execução de código, o que elimina
    /// junto toda a complicação de carregar `container` entre as requisições.
    pub fn ferramentas_de_servidor(self) -> Vec<Value> {
        let busca = json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 3,
        });

        match self {
            Self::Haiku => vec![busca],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usa_a_busca_barata_e_nao_a_de_filtragem_dinamica() {
        let ferramentas = Modelo::Haiku.ferramentas_de_servidor();
        assert_eq!(ferramentas.len(), 1);
        assert_eq!(ferramentas[0]["type"], "web_search_20250305");

        // A filtragem dinâmica da `_20260209` custou 13x mais num turno medido.
        // Se alguém voltar a ligá-la, que seja de olho aberto.
        assert!(
            !ferramentas
                .iter()
                .any(|f| f["type"].as_str().unwrap().ends_with("20260209")),
            "a variante com filtragem dinâmica traz conteúdo de página demais para este uso"
        );
    }

    #[test]
    fn toda_ferramenta_de_servidor_tem_teto_de_uso() {
        // Sem `max_uses` uma pesquisa mal calibrada vira dez buscas num turno.
        for f in Modelo::Haiku.ferramentas_de_servidor() {
            assert!(
                f["max_uses"].is_u64(),
                "{} sem max_uses em {}",
                Modelo::Haiku.id(),
                f["type"]
            );
        }
    }
}
