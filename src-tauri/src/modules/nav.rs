//! Navegação.
//!
//! Ainda não há chave do Google Maps configurada, então este módulo se declara
//! degradado — e isso é honesto, não um placeholder. É também o degradado que
//! aparece na tela hoje, mostrando que o painel convive com um módulo fora do ar.
//! A Fase 6 liga o mapa de verdade.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult};

pub const NAV: ModuleId = ModuleId::new("nav");

#[derive(Default)]
pub struct PlaceholderNav;

#[async_trait]
impl Module for PlaceholderNav {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        ctx.degraded("mapa ainda não configurado");

        // Fica de pé esperando ordens em vez de retornar: assim o módulo continua
        // existindo para receber troca de perfil, sem o supervisor reiniciá-lo à toa.
        while ctx.next_command().await.is_some() {}

        Ok(())
    }
}
