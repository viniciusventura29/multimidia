//! A conversa com a API de Mensagens da Anthropic.
//!
//! Não há SDK oficial em Rust, então é HTTP na mão. O traço [`Transporte`]
//! existe para o laço do agente poder ser testado inteiro contra respostas
//! gravadas — mesmo espírito do `Elm327Transport`, que deixa o protocolo do OBD
//! ser testado sem adaptador nenhum.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

pub const URL_MENSAGENS: &str = "https://api.anthropic.com/v1/messages";
pub const VERSAO_API: &str = "2023-06-01";

/// Beta necessário para o conector de MCP remoto.
pub const BETA_MCP: &str = "mcp-client-2025-11-20";

#[derive(Debug, thiserror::Error)]
pub enum IaError {
    #[error("falta a chave da Anthropic")]
    SemChave,

    #[error("a rede falhou: {0}")]
    Rede(String),

    /// 4xx que não adianta repetir — pedido malformado, chave inválida, modelo
    /// desconhecido. O texto vem junto porque é onde a API explica o que errou.
    #[error("a API recusou ({status}): {corpo}")]
    Recusado { status: u16, corpo: String },

    /// 429 e 5xx: repetir adianta, e o cliente já repetiu o quanto valia.
    #[error("a API está sobrecarregada ({status})")]
    Sobrecarregada { status: u16 },

    #[error("resposta que não entendi: {0}")]
    Resposta(String),

    #[error("o modelo não terminou depois de {0} idas e voltas")]
    NaoTerminou(usize),
}

impl IaError {
    /// Se vale a pena tentar de novo mais tarde. O módulo usa isto para decidir
    /// entre degradar em silêncio e reagendar.
    pub fn temporario(&self) -> bool {
        matches!(self, Self::Rede(_) | Self::Sobrecarregada { .. })
    }
}

#[async_trait]
pub trait Transporte: Send + Sync {
    /// Manda um corpo de `/v1/messages` e devolve a resposta crua.
    ///
    /// `betas` vira o cabeçalho `anthropic-beta`. Vai por chamada, e não fixo no
    /// construtor, porque o beta do MCP só entra quando há servidor remoto
    /// configurado — mandar sempre seria pedir um recurso que não se usa.
    async fn enviar(&self, corpo: &Value, betas: &[String]) -> Result<Value, IaError>;
}

/// Quantas vezes insistir num 429/5xx antes de desistir.
///
/// Três, e não mais: isto roda com o carro andando. Uma saudação que chega
/// quatro minutos depois de ligar o carro não é uma saudação.
const TENTATIVAS: usize = 3;

pub struct TransporteHttp {
    chave: String,
    cliente: reqwest::Client,
}

impl TransporteHttp {
    pub fn novo(chave: String) -> Result<Self, IaError> {
        if chave.trim().is_empty() {
            return Err(IaError::SemChave);
        }

        let cliente = reqwest::Client::builder()
            // Um turno com pesquisa web pode demorar; o teto existe para o
            // módulo não ficar preso para sempre num socket meio aberto.
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| IaError::Rede(e.to_string()))?;

        Ok(Self { chave, cliente })
    }
}

#[async_trait]
impl Transporte for TransporteHttp {
    async fn enviar(&self, corpo: &Value, betas: &[String]) -> Result<Value, IaError> {
        let mut espera = Duration::from_millis(500);

        for tentativa in 1..=TENTATIVAS {
            let mut pedido = self
                .cliente
                .post(URL_MENSAGENS)
                .header("x-api-key", &self.chave)
                .header("anthropic-version", VERSAO_API)
                .json(corpo);

            if !betas.is_empty() {
                pedido = pedido.header("anthropic-beta", betas.join(","));
            }

            let resposta = match pedido.send().await {
                Ok(r) => r,
                Err(err) => {
                    if tentativa == TENTATIVAS {
                        return Err(IaError::Rede(err.to_string()));
                    }
                    tokio::time::sleep(espera).await;
                    espera *= 2;
                    continue;
                }
            };

            let status = resposta.status();
            if status.is_success() {
                return resposta
                    .json()
                    .await
                    .map_err(|e| IaError::Resposta(e.to_string()));
            }

            let repetivel = status.as_u16() == 429 || status.is_server_error();
            if !repetivel {
                let corpo = resposta.text().await.unwrap_or_default();
                return Err(IaError::Recusado {
                    status: status.as_u16(),
                    corpo,
                });
            }

            if tentativa == TENTATIVAS {
                return Err(IaError::Sobrecarregada {
                    status: status.as_u16(),
                });
            }

            // `retry-after` quando a API diz quanto esperar; senão, dobrando.
            let sugerida = resposta
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);

            tokio::time::sleep(sugerida.unwrap_or(espera)).await;
            espera *= 2;
        }

        unreachable!("o laço sempre volta ou erra na última tentativa")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chave_vazia_e_recusada_na_construcao() {
        assert!(matches!(
            TransporteHttp::novo("   ".into()),
            Err(IaError::SemChave)
        ));
    }

    #[test]
    fn so_rede_e_sobrecarga_valem_nova_tentativa() {
        assert!(IaError::Rede("timeout".into()).temporario());
        assert!(IaError::Sobrecarregada { status: 529 }.temporario());
        assert!(!IaError::Recusado {
            status: 400,
            corpo: "modelo desconhecido".into()
        }
        .temporario());
        assert!(!IaError::SemChave.temporario());
    }
}
