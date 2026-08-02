//! O carro como ferramentas MCP.
//!
//! Traduz o que os módulos publicam no barramento para JSON que um modelo lê
//! bem. Não é repassar o estado cru: o estado cru é feito para a tela, tem nome
//! de campo em camelCase, milissegundos, metros e — no caso do `nav` — a chave
//! do Google Maps. O que vai para o modelo é outra coisa.
//!
//! Três regras que valem para toda ferramenta daqui:
//!
//! 1. **Nada de segredo.** O estado do `nav` carrega `apiKey` porque o WebView
//!    precisa dela. O modelo não precisa, e o que ele lê pode acabar num log ou
//!    numa resposta. Fica de fora, e há teste para isso.
//! 2. **Nada de volume.** A rota traz centenas de coordenadas e dezenas de
//!    passos. Isso é token gasto sem nada em troca — vai o resumo.
//! 3. **Degradado é resposta, não erro.** OBD desconectado devolve
//!    `disponivel: false` com o motivo. Se virasse `Err`, o modelo leria "a
//!    ferramenta quebrou" e tentaria de novo, em vez de ler "o adaptador está
//!    fora" e comentar outra coisa.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Local, Timelike};
use eclipse_core::{ModuleId, StateEnvelope, Status};
use eclipse_mcp::{sem_argumentos, Ferramenta, McpError, Provedor};
use serde_json::{json, Value};

/// De onde sai o estado dos módulos.
///
/// É um traço e não o `Supervisor` direto para os testes poderem montar um
/// painel inteiro à mão — inclusive os casos degradados, que são os difíceis de
/// reproduzir com o app rodando.
pub trait FonteDeEstado: Send + Sync {
    fn estados(&self) -> Vec<StateEnvelope>;
}

/// Que horas são. Traço pelo mesmo motivo: "sábado de manhã" precisa ser
/// testável numa terça à tarde.
pub trait Relogio: Send + Sync {
    fn agora(&self) -> DateTime<Local>;
}

/// O relógio de verdade.
pub struct RelogioDoSistema;

impl Relogio for RelogioDoSistema {
    fn agora(&self) -> DateTime<Local> {
        Local::now()
    }
}

pub struct ProvedorCarro {
    fonte: Arc<dyn FonteDeEstado>,
    relogio: Arc<dyn Relogio>,
    /// Nome do perfil que está dirigindo, se houver.
    perfil: Option<String>,
}

impl ProvedorCarro {
    pub fn novo(
        fonte: Arc<dyn FonteDeEstado>,
        relogio: Arc<dyn Relogio>,
        perfil: Option<String>,
    ) -> Self {
        Self {
            fonte,
            relogio,
            perfil,
        }
    }

    fn modulo(&self, id: &str) -> Option<StateEnvelope> {
        self.fonte
            .estados()
            .into_iter()
            .find(|e| e.module == ModuleId(id.to_string().into()))
    }
}

/// O par (dados, motivo-de-indisponibilidade) de um módulo.
///
/// `Degraded` mantém o último valor bom, e ele continua sendo informação útil —
/// "o adaptador soltou, mas 40 segundos atrás o motor estava a 90°" é melhor
/// resposta que "não sei". Por isso os dois voltam juntos.
fn dados_e_motivo(
    envelope: Option<StateEnvelope>,
) -> (Option<Arc<Value>>, Option<String>) {
    match envelope {
        None => (None, Some("o módulo ainda não subiu".to_string())),
        Some(e) => {
            let motivo = match e.status {
                Status::Ready => None,
                Status::Loading => Some("o módulo ainda está subindo".to_string()),
                Status::Degraded => Some(
                    e.reason
                        .clone()
                        .unwrap_or_else(|| "o módulo está degradado".to_string()),
                ),
            };
            (e.data, motivo)
        }
    }
}

/// Corta um texto sem partir caractere multibyte no meio.
fn resumir(texto: &str, limite: usize) -> String {
    if texto.chars().count() <= limite {
        return texto.to_string();
    }
    let cortado: String = texto.chars().take(limite).collect();
    format!("{cortado}…")
}

fn periodo_do_dia(hora: u32) -> &'static str {
    match hora {
        0..=4 => "madrugada",
        5..=11 => "manhã",
        12..=17 => "tarde",
        _ => "noite",
    }
}

fn dia_da_semana(data: &DateTime<Local>) -> &'static str {
    match data.weekday() {
        chrono::Weekday::Mon => "segunda-feira",
        chrono::Weekday::Tue => "terça-feira",
        chrono::Weekday::Wed => "quarta-feira",
        chrono::Weekday::Thu => "quinta-feira",
        chrono::Weekday::Fri => "sexta-feira",
        chrono::Weekday::Sat => "sábado",
        chrono::Weekday::Sun => "domingo",
    }
}

impl ProvedorCarro {
    fn telemetria(&self) -> Value {
        let (dados, motivo) = dados_e_motivo(self.modulo("obd"));
        let Some(d) = dados else {
            return json!({
                "disponivel": false,
                "motivo": motivo.unwrap_or_else(|| "sem leitura".into()),
            });
        };

        let rpm = d["rpm"].as_u64();
        let velocidade = d["speedKmh"].as_u64();
        let consumo = &d["consumo"];
        let tanque = &d["tanque"];
        let viagem = &d["viagem"];

        json!({
            "disponivel": motivo.is_none(),
            "motivo": motivo,
            "rpm": rpm,
            "velocidade_kmh": velocidade,
            "temperatura_motor_c": d["coolantC"].as_i64(),
            "tensao_bateria_v": d["voltage"].as_f64(),
            "motor_ligado": rpm.map(|r| r > 0),
            "andando": velocidade.map(|v| v > 0),

            // Nada aqui vem pronto do barramento: o consumo sai do fluxo de ar
            // por uma cascata de fontes, e os litros são integrados. O `medido`
            // diz se houve sensor de verdade ou modelo — e é ele que decide
            // como o número pode ser dito.
            "consumo": {
                "km_por_litro": consumo["instantaneoKmL"].as_f64(),
                "litros_por_hora": consumo["litrosHora"].as_f64(),
                "medido": consumo["medido"].as_bool(),
                "origem": consumo["metodo"].as_str(),
            },
            "tanque": {
                "litros": tanque["litros"].as_f64(),
                "pct": tanque["pct"].as_f64(),
                "capacidade_l": tanque["capacidadeL"].as_f64(),
                "autonomia_km": tanque["autonomiaKm"].as_f64(),
                "media_do_tanque_km_l": tanque["mediaTanqueKmL"].as_f64(),
                "medido": tanque["medido"].as_bool(),
            },
            "viagem": {
                "distancia_km": viagem["distanciaKm"].as_f64(),
                "media_km_l": viagem["mediaKmL"].as_f64(),
                "litros": viagem["litros"].as_f64(),
            },

            "leia_assim": "campo nulo quer dizer que aquele PID ainda não voltou do \
                barramento — o Eclipse 2000 é ISO 9141-2 e entrega 1 a 3 leituras por \
                segundo, então os lentos demoram alguns segundos. Nulo não é zero. \
                Com `disponivel: false` os números são a última leitura boa, não a de agora.\n\n\
                CONSUMO E TANQUE COM `medido: false` SÃO ESTIMATIVA, não medição — a vazão \
                foi modelada a partir de carga ou pressão do coletor, e os litros foram \
                integrados no tempo. Nesse caso diga `cerca de`, `por volta de` ou `~`, e \
                nunca dê o número como fato. Autonomia é sempre conta, então nunca mande \
                ninguém contar com ela para chegar num lugar exato.",
        })
    }

    fn localizacao(&self) -> Value {
        let (dados, motivo) = dados_e_motivo(self.modulo("nav"));
        let Some(d) = dados else {
            return json!({
                "disponivel": false,
                "motivo": motivo.unwrap_or_else(|| "sem posição".into()),
            });
        };

        // A rota inteira tem `pontos` (centenas de coordenadas) e `passos`
        // (dezenas de instruções). Nada disso ajuda o modelo a dizer algo útil e
        // tudo isso custa token — vai só o resumo.
        let rota = d.get("rota").filter(|v| !v.is_null()).map(|r| {
            json!({
                "destino": r["destino"].as_str(),
                "distancia_total_km": r["distanciaTotalM"].as_f64().map(|m| (m / 100.0).round() / 10.0),
                "duracao_total_min": r["duracaoTotalS"].as_u64().map(|s| s / 60),
            })
        });

        let progresso = d.get("progresso").filter(|v| !v.is_null()).map(|p| {
            json!({
                "restante_km": p["distanciaRestanteM"].as_f64().map(|m| (m / 100.0).round() / 10.0),
                "chegada_em_min": p["chegadaEmS"].as_u64().map(|s| s / 60),
                "proxima_instrucao": p["proximaInstrucao"].as_str(),
                "fora_da_rota": p["foraDaRota"].as_bool(),
                "chegou": p["chegou"].as_bool(),
            })
        });

        let fix = d.get("fix").filter(|v| !v.is_null());

        // De propósito sem `apiKey` nem `mapId`: o estado do `nav` carrega a
        // chave do Maps porque o WebView precisa dela, e o modelo não.
        json!({
            "disponivel": motivo.is_none() && fix.is_some(),
            "motivo": motivo,
            "lat": fix.map(|f| f["lat"].clone()),
            "lon": fix.map(|f| f["lon"].clone()),
            "rumo_graus": fix.map(|f| f["heading"].clone()),
            "velocidade_kmh": fix.map(|f| f["speedKmh"].clone()),
            "rota": rota,
            "progresso": progresso,
        })
    }

    fn musica(&self) -> Value {
        let (dados, motivo) = dados_e_motivo(self.modulo("music"));
        let Some(d) = dados else {
            return json!({ "tocando": false, "motivo": motivo });
        };

        let agora = d.get("nowPlaying").filter(|v| !v.is_null());
        let Some(n) = agora else {
            return json!({ "tocando": false, "motivo": motivo });
        };

        json!({
            "tocando": n["isPlaying"].as_bool().unwrap_or(false),
            "faixa": n["track"].as_str(),
            "artista": n["artist"].as_str(),
            // Serve para ilustrar o quadro sem gastar geração de imagem.
            "capa_url": n["albumArt"].as_str(),
            "motivo": motivo,
        })
    }

    fn mensagens(&self) -> Value {
        const CONVERSAS: usize = 5;

        let (dados, motivo) = dados_e_motivo(self.modulo("messaging"));
        let Some(d) = dados else {
            return json!({ "nao_lidas": 0, "conversas": [], "motivo": motivo });
        };

        let vazio = Vec::new();
        let conversas = d["conversations"].as_array().unwrap_or(&vazio);

        let resumo: Vec<Value> = conversas
            .iter()
            .take(CONVERSAS)
            .map(|c| {
                let ultima = c["messages"]
                    .as_array()
                    .and_then(|m| m.last())
                    .and_then(|m| m["body"].as_str())
                    .map(|b| resumir(b, 80));
                json!({
                    "nome": c["name"].as_str(),
                    "nao_lidas": c["unread"].as_u64().unwrap_or(0),
                    "ultima": ultima,
                })
            })
            .collect();

        let total: u64 = conversas
            .iter()
            .filter_map(|c| c["unread"].as_u64())
            .sum();

        json!({
            "nao_lidas": total,
            "conversas": resumo,
            "motivo": motivo,
            "leia_assim": "só o que virou notificação enquanto o painel estava ligado. \
                Caixa vazia não quer dizer que ninguém escreveu.",
        })
    }

    fn contexto(&self) -> Value {
        let agora = self.relogio.agora();
        json!({
            "data": agora.format("%Y-%m-%d").to_string(),
            "hora": agora.format("%H:%M").to_string(),
            "dia_da_semana": dia_da_semana(&agora),
            "periodo": periodo_do_dia(agora.hour()),
            "fim_de_semana": matches!(
                agora.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            ),
            "fuso": agora.format("%:z").to_string(),
            "perfil": self.perfil,
        })
    }
}

#[async_trait]
impl Provedor for ProvedorCarro {
    fn ferramentas(&self) -> Vec<Ferramenta> {
        vec![
            sem_argumentos(
                "contexto_agora",
                "Chame SEMPRE antes de escrever qualquer coisa. Devolve data, hora, dia da \
                 semana, período do dia e quem está dirigindo. Você não tem relógio próprio: \
                 sem isto não há como saber se é sábado de manhã ou terça à noite, e essa é \
                 justamente a diferença entre um comentário útil e um genérico.",
            ),
            sem_argumentos(
                "carro_telemetria",
                "Chame antes de comentar qualquer coisa sobre o carro — motor, temperatura, \
                 bateria, consumo, quanto tem de gasolina, quantos km ainda dá, e como está \
                 a média da viagem. Devolve a leitura mais recente do ELM327 pelo OBD-II. \
                 Repare no campo `medido` de consumo e tanque: quando é `false`, o número é \
                 estimado e não pode ser dito como fato.",
            ),
            sem_argumentos(
                "carro_localizacao",
                "Chame quando precisar saber ONDE o carro está ou PARA ONDE ele vai: posição, \
                 rumo, destino traçado, distância e tempo restantes. É daqui que sai o nome do \
                 destino para você pesquisar sobre ele na web.",
            ),
            sem_argumentos(
                "carro_musica",
                "Chame quando for comentar sobre o que está tocando. Devolve faixa, artista e \
                 a URL da capa do álbum — a capa serve para ilustrar o quadro sem precisar \
                 gerar imagem.",
            ),
            sem_argumentos(
                "carro_mensagens",
                "Chame quando for útil mencionar mensagens pendentes. Devolve quantas não \
                 lidas e uma prévia curta das conversas mais recentes.",
            ),
        ]
    }

    async fn chamar(&self, nome: &str, _args: &Value) -> Result<Value, McpError> {
        Ok(match nome {
            "contexto_agora" => self.contexto(),
            "carro_telemetria" => self.telemetria(),
            "carro_localizacao" => self.localizacao(),
            "carro_musica" => self.musica(),
            "carro_mensagens" => self.mensagens(),
            outro => return Err(McpError::Desconhecida(outro.to_string())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    struct Painel(Vec<StateEnvelope>);

    impl FonteDeEstado for Painel {
        fn estados(&self) -> Vec<StateEnvelope> {
            self.0.clone()
        }
    }

    struct RelogioParado(DateTime<Local>);

    impl Relogio for RelogioParado {
        fn agora(&self) -> DateTime<Local> {
            self.0
        }
    }

    fn envelope(modulo: &'static str, status: Status, data: Value) -> StateEnvelope {
        StateEnvelope {
            module: ModuleId::new(modulo),
            seq: 1,
            status,
            data: Some(Arc::new(data)),
            reason: match status {
                Status::Degraded => Some("adaptador desconectado".into()),
                _ => None,
            },
        }
    }

    fn provedor(estados: Vec<StateEnvelope>) -> ProvedorCarro {
        ProvedorCarro::novo(
            Arc::new(Painel(estados)),
            Arc::new(RelogioParado(
                Local.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).unwrap(),
            )),
            Some("Vinicius".into()),
        )
    }

    fn obd_normal() -> Value {
        json!({
            "rpm": 2500, "speedKmh": 62, "coolantC": 88,
            "fuelPct": 45, "voltage": 14.1,
            "consumo": {
                "instantaneoKmL": 11.4, "litrosHora": 5.4,
                "metodo": "maf", "medido": true
            },
            "tanque": {
                "capacidadeL": 61.0, "litros": 27.5, "pct": 45.1,
                "faltaParaEncherL": 33.5, "autonomiaKm": 313.0,
                "mediaTanqueKmL": 11.4, "medido": true
            },
            "viagem": { "distanciaKm": 42.0, "duracaoS": 2400, "litros": 3.7, "mediaKmL": 11.3 }
        })
    }

    #[tokio::test]
    async fn telemetria_traduz_e_deriva() {
        let p = provedor(vec![envelope("obd", Status::Ready, obd_normal())]);
        let r = p.chamar("carro_telemetria", &json!({})).await.unwrap();

        assert_eq!(r["disponivel"], true);
        assert_eq!(r["velocidade_kmh"], 62);
        assert_eq!(r["temperatura_motor_c"], 88);
        assert_eq!(r["motor_ligado"], true);
        assert_eq!(r["andando"], true);
    }

    /// O `main` trouxe consumo, tanque e autonomia. Sem expor isso, a
    /// ferramenta continuaria dizendo que sabe falar de combustível e não
    /// saberia responder "dá para chegar lá?".
    #[tokio::test]
    async fn telemetria_entrega_consumo_tanque_e_autonomia() {
        let p = provedor(vec![envelope("obd", Status::Ready, obd_normal())]);
        let r = p.chamar("carro_telemetria", &json!({})).await.unwrap();

        assert_eq!(r["consumo"]["km_por_litro"], 11.4);
        assert_eq!(r["tanque"]["autonomia_km"], 313.0);
        assert_eq!(r["tanque"]["litros"], 27.5);
        assert_eq!(r["viagem"]["media_km_l"], 11.3);
    }

    /// Estimativa não pode se passar por medição: o modelo precisa ver o
    /// `medido` para saber quando dizer "cerca de".
    #[tokio::test]
    async fn numero_estimado_chega_marcado_como_estimado() {
        let mut obd = obd_normal();
        obd["consumo"]["medido"] = json!(false);
        obd["consumo"]["metodo"] = json!("carga");
        obd["tanque"]["medido"] = json!(false);

        let p = provedor(vec![envelope("obd", Status::Ready, obd)]);
        let r = p.chamar("carro_telemetria", &json!({})).await.unwrap();

        assert_eq!(r["consumo"]["medido"], false);
        assert_eq!(r["consumo"]["origem"], "carga");
        assert_eq!(r["tanque"]["medido"], false);
        assert!(
            r["leia_assim"].as_str().unwrap().contains("ESTIMATIVA"),
            "o aviso sobre estimativa tem que viajar junto com o número"
        );
    }

    /// O caso que separa "o carro está frio" de "eu não sei ainda".
    #[tokio::test]
    async fn pid_que_ainda_nao_voltou_fica_nulo_e_nao_vira_zero() {
        let p = provedor(vec![envelope(
            "obd",
            Status::Ready,
            json!({ "rpm": 800, "speedKmh": 0, "coolantC": null, "fuelPct": null, "voltage": null }),
        )]);
        let r = p.chamar("carro_telemetria", &json!({})).await.unwrap();

        assert!(r["temperatura_motor_c"].is_null());
        assert_eq!(r["motor_ligado"], true, "800 rpm é motor ligado");
        assert_eq!(r["andando"], false);
    }

    /// OBD fora do ar é resposta, não erro — e ainda entrega o último valor bom.
    #[tokio::test]
    async fn obd_degradado_responde_com_motivo_e_ultimo_valor() {
        let p = provedor(vec![envelope("obd", Status::Degraded, obd_normal())]);
        let r = p.chamar("carro_telemetria", &json!({})).await.unwrap();

        assert_eq!(r["disponivel"], false);
        assert_eq!(r["motivo"], "adaptador desconectado");
        assert_eq!(r["rpm"], 2500, "o último valor bom continua valendo");
    }

    #[tokio::test]
    async fn modulo_que_nem_subiu_nao_estoura() {
        let p = provedor(vec![]);
        for nome in [
            "carro_telemetria",
            "carro_localizacao",
            "carro_musica",
            "carro_mensagens",
        ] {
            let r = p.chamar(nome, &json!({})).await.unwrap();
            assert!(r.is_object(), "{nome} devolveu {r}");
        }
    }

    /// A chave do Maps viaja no estado do `nav`. Ela não pode chegar ao modelo.
    #[tokio::test]
    async fn localizacao_nunca_vaza_a_chave_do_maps() {
        let p = provedor(vec![envelope(
            "nav",
            Status::Ready,
            json!({
                "apiKey": "AIzaSySEGREDOSEGREDO",
                "mapId": "abc123",
                "fix": { "lat": -23.56, "lon": -46.65, "heading": 90.0, "speedKmh": 62.0 },
                "rota": null, "progresso": null, "fala": null,
            }),
        )]);

        let r = p.chamar("carro_localizacao", &json!({})).await.unwrap();
        let texto = r.to_string();

        assert!(
            !texto.contains("AIzaSySEGREDOSEGREDO"),
            "a chave do Maps vazou para o modelo: {texto}"
        );
        assert!(!texto.contains("abc123"));
        assert_eq!(r["lat"], -23.56);
    }

    /// Centenas de coordenadas e dezenas de passos não podem ir junto.
    #[tokio::test]
    async fn rota_vira_resumo_em_vez_da_geometria_inteira() {
        let pontos: Vec<Value> = (0..400).map(|i| json!([-23.5 + i as f64 * 1e-4, -46.6])).collect();
        let p = provedor(vec![envelope(
            "nav",
            Status::Ready,
            json!({
                "apiKey": "k", "mapId": null,
                "fix": { "lat": -23.56, "lon": -46.65, "heading": 12.0, "speedKmh": 80.0 },
                "rota": {
                    "destino": "Campos do Jordão",
                    "pontos": pontos,
                    "passos": [{ "instrucao": "Siga na Rod. Ayrton Senna", "distanciaM": 12000.0 }],
                    "distanciaTotalM": 172_400.0,
                    "duracaoTotalS": 9_000,
                },
                "progresso": {
                    "distanciaRestanteM": 158_000.0, "chegadaEmS": 8_100,
                    "passoAtual": 0, "proximaInstrucao": "Siga na Rod. Ayrton Senna",
                    "proximoDetalhe": null, "proximaManobra": null,
                    "distanciaParaManobraM": 900.0, "desvioM": 3.0,
                    "foraDaRota": false, "recalcular": false, "chegou": false,
                },
                "fala": null,
            }),
        )]);

        let r = p.chamar("carro_localizacao", &json!({})).await.unwrap();

        assert_eq!(r["rota"]["destino"], "Campos do Jordão");
        assert_eq!(r["rota"]["distancia_total_km"], 172.4);
        assert_eq!(r["rota"]["duracao_total_min"], 150);
        assert_eq!(r["progresso"]["chegada_em_min"], 135);
        assert!(
            r["rota"].get("pontos").is_none() && r["rota"].get("passos").is_none(),
            "a geometria da rota não pode ir para o modelo: {}",
            r["rota"]
        );
    }

    #[tokio::test]
    async fn musica_entrega_a_capa_para_ilustrar() {
        let p = provedor(vec![envelope(
            "music",
            Status::Ready,
            json!({
                "nowPlaying": {
                    "track": "Rosa", "artist": "Pixinguinha", "isPlaying": true,
                    "albumArt": "https://i.scdn.co/image/abc", "progressMs": 1000, "durationMs": 200000
                },
                "busca": { "faixas": [], "albuns": [] },
                "playlists": [], "contexto": null, "problema": null,
            }),
        )]);

        let r = p.chamar("carro_musica", &json!({})).await.unwrap();
        assert_eq!(r["tocando"], true);
        assert_eq!(r["artista"], "Pixinguinha");
        assert_eq!(r["capa_url"], "https://i.scdn.co/image/abc");
    }

    #[tokio::test]
    async fn mensagens_resumem_e_limitam() {
        let conversas: Vec<Value> = (0..9)
            .map(|i| {
                json!({
                    "name": format!("Pessoa {i}"),
                    "unread": 2,
                    "canReply": true,
                    "messages": [{ "autor": "eles", "sender": "x", "body": "a".repeat(300), "at": "2026-08-01T12:00:00Z" }],
                })
            })
            .collect();

        let p = provedor(vec![envelope(
            "messaging",
            Status::Ready,
            json!({ "conversations": conversas }),
        )]);

        let r = p.chamar("carro_mensagens", &json!({})).await.unwrap();

        assert_eq!(r["nao_lidas"], 18, "o total conta todas, não só as mostradas");
        assert_eq!(r["conversas"].as_array().unwrap().len(), 5);
        let previa = r["conversas"][0]["ultima"].as_str().unwrap();
        assert!(previa.chars().count() <= 81, "prévia longa demais: {}", previa.len());
    }

    #[tokio::test]
    async fn contexto_diz_que_e_sabado_de_manha() {
        // 1º de agosto de 2026 cai num sábado.
        let p = provedor(vec![]);
        let r = p.chamar("contexto_agora", &json!({})).await.unwrap();

        assert_eq!(r["dia_da_semana"], "sábado");
        assert_eq!(r["periodo"], "manhã");
        assert_eq!(r["fim_de_semana"], true);
        assert_eq!(r["hora"], "09:30");
        assert_eq!(r["perfil"], "Vinicius");
    }

    #[test]
    fn resumir_nao_parte_caractere_no_meio() {
        assert_eq!(resumir("ação", 10), "ação");
        assert_eq!(resumir("çãoçãoção", 3), "ção…");
    }

    #[tokio::test]
    async fn ferramenta_fora_do_provedor_e_recusada() {
        let p = provedor(vec![]);
        assert!(p.chamar("voar", &json!({})).await.is_err());
    }
}
