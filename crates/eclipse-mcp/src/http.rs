//! Transporte HTTP mínimo para o [`protocolo`](crate::protocolo).
//!
//! HTTP/1.1 escrito à mão: uma requisição por conexão, `POST` com JSON-RPC no
//! corpo, resposta JSON. Existe para o Eclipse Mãos ler o catálogo do Eclipse
//! pela loopback da central (ver `src-tauri/src/mcp_local.rs`); com um cliente
//! só, um servidor HTTP inteiro seria peso no APK sem retorno.
//!
//! Não serve a rede: quem chama escolhe onde ouvir, e o único uso é
//! `127.0.0.1`.

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::{protocolo, Registro};

/// Corpo maior que isto não é JSON-RPC de ferramenta; é engano ou abuso.
const TETO_CORPO: usize = 256 * 1024;

/// Atende para sempre. Conexão com defeito é descartada sem derrubar as outras.
pub async fn servir(ouvinte: TcpListener, registro: Arc<Registro>) {
    loop {
        let Ok((conexao, _)) = ouvinte.accept().await else {
            continue;
        };
        let registro = Arc::clone(&registro);
        tokio::spawn(async move {
            let _ = atender(conexao, &registro).await;
        });
    }
}

async fn atender(conexao: TcpStream, registro: &Registro) -> std::io::Result<()> {
    let mut leitor = BufReader::new(conexao);
    let mut linha = String::new();
    leitor.read_line(&mut linha).await?;
    let post = linha.starts_with("POST ");

    let mut tamanho = 0usize;
    loop {
        linha.clear();
        if leitor.read_line(&mut linha).await? == 0 || linha == "\r\n" || linha == "\n" {
            break;
        }
        if let Some((nome, valor)) = linha.split_once(':') {
            if nome.eq_ignore_ascii_case("content-length") {
                tamanho = valor.trim().parse().unwrap_or(0);
            }
        }
    }

    let (status, corpo) = if !post {
        ("405 Method Not Allowed", String::new())
    } else if tamanho == 0 || tamanho > TETO_CORPO {
        ("400 Bad Request", String::new())
    } else {
        let mut bytes = vec![0u8; tamanho];
        leitor.read_exact(&mut bytes).await?;
        match serde_json::from_slice::<Value>(&bytes) {
            Err(_) => ("400 Bad Request", String::new()),
            Ok(req) => match protocolo::atender(registro, &req).await {
                Some(resposta) => ("200 OK", resposta.to_string()),
                None => ("202 Accepted", String::new()),
            },
        }
    };

    let mut conexao = leitor.into_inner();
    let cabecalho = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        corpo.len()
    );
    conexao.write_all(cabecalho.as_bytes()).await?;
    conexao.write_all(corpo.as_bytes()).await?;
    conexao.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registro::testes::Dublê;
    use serde_json::json;

    async fn subir() -> u16 {
        let registro = Arc::new(Registro::nova().com(Arc::new(Dublê {
            responde: json!({ "rpm": 2500 }),
            ..Dublê::com_nomes(&["carro_telemetria"])
        })));
        let ouvinte = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let porta = ouvinte.local_addr().unwrap().port();
        tokio::spawn(servir(ouvinte, registro));
        porta
    }

    async fn pedir(porta: u16, metodo: &str, corpo: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", porta)).await.unwrap();
        let req = format!(
            "{metodo} /mcp HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{corpo}",
            corpo.len()
        );
        c.write_all(req.as_bytes()).await.unwrap();
        let mut resposta = String::new();
        c.read_to_string(&mut resposta).await.unwrap();
        resposta
    }

    #[tokio::test]
    async fn responde_tools_call() {
        let porta = subir().await;
        let r = pedir(
            porta,
            "POST",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"carro_telemetria"}}"#,
        )
        .await;
        assert!(r.starts_with("HTTP/1.1 200 OK"), "{r}");
        assert!(r.contains("2500"), "{r}");
    }

    #[tokio::test]
    async fn content_length_casa_com_o_corpo() {
        let porta = subir().await;
        let r = pedir(
            porta,
            "POST",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        let (cabecalho, corpo) = r.split_once("\r\n\r\n").unwrap();
        let declarado: usize = cabecalho
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(declarado, corpo.len());
    }

    #[tokio::test]
    async fn notificacao_nao_tem_corpo() {
        let porta = subir().await;
        let r = pedir(
            porta,
            "POST",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert!(r.starts_with("HTTP/1.1 202"), "{r}");
    }

    #[tokio::test]
    async fn so_post() {
        let porta = subir().await;
        let r = pedir(porta, "GET", "").await;
        assert!(r.starts_with("HTTP/1.1 405"), "{r}");
    }

    #[tokio::test]
    async fn json_quebrado_e_400() {
        let porta = subir().await;
        let r = pedir(porta, "POST", "{nem json").await;
        assert!(r.starts_with("HTTP/1.1 400"), "{r}");
    }
}
