//! Navegação.
//!
//! O que dá para fazer aqui é menor do que parece, e vale registrar por quê:
//! **navegação turn-by-turn embutida não existe em nenhuma plataforma**. O Maps
//! SDK entrega o mapa, não a navegação; o Navigation SDK, que entrega, é produto
//! enterprise sem preço público. Então este módulo cuida do mapa seguindo o
//! carro, e guiar de verdade continua sendo abrir o app do Google Maps por cima.
//!
//! Como a UI roda num WebView, o mapa é um elemento comum da página — encolhe
//! para widget e cresce para tela cheia sem truque nenhum. Foi por isso que a
//! Maps JavaScript API venceu o SDK nativo, que seria uma View Java *fora* da
//! nossa árvore e exigiria recortar um buraco transparente no WebView.

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult};
use eclipse_gps::{Fix, LocationSource};
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

    /// O Map ID. Sem ele o mapa é raster, e mapa raster **ignora** `tilt` e
    /// `heading` — fica sempre chapado, olhando para o norte. Inclinar e girar
    /// para o sentido da marcha exige um Map ID configurado como vetorial.
    map_id: Option<String>,

    /// Onde o carro está. `None` enquanto o GPS não fixa — e isso é comum:
    /// garagem, túnel, prédio alto. O mapa continua na tela, só não segue.
    fix: Option<Fix>,
}

pub struct NavModule {
    api_key: Option<String>,
    map_id: Option<String>,
    gps: Box<dyn LocationSource>,
}

impl NavModule {
    pub fn new(
        api_key: Option<String>,
        map_id: Option<String>,
        gps: Box<dyn LocationSource>,
    ) -> Self {
        Self {
            api_key,
            map_id,
            gps,
        }
    }
}

#[async_trait]
impl Module for NavModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        let Some(api_key) = self.api_key.clone() else {
            ctx.degraded(
                "falta a chave do Google Maps — defina ECLIPSE_MAPS_API_KEY \
                 ou crie maps_api_key.txt no diretório de dados",
            );
            while ctx.next_command().await.is_some() {}
            return Ok(());
        };

        let mut estado = Mapa {
            api_key,
            map_id: self.map_id.clone(),
            fix: None,
        };
        ctx.ready(&estado);

        loop {
            tokio::select! {
                posicao = self.gps.next_fix() => match posicao {
                    Ok(fix) => {
                        estado.fix = Some(fix);
                        ctx.ready(&estado);
                    }
                    // Perder sinal não apaga o mapa: ele fica no último ponto
                    // conhecido, esmaecido, como todo bom navegador faz no túnel.
                    Err(err) => ctx.degraded(err.to_string()),
                },

                comando = ctx.next_command() => {
                    if comando.is_none() {
                        return Ok(());
                    }
                }
            }
        }
    }
}
