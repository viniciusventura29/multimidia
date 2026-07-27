//! OBD-II.
//!
//! Virou fiação: a cadência e o trajeto moram no `eclipse-obd`. Trocar o
//! simulador pelo ELM327 é trocar a fonte passada ao `Poller` — nada aqui muda.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult};
use eclipse_obd::{Poller, SimulatedSource};

pub const OBD: ModuleId = ModuleId::new("obd");

#[derive(Default)]
pub struct ObdModule;

#[async_trait]
impl Module for ObdModule {
    async fn run(&mut self, ctx: ModuleCtx) -> ModuleResult {
        let mut poller = Poller::new(SimulatedSource::default());

        loop {
            // Uma falha de barramento sobe daqui: o supervisor degrada o módulo
            // — os cinco mostradores escurecem juntos, guardando o último valor —
            // e reinicia com uma conexão nova. É o que se quer quando o adaptador
            // solta do conector no meio da estrada.
            let leituras = poller.step().await?;
            ctx.ready(leituras);
        }
    }
}
