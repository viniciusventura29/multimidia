//! Pedir uma rota ao Google.
//!
//! A busca morava no JavaScript porque o `DirectionsService` é parte do SDK do
//! mapa. Com o SDK do Google saindo do desenho (MapLibre no lugar), esse motivo
//! deixou de existir — e a rota passou a morar aqui, junto da posição e da
//! guiagem. É a mesma razão de sempre: quem responde "quanto falta" e "saí do
//! caminho" precisa da rota e do GPS ao mesmo tempo.
//!
//! Fazer daqui também resolve um problema concreto: a Routes API é web service
//! e não é feita para ser chamada do navegador. A Places API (New) é — e por
//! isso a busca de endereço continua no JS, onde ela já funciona.
//!
//! Isto **não** é o Navigation SDK: continua sem orientação de faixa e sem
//! trânsito ao vivo desviando a rota. O trânsito entra só no tempo estimado,
//! via `TRAFFIC_AWARE`, que é o que a Routes API sabe informar na hora.

use serde::{Deserialize, Serialize};

use crate::guia::{Passo, Route};

const ENDERECO: &str = "https://routes.googleapis.com/directions/v2:computeRoutes";

/// Exatamente os campos que a tela e a guiagem consomem, e nada além.
///
/// A máscara não é economia de bytes: é ela que decide o tier de cobrança da
/// requisição. Mesma disciplina do `pois.tsx` com a Places.
const CAMPOS: &str = "routes.distanceMeters,\
                      routes.duration,\
                      routes.polyline.geoJsonLinestring,\
                      routes.legs.steps.distanceMeters,\
                      routes.legs.steps.navigationInstruction";

/// Para onde o motorista quer ir.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Alvo {
    /// O lugar exato que ele tocou na lista de sugestões. Vale mais que o
    /// texto: o Google não precisa adivinhar de novo a partir da frase.
    #[serde(default)]
    pub place_id: Option<String>,
    /// O que ele digitou, quando não tocou em sugestão nenhuma.
    #[serde(default)]
    pub texto: Option<String>,
    /// Como chamar esse destino na tela.
    ///
    /// Vem de fora porque quem sabe disso é a sugestão que foi tocada
    /// ("Padaria Real"), e isso é melhor num painel de carro que o endereço
    /// formatado que a API devolveria ("R. Fulano, 123 - Bairro, São Paulo").
    pub rotulo: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DirectionsError {
    #[error("não deu para falar com o Google")]
    Rede,
    #[error("não achei esse endereço")]
    SemRota,
}

/* ------------------------------------------------------------------ */
/* O que se manda                                                      */
/* ------------------------------------------------------------------ */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pedido<'a> {
    origin: Ponto,
    destination: Destino<'a>,
    travel_mode: &'static str,
    /// Pede a duração já considerando o trânsito do momento. Sem isto o tempo
    /// estimado é o de uma via vazia, que ninguém encontra.
    routing_preference: &'static str,
    /// GeoJSON em vez da polyline codificada: chega pronto para uso e poupa um
    /// decodificador inteiro de existir e de ser mantido.
    polyline_encoding: &'static str,
    language_code: &'static str,
    region_code: &'static str,
    units: &'static str,
}

#[derive(Serialize)]
struct Ponto {
    location: Local,
}

#[derive(Serialize)]
struct Local {
    #[serde(rename = "latLng")]
    lat_lng: LatLng,
}

#[derive(Serialize)]
struct LatLng {
    latitude: f64,
    longitude: f64,
}

/// Um `Waypoint` da Routes API, na forma que interessa: ou o lugar exato, ou o
/// endereço em texto. Enum externamente tagueado sai como `{"placeId": "…"}`,
/// que é exatamente o formato do corpo.
#[derive(Serialize)]
enum Destino<'a> {
    #[serde(rename = "placeId")]
    PlaceId(&'a str),
    #[serde(rename = "address")]
    Endereco(&'a str),
}

/* ------------------------------------------------------------------ */
/* O que volta                                                         */
/* ------------------------------------------------------------------ */

#[derive(Debug, Deserialize)]
struct Resposta {
    #[serde(default)]
    routes: Vec<RotaBruta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RotaBruta {
    #[serde(default)]
    distance_meters: f64,
    /// Vem como texto com sufixo: `"1543s"`.
    #[serde(default)]
    duration: String,
    #[serde(default)]
    polyline: Option<Polyline>,
    #[serde(default)]
    legs: Vec<Perna>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Polyline {
    #[serde(default)]
    geo_json_linestring: Option<LineString>,
}

#[derive(Debug, Deserialize)]
struct LineString {
    /// GeoJSON é **[longitude, latitude]**, nesta ordem — o contrário do resto
    /// do sistema. Trocar isso de lugar em silêncio põe o carro na China.
    #[serde(default)]
    coordinates: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct Perna {
    #[serde(default)]
    steps: Vec<PassoBruto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PassoBruto {
    #[serde(default)]
    distance_meters: f64,
    #[serde(default)]
    navigation_instruction: Option<Instrucao>,
}

#[derive(Debug, Deserialize)]
struct Instrucao {
    #[serde(default)]
    maneuver: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
}

/* ------------------------------------------------------------------ */
/* Tradução                                                            */
/* ------------------------------------------------------------------ */

/// `"1543s"` -> `1543`. Formato de `Duration` do protobuf em JSON.
fn segundos(texto: &str) -> u32 {
    texto
        .trim_end_matches('s')
        .parse::<f64>()
        .map(|s| s.round() as u32)
        .unwrap_or(0)
}

/// `TURN_LEFT` -> `turn-left`.
///
/// A Routes API usa SCREAMING_SNAKE onde o SDK antigo usava kebab minúsculo, e
/// é o kebab que a tela já mapeia para as setas de manobra. Traduzir aqui
/// mantém o contrato do `Passo` intacto — e a tela não precisa saber que a
/// origem dos dados mudou.
fn manobra(codigo: &str) -> Option<String> {
    if codigo.is_empty() || codigo == "MANEUVER_UNSPECIFIED" {
        return None;
    }
    Some(codigo.to_ascii_lowercase().replace('_', "-"))
}

impl RotaBruta {
    fn em_rota(self, rotulo: &str) -> Route {
        let pontos = self
            .polyline
            .and_then(|p| p.geo_json_linestring)
            .map(|l| l.coordinates)
            .unwrap_or_default()
            .into_iter()
            // De [lon, lat] do GeoJSON para o (lat, lon) do resto do sistema.
            .map(|[lon, lat]| (lat, lon))
            .collect();

        let passos = self
            .legs
            .into_iter()
            .flat_map(|perna| perna.steps)
            .map(|passo| {
                let instrucao = passo.navigation_instruction.unwrap_or(Instrucao {
                    maneuver: None,
                    instructions: None,
                });
                Passo {
                    instrucao: instrucao.instructions.unwrap_or_default(),
                    // A Routes API entrega a instrução em texto puro, sem a
                    // referência visual de apoio que o Directions antigo
                    // enfiava num `<div>` do HTML. Não há o que separar.
                    detalhe: None,
                    distancia_m: passo.distance_meters,
                    manobra: instrucao.maneuver.as_deref().and_then(manobra),
                }
            })
            .collect();

        Route {
            destino: rotulo.to_string(),
            pontos,
            passos,
            distancia_total_m: self.distance_meters,
            duracao_total_s: segundos(&self.duration),
        }
    }
}

/* ------------------------------------------------------------------ */
/* A chamada                                                           */
/* ------------------------------------------------------------------ */

/// Traça uma rota de `origem` até `alvo`.
pub async fn buscar(
    cliente: &reqwest::Client,
    chave: &str,
    origem: (f64, f64),
    alvo: &Alvo,
) -> Result<Route, DirectionsError> {
    let destino = match (&alvo.place_id, &alvo.texto) {
        (Some(id), _) => Destino::PlaceId(id),
        (None, Some(texto)) => Destino::Endereco(texto),
        (None, None) => return Err(DirectionsError::SemRota),
    };

    let pedido = Pedido {
        origin: Ponto {
            location: Local {
                lat_lng: LatLng {
                    latitude: origem.0,
                    longitude: origem.1,
                },
            },
        },
        destination: destino,
        travel_mode: "DRIVE",
        routing_preference: "TRAFFIC_AWARE",
        polyline_encoding: "GEO_JSON_LINESTRING",
        language_code: "pt-BR",
        region_code: "BR",
        units: "METRIC",
    };

    let resposta = cliente
        .post(ENDERECO)
        .header("X-Goog-Api-Key", chave)
        .header("X-Goog-FieldMask", CAMPOS)
        .json(&pedido)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!(%err, "a requisição de rota falhou");
            DirectionsError::Rede
        })?;

    if !resposta.status().is_success() {
        // O corpo do erro do Google é técnico demais para um painel de carro,
        // mas serve no log — é ele que diz se foi cota, chave ou destino.
        let status = resposta.status();
        let corpo = resposta.text().await.unwrap_or_default();
        tracing::warn!(%status, %corpo, "o Google recusou a rota");
        return Err(DirectionsError::SemRota);
    }

    let dados: Resposta = resposta.json().await.map_err(|err| {
        tracing::warn!(%err, "resposta de rota ilegível");
        DirectionsError::Rede
    })?;

    dados
        .routes
        .into_iter()
        .next()
        .map(|rota| rota.em_rota(&alvo.rotulo))
        .ok_or(DirectionsError::SemRota)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma resposta como a Routes API devolve, recortada nos campos da máscara.
    const RESPOSTA: &str = r#"{
      "routes": [{
        "distanceMeters": 940,
        "duration": "240s",
        "polyline": {
          "geoJsonLinestring": {
            "type": "LineString",
            "coordinates": [
              [-46.6443, -23.5713],
              [-46.6480, -23.5680],
              [-46.6515, -23.5650]
            ]
          }
        },
        "legs": [{
          "steps": [
            {
              "distanceMeters": 480,
              "navigationInstruction": {
                "maneuver": "MANEUVER_UNSPECIFIED",
                "instructions": "Siga pela Av. Paulista"
              }
            },
            {
              "distanceMeters": 460,
              "navigationInstruction": {
                "maneuver": "TURN_RIGHT",
                "instructions": "Vire à direita na R. da Consolação"
              }
            }
          ]
        }]
      }]
    }"#;

    fn traduzida() -> Route {
        let resposta: Resposta = serde_json::from_str(RESPOSTA).unwrap();
        resposta
            .routes
            .into_iter()
            .next()
            .unwrap()
            .em_rota("Consolação")
    }

    /// A troca silenciosa de [lon, lat] por (lat, lon) põe o carro do outro
    /// lado do planeta, e nada no tipo denuncia — só um teste.
    #[test]
    fn o_geojson_chega_como_lat_lon_e_nao_ao_contrario() {
        let rota = traduzida();

        assert_eq!(rota.pontos.len(), 3);
        let (lat, lon) = rota.pontos[0];
        assert!((-24.0..-23.0).contains(&lat), "latitude errada: {lat}");
        assert!((-47.0..-46.0).contains(&lon), "longitude errada: {lon}");
    }

    #[test]
    fn os_passos_viram_manobras_que_a_tela_conhece() {
        let rota = traduzida();

        assert_eq!(rota.passos.len(), 2);
        assert_eq!(rota.passos[0].instrucao, "Siga pela Av. Paulista");
        assert_eq!(
            rota.passos[0].manobra, None,
            "\"siga em frente\" não é manobra"
        );
        assert_eq!(rota.passos[1].manobra.as_deref(), Some("turn-right"));
        assert_eq!(rota.passos[1].distancia_m, 460.0);
    }

    #[test]
    fn o_rotulo_vem_de_fora_e_nao_da_api() {
        assert_eq!(traduzida().destino, "Consolação");
    }

    #[test]
    fn a_duracao_perde_o_sufixo_de_segundos() {
        assert_eq!(traduzida().duracao_total_s, 240);
        assert_eq!(segundos("1543s"), 1543);
        assert_eq!(segundos("1543.5s"), 1544);
        assert_eq!(segundos(""), 0, "resposta sem duração não pode explodir");
    }

    /// Uma resposta vazia é o "não achei esse endereço" da Routes API — ela
    /// devolve 200 com `routes: []` em vez de erro.
    #[test]
    fn resposta_sem_rota_nenhuma_nao_quebra() {
        let vazia: Resposta = serde_json::from_str("{}").unwrap();
        assert!(vazia.routes.is_empty());
    }

    #[test]
    fn manobra_desconhecida_vira_kebab_minusculo() {
        assert_eq!(
            manobra("ROUNDABOUT_LEFT").as_deref(),
            Some("roundabout-left")
        );
        assert_eq!(manobra("MERGE").as_deref(), Some("merge"));
        assert_eq!(manobra("MANEUVER_UNSPECIFIED"), None);
        assert_eq!(manobra(""), None);
    }

    /// O destino escolhido na lista de sugestões vai por `placeId`; o digitado
    /// à mão, por endereço. Sem nenhum dos dois não há o que buscar.
    #[test]
    fn alvo_sem_lugar_nem_texto_e_recusado_antes_de_gastar_requisicao() {
        let alvo = Alvo {
            place_id: None,
            texto: None,
            rotulo: "nada".into(),
        };
        let cliente = reqwest::Client::new();
        let erro = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(buscar(&cliente, "chave", (-23.5, -46.6), &alvo));

        assert!(matches!(erro, Err(DirectionsError::SemRota)));
    }
}
