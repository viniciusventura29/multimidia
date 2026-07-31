//! A ferramenta `pintar_quadro`: a única saída do assistente.
//!
//! Tudo que o modelo descobre só chega ao motorista se ele chamar esta
//! ferramenta. Texto solto na resposta final é ignorado de propósito — a tela
//! desenha cartões tipados, não prosa, e uma saída só evita duas maneiras de
//! dizer a mesma coisa.
//!
//! **Por que ferramenta e não `output_config.format`.** Saída estruturada
//! obriga a resposta inteira a ser um JSON, o que não compõe com uso de
//! ferramenta: o modelo precisa pesquisar *antes* de pintar. Como ferramenta,
//! pintar é só mais um passo do laço — e ele pode pintar o que já sabe,
//! continuar pesquisando e repintar.

use std::sync::Mutex;

use async_trait::async_trait;
use eclipse_mcp::{Ferramenta, McpError, Provedor};
use serde_json::{json, Value};

use crate::cartao::{Quadro, TipoGrafico, Tom, MAXIMO_CARTOES};

pub const PINTAR: &str = "pintar_quadro";

/// Guarda o último quadro pintado no turno.
#[derive(Default)]
pub struct ProvedorQuadro {
    ultimo: Mutex<Option<Quadro>>,
}

impl ProvedorQuadro {
    pub fn novo() -> Self {
        Self::default()
    }

    /// Pega o que foi pintado e esvazia, deixando o provedor pronto para o
    /// próximo turno.
    pub fn tomar(&self) -> Option<Quadro> {
        self.ultimo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

/// Um campo que pode vir nulo.
///
/// Sob `strict: true` a Anthropic exige que **todo** campo esteja em `required`
/// e que `additionalProperties` seja `false`. Campo opcional não se faz tirando
/// da lista de obrigatórios — se faz aceitando `null` no tipo.
fn ou_nulo(tipo: &str, descricao: &str) -> Value {
    json!({ "type": [tipo, "null"], "description": descricao })
}

fn variante(tipo: &str, campos: &[(&str, Value)]) -> Value {
    let mut propriedades = serde_json::Map::new();
    let mut obrigatorios = vec![json!("tipo")];

    propriedades.insert("tipo".into(), json!({ "const": tipo }));
    for (nome, esquema) in campos {
        propriedades.insert((*nome).to_string(), esquema.clone());
        obrigatorios.push(json!(nome));
    }

    json!({
        "type": "object",
        "properties": propriedades,
        "required": obrigatorios,
        "additionalProperties": false,
    })
}

/// O esquema de um cartão: união marcada pelo campo `tipo`.
pub fn esquema_cartao() -> Value {
    let tom = json!({
        "enum": Tom::TODOS,
        "description": "a cor do cartão. `alerta` só para o que exige ação agora.",
    });

    json!({
        "anyOf": [
            variante("texto", &[
                ("titulo", ou_nulo("string", "título curto, 3 palavras no máximo")),
                ("corpo", json!({
                    "type": "string",
                    "description": "uma ou duas frases. A coluna é estreita: texto longo não cabe."
                })),
                ("tom", tom.clone()),
            ]),
            variante("metrica", &[
                ("rotulo", json!({ "type": "string", "description": "o que o número é, curto" })),
                ("valor", json!({ "type": "string", "description": "o número já formatado" })),
                ("unidade", ou_nulo("string", "km, °C, %, min…")),
                ("tom", tom),
            ]),
            variante("grafico", &[
                ("titulo", json!({ "type": "string" })),
                ("grafico", json!({ "enum": TipoGrafico::TODOS })),
                ("unidade", ou_nulo("string", "a unidade do eixo dos valores")),
                ("pontos", json!({
                    "type": "array",
                    "description": "entre 2 e 8 pontos. Menos que 2 não é gráfico; mais que 8 não cabe na coluna.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "rotulo": { "type": "string", "description": "rótulo curto do eixo" },
                            "valor": { "type": "number" },
                        },
                        "required": ["rotulo", "valor"],
                        "additionalProperties": false,
                    },
                })),
            ]),
            variante("imagem", &[
                ("url", json!({
                    "type": "string",
                    "description": "um https:// (capa de álbum, foto de lugar) ou o `arquivo:…` \
                        que `gerar_imagem` devolveu. Copie exatamente como veio.",
                })),
                ("legenda", ou_nulo("string", "legenda curta")),
            ]),
            variante("lista", &[
                ("titulo", ou_nulo("string", "título curto")),
                ("itens", json!({
                    "type": "array",
                    "description": "de 2 a 5 itens, uma linha cada",
                    "items": { "type": "string" },
                })),
            ]),
        ],
    })
}

#[async_trait]
impl Provedor for ProvedorQuadro {
    fn ferramentas(&self) -> Vec<Ferramenta> {
        vec![Ferramenta::nova(
            PINTAR,
            format!(
                "Escreve no quadro do painel. **É a única forma de o motorista ver alguma \
                 coisa** — o que você responder em texto não aparece em lugar nenhum. Chame \
                 uma vez, no fim, com no máximo {MAXIMO_CARTOES} cartões. Se ainda estiver \
                 pesquisando, termine antes: pode chamar de novo e o quadro é substituído."
            ),
            json!({
                "type": "object",
                "properties": {
                    "cartoes": {
                        "type": "array",
                        "description": "os cartões, do mais importante para o menos",
                        "items": esquema_cartao(),
                    },
                },
                "required": ["cartoes"],
                "additionalProperties": false,
            }),
        )]
    }

    async fn chamar(&self, nome: &str, args: &Value) -> Result<Value, McpError> {
        if nome != PINTAR {
            return Err(McpError::Desconhecida(nome.to_string()));
        }

        // Recusar com o erro do serde por escrito é de propósito: o modelo lê o
        // motivo e conserta na próxima iteração. Aceitar um quadro meio
        // entendido pintaria a tela errada em silêncio, que é bem pior.
        let quadro: Quadro = serde_json::from_value(args.clone())
            .map_err(|err| McpError::argumento(format!("cartão inválido: {err}")))?;

        if quadro.vazio() {
            return Err(McpError::argumento(
                "quadro sem cartão nenhum — se não há o que dizer, não chame esta ferramenta",
            ));
        }

        let quadro = quadro.aparado();
        let quantos = quadro.cartoes.len();
        *self.ultimo.lock().unwrap_or_else(|e| e.into_inner()) = Some(quadro);

        Ok(json!({ "pintado": true, "cartoes": quantos }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartao::Cartao;

    #[tokio::test]
    async fn pintar_guarda_o_quadro() {
        let p = ProvedorQuadro::novo();
        let r = p
            .chamar(
                PINTAR,
                &json!({ "cartoes": [
                    { "tipo": "texto", "corpo": "Sábado limpo, 19°C.", "tom": "bom", "titulo": null },
                    { "tipo": "metrica", "rotulo": "Combustível", "valor": "45", "unidade": "%", "tom": "atencao" },
                ]}),
            )
            .await
            .unwrap();

        assert_eq!(r["cartoes"], 2);
        let quadro = p.tomar().unwrap();
        assert_eq!(quadro.cartoes.len(), 2);
        assert!(matches!(quadro.cartoes[1], Cartao::Metrica { .. }));
    }

    #[tokio::test]
    async fn tomar_esvazia_para_o_proximo_turno() {
        let p = ProvedorQuadro::novo();
        p.chamar(PINTAR, &json!({ "cartoes": [{ "tipo": "texto", "corpo": "oi" }] }))
            .await
            .unwrap();

        assert!(p.tomar().is_some());
        assert!(p.tomar().is_none(), "o quadro não pode repetir no turno seguinte");
    }

    /// O erro tem que ser legível: é ele que o modelo lê para consertar.
    #[tokio::test]
    async fn cartao_malformado_e_recusado_com_motivo() {
        let p = ProvedorQuadro::novo();
        let err = p
            .chamar(PINTAR, &json!({ "cartoes": [{ "tipo": "grafico", "titulo": "x" }] }))
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("cartão inválido"),
            "erro pouco útil: {err}"
        );
        assert!(p.tomar().is_none(), "nada pode ter sido guardado");
    }

    #[tokio::test]
    async fn quadro_vazio_e_recusado() {
        let p = ProvedorQuadro::novo();
        assert!(p.chamar(PINTAR, &json!({ "cartoes": [] })).await.is_err());
    }

    #[tokio::test]
    async fn repintar_substitui_em_vez_de_acumular() {
        let p = ProvedorQuadro::novo();
        p.chamar(PINTAR, &json!({ "cartoes": [{ "tipo": "texto", "corpo": "rascunho" }] }))
            .await
            .unwrap();
        p.chamar(PINTAR, &json!({ "cartoes": [{ "tipo": "texto", "corpo": "final" }] }))
            .await
            .unwrap();

        let quadro = p.tomar().unwrap();
        assert_eq!(quadro.cartoes.len(), 1);
        assert!(matches!(&quadro.cartoes[0], Cartao::Texto { corpo, .. } if corpo == "final"));
    }

    /// Sob `strict: true` a API recusa esquema sem `additionalProperties: false`
    /// ou com campo fora de `required`. Um erro aqui vira 400 na primeira
    /// chamada de verdade, dentro do carro — melhor pegar no teste.
    #[test]
    fn esquema_obedece_as_regras_do_modo_estrito() {
        fn conferir(esquema: &Value, caminho: &str) {
            if esquema["type"] == "object" {
                assert_eq!(
                    esquema["additionalProperties"], false,
                    "{caminho} não fecha o objeto"
                );

                let props: Vec<&String> = esquema["properties"]
                    .as_object()
                    .expect("objeto sem properties")
                    .keys()
                    .collect();
                let obrigatorios: Vec<String> = esquema["required"]
                    .as_array()
                    .expect("objeto sem required")
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();

                for p in &props {
                    assert!(
                        obrigatorios.contains(p),
                        "{caminho}.{p} está fora de `required`"
                    );
                }

                for (nome, sub) in esquema["properties"].as_object().unwrap() {
                    conferir(sub, &format!("{caminho}.{nome}"));
                }
            }

            if let Some(itens) = esquema.get("items") {
                conferir(itens, &format!("{caminho}[]"));
            }
            if let Some(uniao) = esquema.get("anyOf").and_then(Value::as_array) {
                for (i, sub) in uniao.iter().enumerate() {
                    conferir(sub, &format!("{caminho}|{i}"));
                }
            }
        }

        let p = ProvedorQuadro::novo();
        let ferramentas = p.ferramentas();
        conferir(&ferramentas[0].esquema, "pintar_quadro");
    }

    /// Cada variante precisa de um `const` em `tipo`, senão a união fica ambígua
    /// e o modelo tem como montar um cartão que não desserializa.
    #[test]
    fn cada_variante_fixa_o_proprio_tipo() {
        let esquema = esquema_cartao();
        let variantes = esquema["anyOf"].as_array().unwrap();
        assert_eq!(variantes.len(), 5);

        let tipos: Vec<&str> = variantes
            .iter()
            .map(|v| v["properties"]["tipo"]["const"].as_str().expect("sem const"))
            .collect();
        assert_eq!(tipos, ["texto", "metrica", "grafico", "imagem", "lista"]);
    }
}
