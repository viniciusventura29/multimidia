//! O que aparece no quadro.
//!
//! Um cartão é a unidade de saída do assistente: ele não devolve prosa solta,
//! devolve uma lista tipada que a tela sabe desenhar. Isso resolve dois
//! problemas de uma vez — a coluna do assistente é estreita e alta, então texto
//! corrido caberia mal; e um número que o modelo quer destacar vira mostrador de
//! verdade em vez de ficar perdido no meio de um parágrafo.
//!
//! Os tipos aqui têm espelho em `src/modules/assistente/tipos.ts`. Mexer num
//! lado sem mexer no outro quebra a tela em silêncio.

use serde::{Deserialize, Serialize};

/// Quantos cartões cabem num quadro.
///
/// A coluna tem quatro linhas do grid e o motorista está dirigindo. Mais que
/// isso vira parede de texto que ninguém lê a 80 km/h — e o teto também impede
/// um turno caro de encher o estado do módulo.
pub const MAXIMO_CARTOES: usize = 6;

/// A cor de um cartão. Mapeia nas constantes de `src/core/telemetria.ts`, que
/// já pintam os mostradores do OBD — assistente e painel falam a mesma língua
/// de cor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tom {
    #[default]
    Neutro,
    Bom,
    Atencao,
    Alerta,
}

impl Tom {
    pub const TODOS: [&'static str; 4] = ["neutro", "bom", "atencao", "alerta"];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipoGrafico {
    Barras,
    Linha,
}

impl TipoGrafico {
    pub const TODOS: [&'static str; 2] = ["barras", "linha"];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ponto {
    pub rotulo: String,
    pub valor: f64,
}

/// Um quadro na coluna do assistente.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "camelCase")]
pub enum Cartao {
    /// Prosa curta.
    Texto {
        #[serde(default)]
        titulo: Option<String>,
        corpo: String,
        #[serde(default)]
        tom: Tom,
    },

    /// Um número em destaque.
    Metrica {
        rotulo: String,
        /// Já formatado pelo modelo, como texto: é ele quem sabe se cabe uma
        /// casa decimal ou se é melhor arredondar.
        valor: String,
        #[serde(default)]
        unidade: Option<String>,
        #[serde(default)]
        tom: Tom,
    },

    Grafico {
        titulo: String,
        grafico: TipoGrafico,
        #[serde(default)]
        unidade: Option<String>,
        pontos: Vec<Ponto>,
    },

    /// Uma imagem.
    ///
    /// `url` aceita duas formas: um `https://…` comum (capa de álbum, foto de
    /// lugar, imagem achada na web), ou `arquivo:<nome>.png` para o que a
    /// ferramenta `gerar_imagem` gravou no aparelho. A tela distingue pelo
    /// prefixo — um campo só, porque é um campo a menos para o modelo errar.
    Imagem {
        url: String,
        #[serde(default)]
        legenda: Option<String>,
    },

    Lista {
        #[serde(default)]
        titulo: Option<String>,
        itens: Vec<String>,
    },
}

/// O conteúdo inteiro do quadro.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Quadro {
    pub cartoes: Vec<Cartao>,
}

impl Quadro {
    /// Corta o excedente.
    ///
    /// Silenciosamente, e de propósito: devolver erro faria o modelo gastar um
    /// turno inteiro repintando, e os primeiros cartões já são os que ele
    /// considerou mais importantes.
    pub fn aparado(mut self) -> Self {
        self.cartoes.truncate(MAXIMO_CARTOES);
        self
    }

    pub fn vazio(&self) -> bool {
        self.cartoes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cartao_serializa_com_a_etiqueta_que_a_tela_espera() {
        let c = Cartao::Metrica {
            rotulo: "Combustível".into(),
            valor: "45".into(),
            unidade: Some("%".into()),
            tom: Tom::Atencao,
        };
        let v = serde_json::to_value(&c).unwrap();

        assert_eq!(v["tipo"], "metrica");
        assert_eq!(v["tom"], "atencao");
        assert_eq!(v["unidade"], "%");
    }

    /// O modelo pode omitir campo opcional; isso não pode derrubar o turno.
    #[test]
    fn campo_opcional_ausente_desserializa_com_padrao() {
        let c: Cartao = serde_json::from_value(json!({
            "tipo": "texto",
            "corpo": "Trânsito limpo até Campos do Jordão."
        }))
        .unwrap();

        assert!(matches!(
            c,
            Cartao::Texto { titulo: None, tom: Tom::Neutro, .. }
        ));
    }

    #[test]
    fn campo_opcional_nulo_tambem_desserializa() {
        let c: Cartao = serde_json::from_value(json!({
            "tipo": "imagem",
            "url": "https://exemplo/foto.jpg",
            "legenda": null
        }))
        .unwrap();

        assert!(matches!(c, Cartao::Imagem { legenda: None, .. }));
    }

    #[test]
    fn tipo_desconhecido_falha_em_vez_de_virar_cartao_mudo() {
        let r: Result<Cartao, _> =
            serde_json::from_value(json!({ "tipo": "video", "url": "x" }));
        assert!(r.is_err(), "cartão inventado tem que ser recusado");
    }

    #[test]
    fn quadro_grande_demais_e_aparado() {
        let quadro = Quadro {
            cartoes: (0..12)
                .map(|i| Cartao::Texto {
                    titulo: None,
                    corpo: format!("{i}"),
                    tom: Tom::Neutro,
                })
                .collect(),
        }
        .aparado();

        assert_eq!(quadro.cartoes.len(), MAXIMO_CARTOES);
        // Os primeiros é que ficam — são os que o modelo priorizou.
        assert!(matches!(&quadro.cartoes[0], Cartao::Texto { corpo, .. } if corpo == "0"));
    }

    #[test]
    fn ida_e_volta_preserva_o_quadro() {
        let original = Quadro {
            cartoes: vec![
                Cartao::Grafico {
                    titulo: "Consumo".into(),
                    grafico: TipoGrafico::Linha,
                    unidade: Some("km/l".into()),
                    pontos: vec![Ponto {
                        rotulo: "seg".into(),
                        valor: 9.4,
                    }],
                },
                Cartao::Lista {
                    titulo: Some("No caminho".into()),
                    itens: vec!["Pedágio em 12 km".into()],
                },
            ],
        };

        let json = serde_json::to_string(&original).unwrap();
        let volta: Quadro = serde_json::from_str(&json).unwrap();
        assert_eq!(original, volta);
    }
}
