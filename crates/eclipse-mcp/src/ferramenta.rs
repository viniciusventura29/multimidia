//! A declaração de uma ferramenta: nome, para que serve, e o que ela aceita.

use serde_json::{json, Value};

/// Uma ferramenta do catálogo.
///
/// Não deriva `Serialize` de propósito. Os dois formatos de fio que interessam
/// discordam justamente no nome deste campo — o MCP pede `inputSchema`, a API da
/// Anthropic pede `input_schema` — então cada um monta o seu explicitamente
/// (veja [`crate::protocolo`] e o cliente do `eclipse-ia`). Um derive aqui só
/// serviria para um dos dois e enganaria quem lesse o outro.
#[derive(Clone, Debug, PartialEq)]
pub struct Ferramenta {
    pub nome: String,

    /// **Escreva prescritivo: diga _quando_ chamar, não só o que devolve.**
    ///
    /// Não é preciosismo de estilo. Os modelos recentes são conservadores ao
    /// pegar ferramenta, e uma descrição que começa com "chame isto quando…"
    /// muda de forma mensurável a frequência com que a ferramenta certa é
    /// escolhida. "Devolve a telemetria" é pior que "chame antes de comentar
    /// qualquer coisa sobre o carro".
    pub descricao: String,

    /// JSON Schema dos argumentos. Use [`esquema_objeto`] ou [`sem_argumentos`],
    /// que já saem no formato aceito por `strict: true`.
    pub esquema: Value,
}

impl Ferramenta {
    pub fn nova(
        nome: impl Into<String>,
        descricao: impl Into<String>,
        esquema: Value,
    ) -> Self {
        Self {
            nome: nome.into(),
            descricao: descricao.into(),
            esquema,
        }
    }
}

/// Uma ferramenta que não recebe argumento nenhum — o caso comum aqui, já que
/// quase tudo que o carro sabe é "me diga como está agora".
pub fn sem_argumentos(nome: impl Into<String>, descricao: impl Into<String>) -> Ferramenta {
    Ferramenta::nova(nome, descricao, esquema_objeto(&[]))
}

/// Monta o esquema de um objeto com todos os campos obrigatórios.
///
/// `additionalProperties: false` e `required` completo são exigência do
/// `strict: true` da Anthropic — sem os dois, a validação estrita é recusada com
/// 400. Campo opcional se faz aceitando `null` no tipo (`["string", "null"]`),
/// não tirando da lista de obrigatórios.
pub fn esquema_objeto(campos: &[(&str, Value)]) -> Value {
    let mut propriedades = serde_json::Map::new();
    let mut obrigatorios = Vec::with_capacity(campos.len());

    for (nome, esquema) in campos {
        propriedades.insert((*nome).to_string(), esquema.clone());
        obrigatorios.push(Value::String((*nome).to_string()));
    }

    json!({
        "type": "object",
        "properties": Value::Object(propriedades),
        "required": Value::Array(obrigatorios),
        "additionalProperties": false,
    })
}

/// Um campo simples do esquema: tipo e descrição.
pub fn campo(tipo: &str, descricao: &str) -> Value {
    json!({ "type": tipo, "description": descricao })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esquema_marca_tudo_como_obrigatorio_e_fecha_o_objeto() {
        let esquema = esquema_objeto(&[
            ("lugar", campo("string", "o nome do lugar")),
            ("largura", campo("integer", "largura em pixels")),
        ]);

        assert_eq!(esquema["type"], "object");
        assert_eq!(esquema["additionalProperties"], false);
        assert_eq!(esquema["required"], json!(["lugar", "largura"]));
        assert_eq!(esquema["properties"]["lugar"]["type"], "string");
    }

    #[test]
    fn sem_argumentos_ainda_e_um_objeto_valido() {
        let f = sem_argumentos("carro_telemetria", "chame antes de falar do carro");
        assert_eq!(f.esquema["type"], "object");
        assert_eq!(f.esquema["required"], json!([]));
        assert_eq!(f.esquema["additionalProperties"], false);
    }
}
