//! O catálogo MCP do Eclipse, servido em `127.0.0.1` para o Eclipse Mãos.
//!
//! O Mãos (`android-maos/`) é quem liga o carro ao relay e fala com o Claude
//! de fora. Os dados do carro (OBD, GPS, música) moram aqui, então ele os
//! busca nesta porta e os repassa como ferramentas suas — e pergunta a
//! velocidade antes de tocar na tela.
//!
//! **Por que não o Eclipse direto no relay.** O APK do Eclipse vai para um
//! release PÚBLICO no GitHub (o canal de atualização), e o relay exige uma
//! chave que dá controle da tela do carro. Chave dentro de APK público é chave
//! publicada. O Mãos nunca vai a público; o Eclipse só precisa ouvir em
//! loopback, onde nada fora da própria central alcança.
//!
//! Só leitura: o `ProvedorCarro` (o mesmo que o assistente usa, com as mesmas
//! regras de não vazar segredo) e o estado dos módulos. Nada de imagem nem do
//! quadro, que gastam dinheiro ou mexem na tela.

use std::sync::Arc;

use async_trait::async_trait;
use eclipse_core::{StateEnvelope, Status};
use eclipse_mcp::{sem_argumentos, Ferramenta, McpError, Provedor, Registro};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::assistente::carro::{FonteDeEstado, ProvedorCarro, RelogioDoSistema};

/// Combinada com `android-maos/.../Eclipse.kt`.
pub const PORTA: u16 = 47823;

pub fn iniciar(fonte: Arc<dyn FonteDeEstado>, perfil: Option<String>) {
    let registro = Arc::new(
        Registro::nova()
            .com(Arc::new(ProvedorCarro::novo(
                Arc::clone(&fonte),
                Arc::new(RelogioDoSistema),
                perfil,
            )))
            .com(Arc::new(ProvedorModulos { fonte })),
    );
    tauri::async_runtime::spawn(async move {
        match TcpListener::bind(("127.0.0.1", PORTA)).await {
            Ok(ouvinte) => eclipse_mcp::http::servir(ouvinte, registro).await,
            // Outra instância já ouvindo (o Eclipse DEV ao lado do de
            // verdade): fica sem, e o Mãos fala com a outra.
            Err(e) => tracing::warn!("mcp local: não abriu 127.0.0.1:{PORTA}: {e}"),
        }
    });
}

/// "Por que o OBD não sobe?" sem abrir a tela de diagnóstico.
struct ProvedorModulos {
    fonte: Arc<dyn FonteDeEstado>,
}

#[async_trait]
impl Provedor for ProvedorModulos {
    fn ferramentas(&self) -> Vec<Ferramenta> {
        vec![sem_argumentos(
            "eclipse_modulos",
            "Estado de cada módulo do Eclipse (obd, nav, music, messaging, clima, assistente…): \
             pronto, subindo ou degradado, e o motivo quando degradado. Use para descobrir por \
             que uma parte do painel não funciona antes de olhar a tela.",
        )]
    }

    async fn chamar(&self, nome: &str, _args: &Value) -> Result<Value, McpError> {
        if nome != "eclipse_modulos" {
            return Err(McpError::Desconhecida(nome.to_string()));
        }
        Ok(Value::Array(
            self.fonte.estados().iter().map(resumo_do_modulo).collect(),
        ))
    }
}

/// Status e motivo, sem o payload: o do `nav` carrega a chave do Google.
fn resumo_do_modulo(e: &StateEnvelope) -> Value {
    json!({
        "modulo": e.module.as_str(),
        "status": match e.status {
            Status::Ready => "pronto",
            Status::Loading => "subindo",
            Status::Degraded => "degradado",
        },
        "motivo": e.reason,
        "atualizacoes": e.seq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eclipse_core::ModuleId;

    #[test]
    fn o_payload_do_modulo_nao_vaza() {
        let e = StateEnvelope {
            module: ModuleId::new("nav"),
            seq: 7,
            status: Status::Degraded,
            data: Some(Arc::new(json!({ "apiKey": "SEGREDO" }))),
            reason: Some("sem GPS".into()),
        };
        let r = resumo_do_modulo(&e).to_string();
        assert!(r.contains("degradado") && r.contains("sem GPS"), "{r}");
        assert!(!r.contains("SEGREDO"), "{r}");
    }
}
