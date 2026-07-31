//! O módulo do assistente.
//!
//! Junta as três peças: o [`Detector`] diz *quando* falar, o [`Orcamento`] diz
//! *se pode*, e o [`Agente`] (do `eclipse-ia`) faz a fala. O que sai daqui é um
//! [`Painel`] no barramento, que o tile da coluna esquerda desenha.
//!
//! **Nenhuma chamada à API bloqueia o laço.** Um turno com pesquisa web pode
//! levar meio minuto; esperar por ele aqui pararia de consumir o barramento e o
//! detector perderia transições — inclusive um alerta de temperatura. Por isso o
//! turno roda numa task à parte e volta por canal.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use eclipse_core::{Module, ModuleCtx, ModuleId, ModuleResult, StateEnvelope, Supervisor};
use eclipse_ia::{
    Agente, Cartao, Config as ConfigAgente, IaError, McpRemoto, ProvedorQuadro, Tom, Turno,
};
use eclipse_mcp::Registro;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::assistente::carro::{FonteDeEstado, ProvedorCarro, RelogioDoSistema};
use crate::assistente::gatilho::{Acionamento, Detector, Gatilho};
use crate::assistente::imagem::ProvedorImagem;
use crate::assistente::orcamento::{Config, Orcamento, Recusa};

pub const ASSISTENTE: ModuleId = ModuleId::new("assistente");

/// De quanto em quanto tempo olhar se cabe um comentário de estrada.
const RITMO_PERIODICO: Duration = Duration::from_secs(5 * 60);

/// Quanto esperar antes da saudação de partida.
///
/// Os outros módulos precisam ter publicado alguma coisa: falar antes de o OBD
/// e o GPS abrirem a boca daria uma saudação sem carro e sem lugar.
const ESPERA_IGNICAO: Duration = Duration::from_secs(6);

/// O que o tile desenha.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Painel {
    cartoes: Vec<Cartao>,
    /// ISO 8601, para a tela saber quando o quadro envelheceu e trocar pela
    /// animação. `None` enquanto nada foi pintado.
    gerado_em: Option<String>,
    pensando: bool,
    /// Qual acontecimento gerou o quadro atual. Só informativo.
    gatilho: Option<String>,
}

/// Lê o estado dos módulos vizinhos pelo estado gerenciado do Tauri.
struct EstadoDoApp(AppHandle);

impl FonteDeEstado for EstadoDoApp {
    fn estados(&self) -> Vec<StateEnvelope> {
        self.0
            .try_state::<Supervisor>()
            .map(|s| s.snapshot())
            .unwrap_or_default()
    }
}

pub struct AssistenteModule {
    app: AppHandle,
}

impl AssistenteModule {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// Espera o supervisor virar estado gerenciado.
///
/// Ordem inevitável: `supervisor.spawn(...)` põe este módulo para rodar, e o
/// `app.manage(supervisor)` só acontece depois — o supervisor ainda está sendo
/// construído quando a task começa. São milissegundos na prática; o teto existe
/// para não esperar para sempre se algo mudar no `setup`.
async fn esperar_supervisor(app: &AppHandle) -> bool {
    for _ in 0..100 {
        if app.try_state::<Supervisor>().is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[async_trait]
impl Module for AssistenteModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        let dir = self
            .app
            .path()
            .app_data_dir()
            .map_err(|e| format!("sem diretório de dados: {e}"))?;

        let config = Config::carregar(&dir);
        let mut painel = Painel::default();
        // Nasce pronto e vazio: a coluna mostra a animação do carro em vez de
        // ficar "carregando" à toa até o primeiro gatilho.
        ctx.ready(&painel);

        if !config.ligado {
            ctx.degraded("o assistente está desligado em assistente.json");
            while ctx.next_command().await.is_some() {}
            return Ok(());
        }

        if !esperar_supervisor(&self.app).await {
            return Err("o supervisor não ficou disponível".into());
        }

        let demo = std::env::var("ECLIPSE_IA_DEMO").is_ok_and(|v| v == "1");
        if demo {
            gravar_imagem_de_demonstracao(&dir);
        }
        let chave = crate::anthropic_api_key(&dir);

        if chave.is_none() && !demo {
            ctx.degraded(
                "falta a chave da Anthropic — defina ECLIPSE_ANTHROPIC_API_KEY \
                 ou crie anthropic_api_key.txt no diretório de dados",
            );
            while ctx.next_command().await.is_some() {}
            return Ok(());
        }

        let perfil = self
            .app
            .try_state::<crate::Perfis>()
            .and_then(|p| p.lock().active().map(|a| a.name.clone()));

        let orcamento = Arc::new(Mutex::new(Orcamento::carregar(&dir, config.clone())));
        let quadro = Arc::new(ProvedorQuadro::novo());

        // A ordem de registro é a ordem que vai no prompt, e ela precisa ser
        // estável entre chamadas para o cache de prefixo valer.
        let registro = Arc::new(
            Registro::nova()
                .com(Arc::new(ProvedorCarro::novo(
                    Arc::new(EstadoDoApp(self.app.clone())),
                    Arc::new(RelogioDoSistema),
                    perfil,
                )))
                .com(Arc::new(ProvedorImagem::novo(
                    reqwest::Client::new(),
                    crate::maps_api_key(&dir),
                    crate::openrouter_api_key(&dir),
                    config.modelo_imagem.clone(),
                    &dir,
                    Arc::clone(&orcamento),
                )))
                .com(Arc::clone(&quadro) as Arc<dyn eclipse_mcp::Provedor>),
        );

        let mcp_remotos: Vec<McpRemoto> = config
            .mcp
            .iter()
            .map(|s| McpRemoto {
                nome: s.nome.clone(),
                url: s.url.clone(),
                token: s.token.clone(),
            })
            .collect();

        let transporte: Option<Arc<dyn eclipse_ia::Transporte>> = match &chave {
            Some(k) => Some(Arc::new(eclipse_ia::TransporteHttp::novo(k.clone())?)),
            None => None,
        };

        let mut detector = Detector::novo();
        let (tx, mut rx_turno) = tokio::sync::mpsc::channel::<(Gatilho, Result<Turno, IaError>)>(4);
        let mut eventos = self.app.state::<Supervisor>().subscribe();
        let mut ritmo = tokio::time::interval(RITMO_PERIODICO);
        ritmo.tick().await; // o primeiro tick de um interval sai na hora

        let mut ignicao = Some(Box::pin(tokio::time::sleep(ESPERA_IGNICAO)));
        let mut ocupado = false;

        loop {
            tokio::select! {
                // Saudação de partida, uma vez, depois que os vizinhos falarem.
                _ = async { ignicao.as_mut().unwrap().await }, if ignicao.is_some() => {
                    ignicao = None;
                    let acionamento = gatilho_forcado().unwrap_or_else(Acionamento::ignicao);
                    self.acionar(
                        acionamento, &mut painel, &ctx, &mut ocupado, demo,
                        &orcamento, &registro, &quadro, &transporte, &config,
                        &mcp_remotos, &tx,
                    );
                }

                recebido = eventos.recv() => match recebido {
                    Ok(envelope) => {
                        if envelope.module == ASSISTENTE {
                            continue; // o próprio estado não é gatilho de nada
                        }
                        if let Some(acionamento) = detector.observar(&envelope) {
                            self.acionar(
                                acionamento, &mut painel, &ctx, &mut ocupado, demo,
                                &orcamento, &registro, &quadro, &transporte, &config,
                                &mcp_remotos, &tx,
                            );
                        }
                    }
                    // Ficar para trás não é falha: o detector compara com o
                    // estado que ele guarda, então a próxima leitura corrige.
                    Err(RecvError::Lagged(perdidos)) => {
                        tracing::warn!(perdidos, "o assistente ficou pra trás no barramento");
                    }
                    Err(RecvError::Closed) => return Ok(()),
                },

                _ = ritmo.tick() => {
                    if detector.em_viagem() {
                        self.acionar(
                            Acionamento::periodico(), &mut painel, &ctx, &mut ocupado, demo,
                            &orcamento, &registro, &quadro, &transporte, &config,
                            &mcp_remotos, &tx,
                        );
                    }
                }

                Some((gatilho, resultado)) = rx_turno.recv() => {
                    ocupado = false;
                    painel.pensando = false;

                    match resultado {
                        Ok(turno) => {
                            tracing::info!(
                                gatilho = gatilho.como_texto(),
                                iteracoes = turno.iteracoes,
                                entrada = turno.uso.entrada,
                                saida = turno.uso.saida,
                                cache_leitura = turno.uso.cache_leitura,
                                cache_escrita = turno.uso.cache_escrita,
                                "turno do assistente",
                            );

                            // Turno sem quadro é resposta legítima: o modelo foi
                            // instruído a não pintar quando não há novidade. O
                            // quadro anterior fica e envelhece sozinho, até a
                            // tela trocá-lo pela animação.
                            if let Some(novo) = turno.quadro {
                                painel.cartoes = novo.cartoes;
                                painel.gerado_em = Some(Utc::now().to_rfc3339());
                                painel.gatilho = Some(gatilho.como_texto().to_string());
                            }
                            ctx.ready(&painel);
                        }
                        Err(err) => {
                            tracing::warn!(%err, gatilho = gatilho.como_texto(), "turno falhou");

                            // Os dois publicares são necessários, nesta ordem.
                            // `degraded` publica sem dados, e o barramento
                            // então **herda** o último valor bom — que ainda
                            // teria `pensando: true`, e a coluna ficaria dizendo
                            // "vendo o que tem de novo…" para sempre. Publicar
                            // o painel corrigido primeiro é o que faz a herança
                            // pegar o valor certo.
                            ctx.ready(&painel);
                            // Degradar mantém os cartões anteriores na tela,
                            // esmaecidos, com o motivo — em vez de apagar o que
                            // ainda era útil.
                            ctx.degraded(err.to_string());
                        }
                    }
                }

                // O assistente não recebe ação da tela — ele é proativo por
                // desenho. Este ramo existe só para notar o barramento fechando.
                comando = ctx.next_command() => {
                    if comando.is_none() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

impl AssistenteModule {
    /// Dispara um turno, se o orçamento deixar e não houver outro em curso.
    #[allow(clippy::too_many_arguments)]
    fn acionar(
        &self,
        acionamento: Acionamento,
        painel: &mut Painel,
        ctx: &ModuleCtx,
        ocupado: &mut bool,
        demo: bool,
        orcamento: &Arc<Mutex<Orcamento>>,
        registro: &Arc<Registro>,
        quadro: &Arc<ProvedorQuadro>,
        transporte: &Option<Arc<dyn eclipse_ia::Transporte>>,
        config: &Config,
        mcp_remotos: &[McpRemoto],
        tx: &tokio::sync::mpsc::Sender<(Gatilho, Result<Turno, IaError>)>,
    ) {
        let gatilho = acionamento.gatilho;

        if *ocupado {
            tracing::debug!(gatilho = gatilho.como_texto(), "turno anterior ainda rodando");
            return;
        }

        let agora = Utc::now();

        // O orçamento não vale em demonstração: ele existe para conter gasto, e
        // aqui não há chamada paga nenhuma. Sem esta saída, o descanso de seis
        // horas da ignição — que é gravado em disco de propósito — impediria a
        // segunda subida do app de pintar qualquer coisa, que é justamente
        // quando se está ajustando o layout.
        if demo {
            painel.cartoes = cartoes_de_demonstracao(gatilho);
            painel.gerado_em = Some(agora.to_rfc3339());
            painel.gatilho = Some(gatilho.como_texto().to_string());
            painel.pensando = false;
            ctx.ready(painel);
            return;
        }

        {
            let mut o = orcamento.lock().unwrap_or_else(|e| e.into_inner());
            match o.pode_falar(gatilho, agora) {
                Ok(()) => {
                    // Debita antes de gastar. Se a resposta demorar 40 segundos
                    // e outro gatilho chegar no meio, os dois passariam pelo
                    // mesmo teto.
                    o.registrar_fala(gatilho, agora);
                    tracing::info!(
                        gatilho = gatilho.como_texto(),
                        hoje = o.chamadas_hoje(),
                        teto = config.chamadas_por_dia,
                        "acionando o assistente",
                    );
                }
                Err(motivo) => {
                    tracing::debug!(
                        gatilho = gatilho.como_texto(),
                        ?motivo,
                        "gatilho barrado pelo orçamento"
                    );
                    if motivo == Recusa::TetoDiario {
                        ctx.degraded("o teto diário de chamadas do assistente acabou");
                    }
                    return;
                }
            }
        }

        let Some(transporte) = transporte.clone() else {
            return;
        };

        let mut cfg = ConfigAgente::nova(gatilho.modelo());
        cfg.estrito = config.estrito;
        cfg.mcp_remotos = mcp_remotos.to_vec();

        let agente = Agente::novo(
            transporte,
            Arc::clone(registro),
            Arc::clone(quadro),
            cfg,
        );

        *ocupado = true;
        painel.pensando = true;
        ctx.ready(painel);

        let tx = tx.clone();
        tokio::spawn(async move {
            let resultado = agente.rodar(&acionamento.pedido).await;
            let _ = tx.send((gatilho, resultado)).await;
        });
    }
}

/// `ECLIPSE_IA_GATILHO=rota` força um gatilho na subida.
///
/// Sem isto, ver o caminho do Opus (rota, alerta, chegada) exigiria sair
/// dirigindo até traçar um destino ou até o motor esquentar — o que é caro de
/// fazer a cada ajuste de texto. Rota e chegada usam um destino de mentira; o
/// resto o modelo busca sozinho, como faria de verdade.
fn gatilho_forcado() -> Option<Acionamento> {
    use crate::assistente::gatilho::Alerta;

    let pedido = std::env::var("ECLIPSE_IA_GATILHO").ok()?;
    match pedido.as_str() {
        "ignicao" => Some(Acionamento::ignicao()),
        "periodico" => Some(Acionamento::periodico()),
        "rota" => Some(Acionamento::rota("Campos do Jordão, SP")),
        "chegada" => Some(Acionamento::chegada("Campos do Jordão, SP")),
        "alerta" => Some(Acionamento::alerta(Alerta::TemperaturaAlta, 108.0)),
        outro => {
            tracing::warn!(
                outro,
                "ECLIPSE_IA_GATILHO desconhecido — use ignicao, periodico, rota, chegada ou alerta"
            );
            None
        }
    }
}

/// Cartões de mentira para `ECLIPSE_IA_DEMO=1`: exercita os cinco tipos e os
/// dois gráficos sem gastar token nenhum.
fn cartoes_de_demonstracao(gatilho: Gatilho) -> Vec<Cartao> {
    use eclipse_ia::{Ponto, TipoGrafico};

    vec![
        Cartao::Texto {
            titulo: Some("Sábado".into()),
            corpo: format!(
                "Céu limpo, 19°C. Trânsito leve na Bandeirantes. (demo: {})",
                gatilho.como_texto()
            ),
            tom: Tom::Bom,
        },
        Cartao::Metrica {
            rotulo: "Combustível".into(),
            valor: "38".into(),
            unidade: Some("%".into()),
            tom: Tom::Atencao,
        },
        Cartao::Grafico {
            titulo: "Temperatura".into(),
            grafico: TipoGrafico::Linha,
            unidade: Some("°C".into()),
            pontos: vec![
                Ponto { rotulo: "0".into(), valor: 62.0 },
                Ponto { rotulo: "5".into(), valor: 78.0 },
                Ponto { rotulo: "10".into(), valor: 88.0 },
                Ponto { rotulo: "15".into(), valor: 91.0 },
            ],
        },
        Cartao::Grafico {
            titulo: "Consumo".into(),
            grafico: TipoGrafico::Barras,
            unidade: Some("km/l".into()),
            pontos: vec![
                Ponto { rotulo: "seg".into(), valor: 8.2 },
                Ponto { rotulo: "ter".into(), valor: 9.1 },
                Ponto { rotulo: "qua".into(), valor: 7.4 },
                Ponto { rotulo: "qui".into(), valor: 10.3 },
            ],
        },
        Cartao::Lista {
            titulo: Some("No caminho".into()),
            itens: vec![
                "Pedágio em 12 km".into(),
                "Posto Graal, 40 km".into(),
                "Serra começa em 60 km".into(),
            ],
        },
        Cartao::Imagem {
            url: format!("arquivo:{ARQUIVO_DEMO}"),
            legenda: Some("Campos do Jordão".into()),
        },
    ]
}

const ARQUIVO_DEMO: &str = "demonstracao.svg";

/// Grava a imagem da demonstração no mesmo lugar onde as de verdade ficam.
///
/// Podia ser uma URL da internet, mas aí a demonstração — que existe justamente
/// para trabalhar no layout sem depender de nada — passaria a depender de rede e
/// de um link que apodrece. Do jeito que está, ela exercita o caminho de
/// verdade: arquivo no disco, comando `imagem_ia`, object URL no WebView.
fn gravar_imagem_de_demonstracao(dir_dados: &std::path::Path) {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">
  <rect width="320" height="180" fill="#12161d"/>
  <circle cx="252" cy="46" r="17" fill="#f5a524" opacity="0.85"/>
  <path d="M0,180 L74,84 L124,132 L176,66 L246,146 L320,96 L320,180 Z" fill="#1d2430"/>
  <path d="M176,66 L200,94 L152,94 Z" fill="#e8edf5" opacity="0.55"/>
  <path d="M0,180 L60,126 L128,168 L200,120 L268,164 L320,132 L320,180 Z" fill="#283040"/>
</svg>"##;

    let dir = dir_dados.join("ia_imagens");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(ARQUIVO_DEMO), SVG);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painel_vazio_serializa_no_formato_que_a_tela_espera() {
        let v = serde_json::to_value(Painel::default()).unwrap();
        assert_eq!(v["cartoes"], serde_json::json!([]));
        assert!(v["geradoEm"].is_null());
        assert_eq!(v["pensando"], false);
    }

    #[test]
    fn a_demonstracao_exercita_os_cinco_tipos_de_cartao() {
        let cartoes = cartoes_de_demonstracao(Gatilho::Ignicao);

        let tipos: Vec<String> = cartoes
            .iter()
            .map(|c| serde_json::to_value(c).unwrap()["tipo"].as_str().unwrap().to_string())
            .collect();

        for esperado in ["texto", "metrica", "grafico", "imagem", "lista"] {
            assert!(tipos.contains(&esperado.to_string()), "faltou {esperado}");
        }
    }

    /// Se a demonstração passar do teto, ela não representa o que a tela recebe.
    #[test]
    fn a_demonstracao_cabe_no_teto_de_cartoes() {
        assert!(cartoes_de_demonstracao(Gatilho::Ignicao).len() <= eclipse_ia::MAXIMO_CARTOES);
    }
}
