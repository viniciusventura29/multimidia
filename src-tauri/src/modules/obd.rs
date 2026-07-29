//! OBD-II.
//!
//! Virou fiação: a cadência e o protocolo moram no `eclipse-obd`; o socket
//! Bluetooth mora no `crate::obd_bt`. Aqui só se conecta ao adaptador e se roda
//! a varredura, publicando as leituras no barramento.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult};

pub const OBD: ModuleId = ModuleId::new("obd");

/// Precisa do `AppHandle` para alcançar o plugin de Bluetooth. O supervisor
/// reconstrói o módulo a cada reconexão, então guardamos só o handle (que clona).
pub struct ObdModule {
    app: tauri::AppHandle,
}

impl ObdModule {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl Module for ObdModule {
    async fn run(&mut self, ctx: ModuleCtx) -> ModuleResult {
        // Bluetooth clássico (SPP) só existe no Android. No desktop o módulo fica
        // quieto — mostradores escuros — em vez de reiniciar em loop tentando um
        // rádio que não existe.
        #[cfg(not(mobile))]
        {
            let _ = &self.app;
            ctx.degraded("telemetria OBD só no Android (Bluetooth)");
            std::future::pending::<()>().await;
            #[allow(unreachable_code)]
            Ok(())
        }

        #[cfg(mobile)]
        {
            // Conecta e faz o handshake. Se falhar (adaptador não pareado, carro
            // desligado, permissão negada), o erro sobe: o supervisor degrada os
            // cinco mostradores juntos e reconecta com backoff — o que se quer
            // quando o adaptador solta do conector no meio da estrada.
            let source = crate::obd_bt::conectar(&self.app).await?;
            let mut poller = eclipse_obd::Poller::new(source);

            loop {
                let leituras = poller.step().await?;
                ctx.ready(leituras);
            }
        }
    }
}
