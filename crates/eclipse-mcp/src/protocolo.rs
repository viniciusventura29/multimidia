//! O lado servidor do MCP: JSON-RPC 2.0 sobre um [`Registro`].
//!
//! Isto existe para que "tudo é MCP" seja verdade e não figura de linguagem. Um
//! [`Registro`] respondendo por aqui **é** um servidor MCP; o que falta é só um
//! transporte que leve bytes até [`atender`] e traga a resposta de volta —
//! stdio, um socket em localhost, o que for. Nada disso mexe no catálogo.
//!
//! Enquanto o único consumidor for o agente no mesmo processo, ele fala direto
//! com o [`Registro`] e pula tudo isto — não faz sentido serializar para si
//! mesmo. O valor deste módulo é manter honesta a promessa de que dá para
//! plugar um cliente externo sem redesenhar nada.

use serde_json::{json, Value};

use crate::{Ferramenta, Registro};

/// A versão do protocolo que anunciamos no `initialize`.
pub const VERSAO_PROTOCOLO: &str = "2025-06-18";

const ERRO_METODO: i64 = -32601;
const ERRO_PARAMS: i64 = -32602;

/// Uma ferramenta na forma do fio do MCP.
pub fn ferramenta_em_mcp(f: &Ferramenta) -> Value {
    json!({
        "name": f.nome,
        "description": f.descricao,
        "inputSchema": f.esquema,
    })
}

/// Responde uma requisição JSON-RPC.
///
/// Devolve `None` para notificação (requisição sem `id`), que por definição não
/// tem resposta — `notifications/initialized` é o caso que aparece na prática.
pub async fn atender(registro: &Registro, requisicao: &Value) -> Option<Value> {
    let id = requisicao.get("id").cloned();
    let metodo = requisicao
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("");
    let params = requisicao.get("params").cloned().unwrap_or(json!({}));

    // Notificação: sem `id`, sem resposta. Responder mesmo assim quebra
    // clientes estritos.
    let id = id?;

    let resposta = match metodo {
        "initialize" => Ok(json!({
            "protocolVersion": VERSAO_PROTOCOLO,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "eclipse-os", "version": env!("CARGO_PKG_VERSION") },
        })),

        "tools/list" => Ok(json!({
            "tools": registro.listar().iter().map(ferramenta_em_mcp).collect::<Vec<_>>(),
        })),

        "tools/call" => {
            let nome = params.get("name").and_then(Value::as_str);
            match nome {
                None => Err((ERRO_PARAMS, "falta `name` em tools/call".to_string())),
                Some(nome) => {
                    let args = params.get("arguments").cloned().unwrap_or(json!({}));
                    let r = registro.chamar(nome, &args).await;
                    Ok(json!({
                        "content": [{ "type": "text", "text": r.texto() }],
                        "isError": r.erro,
                    }))
                }
            }
        }

        outro => Err((ERRO_METODO, format!("método desconhecido: `{outro}`"))),
    };

    Some(match resposta {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registro::testes::Dublê;
    use std::sync::Arc;

    fn registro() -> Registro {
        Registro::nova().com(Arc::new(Dublê {
            responde: json!({ "rpm": 2500 }),
            ..Dublê::com_nomes(&["carro_telemetria"])
        }))
    }

    #[tokio::test]
    async fn tools_list_sai_no_formato_do_mcp() {
        let r = atender(
            &registro(),
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .await
        .unwrap();

        let ferramenta = &r["result"]["tools"][0];
        assert_eq!(ferramenta["name"], "carro_telemetria");
        // `inputSchema`, não `esquema` nem `input_schema`.
        assert_eq!(ferramenta["inputSchema"]["type"], "object");
    }

    #[tokio::test]
    async fn tools_call_embrulha_o_resultado_em_content() {
        let r = atender(
            &registro(),
            &json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "carro_telemetria", "arguments": {} },
            }),
        )
        .await
        .unwrap();

        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["content"][0]["type"], "text");
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("2500"));
    }

    #[tokio::test]
    async fn ferramenta_que_falha_vira_is_error_e_nao_erro_de_rpc() {
        let registro = Registro::nova().com(Arc::new(Dublê {
            falha: true,
            ..Dublê::com_nomes(&["quebrada"])
        }));

        let r = atender(
            &registro,
            &json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "quebrada" },
            }),
        )
        .await
        .unwrap();

        // Erro de ferramenta é resultado com `isError`; erro de JSON-RPC é outra
        // coisa (protocolo malformado). Confundir os dois faz o cliente achar
        // que o servidor caiu quando só o barramento do carro não respondeu.
        assert!(r.get("error").is_none());
        assert_eq!(r["result"]["isError"], true);
    }

    #[tokio::test]
    async fn notificacao_nao_tem_resposta() {
        let r = atender(
            &registro(),
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn metodo_desconhecido_vira_erro_de_rpc() {
        let r = atender(
            &registro(),
            &json!({ "jsonrpc": "2.0", "id": 4, "method": "voar" }),
        )
        .await
        .unwrap();
        assert_eq!(r["error"]["code"], ERRO_METODO);
    }
}
