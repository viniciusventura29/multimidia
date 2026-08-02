//! O laço do agente: monta o pedido, executa ferramenta, repete até o modelo
//! parar.

use std::sync::Arc;

use eclipse_mcp::Registro;
use serde_json::{json, Value};

use crate::cartao::{Quadro, MAXIMO_CARTOES};
use crate::cliente::{IaError, Transporte, BETA_MCP};
use crate::modelo::Modelo;
use crate::quadro::ProvedorQuadro;

/// Um servidor MCP remoto plugado pelo `mcp_servers.json`.
///
/// Repare que só servidor **remoto** entra aqui. O conector de MCP da Anthropic
/// conecta do lado dela, então o alvo precisa de endereço público — e uma head
/// unit atrás do 4G do celular não tem. É por isso que o carro é ferramenta
/// local no `Registro`, e não um servidor MCP que a Anthropic viesse buscar.
#[derive(Clone, Debug)]
pub struct McpRemoto {
    pub nome: String,
    pub url: String,
    pub token: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub modelo: Modelo,
    pub sistema: String,
    /// Teto de idas e voltas com a API num turno.
    pub max_iteracoes: usize,
    /// Teto de tokens de entrada somados num turno.
    ///
    /// O `max_iteracoes` limita as idas e voltas, não o tamanho delas — e o que
    /// custa caro é o tamanho. A busca web com filtragem dinâmica traz o
    /// conteúdo das páginas para dentro do contexto, e uma página longa põe
    /// dezenas de milhares de tokens num turno só: um caso medido em produção
    /// chegou a 150 mil de entrada, ~US$ 1,37 num turno de Opus.
    ///
    /// Aqui o corte é nosso e é duro. Se o modelo já pintou quando o teto
    /// estoura, o quadro vale; se não pintou, o turno vira erro — e é melhor
    /// perder um comentário sobre a estrada do que a fatura do mês.
    pub max_tokens_turno: u64,
    /// Validação estrita do esquema das ferramentas.
    ///
    /// Fica configurável para poder ser desligado pelo `assistente.json`, sem
    /// recompilar, se algum dia a API recusar o esquema da união de cartões. O
    /// aparelho vive preso ao painel de um carro; um interruptor que não exige
    /// build de APK vale o campo a mais.
    pub estrito: bool,
    pub mcp_remotos: Vec<McpRemoto>,
}

impl Config {
    pub fn nova(modelo: Modelo) -> Self {
        Self {
            modelo,
            sistema: sistema_padrao(),
            // Seis, e não oito: cada ida reenvia a conversa inteira, então as
            // últimas são as mais caras. Seis dá espaço para consultar,
            // pesquisar, pintar e refinar uma vez.
            max_iteracoes: 6,
            // Rede de segurança, não controle principal — quem segura o
            // tamanho é o `max_content_tokens` do `web_fetch`. Precisa ser
            // folgado: cada ida reenvia a conversa inteira, então três idas de
            // pesquisa somam 60–70 mil naturalmente. Com 40 mil o corte pegava
            // a assistente no meio do trabalho e deixava rascunho na tela.
            max_tokens_turno: 120_000,
            estrito: true,
            mcp_remotos: Vec::new(),
        }
    }
}

/// Quanto custou o turno.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uso {
    pub entrada: u64,
    pub saida: u64,
    pub cache_escrita: u64,
    pub cache_leitura: u64,
}

impl Uso {
    fn somar(&mut self, usage: &Value) {
        let campo = |n: &str| usage.get(n).and_then(Value::as_u64).unwrap_or(0);
        self.entrada += campo("input_tokens");
        self.saida += campo("output_tokens");
        self.cache_escrita += campo("cache_creation_input_tokens");
        self.cache_leitura += campo("cache_read_input_tokens");
    }
}

#[derive(Debug)]
pub struct Turno {
    /// O que foi pintado. `None` quando o modelo não achou o que dizer — e isso
    /// é resultado legítimo, não falha: quadro velho é pior que quadro nenhum.
    pub quadro: Option<Quadro>,
    pub uso: Uso,
    pub iteracoes: usize,
    /// Os classificadores de segurança recusaram o pedido.
    pub recusou: bool,
}

pub struct Agente {
    transporte: Arc<dyn Transporte>,
    registro: Arc<Registro>,
    quadro: Arc<ProvedorQuadro>,
    config: Config,
    betas: Vec<String>,
}

impl Agente {
    /// `quadro` precisa ser o mesmo provedor que está dentro de `registro` — é
    /// por ele que o resultado do turno sai.
    pub fn novo(
        transporte: Arc<dyn Transporte>,
        registro: Arc<Registro>,
        quadro: Arc<ProvedorQuadro>,
        config: Config,
    ) -> Self {
        let betas = if config.mcp_remotos.is_empty() {
            Vec::new()
        } else {
            vec![BETA_MCP.to_string()]
        };

        Self {
            transporte,
            registro,
            quadro,
            config,
            betas,
        }
    }

    /// As ferramentas na forma que a API espera.
    ///
    /// A ordem importa: `tools` é a primeira coisa renderizada no prompt, e o
    /// cache de prefixo morre se a lista mudar de ordem entre chamadas. O
    /// `Registro` preserva a ordem de registro, então ela é estável.
    fn ferramentas(&self) -> Vec<Value> {
        let mut lista: Vec<Value> = self
            .registro
            .listar()
            .iter()
            .map(|f| {
                let mut v = json!({
                    "name": f.nome,
                    "description": f.descricao,
                    // A API pede `input_schema`; o MCP pede `inputSchema`. Aqui
                    // é o lado da API.
                    "input_schema": f.esquema,
                });
                if self.config.estrito {
                    v["strict"] = json!(true);
                }
                v
            })
            .collect();

        lista.extend(self.config.modelo.ferramentas_de_servidor());

        // Cada servidor MCP declarado em `mcp_servers` precisa de um
        // `mcp_toolset` correspondente aqui — sem o par, a API devolve 400.
        for remoto in &self.config.mcp_remotos {
            lista.push(json!({
                "type": "mcp_toolset",
                "mcp_server_name": remoto.nome,
            }));
        }

        lista
    }

    fn montar_corpo(&self, mensagens: &[Value], container: Option<&str>) -> Value {
        let mut corpo = json!({
            "model": self.config.modelo.id(),
            "max_tokens": self.config.modelo.max_tokens(),
            "system": [{
                "type": "text",
                "text": self.config.sistema,
                // A ordem de renderização é tools -> system -> messages, então
                // marcar o último bloco do system guarda ferramentas E system
                // juntos. É o que faz o gatilho repetido custar ~10% do prompt.
                // Para isso funcionar, nada aqui pode variar entre chamadas: a
                // data entra por `contexto_agora`, que é ferramenta, e não pelo
                // texto do system.
                "cache_control": { "type": "ephemeral" },
            }],
            "tools": self.ferramentas(),
            "messages": mensagens,
        });

        // A busca web `_20260209` filtra os resultados rodando código do lado da
        // Anthropic, num contêiner. Quando isso acontece, a resposta traz um
        // `container` e a API **exige** que ele volte nas requisições seguintes
        // do mesmo turno — senão é 400 com "container_id is required when there
        // are pending tool uses generated by code execution".
        //
        // Só acontece com o Opus: o Haiku usa a busca básica, que não executa
        // código. Foi assim que isto passou pelos testes e só apareceu na
        // primeira conversa de verdade.
        if let Some(id) = container {
            corpo["container"] = json!(id);
        }

        if let Some(pensamento) = self.config.modelo.pensamento() {
            corpo["thinking"] = pensamento;
        }
        if let Some(esforco) = self.config.modelo.esforco() {
            corpo["output_config"] = json!({ "effort": esforco });
        }
        if !self.config.mcp_remotos.is_empty() {
            corpo["mcp_servers"] = Value::Array(
                self.config
                    .mcp_remotos
                    .iter()
                    .map(|r| {
                        let mut s = json!({ "type": "url", "name": r.nome, "url": r.url });
                        if let Some(token) = &r.token {
                            s["authorization_token"] = json!(token);
                        }
                        s
                    })
                    .collect(),
            );
        }

        corpo
    }

    /// As chamadas de ferramenta **nossas** dentro da resposta.
    ///
    /// Blocos de `server_tool_use`, `web_search_tool_result` e `mcp_tool_use`
    /// ficam de fora: quem executa esses é a Anthropic, e responder a eles com
    /// um `tool_result` nosso seria pedido malformado.
    fn chamadas_locais(conteudo: &Value) -> Vec<(String, String, Value)> {
        conteudo
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .filter_map(|b| {
                Some((
                    b["id"].as_str()?.to_string(),
                    b["name"].as_str()?.to_string(),
                    b.get("input").cloned().unwrap_or(json!({})),
                ))
            })
            .collect()
    }

    pub async fn rodar(&self, pedido: &str) -> Result<Turno, IaError> {
        let mut mensagens = vec![json!({ "role": "user", "content": pedido })];
        let mut uso = Uso::default();
        let mut iteracoes = 0;
        let mut terminou = false;
        let mut container: Option<String> = None;
        let mut falha: Option<IaError> = None;

        while iteracoes < self.config.max_iteracoes {
            iteracoes += 1;

            let corpo = self.montar_corpo(&mensagens, container.as_deref());
            let resposta = match self.transporte.enviar(&corpo, &self.betas).await {
                Ok(r) => r,
                // Guardar em vez de propagar com `?`. O modelo pode já ter
                // chamado `pintar_quadro` numa iteração anterior, e sair daqui
                // direto jogaria fora um quadro pronto e pago por causa de uma
                // falha no passo seguinte.
                Err(err) => {
                    falha = Some(err);
                    break;
                }
            };
            uso.somar(&resposta["usage"]);

            if let Some(id) = resposta["container"]["id"].as_str() {
                container = Some(id.to_string());
            }

            let entrada_total = uso.entrada + uso.cache_escrita + uso.cache_leitura;
            let estourou = entrada_total > self.config.max_tokens_turno;

            let conteudo = resposta.get("content").cloned().unwrap_or_else(|| json!([]));
            let parada = resposta["stop_reason"].as_str().unwrap_or("");

            match parada {
                // Os classificadores recusaram. Não é para insistir nem para
                // alarmar o motorista — o quadro simplesmente não muda.
                "refusal" => {
                    tracing::info!("a API recusou o pedido do assistente");
                    return Ok(Turno {
                        quadro: None,
                        uso,
                        iteracoes,
                        recusou: true,
                    });
                }

                // Uma ferramenta de servidor bateu no teto de iterações do lado
                // de lá. Reenviar a conversa como está faz a API continuar de
                // onde parou — sem mensagem nova, que atrapalharia.
                "pause_turn" => {
                    mensagens.push(json!({ "role": "assistant", "content": conteudo }));
                }

                "tool_use" => {
                    let chamadas = Self::chamadas_locais(&conteudo);
                    if chamadas.is_empty() {
                        // Parou por ferramenta mas nenhuma é nossa: não há o que
                        // responder, e insistir seria laço infinito.
                        tracing::warn!("parada em tool_use sem chamada local");
                        terminou = true;
                        break;
                    }

                    mensagens.push(json!({ "role": "assistant", "content": conteudo }));

                    // Em sequência, e não em paralelo: as ferramentas do carro
                    // leem um retrato já pronto do barramento e voltam em
                    // microssegundos. A única demorada é gerar imagem, e essa o
                    // modelo pede sozinha. Paralelizar custaria uma dependência
                    // a mais para ganhar nada.
                    let mut resultados = Vec::with_capacity(chamadas.len());
                    for (id, nome, args) in chamadas {
                        let r = self.registro.chamar(&nome, &args).await;
                        if r.erro {
                            tracing::warn!(ferramenta = %nome, "ferramenta falhou: {}", r.texto());
                        } else {
                            // O comportamento aqui é decidido pelo modelo, não
                            // pelo código: sem ver quais ferramentas ele pegou,
                            // um turno que não pinta nada é indistinguível de um
                            // turno quebrado.
                            tracing::debug!(ferramenta = %nome, "ferramenta ok");
                        }
                        resultados.push(json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": r.texto(),
                            "is_error": r.erro,
                        }));
                    }

                    // Encerrar o turno na primeira pintura foi tentado, e foi
                    // pior. O modelo trabalha rascunhando: pinta um esboço,
                    // olha, e refina — e como só o último quadro vale, isso
                    // sempre funcionou. Cortar na primeira pintura não elimina o
                    // rascunho, só congela ele na tela: as medições devolveram
                    // `{"titulo":"placeholder","pontos":[]}` e
                    // `{"rotulo":"tipo","valor":"—"}` como resposta final.
                    //
                    // O custo se controla no tamanho (`max_tokens_turno`, o
                    // `max_content_tokens` da busca) e no número de idas
                    // (`max_iteracoes`) — não interrompendo o método de trabalho
                    // dele no meio.

                    // TODOS os resultados numa mensagem só. Espalhar em várias
                    // mensagens de user ensina o modelo a parar de pedir
                    // ferramentas em paralelo.
                    mensagens.push(json!({ "role": "user", "content": resultados }));
                }

                // end_turn, max_tokens, stop_sequence: acabou.
                _ => {
                    terminou = true;
                    break;
                }
            }

            // O corte por tamanho vem no fim da volta, e não no começo: as
            // ferramentas pendentes já foram executadas acima, e executá-las é
            // de graça — inclusive `pintar_quadro`. O que custa é a requisição
            // seguinte, e é ela que não sai.
            if estourou {
                tracing::warn!(
                    entrada_total,
                    teto = self.config.max_tokens_turno,
                    "turno cortado por tamanho — a pesquisa trouxe conteúdo demais"
                );
                terminou = true;
                break;
            }
        }

        let quadro = self.quadro.tomar();

        // O que já foi pintado vale, mesmo que o turno tenha terminado mal.
        // Falhar sem quadro é falha de verdade — gastou token e não produziu
        // tela; falhar com quadro é só o fim de um turno que já entregou.
        if quadro.is_none() {
            // Turno que falha gastou token igual. Sem esta linha o consumo dele
            // some junto com o erro — e foi exatamente o que aconteceu no
            // primeiro 400 de verdade: deu para ver que falhou, não quanto
            // custou.
            if falha.is_some() || !terminou {
                tracing::warn!(
                    iteracoes,
                    entrada = uso.entrada,
                    saida = uso.saida,
                    cache_leitura = uso.cache_leitura,
                    cache_escrita = uso.cache_escrita,
                    "turno sem quadro; o consumo abaixo foi gasto do mesmo jeito",
                );
            }
            if let Some(err) = falha {
                return Err(err);
            }
            if !terminou {
                return Err(IaError::NaoTerminou(self.config.max_iteracoes));
            }
        } else if let Some(err) = falha {
            tracing::warn!(%err, "o turno falhou depois de pintar; fico com o quadro");
        }

        Ok(Turno {
            quadro,
            uso,
            iteracoes,
            recusou: false,
        })
    }
}

/// O prompt de sistema.
///
/// **Não interpole nada variável aqui.** Ele é o prefixo cacheado; um byte
/// diferente entre chamadas invalida o cache inteiro e cada gatilho volta a
/// pagar o prompt completo. Hora e data entram por `contexto_agora`, que é
/// ferramenta. `MAXIMO_CARTOES` é constante de compilação, então pode.
pub fn sistema_padrao() -> String {
    format!(
        "Você é a assistente do Eclipse OS, o computador de bordo de um Mitsubishi Eclipse \
GT 2000.

COMO ISTO FUNCIONA
Você escreve num quadro estreito e alto, na lateral do painel, enquanto a pessoa dirige. \
Ela não digita, não fala com você e não tem como responder — não existe entrada. Você é \
acionada por acontecimentos: o carro ligou, uma rota foi traçada, o motor esquentou, a \
viagem está longa. O que você pintar fica na tela até a próxima vez.

Nada do que você escrever como texto aparece para ninguém. A ÚNICA saída é a ferramenta \
`pintar_quadro`. Turno que termina sem chamá-la é turno perdido.

O que você pinta aparece na tela imediatamente, e o turno pode ser interrompido a qualquer \
momento por limite de tempo ou de custo. Então **nunca pinte rascunho nem placeholder**: \
pesquise primeiro, pinte uma vez, com o conteúdo pronto.

ANTES DE ESCREVER
Chame `contexto_agora` sempre — você não tem relógio próprio, e sábado de manhã pede uma \
coisa que terça à noite não pede. Chame as ferramentas do carro antes de comentar qualquer \
coisa sobre o carro, e nunca invente número de telemetria. Se houver destino traçado, \
pesquise sobre ele: como está o tempo lá, como está o caminho, o que existe por perto.

COMO ESCREVER
Português do Brasil, direto. Sem saudação formal, sem se apresentar, sem emoji.

**Seja econômico. Este é o ponto mais importante.** O teto é {MAXIMO_CARTOES} cartões, mas \
dois quase sempre são melhores que três, e um bom cartão é melhor que dois medianos. O teto \
é teto, não meta. Cada cartão tem que ganhar o seu lugar: se você tirasse ele, o motorista \
perderia alguma coisa? Se não, não ponha.

**Um cartão de texto é UMA frase, de no máximo 20 palavras.** Não duas frases curtas: uma. \
A coluna tem a largura de um celular em pé, e cada linha a mais empurra o resto para fora \
da tela.

Assim, não:
\"Cidade em pleno inverno: máximas perto de 19°C e mínimas que caem a 6°C, podendo beirar \
zero de madrugada. Mês seco, quase não chove. Leve agasalho pesado.\"

Assim, sim:
\"Mínima de 6°C de madrugada, e seco. Leve agasalho pesado.\"

Corte o que o motorista já sabe (que é inverno), o que não muda decisão nenhuma (a média \
de chuva do mês) e o que é detalhe de origem (\"não consigo ler o OBD porque só funciona no \
Android\" — basta \"sem leitura do carro agora\").

**Nunca diga a mesma coisa duas vezes.** Se a temperatura já está na frase, não faça um \
cartão de métrica com ela. Se o clima já está no aviso, não faça um cartão de clima. \
Escolha o melhor formato para cada informação e use só ele.

Não narre o que você fez (\"consultei a telemetria e...\"). Não peça desculpas. Não ofereça \
ajuda (\"quer que eu...\") — ela não tem como aceitar. Não repita o que já está em outro \
lugar do painel: velocidade, RPM, temperatura e a música que toca já têm mostrador próprio. \
Diga o que os outros quadros não dizem.

Se não houver nada que valha a pena, não pinte. A tela tem uma animação para as horas em \
que não há novidade, e ela é melhor que um cartão dizendo que está tudo normal.

IMAGEM
Foto de verdade primeiro: a capa do álbum que está tocando, a foto de um lugar, uma imagem \
achada na web. `gerar_imagem` custa dinheiro e demora — use só quando a imagem for o ponto \
do cartão e não existir foto que sirva.

SEGURANÇA
Nunca sugira mexer no celular, digitar ou encarar a tela. Se a telemetria mostrar algo \
grave — temperatura subindo, combustível no fim, tensão baixa — esse é o primeiro cartão, \
com tom `alerta`, dizendo em uma frase o que fazer."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartao::Cartao;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Um transporte que devolve respostas gravadas e guarda o que recebeu.
    struct Dublê {
        respostas: Mutex<VecDeque<Value>>,
        pedidos: Mutex<Vec<Value>>,
        betas: Mutex<Vec<Vec<String>>>,
    }

    impl Dublê {
        fn com(respostas: Vec<Value>) -> Arc<Self> {
            Arc::new(Self {
                respostas: Mutex::new(respostas.into()),
                pedidos: Mutex::new(Vec::new()),
                betas: Mutex::new(Vec::new()),
            })
        }

        fn pedidos(&self) -> Vec<Value> {
            self.pedidos.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Transporte for Dublê {
        async fn enviar(&self, corpo: &Value, betas: &[String]) -> Result<Value, IaError> {
            self.pedidos.lock().unwrap().push(corpo.clone());
            self.betas.lock().unwrap().push(betas.to_vec());
            self.respostas
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| IaError::Resposta("o dublê ficou sem resposta".into()))
        }
    }

    fn pede_ferramentas(chamadas: &[(&str, &str, Value)]) -> Value {
        json!({
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 100, "output_tokens": 20 },
            "content": chamadas.iter().map(|(id, nome, args)| json!({
                "type": "tool_use", "id": id, "name": nome, "input": args
            })).collect::<Vec<_>>(),
        })
    }

    fn fim() -> Value {
        json!({
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 50, "output_tokens": 5 },
            "content": [{ "type": "text", "text": "pronto" }],
        })
    }

    fn pintura(corpo: &str) -> Value {
        json!({ "cartoes": [{ "tipo": "texto", "corpo": corpo, "titulo": null, "tom": "neutro" }] })
    }

    /// Monta um agente com o provedor do quadro e, opcionalmente, um provedor de
    /// carro de mentira.
    fn agente(dublê: Arc<Dublê>, config: Config) -> (Agente, Arc<ProvedorQuadro>) {
        let quadro = Arc::new(ProvedorQuadro::novo());
        let registro = Registro::nova().com(quadro.clone());
        (
            Agente::novo(dublê, Arc::new(registro), quadro.clone(), config),
            quadro,
        )
    }

    #[tokio::test]
    async fn laco_executa_ferramenta_e_devolve_o_quadro() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("t0", "carro_telemetria", json!({}))]),
            pede_ferramentas(&[("t1", "pintar_quadro", pintura("Sábado limpo."))]),
            fim(),
        ]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Haiku));

        let turno = agente.rodar("o carro ligou").await.unwrap();

        // Consultar, pintar, encerrar.
        assert_eq!(turno.iteracoes, 3);
        let quadro = turno.quadro.expect("nada foi pintado");
        assert!(matches!(&quadro.cartoes[0], Cartao::Texto { corpo, .. } if corpo == "Sábado limpo."));
    }

    /// Espalhar os resultados em várias mensagens de user ensina o modelo a
    /// parar de pedir ferramentas em paralelo.
    /// Rascunhar e refinar é o método do modelo, e só o último quadro vale.
    /// Interromper na primeira pintura deixava o rascunho na tela.
    #[tokio::test]
    async fn refinar_o_quadro_e_permitido_e_vale_o_ultimo() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "pintar_quadro", pintura("rascunho"))]),
            pede_ferramentas(&[("b", "pintar_quadro", pintura("versão final"))]),
            fim(),
        ]);

        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.unwrap();

        assert!(
            matches!(&turno.quadro.unwrap().cartoes[0], Cartao::Texto { corpo, .. } if corpo == "versão final"),
            "o refinamento tem que substituir o rascunho"
        );
    }

    /// Mas uma pintura recusada não conta como entrega — ela precisa poder
    /// consertar e tentar de novo.
    #[tokio::test]
    async fn pintura_recusada_nao_encerra_o_turno() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "pintar_quadro", json!({ "cartoes": [] }))]),
            pede_ferramentas(&[("b", "pintar_quadro", pintura("agora vai"))]),
            fim(),
        ]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.unwrap();
        assert!(dublê.pedidos().len() >= 2, "precisa ter tido uma segunda chance");
        assert!(turno.quadro.is_some());
    }

    #[tokio::test]
    async fn resultados_paralelos_voltam_numa_mensagem_so() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[
                ("a", "carro_telemetria", json!({})),
                ("b", "carro_musica", json!({})),
            ]),
            fim(),
        ]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Haiku));
        agente.rodar("x").await.unwrap();

        let segundo = &dublê.pedidos()[1];
        let mensagens = segundo["messages"].as_array().unwrap();

        assert_eq!(mensagens.len(), 3, "user, assistant, user — não mais");
        let resultados = mensagens[2]["content"].as_array().unwrap();
        assert_eq!(resultados.len(), 2);
        assert!(resultados.iter().all(|r| r["type"] == "tool_result"));
    }

    /// Ferramenta que falha não derruba o turno: o erro volta ao modelo por
    /// escrito para ele tentar outro caminho.
    #[tokio::test]
    async fn ferramenta_que_falha_volta_como_is_error_e_o_laco_segue() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "inexistente", json!({}))]),
            pede_ferramentas(&[("b", "pintar_quadro", pintura("consegui"))]),
            fim(),
        ]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Haiku));

        let turno = agente.rodar("x").await.unwrap();
        assert!(turno.quadro.is_some());

        let resultado = &dublê.pedidos()[1]["messages"][2]["content"][0];
        assert_eq!(resultado["is_error"], true);
        assert!(resultado["content"].as_str().unwrap().contains("inexistente"));
    }

    #[tokio::test]
    async fn pause_turn_reenvia_a_conversa_sem_mensagem_nova() {
        let dublê = Dublê::com(vec![
            json!({
                "stop_reason": "pause_turn",
                "usage": { "input_tokens": 10, "output_tokens": 2 },
                "content": [{ "type": "server_tool_use", "id": "s1", "name": "web_search", "input": {} }],
            }),
            pede_ferramentas(&[("t", "pintar_quadro", pintura("achei"))]),
            fim(),
        ]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.unwrap();
        assert!(turno.quadro.is_some());

        let segundo = &dublê.pedidos()[1]["messages"];
        assert_eq!(segundo.as_array().unwrap().len(), 2);
        assert_eq!(
            segundo[1]["role"], "assistant",
            "o turno pausado volta como assistant, sem user novo no meio"
        );
    }

    /// Bloco de ferramenta de servidor não é nosso: responder a ele seria
    /// pedido malformado.
    #[test]
    fn so_tool_use_local_vira_chamada() {
        let conteudo = json!([
            { "type": "text", "text": "pensando" },
            { "type": "server_tool_use", "id": "s1", "name": "web_search", "input": {} },
            { "type": "web_search_tool_result", "tool_use_id": "s1", "content": [] },
            { "type": "mcp_tool_use", "id": "m1", "name": "clima", "input": {} },
            { "type": "tool_use", "id": "t1", "name": "carro_telemetria", "input": {} },
        ]);

        let chamadas = Agente::chamadas_locais(&conteudo);
        assert_eq!(chamadas.len(), 1);
        assert_eq!(chamadas[0].1, "carro_telemetria");
    }

    #[tokio::test]
    async fn recusa_encerra_em_silencio_e_nao_e_erro() {
        let dublê = Dublê::com(vec![json!({
            "stop_reason": "refusal",
            "usage": { "input_tokens": 30, "output_tokens": 0 },
            "content": [],
        })]);
        let (agente, _) = agente(dublê, Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.unwrap();
        assert!(turno.recusou);
        assert!(turno.quadro.is_none());
    }

    #[tokio::test]
    async fn estourar_o_teto_sem_pintar_e_erro() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "inexistente", json!({}))]),
            pede_ferramentas(&[("b", "inexistente", json!({}))]),
        ]);
        let mut config = Config::nova(Modelo::Haiku);
        config.max_iteracoes = 2;
        let (agente, _) = agente(dublê, config);

        assert!(matches!(
            agente.rodar("x").await,
            Err(IaError::NaoTerminou(2))
        ));
    }

    /// Mas se pintou antes de estourar, o que foi pintado vale.
    #[tokio::test]
    async fn estourar_o_teto_depois_de_pintar_entrega_o_quadro() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "pintar_quadro", pintura("deu tempo"))]),
            pede_ferramentas(&[("b", "inexistente", json!({}))]),
        ]);
        let mut config = Config::nova(Modelo::Haiku);
        config.max_iteracoes = 2;
        let (agente, _) = agente(dublê, config);

        let turno = agente.rodar("x").await.unwrap();
        assert!(turno.quadro.is_some());
    }

    /// A busca web do Opus filtra resultados rodando código num contêiner do
    /// lado da Anthropic. A resposta traz o `container`, e a API exige que ele
    /// volte nas requisições seguintes do mesmo turno — sem isso é 400.
    ///
    /// Este teste existe porque a versão anterior não o tinha: o dublê nunca
    /// devolvia `container`, então o laço passava verde e o erro só apareceu na
    /// primeira conversa de verdade com o Opus.
    #[tokio::test]
    async fn container_da_busca_web_volta_nas_requisicoes_seguintes() {
        // Uma ferramenta que não é a pintura: pintar encerraria o turno e não
        // haveria requisição seguinte para conferir.
        let mut com_container = pede_ferramentas(&[("a", "carro_telemetria", json!({}))]);
        com_container["container"] = json!({ "id": "cntr_abc123" });

        let dublê = Dublê::com(vec![com_container, fim()]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));
        agente.rodar("x").await.unwrap();

        let pedidos = dublê.pedidos();
        assert!(
            pedidos[0].get("container").is_none(),
            "o primeiro pedido não tem contêiner para mandar"
        );
        assert_eq!(
            pedidos[1]["container"], "cntr_abc123",
            "o contêiner da resposta anterior precisa voltar, senão é 400"
        );
    }

    /// Falhar depois de pintar não pode apagar o que já foi pintado — foi pago.
    #[tokio::test]
    async fn quadro_pintado_sobrevive_a_falha_no_passo_seguinte() {
        let dublê = Dublê::com(vec![pede_ferramentas(&[(
            "a",
            "pintar_quadro",
            pintura("já estava pronto"),
        )])]);
        // A segunda chamada não tem resposta gravada: o dublê erra.
        let (agente, _) = agente(dublê, Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.expect("a falha apagou o quadro");
        let quadro = turno.quadro.expect("o quadro sumiu");
        assert!(
            matches!(&quadro.cartoes[0], Cartao::Texto { corpo, .. } if corpo == "já estava pronto")
        );
    }

    /// Mas falhar sem ter pintado continua sendo falha.
    #[tokio::test]
    async fn falha_sem_quadro_continua_sendo_erro() {
        let dublê = Dublê::com(vec![]);
        let (agente, _) = agente(dublê, Config::nova(Modelo::Haiku));
        assert!(agente.rodar("x").await.is_err());
    }

    /// O teto de tamanho é o que segura o custo quando a busca web traz uma
    /// página enorme. Um turno medido em produção chegou a 150 mil tokens de
    /// entrada; `max_iteracoes` não pegava isso, porque foram poucas idas e
    /// voltas — só muito grandes.
    #[tokio::test]
    async fn turno_gordo_demais_e_cortado() {
        let mut gordo = pede_ferramentas(&[("a", "pintar_quadro", pintura("pintei antes"))]);
        gordo["usage"] = json!({ "input_tokens": 90_000, "output_tokens": 100 });

        let dublê = Dublê::com(vec![gordo, fim()]);
        let mut config = Config::nova(Modelo::Opus);
        config.max_tokens_turno = 40_000;
        let (agente, _) = agente(dublê.clone(), config);

        let turno = agente.rodar("x").await.unwrap();

        assert_eq!(dublê.pedidos().len(), 1, "não pode pedir mais depois de estourar");
        assert!(
            turno.quadro.is_some(),
            "o que já tinha sido pintado continua valendo"
        );
    }

    /// E o corte conta o cache junto: 80 mil lidos do cache são baratos, mas
    /// ainda são contexto — e é o contexto que a próxima ida vai reprocessar.
    #[tokio::test]
    async fn o_corte_conta_o_cache_tambem() {
        let mut gordo = pede_ferramentas(&[("a", "inexistente", json!({}))]);
        gordo["usage"] = json!({
            "input_tokens": 1_000,
            "output_tokens": 50,
            "cache_read_input_tokens": 60_000,
        });

        let dublê = Dublê::com(vec![gordo, fim()]);
        let mut config = Config::nova(Modelo::Opus);
        config.max_tokens_turno = 40_000;
        let (agente, _) = agente(dublê.clone(), config);

        let _ = agente.rodar("x").await;
        assert_eq!(dublê.pedidos().len(), 1);
    }

    #[tokio::test]
    async fn uso_acumula_entre_as_idas_e_voltas() {
        let dublê = Dublê::com(vec![
            pede_ferramentas(&[("a", "carro_telemetria", json!({}))]),
            fim(),
        ]);
        let (agente, _) = agente(dublê, Config::nova(Modelo::Haiku));

        let turno = agente.rodar("x").await.unwrap();
        assert_eq!(turno.uso.entrada, 150);
        assert_eq!(turno.uso.saida, 25);
    }

    /// O cache é o que segura o custo dos gatilhos repetidos.
    #[tokio::test]
    async fn o_system_vai_marcado_para_cache() {
        let dublê = Dublê::com(vec![fim()]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Haiku));
        let _ = agente.rodar("x").await;

        let corpo = &dublê.pedidos()[0];
        assert_eq!(corpo["system"][0]["cache_control"]["type"], "ephemeral");
    }

    /// E de nada adianta marcar se o prefixo mudar sozinho entre chamadas.
    #[tokio::test]
    async fn o_prefixo_e_identico_entre_dois_turnos() {
        let dublê = Dublê::com(vec![fim(), fim()]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));
        let _ = agente.rodar("primeiro gatilho").await;
        let _ = agente.rodar("outro gatilho, texto diferente").await;

        let pedidos = dublê.pedidos();
        assert_eq!(
            pedidos[0]["system"], pedidos[1]["system"],
            "o system mudou entre turnos e matou o cache"
        );
        assert_eq!(
            pedidos[0]["tools"], pedidos[1]["tools"],
            "a lista de ferramentas mudou de ordem e matou o cache"
        );
    }

    #[tokio::test]
    async fn haiku_nao_manda_effort_nem_thinking_e_opus_manda() {
        let d1 = Dublê::com(vec![fim()]);
        let (a1, _) = agente(d1.clone(), Config::nova(Modelo::Haiku));
        let _ = a1.rodar("x").await;
        let haiku = &d1.pedidos()[0];
        assert!(haiku.get("output_config").is_none());
        assert!(haiku.get("thinking").is_none());
        assert_eq!(haiku["model"], "claude-haiku-4-5");

        let d2 = Dublê::com(vec![fim()]);
        let (a2, _) = agente(d2.clone(), Config::nova(Modelo::Opus));
        let _ = a2.rodar("x").await;
        let opus = &d2.pedidos()[0];
        assert_eq!(opus["output_config"]["effort"], "medium");
        assert_eq!(opus["thinking"]["type"], "adaptive");
        assert_eq!(opus["model"], "claude-opus-4-8");
    }

    /// `mcp_servers` sem o `mcp_toolset` correspondente é 400.
    #[tokio::test]
    async fn mcp_remoto_leva_servidor_toolset_e_beta_juntos() {
        let dublê = Dublê::com(vec![fim()]);
        let mut config = Config::nova(Modelo::Opus);
        config.mcp_remotos = vec![McpRemoto {
            nome: "clima".into(),
            url: "https://mcp.exemplo/sse".into(),
            token: Some("segredo".into()),
        }];
        let (agente, _) = agente(dublê.clone(), config);
        let _ = agente.rodar("x").await;

        let corpo = &dublê.pedidos()[0];
        assert_eq!(corpo["mcp_servers"][0]["name"], "clima");
        assert_eq!(corpo["mcp_servers"][0]["authorization_token"], "segredo");

        let tem_toolset = corpo["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["type"] == "mcp_toolset" && t["mcp_server_name"] == "clima");
        assert!(tem_toolset, "faltou o mcp_toolset do servidor declarado");

        assert_eq!(dublê.betas.lock().unwrap()[0], vec![BETA_MCP.to_string()]);
    }

    #[tokio::test]
    async fn sem_mcp_remoto_nao_pede_o_beta() {
        let dublê = Dublê::com(vec![fim()]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Haiku));
        let _ = agente.rodar("x").await;

        assert!(dublê.betas.lock().unwrap()[0].is_empty());
    }

    #[tokio::test]
    async fn modo_estrito_desligado_tira_o_strict_das_ferramentas() {
        let dublê = Dublê::com(vec![fim()]);
        let mut config = Config::nova(Modelo::Haiku);
        config.estrito = false;
        let (agente, _) = agente(dublê.clone(), config);
        let _ = agente.rodar("x").await;

        let ferramenta = &dublê.pedidos()[0]["tools"][0];
        assert!(ferramenta.get("strict").is_none());
        assert!(ferramenta.get("input_schema").is_some(), "a API pede input_schema");
    }

    #[tokio::test]
    async fn parada_em_tool_use_sem_chamada_nossa_nao_vira_laco_infinito() {
        let dublê = Dublê::com(vec![json!({
            "stop_reason": "tool_use",
            "usage": {},
            "content": [{ "type": "mcp_tool_use", "id": "m1", "name": "clima", "input": {} }],
        })]);
        let (agente, _) = agente(dublê.clone(), Config::nova(Modelo::Opus));

        let turno = agente.rodar("x").await.unwrap();
        assert_eq!(turno.iteracoes, 1);
        assert_eq!(dublê.pedidos().len(), 1);
    }
}
