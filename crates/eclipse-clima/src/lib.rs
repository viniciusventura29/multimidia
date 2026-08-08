//! Que tempo faz aqui fora.
//!
//! O painel de carro ganha pouco com previsão de sete dias e ganha muito com
//! uma linha: quanto está fazendo agora e se vai chover no para-brisa. É essa
//! linha que este crate busca.
//!
//! **Open-Meteo, e não a API do Google.** Não pede chave, não cobra, e não
//! obriga a guardar mais um segredo no aparelho — o Eclipse já carrega a chave
//! do Maps e as credenciais do Spotify, e cada segredo a mais é mais uma coisa
//! para vazar num APK que roda numa central de AliExpress. Em troca aceita-se
//! um provedor a menos de SLA, o que para "está chovendo?" é barganha boa.
//!
//! Como todo crate daqui, isto é só o domínio: quem decide **quando** perguntar
//! é o módulo `nav` do app, que é quem tem a posição do carro.

pub mod codigo;

pub use codigo::Familia;

use serde::{Deserialize, Serialize};

const ENDERECO: &str = "https://api.open-meteo.com/v1/forecast";

/// O tempo agora, do jeito que o painel consome.
///
/// `camelCase` porque isto atravessa o barramento até o WebView inteirinho, e
/// do outro lado é TypeScript.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clima {
    /// Temperatura do ar em graus Celsius.
    pub temp_c: f64,
    /// Qual ícone desenhar.
    pub familia: Familia,
    /// Como dizer isso em português — "chuva fraca", "céu limpo".
    pub rotulo: String,
    /// O código WMO cru, preservado para depuração e para a IA raciocinar sobre
    /// intensidade sem que a `Familia` precise crescer.
    pub codigo: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum ClimaError {
    #[error("não deu para falar com o serviço de clima")]
    Rede,
    #[error("o serviço de clima respondeu sem tempo atual")]
    SemLeitura,
}

/* ------------------------------------------------------------------ */
/* O que chega                                                         */
/* ------------------------------------------------------------------ */

#[derive(Deserialize)]
struct Resposta {
    current: Option<Atual>,
}

#[derive(Deserialize)]
struct Atual {
    /// O nome do campo é o do parâmetro pedido — `temperature_2m` é a leitura
    /// a dois metros do chão, que é a convenção meteorológica de "temperatura".
    temperature_2m: Option<f64>,
    weather_code: Option<u8>,
}

impl Resposta {
    fn em_clima(self) -> Result<Clima, ClimaError> {
        let atual = self.current.ok_or(ClimaError::SemLeitura)?;
        let temp_c = atual.temperature_2m.ok_or(ClimaError::SemLeitura)?;
        // Sem código não há como dizer o que está caindo do céu, mas a
        // temperatura ainda vale; `0` é o "céu limpo" que o catálogo já usa
        // como neutro, e a família Limpo é a menos alarmante das seis.
        let codigo = atual.weather_code.unwrap_or(0);
        let (familia, rotulo) = codigo::descrever(codigo);

        Ok(Clima {
            temp_c,
            familia,
            rotulo: rotulo.to_string(),
            codigo,
        })
    }
}

/* ------------------------------------------------------------------ */
/* A chamada                                                           */
/* ------------------------------------------------------------------ */

/// O tempo em (`lat`, `lon`) neste instante.
pub async fn buscar(cliente: &reqwest::Client, lat: f64, lon: f64) -> Result<Clima, ClimaError> {
    let resposta = cliente
        .get(ENDERECO)
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("current", "temperature_2m,weather_code".to_string()),
        ])
        .send()
        .await
        .map_err(|err| {
            tracing::warn!(%err, "a requisição de clima falhou");
            ClimaError::Rede
        })?;

    if !resposta.status().is_success() {
        let status = resposta.status();
        tracing::warn!(%status, "o serviço de clima recusou a consulta");
        return Err(ClimaError::Rede);
    }

    let dados: Resposta = resposta.json().await.map_err(|err| {
        tracing::warn!(%err, "resposta de clima ilegível");
        ClimaError::Rede
    })?;

    dados.em_clima()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma resposta como o Open-Meteo devolve, recortada nos campos pedidos.
    const RESPOSTA: &str = r#"{
      "latitude": -23.5,
      "longitude": -46.625,
      "current_units": {
        "time": "iso8601",
        "temperature_2m": "°C",
        "weather_code": "wmo code"
      },
      "current": {
        "time": "2026-08-08T15:00",
        "interval": 900,
        "temperature_2m": 21.4,
        "weather_code": 61
      }
    }"#;

    fn ler(json: &str) -> Result<Clima, ClimaError> {
        serde_json::from_str::<Resposta>(json).unwrap().em_clima()
    }

    #[test]
    fn le_a_resposta_do_open_meteo() {
        let clima = ler(RESPOSTA).unwrap();

        assert_eq!(clima.temp_c, 21.4);
        assert_eq!(clima.codigo, 61);
        assert_eq!(clima.familia, Familia::Chuva);
        assert_eq!(clima.rotulo, "chuva fraca");
    }

    /// Temperatura negativa não é erro nem ausência — o campo é `Option` e
    /// `-3.0` precisa atravessar inteiro, sem virar `None` por engano.
    #[test]
    fn temperatura_negativa_atravessa() {
        let json = r#"{"current":{"temperature_2m":-3.5,"weather_code":71}}"#;
        let clima = ler(json).unwrap();

        assert_eq!(clima.temp_c, -3.5);
        assert_eq!(clima.familia, Familia::Neve);
    }

    /// Zero grau é uma leitura legítima. É o mesmo cuidado do resto do painel:
    /// "não li" e "li e deu zero" não podem colapsar num valor só.
    #[test]
    fn zero_grau_e_leitura_e_nao_ausencia() {
        let json = r#"{"current":{"temperature_2m":0.0,"weather_code":3}}"#;
        assert_eq!(ler(json).unwrap().temp_c, 0.0);
    }

    #[test]
    fn sem_temperatura_nao_ha_clima() {
        let json = r#"{"current":{"weather_code":3}}"#;
        assert!(matches!(ler(json), Err(ClimaError::SemLeitura)));
    }

    #[test]
    fn sem_bloco_atual_nao_ha_clima() {
        assert!(matches!(ler("{}"), Err(ClimaError::SemLeitura)));
    }

    /// Falta só o código: a temperatura ainda serve, e o painel mostra o número
    /// com um ícone neutro em vez de apagar a informação inteira.
    #[test]
    fn sem_codigo_a_temperatura_ainda_vale() {
        let json = r#"{"current":{"temperature_2m":19.0}}"#;
        let clima = ler(json).unwrap();

        assert_eq!(clima.temp_c, 19.0);
        assert_eq!(clima.familia, Familia::Limpo);
    }
}
