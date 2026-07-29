//! O transporte Bluetooth do ELM327.
//!
//! O `eclipse-obd` sabe o protocolo (o que mandar, como ler de volta) mas não
//! sabe carregar os bytes — isso é [`Elm327Transport`], e no Android quem carrega
//! é o plugin `tauri-plugin-obd-bt` (socket SPP nativo). Este arquivo é a cola:
//! escolhe o adaptador pareado, conecta, e adapta cada comando à ponte do plugin.
//!
//! As chamadas ao plugin (`run_mobile_plugin`) são **bloqueantes** — cada leitura
//! de PID espera o barramento do carro responder (centenas de ms). Por isso vão
//! em `spawn_blocking`, para não travar o executor async onde o poller vive.

// Só é *usado* no Android (o desktop não chama `conectar`), mas segue sendo
// compilado no macOS para type-check. Sem isto, o desktop reclamaria de código
// morto em tudo aqui.
#![cfg_attr(not(mobile), allow(dead_code))]

use async_trait::async_trait;
use eclipse_obd::{Elm327Source, Elm327Transport, ObdError};
use tauri_plugin_obd_bt::{BtDevice, ObdBtExt};

/// Teto por comando. Folgado de propósito: o `ATZ` do handshake pode levar ~1 s,
/// e o barramento ISO 9141-2 do carro é lento. Só estoura quando o adaptador
/// realmente ficou mudo (soltou do conector) — e aí vira erro de barramento.
const TIMEOUT_MS: u32 = 5000;

/// Nomes comuns de adaptadores ELM327/OBD, para achar o certo entre os pareados
/// quando o usuário não fixa um por `ECLIPSE_OBD_DEVICE`.
const PADROES_NOME: [&str; 6] = ["OBD", "ELM", "VLINK", "VIECAR", "OBDII", "KONNWEI"];

/// Um comando falhou na ponte do plugin. Qualquer falha de socket vira
/// [`ObdError::Bus`]: sobe pelo poller e faz o supervisor reconectar. (O carro
/// não responder um PID específico **não** passa por aqui — vem como texto
/// `NO DATA` e o parser do `eclipse-obd` trata como `Unsupported`.)
fn erro(e: tauri_plugin_obd_bt::Error) -> ObdError {
    ObdError::Bus(e.to_string())
}

/// O transporte de verdade: cada `command` vira uma chamada ao plugin Android.
pub struct AndroidBtTransport {
    app: tauri::AppHandle,
}

#[async_trait]
impl Elm327Transport for AndroidBtTransport {
    async fn command(&mut self, cmd: &str) -> Result<String, ObdError> {
        let app = self.app.clone();
        let cmd = cmd.to_string();
        tokio::task::spawn_blocking(move || app.obd_bt().command(&cmd, TIMEOUT_MS).map_err(erro))
            .await
            .map_err(|e| ObdError::Bus(format!("task de leitura falhou: {e}")))?
    }
}

/// Escolhe qual adaptador pareado usar.
///
/// Com `ECLIPSE_OBD_DEVICE` (nome ou MAC), casa por ele; senão pega o primeiro
/// cujo nome pareça de um ELM327.
fn escolher<'a>(pareados: &'a [BtDevice], alvo: Option<&str>) -> Option<&'a BtDevice> {
    if let Some(alvo) = alvo {
        let alvo_up = alvo.to_uppercase();
        return pareados.iter().find(|d| {
            d.address.eq_ignore_ascii_case(alvo) || d.name.to_uppercase().contains(&alvo_up)
        });
    }
    pareados.iter().find(|d| {
        let nome = d.name.to_uppercase();
        PADROES_NOME.iter().any(|p| nome.contains(p))
    })
}

/// Garante permissão, escolhe e abre o adaptador; devolve um rótulo para o log.
///
/// Tudo bloqueante num `spawn_blocking` só: pedir permissão espera o usuário
/// responder o diálogo, e listar/conectar falam com o rádio.
async fn preparar(app: &tauri::AppHandle) -> Result<String, ObdError> {
    let app = app.clone();
    let alvo = std::env::var("ECLIPSE_OBD_DEVICE").ok();

    tokio::task::spawn_blocking(move || -> Result<String, ObdError> {
        let bt = app.obd_bt();

        bt.ensure_permissions().map_err(erro)?;

        let pareados = bt.list_bonded().map_err(erro)?;
        for d in &pareados {
            tracing::info!(nome = %d.name, mac = %d.address, "adaptador Bluetooth pareado");
        }

        let escolhido = escolher(&pareados, alvo.as_deref()).ok_or_else(|| {
            ObdError::Bus(
                "nenhum adaptador OBD pareado; pareie o ELM327 nas configurações do Android \
                 (ou defina ECLIPSE_OBD_DEVICE com o nome/MAC)"
                    .to_string(),
            )
        })?;

        bt.connect(&escolhido.address).map_err(erro)?;
        Ok(format!("{} ({})", escolhido.name, escolhido.address))
    })
    .await
    .map_err(|e| ObdError::Bus(format!("task de conexão falhou: {e}")))?
}

/// Conecta ao adaptador e faz o handshake ELM327, devolvendo a fonte pronta.
pub async fn conectar(app: &tauri::AppHandle) -> Result<Elm327Source<AndroidBtTransport>, ObdError> {
    let rotulo = preparar(app).await?;
    tracing::info!(adaptador = %rotulo, "conectado; iniciando handshake do ELM327");
    Elm327Source::conectar(AndroidBtTransport { app: app.clone() }).await
}
