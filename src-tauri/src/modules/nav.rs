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

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_gps::{sol, FiltroDeParada, Fix, Guia, LocationSource, Progresso, Route};
use serde::Serialize;

pub const NAV: ModuleId = ModuleId::new("nav");

/// Onde assumir que o carro está enquanto o GPS não fixa — a mesma São Paulo
/// do `CENTRO_PADRAO` do frontend, para o tema inicial casar com o mapa
/// inicial. Erra o tema por minutos em outra cidade; o primeiro fix corrige.
const CENTRO_PADRAO: (f64, f64) = (-23.5505, -46.6333);

/// De quanto em quanto tempo reavaliar o tema sem depender de fix novo —
/// parado na garagem o sol se põe do mesmo jeito.
const RELOGIO_DO_SOL: Duration = Duration::from_secs(60);

fn agora_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Mapa {
    /// A chave da Maps JavaScript API.
    ///
    /// Vai para o WebView de propósito: numa API de mapa web a chave é pública
    /// por natureza, ela viaja em toda requisição de tile. Quem protege não é o
    /// sigilo, é o teto de cota configurado no Google Cloud.
    api_key: String,

    /// O Map ID. Sem ele o mapa é raster, e mapa raster **ignora** `heading` —
    /// fica sempre olhando para o norte. Girar para o sentido da marcha exige
    /// um Map ID configurado como vetorial.
    map_id: Option<String>,

    /// Onde o carro está. `None` enquanto o GPS não fixa — e isso é comum:
    /// garagem, túnel, prédio alto. O mapa continua na tela, só não segue.
    fix: Option<Fix>,

    /// A rota traçada, se houver destino.
    rota: Option<Route>,

    /// Onde estamos dentro dela. Derivado a cada posição, nunca guardado à
    /// parte — assim não existe campo que possa discordar de onde o carro está.
    progresso: Option<Progresso>,

    /// A frase a ser falada agora, se houver.
    ///
    /// Vai junto do estado em vez de num evento próprio para carregar o número
    /// de sequência do envelope: a tela ignora o que já falou, e uma fala
    /// atrasada nunca atropela uma mais recente.
    fala: Option<String>,

    /// O sol já se pôs onde o carro está? Decide o tema do mapa (ver `sol.rs`
    /// no `eclipse-gps`). Calculado aqui e não na tela porque é aqui que dá
    /// para testar — e o painel inteiro lê estado, não relógio.
    noite: bool,
}

pub struct NavModule {
    api_key: Option<String>,
    map_id: Option<String>,
    gps: Box<dyn LocationSource>,
    guia: Option<Guia>,
    /// Congela a posição com o carro parado — sem ele, o jitter do GPS
    /// (dezenas de metros em Wi-Fi) faz o carro sambar no mapa e o progresso
    /// da rota tremer no semáforo. Ver `parada.rs` no `eclipse-gps`.
    filtro: FiltroDeParada,
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
            guia: None,
            filtro: FiltroDeParada::novo(),
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
            rota: None,
            progresso: None,
            fala: None,
            noite: sol::e_noite(CENTRO_PADRAO.0, CENTRO_PADRAO.1, agora_unix()),
        };
        ctx.ready(&estado);

        let mut relogio = tokio::time::interval(RELOGIO_DO_SOL);

        loop {
            tokio::select! {
                posicao = self.gps.next_fix() => match posicao {
                    Ok(fix) => {
                        let fix = self.filtro.filtrar(fix);
                        estado.noite = sol::e_noite(fix.lat, fix.lon, agora_unix());
                        match self.guia.as_mut() {
                            Some(guia) => {
                                let (progresso, fala) = guia.avaliar(&fix);
                                estado.progresso = Some(progresso);
                                estado.fala = fala;
                            }
                            None => {
                                estado.progresso = None;
                                estado.fala = None;
                            }
                        }
                        estado.fix = Some(fix);
                        ctx.ready(&estado);
                    }
                    // Perder sinal não apaga o mapa: ele fica no último ponto
                    // conhecido, esmaecido, como todo bom navegador faz no túnel.
                    Err(err) => ctx.degraded(err.to_string()),
                },

                // Parado na garagem não chega fix novo, mas o sol se põe do
                // mesmo jeito — o relógio reavalia o tema. Só publica na
                // virada: acordar a UI a cada minuto seria IPC à toa.
                _ = relogio.tick() => {
                    let (lat, lon) = estado
                        .fix
                        .map(|f| (f.lat, f.lon))
                        .unwrap_or(CENTRO_PADRAO);
                    let noite = sol::e_noite(lat, lon, agora_unix());
                    if noite != estado.noite {
                        estado.noite = noite;
                        ctx.ready(&estado);
                    }
                },

                comando = ctx.next_command() => match comando {
                    None => return Ok(()),

                    Some(ModuleCommand::Action { payload, .. }) => {
                        match payload.get("acao").and_then(|v| v.as_str()) {
                            // A rota vem pronta do lado JavaScript, que é onde
                            // mora o DirectionsService. Daqui pra frente ela é
                            // do Rust, junto com a posição — que é o que permite
                            // raciocinar sobre as duas juntas.
                            Some("rota") => {
                                match serde_json::from_value::<Route>(
                                    payload.get("rota").cloned().unwrap_or_default(),
                                ) {
                                    Ok(rota) => {
                                        estado.rota = Some(rota.clone());
                                        let mut guia = Guia::nova(rota);

                                        if let Some(fix) = &estado.fix {
                                            let (progresso, fala) = guia.avaliar(fix);
                                            estado.progresso = Some(progresso);
                                            estado.fala = fala;
                                        }

                                        self.guia = Some(guia);
                                        ctx.ready(&estado);
                                    }
                                    Err(err) => {
                                        tracing::warn!(%err, "rota malformada");
                                    }
                                }
                            }
                            Some("cancelar") => {
                                self.guia = None;
                                estado.rota = None;
                                estado.progresso = None;
                                estado.fala = None;
                                ctx.ready(&estado);
                            }
                            _ => {}
                        }
                    }

                    Some(_) => {}
                }
            }
        }
    }
}
