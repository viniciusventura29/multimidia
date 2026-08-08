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
//!
//! A rota é buscada **daqui** (ver `directions.rs` no `eclipse-gps`), não do
//! JavaScript: quem responde "quanto falta" e "saí do caminho" precisa da rota
//! e da posição ao mesmo tempo, e as duas moram deste lado. Como consequência
//! o recálculo virou uma chamada, não um pedido — antes o Rust levantava uma
//! bandeira e torcia para a tela obedecer.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use eclipse_clima::{Clima, ClimaError};
use eclipse_core::{Module, ModuleCommand, ModuleCtx, ModuleId, ModuleResult};
use eclipse_gps::{
    directions, fix::distancia_m, sol, Alvo, DirectionsError, FiltroDeParada, Fix, Guia,
    LocationSource, Progresso, Route,
};
use serde::Serialize;
use tokio::sync::mpsc;

pub const NAV: ModuleId = ModuleId::new("nav");

/// Onde assumir que o carro está enquanto o GPS não fixa — a mesma São Paulo
/// do `CENTRO_PADRAO` do frontend, para o tema inicial casar com o mapa
/// inicial. Erra o tema por minutos em outra cidade; o primeiro fix corrige.
const CENTRO_PADRAO: (f64, f64) = (-23.5505, -46.6333);

/// De quanto em quanto tempo reavaliar o tema sem depender de fix novo —
/// parado na garagem o sol se põe do mesmo jeito.
const RELOGIO_DO_SOL: Duration = Duration::from_secs(60);

/// Intervalo mínimo entre duas buscas **automáticas** de rota.
///
/// O recálculo que falha precisa poder tentar de novo — senão uma queda de rede
/// de dez segundos deixa o motorista sem rota até ele mesmo perceber. Mas
/// tentar a cada leitura de GPS seria uma requisição por segundo enquanto a
/// rede estiver fora. Vale só para o automático: o toque em "ir" é do
/// motorista e nunca espera.
const DESCANSO_ENTRE_RECALCULOS: Duration = Duration::from_secs(10);

/// Teto de uma busca de rota, ponta a ponta.
///
/// O `reqwest` **não tem timeout por padrão**, e um socket meio aberto é o
/// estado normal de um celular entrando em túnel: a conexão não cai, ela para
/// de responder. Sem teto, a task nunca volta pelo canal, `buscando` fica de pé
/// para sempre, o botão "ir" fica preso no `…` e o recálculo automático — que
/// desiste enquanto houver busca em curso — não sai mais pelo resto da ignição.
///
/// Vinte segundos: a Routes API responde em ~1 s, então isto é folga para rede
/// ruim, não para rede morta.
const TETO_DA_BUSCA: Duration = Duration::from_secs(20);

/// De quanto em quanto tempo perguntar o tempo de novo.
///
/// O Open-Meteo publica de quinze em quinze minutos; perguntar mais miúdo que
/// isso é gastar rede da head unit para receber o mesmo número.
const VALIDADE_DO_CLIMA: Duration = Duration::from_secs(15 * 60);

/// Quanto o carro precisa andar para o clima de onde ele estava não servir mais.
///
/// Vinte e cinco quilômetros é a ordem de grandeza em que a frente de chuva
/// muda numa viagem de estrada — e, dentro da cidade, é longe o bastante para
/// atravessá-la inteira sem disparar busca nenhuma.
const DERIVA_DO_CLIMA_M: f64 = 25_000.0;

fn agora_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Mapa {
    /// A chave do Google, para o que ainda passa por ele: sugestão de endereço
    /// e busca de postos, ambas chamadas do WebView.
    ///
    /// `None` é um estado de trabalho, não uma falha: o mapa é desenhado com
    /// tiles do OpenStreetMap, que não pedem chave nenhuma. Sem ela o painel
    /// continua mostrando onde o carro está — só não traça rota nem lista
    /// postos.
    ///
    /// Vai para o WebView de propósito: numa API de mapa web a chave é pública
    /// por natureza. Quem protege não é o sigilo, é o teto de cota do Google
    /// Cloud.
    api_key: Option<String>,

    /// Onde o carro está, **já encaixado na rua** quando há rota e ele está
    /// sobre ela (ver `Guia::grudar`). `None` enquanto o GPS não fixa — e isso
    /// é comum: garagem, túnel, prédio alto. O mapa continua na tela, só não
    /// segue.
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

    /// Tem uma rota sendo calculada agora. A tela usa para não deixar o
    /// motorista tocar "ir" duas vezes achando que não pegou.
    buscando: bool,

    /// Que tempo faz onde o carro está.
    ///
    /// Mora no `nav` e não num módulo próprio porque a pergunta "que tempo faz
    /// aqui" precisa do "aqui", e o "aqui" é deste módulo. Um módulo de clima
    /// separado teria que espiar o estado do vizinho — coisa que só o
    /// assistente faz, e por um motivo declarado.
    ///
    /// `None` até a primeira resposta; a barra de status mostra `--` e segue.
    clima: Option<Clima>,

    /// Por que a última busca não deu certo, se não deu.
    erro: Option<String>,
}

pub struct NavModule {
    api_key: Option<String>,
    gps: Box<dyn LocationSource>,
    guia: Option<Guia>,
    /// Congela a posição com o carro parado — sem ele, o jitter do GPS
    /// (dezenas de metros em Wi-Fi) faz o carro sambar no mapa e o progresso
    /// da rota tremer no semáforo. Ver `parada.rs` no `eclipse-gps`.
    filtro: FiltroDeParada,
    /// A última posição como o sensor a relatou, antes de encaixar na rua.
    ///
    /// Separada do que vai para a tela de propósito: uma rota nova precisa ser
    /// avaliada contra o que o GPS diz, não contra a posição já encaixada na
    /// rota **anterior** — senão a rota velha influenciaria a nova.
    ultimo_fix: Option<Fix>,
    /// Para onde o motorista pediu para ir. Guardado porque o recálculo é a
    /// mesma pergunta feita de outro lugar: o destino não muda, a origem sim.
    alvo: Option<Alvo>,
    /// Já disparamos o recálculo deste desvio? Sem esta marca, o pedido do
    /// `Guia` — que fica de pé enquanto o carro estiver fora da rota — viraria
    /// uma requisição por segundo até ele voltar. É zerada quando a busca
    /// falha, para uma queda de rede não deixar o motorista sem rota nova.
    recalculo_disparado: bool,
    /// Quando saiu a última busca automática. Ver `DESCANSO_ENTRE_RECALCULOS`.
    ultimo_recalculo: Option<Instant>,
    /// Quando e onde o clima foi buscado pela última vez.
    ///
    /// Guarda o ponto junto do instante porque as duas condições de reforço são
    /// "faz tempo" e "estou longe" — e o ponto é o de **onde a busca saiu**,
    /// não o de onde ela chegou: se ela falhar, a próxima leitura de GPS deve
    /// poder tentar de novo sem esperar os quinze minutos.
    clima_buscado: Option<(Instant, (f64, f64))>,
    /// Tem uma consulta de clima em voo. Sem isto, uma sequência de fixes
    /// dispararia várias antes da primeira responder.
    buscando_clima: bool,
    cliente: reqwest::Client,
}

impl NavModule {
    pub fn new(api_key: Option<String>, gps: Box<dyn LocationSource>) -> Self {
        Self {
            api_key,
            gps,
            guia: None,
            filtro: FiltroDeParada::novo(),
            ultimo_fix: None,
            alvo: None,
            recalculo_disparado: false,
            ultimo_recalculo: None,
            clima_buscado: None,
            buscando_clima: false,
            // Cliente com teto próprio, além do `TETO_DA_BUSCA` que envolve a
            // task: um socket que não responde precisa ser cortado nas duas
            // camadas, senão o `reqwest` segura a conexão viva sem nunca voltar.
            // Vale para o clima pela mesma razão — é o mesmo cliente, e um
            // Open-Meteo mudo deixaria `buscando_clima` de pé para sempre.
            cliente: reqwest::Client::builder()
                .timeout(TETO_DA_BUSCA)
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Module for NavModule {
    async fn run(&mut self, mut ctx: ModuleCtx) -> ModuleResult {
        // Faltar a chave do Google não é mais motivo para o módulo desistir: o
        // mapa vem do OpenStreetMap e a posição vem do aparelho. Só a busca de
        // destino e a lista de postos ficam de fora, e a tela sabe dizer isso.
        if self.api_key.is_none() {
            tracing::info!(
                "sem chave do Google — mapa e posição seguem, sem rota nem postos; \
                 defina ECLIPSE_MAPS_API_KEY ou crie maps_api_key.txt"
            );
        }

        let mut estado = Mapa {
            api_key: self.api_key.clone(),
            fix: None,
            rota: None,
            progresso: None,
            fala: None,
            noite: sol::e_noite(CENTRO_PADRAO.0, CENTRO_PADRAO.1, agora_unix()),
            buscando: false,
            erro: None,
            clima: None,
        };
        ctx.ready(&estado);

        let mut relogio = tokio::time::interval(RELOGIO_DO_SOL);

        // A busca de rota leva perto de um segundo. Fazê-la no meio do laço
        // seguraria as leituras de GPS por esse tempo — o carro congelaria no
        // mapa justo quando o motorista está esperando o caminho aparecer.
        // Então ela roda numa tarefa e volta por aqui, como o assistente faz.
        //
        // O `Alvo` volta junto com o resultado porque o destino pode ter mudado
        // no meio do caminho: sem ele não há como saber se a rota que chegou
        // ainda é a que se pediu.
        let (tx_rota, mut rx_rota) = mpsc::channel::<(Alvo, Result<Route, DirectionsError>)>(2);
        // Mesmo arranjo para o clima, e pelo mesmo motivo: uma consulta HTTP no
        // meio do laço seguraria o mapa parado enquanto ela não voltasse.
        let (tx_clima, mut rx_clima) = mpsc::channel::<Result<Clima, ClimaError>>(2);

        loop {
            tokio::select! {
                posicao = self.gps.next_fix() => match posicao {
                    Ok(fix) => {
                        let fix = self.filtro.filtrar(fix);
                        self.ultimo_fix = Some(fix);
                        estado.noite = sol::e_noite(fix.lat, fix.lon, agora_unix());
                        // A guiagem raciocina sobre a posição **crua**: é dela
                        // que saem o desvio e o recálculo. O que vai para a
                        // tela é a posição encaixada na rua — encaixar antes
                        // esconderia justamente o desvio que precisa ser visto.
                        estado.fix = Some(match self.guia.as_mut() {
                            Some(guia) => {
                                let (progresso, fala) = guia.avaliar(&fix);
                                estado.progresso = Some(progresso);
                                estado.fala = fala;
                                guia.grudar(&fix)
                            }
                            None => {
                                estado.progresso = None;
                                estado.fala = None;
                                fix
                            }
                        });

                        // Saímos do caminho tempo suficiente: a rota nova parte
                        // de onde o carro está agora, para o mesmo destino. Uma
                        // vez por desvio — a bandeira fica de pé enquanto ele
                        // durar, e obedecer a cada leitura seria uma requisição
                        // por segundo.
                        let fora = estado.progresso.as_ref().is_some_and(|p| p.recalcular);
                        if !fora {
                            self.recalculo_disparado = false;
                        } else if !self.recalculo_disparado && self.descansou() {
                            self.ultimo_recalculo = Some(Instant::now());
                            self.recalculo_disparado = self.tracar(&mut estado, &tx_rota);
                        }

                        // O primeiro fix é o que estreia o clima na barra; daí
                        // em diante só a distância percorrida força a mão.
                        self.talvez_clima((fix.lat, fix.lon), &tx_clima);

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
                    // O relógio é também quem envelhece o clima: parado na
                    // garagem não chega fix novo, mas a chuva chega.
                    self.talvez_clima((lat, lon), &tx_clima);
                },

                // A rota ficou pronta (ou não deu).
                recebido = rx_rota.recv() => {
                    estado.buscando = false;
                    match recebido {
                        // O destino mudou enquanto esta busca corria. Quem tocou
                        // duas vezes quer o segundo lugar, não o primeiro — e o
                        // pedido novo tinha sido engolido em silêncio, porque
                        // `tracar` desiste enquanto há busca em curso.
                        Some((alvo, _)) if self.alvo.as_ref() != Some(&alvo) => {
                            tracing::info!(
                                antes = %alvo.rotulo,
                                "o destino mudou durante a busca; refazendo"
                            );
                            estado.erro = None;
                            self.tracar(&mut estado, &tx_rota);
                        }
                        Some((_, Ok(rota))) => {
                            estado.erro = None;
                            self.assumir(rota, &mut estado);
                        }
                        Some((_, Err(err))) => {
                            tracing::warn!(%err, "não deu para traçar a rota");
                            estado.erro = Some(err.to_string());
                            // Falhou: o desvio continua de pé e merece outra
                            // tentativa. Quem impede a enxurrada é o descanso.
                            self.recalculo_disparado = false;
                        }
                        // O emissor é nosso e vive tanto quanto o laço.
                        None => {}
                    }
                    ctx.ready(&estado);
                },

                // O clima chegou (ou não deu).
                recebido = rx_clima.recv() => {
                    self.buscando_clima = false;
                    match recebido {
                        Some(Ok(clima)) => {
                            let mudou = estado.clima.as_ref() != Some(&clima);
                            estado.clima = Some(clima);
                            // Publicar só na mudança: de quinze em quinze
                            // minutos a temperatura costuma vir igual, e acordar
                            // o WebView para redesenhar o mesmo "22°" é IPC à toa.
                            if mudou {
                                ctx.ready(&estado);
                            }
                        }
                        // **Clima que falha não degrada o `nav`.** O mapa é o
                        // que este módulo existe para entregar; virar "sem
                        // sinal" porque a previsão não respondeu seria trocar o
                        // essencial pelo enfeite. Some o chip, fica o resto — e
                        // o `clima_buscado` já marcou o instante, então a
                        // próxima tentativa espera o mesmo descanso de sempre.
                        Some(Err(err)) => tracing::warn!(%err, "sem clima desta vez"),
                        None => {}
                    }
                },

                comando = ctx.next_command() => match comando {
                    None => return Ok(()),

                    Some(ModuleCommand::Action { payload, .. }) => {
                        match payload.get("acao").and_then(|v| v.as_str()) {
                            // A tela manda só **para onde** ir; quem busca é
                            // este módulo. O que chega daqui em diante — a
                            // rota, o progresso, o recálculo — é tudo derivado
                            // aqui, junto da posição.
                            Some("rota") => {
                                match serde_json::from_value::<Alvo>(
                                    payload.get("alvo").cloned().unwrap_or_default(),
                                ) {
                                    Ok(alvo) => {
                                        self.alvo = Some(alvo);
                                        self.recalculo_disparado = false;
                                        estado.erro = None;
                                        self.tracar(&mut estado, &tx_rota);
                                        ctx.ready(&estado);
                                    }
                                    Err(err) => {
                                        tracing::warn!(%err, "destino malformado");
                                    }
                                }
                            }
                            Some("cancelar") => {
                                self.guia = None;
                                self.alvo = None;
                                self.recalculo_disparado = false;
                                estado.rota = None;
                                estado.progresso = None;
                                estado.fala = None;
                                estado.erro = None;
                                // Sem rota não há em que encaixar: o carro volta
                                // para onde o sensor diz, em vez de ficar preso
                                // na linha de uma rota que não existe mais.
                                estado.fix = self.ultimo_fix;
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

impl NavModule {
    /// Já passou tempo bastante desde a última busca automática?
    fn descansou(&self) -> bool {
        self.ultimo_recalculo
            .is_none_or(|quando| quando.elapsed() >= DESCANSO_ENTRE_RECALCULOS)
    }

    /// Pergunta o tempo em (`lat`, `lon`) — se ainda valer a pena perguntar.
    ///
    /// Chamado de dois lugares (a cada fix e a cada minuto do relógio do sol),
    /// então a decisão de *não* perguntar mora aqui dentro: é o único jeito de
    /// as duas chamadas não precisarem repetir a regra.
    fn talvez_clima(&mut self, onde: (f64, f64), tx: &mpsc::Sender<Result<Clima, ClimaError>>) {
        if self.buscando_clima {
            return;
        }

        let vale = match self.clima_buscado {
            None => true,
            Some((quando, ponto)) => {
                quando.elapsed() >= VALIDADE_DO_CLIMA
                    || distancia_m(ponto, onde) >= DERIVA_DO_CLIMA_M
            }
        };
        if !vale {
            return;
        }

        self.buscando_clima = true;
        self.clima_buscado = Some((Instant::now(), onde));

        let cliente = self.cliente.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let clima = eclipse_clima::buscar(&cliente, onde.0, onde.1).await;
            let _ = tx.send(clima).await;
        });
    }

    /// Dispara a busca de rota numa tarefa à parte. Devolve se disparou mesmo.
    ///
    /// Silencioso quando não há para onde ir, de onde sair, ou já há uma busca
    /// em curso: são as três formas de gastar uma requisição à toa.
    fn tracar(
        &mut self,
        estado: &mut Mapa,
        tx: &mpsc::Sender<(Alvo, Result<Route, DirectionsError>)>,
    ) -> bool {
        if estado.buscando {
            return false;
        }
        let (Some(alvo), Some(fix)) = (self.alvo.clone(), self.ultimo_fix) else {
            return false;
        };

        estado.buscando = true;

        let cliente = self.cliente.clone();
        let Some(chave) = estado.api_key.clone() else {
            estado.buscando = false;
            estado.erro = Some("falta a chave do Google para traçar rota".into());
            return false;
        };
        let tx = tx.clone();
        tokio::spawn(async move {
            // Teto por fora do cliente também: o que não pode acontecer é a
            // task morrer sem responder. Enquanto o canal não devolve nada,
            // `buscando` fica de pé e trava a busca e o recálculo juntos.
            let rota = match tokio::time::timeout(
                TETO_DA_BUSCA,
                directions::buscar(&cliente, &chave, (fix.lat, fix.lon), &alvo),
            )
            .await
            {
                Ok(rota) => rota,
                Err(_) => {
                    tracing::warn!(destino = %alvo.rotulo, "a busca de rota estourou o tempo");
                    Err(DirectionsError::Rede)
                }
            };
            let _ = tx.send((alvo, rota)).await;
        });
        true
    }

    /// Passa a guiar por uma rota recém-chegada.
    fn assumir(&mut self, rota: Route, estado: &mut Mapa) {
        estado.rota = Some(rota.clone());
        let mut guia = Guia::nova(rota);

        // Traçar a rota já encaixa o carro nela: esperar a próxima leitura
        // deixaria o carro um segundo ao lado da linha que acabou de aparecer.
        if let Some(fix) = &self.ultimo_fix {
            let (progresso, fala) = guia.avaliar(fix);
            estado.progresso = Some(progresso);
            estado.fala = fala;
            estado.fix = Some(guia.grudar(fix));
        }

        self.guia = Some(guia);
    }
}
