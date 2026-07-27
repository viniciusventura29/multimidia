//! Navegação.
//!
//! O que dá para fazer aqui é menor do que parece, e vale registrar por quê:
//! **navegação turn-by-turn embutida não existe em nenhuma plataforma**. O Maps
//! SDK entrega o mapa, não a navegação; o Navigation SDK, que entrega, é produto
//! enterprise com preço sob negociação. Então este módulo cuida do mapa, e
//! navegar de verdade é abrir o app do Google Maps por cima.
//!
//! Como a UI roda num WebView, o mapa é um elemento comum da página — encolhe
//! para widget e cresce para tela cheia sem truque nenhum. Foi por isso que a
//! Maps JavaScript API venceu o SDK nativo, que seria uma View Java *fora* da
//! nossa árvore e exigiria recortar um buraco transparente no WebView.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult};
use serde::Serialize;

pub const NAV: ModuleId = ModuleId::new("nav");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Mapa {
    /// A chave da Maps JavaScript API.
    ///
    /// Vai para o WebView de propósito: numa API de mapa web a chave é pública
    /// por natureza, ela viaja em toda requisição de tile. Quem protege não é o
    /// sigilo, é o teto de cota configurado no Google Cloud.
    api_key: String,
}

pub struct NavModule {
    api_key: Option<String>,
}

impl NavModule {
    pub fn new(api_key: Option<String>) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl Module for NavModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        match self.api_key.clone() {
            Some(api_key) => ctx.ready(&Mapa { api_key }),
            None => ctx.degraded(
                "falta a chave do Google Maps — defina ECLIPSE_MAPS_API_KEY \
                 ou crie maps_api_key.txt no diretório de dados",
            ),
        }

        // Fica de pé esperando ordens em vez de retornar: assim o módulo segue
        // existindo para receber troca de perfil, sem o supervisor reiniciá-lo à toa.
        while ctx.next_command().await.is_some() {}

        Ok(())
    }
}
