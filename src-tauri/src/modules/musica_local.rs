//! A sessão de mídia do próprio aparelho, na frente da Web API.
//!
//! Hoje todo toque de música é uma ida à nuvem do Spotify pela internet do
//! celular preso por tethering. O diário do carro mostra o preço em números:
//!
//! ```text
//! aviso music  o Spotify demorou a responder o toque   {"acao":"playlists","ms":1558}
//! aviso music  ler o que está tocando demora mais que
//!              o esperado                              {"intervalo_ms":3000,"ms":2112}
//! ```
//!
//! Dois segundos para saber o que está tocando, num laço que pergunta a cada
//! três — a leitura não cabe no próprio intervalo. E o app do Spotify está
//! instalado NA CENTRAL, publicando uma `MediaSession` que responde em
//! microssegundos e já traz a capa pronta.
//!
//! # O que este decorador faz, e o que ele não faz
//!
//! Ele **não substitui** a Web API. Divide por natureza:
//!
//! - O que é local e rápido (o que está tocando, a capa, tocar/pausar/pular)
//!   vai pela sessão do aparelho.
//! - O que é biblioteca (busca, playlists, tocar um URI) continua na Web API.
//!
//! A divisão não é preguiça: navegar a biblioteca por `MediaBrowser` depende de
//! o Spotify aceitar o Eclipse como cliente, e essa pergunta ainda não tem
//! resposta — a sonda que ia respondê-la estourava antes de perguntar.
//!
//! # Por que isto é seguro
//!
//! Toda falha da sessão local cai na Web API, que é exatamente o caminho de
//! hoje. Se o Spotify recusar a conexão, o pior caso é o comportamento atual.

use async_trait::async_trait;
use eclipse_music::{Busca, Contexto, MusicError, MusicSource, NowPlaying};
use eclipse_obd::veiculo::Arquivo;
use serde::Deserialize;
use tauri_plugin_obd_bt::ObdBtExt;

/// O que o Kotlin devolve. Ver `SessaoMedia.estado`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct EstadoLocal {
    #[serde(default)]
    conectado: bool,
    #[serde(default)]
    tem: bool,
    #[serde(default)]
    motivo: String,
    #[serde(default)]
    faixa: String,
    #[serde(default)]
    artista: String,
    #[serde(default)]
    tocando: bool,
    #[serde(default)]
    posicao_ms: i64,
    #[serde(default)]
    duracao_ms: i64,
    /// Só vem quando a faixa MUDA — ver `capaEnviadaDe` no Kotlin.
    #[serde(default)]
    capa: Option<String>,
}

pub struct SessaoLocal {
    app: tauri::AppHandle,
    /// Credenciais do App Remote. O plugin não conhece credencial; elas
    /// atravessam a ponte a cada chamada.
    client_id: String,
    redirect_uri: &'static str,
    /// O App Remote já se recusou nesta sessão? Evita insistir numa central
    /// sem Spotify instalado — cada tentativa custa 12 s de espera.
    app_remote_desistiu: bool,
    /// Onde guardar o que for aprendido sobre o dispositivo da central.
    dir: std::path::PathBuf,
    /// A Web API. Continua dona da biblioteca, e é o plano B de tudo.
    nuvem: Box<dyn MusicSource>,
    /// A última capa recebida.
    ///
    /// O Kotlin só manda a capa quando a faixa muda — ela custa uns 30 KB de
    /// base64, e repeti-la a cada segundo seria desperdício. Quem guarda entre
    /// uma leitura e outra é este campo.
    capa: Option<String>,
    /// Por que a sessão local não está sendo usada. Para o diário, uma vez só.
    ja_reclamou: bool,
}

/// Quanto esperar o Spotify aparecer na lista do Connect depois de acordado.
///
/// Não é chute: no diário de 24/09 o app da central apareceu entre 28 e 31
/// segundos depois do bind. Mas a maior parte disso é o intervalo de leitura,
/// não o tempo de acordar — três segundos é o que separa "ainda subindo" de
/// "não vai subir", e esperar mais que isso com o dedo do dono no botão seria
/// pior que falhar.
const ESPERA_ACORDAR_MS: u64 = 3_000;

/// Onde fica guardado qual é o Spotify da central.
///
/// Aprendido uma vez, vale para sempre — inclusive depois de reinstalar o app,
/// porque mora no diretório de dados e não no binário.
const APRENDIDO_JSON: &str = "spotify_da_central.json";

impl SessaoLocal {
    pub fn nova(
        app: tauri::AppHandle,
        dir: std::path::PathBuf,
        client_id: String,
        mut nuvem: Box<dyn MusicSource>,
    ) -> Self {
        // Acorda o Spotify JÁ, na subida do módulo, sem esperar o primeiro
        // toque. É o que evita o fluxo que o dono recusou com razão — ligar o
        // carro, abrir o Spotify na mão, e só então abrir o Eclipse.
        despertar(app.clone());

        // O que já foi aprendido em viagens anteriores vale desde o primeiro
        // toque desta — sem repetir a descoberta.
        if let Some(nome) = aprendido(&dir) {
            tracing::info!(dispositivo = %nome, "já sei qual é o Spotify da central");
            nuvem.fixar_dispositivo(Some(nome));
        }

        Self {
            app,
            dir,
            client_id,
            redirect_uri: eclipse_music::REDIRECT_URI,
            app_remote_desistiu: false,
            nuvem,
            capa: None,
            ja_reclamou: false,
        }
    }

    /// Manda tocar pelo App Remote. `true` = o app da central atendeu.
    ///
    /// É o caminho PREFERIDO, e não um plano B: ele fala com o app do Spotify
    /// deste aparelho, inicia o processo dele sozinho, e não depende de o
    /// dispositivo estar anunciado no Spotify Connect — que é o muro em que as
    /// três tentativas anteriores bateram.
    async fn tocar_no_app_da_central(
        &mut self,
        faixa: Option<&str>,
        contexto: Option<&str>,
    ) -> bool {
        if self.app_remote_desistiu || self.client_id.is_empty() {
            return false;
        }
        let app = self.app.clone();
        let id = self.client_id.clone();
        let redirect = self.redirect_uri;
        let uri = faixa.map(str::to_string);
        let ctx = contexto.map(str::to_string);

        let resultado = tokio::task::spawn_blocking(move || {
            app.obd_bt().app_remote_tocar(
                &id,
                redirect,
                uri.as_deref(),
                ctx.as_deref(),
                // O índice da faixa dentro do contexto ainda não é sabido
                // aqui; `-1` toca o contexto do começo. Melhorar isto exige a
                // posição vir da tela, e tocar o álbum certo já é mais do que
                // havia antes.
                -1,
            )
        })
        .await;

        match resultado {
            Ok(Ok(None)) => {
                tracing::info!("o app do Spotify da central atendeu");
                true
            }
            Ok(Ok(Some(motivo))) => {
                tracing::warn!(%motivo, "o App Remote não atendeu; tentando pela Web API");
                // "não instalado" não melhora tentando de novo; qualquer outra
                // coisa pode ser transitória.
                if motivo.contains("não está instalado") {
                    self.app_remote_desistiu = true;
                }
                false
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, "a ponte do App Remote falhou; tentando pela Web API");
                self.app_remote_desistiu = true;
                false
            }
            Err(err) => {
                tracing::warn!(%err, "a task do App Remote morreu");
                false
            }
        }
    }

    /// Um toque de transporte pelo App Remote. `true` = atendeu.
    async fn transporte_no_app_da_central(&mut self, acao: &'static str, valor: i64) -> bool {
        if self.app_remote_desistiu || self.client_id.is_empty() {
            return false;
        }
        let app = self.app.clone();
        let id = self.client_id.clone();
        let redirect = self.redirect_uri;
        matches!(
            tokio::task::spawn_blocking(move || app
                .obd_bt()
                .app_remote_comando(&id, redirect, acao, valor))
            .await,
            Ok(Ok(None))
        )
    }

    /// Descobre qual dispositivo do Connect é o Spotify DESTA central.
    ///
    /// # Por que observar, e não perguntar
    ///
    /// O nome não serve. O Android chama este aparelho de "K706" em todos os
    /// campos que conhece; o app do Spotify se anuncia como "HT-9960CA". Não há
    /// letra em comum, e não existe API que ligue um ao outro.
    ///
    /// Mas existe um fato que só vale para o app local: **ele aparece na lista
    /// porque NÓS o acordamos**. Ligar no `MediaBrowserService` inicia o
    /// processo do Spotify deste aparelho — e de nenhum outro. O celular do
    /// dono, ou o computador da casa dele, não ligam por causa de um bind aqui.
    ///
    /// Então: fotografa a lista, acorda, fotografa de novo. Quem nasceu no meio
    /// é o daqui. O diário de 24/09 mostra exatamente essa sequência:
    ///
    /// ```text
    /// 17:37:20  candidatos: []                 <- antes
    /// 17:38:11  candidatos: ["HT-9960CA"]      <- depois do despertador
    /// ```
    ///
    /// Computadores ficam de fora mesmo assim: um PC não nasce de um bind
    /// local, e se aparecer no meio é coincidência — e coincidência não pode
    /// mandar som para a casa vazia do dono.
    ///
    /// Aprendido uma vez, fica guardado em disco e vale para sempre.
    async fn aprender_qual_e_o_carro(&mut self) {
        let antes: Vec<String> = self
            .nuvem
            .dispositivos()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(nome, _)| nome)
            .collect();

        Self::acordar_e_esperar(self.app.clone()).await;

        let depois = self.nuvem.dispositivos().await.unwrap_or_default();
        let novos = quem_nasceu_agora(&antes, &depois);

        // Exatamente um. Dois ao mesmo tempo seria coincidência (o celular do
        // dono acordando junto), e na dúvida é melhor não tocar do que tocar
        // no lugar errado — foi assim que o som foi parar no computador dele.
        match novos.as_slice() {
            [unico] => {
                tracing::info!(
                    dispositivo = %unico,
                    "descobri qual é o Spotify da central: ele nasceu quando eu acordei o app daqui"
                );
                guardar_aprendido(&self.dir, unico);
                self.nuvem.fixar_dispositivo(Some(unico.clone()));
            }
            [] => tracing::warn!("acordei o app e nenhum dispositivo novo apareceu"),
            varios => tracing::warn!(
                ?varios,
                "mais de um dispositivo novo apareceu; não dá para saber qual é o do carro"
            ),
        }
    }

    /// Acorda o app do Spotify e espera ele se anunciar.
    ///
    /// Recebe o `AppHandle` em vez de usar o `self` pelo mesmo motivo do
    /// `mandar`: `MusicSource` é `Send` mas não `Sync`, e segurar um `&self`
    /// através do `await` tornaria a future não-`Send`.
    async fn acordar_e_esperar(app: tauri::AppHandle) {
        let _ = tokio::task::spawn_blocking(move || app.obd_bt().sessao_media_estado()).await;
        tokio::time::sleep(std::time::Duration::from_millis(ESPERA_ACORDAR_MS)).await;
    }

    /// Lê a sessão local. `None` = não deu, siga pela nuvem.
    ///
    /// `spawn_blocking` porque `run_mobile_plugin` é bloqueante — é uma chamada
    /// de binder atravessando a ponte do Tauri, e segurar a thread do runtime
    /// com ela travaria os outros módulos.
    async fn ler(&mut self) -> Option<EstadoLocal> {
        let app = self.app.clone();
        let bruto = tokio::task::spawn_blocking(move || app.obd_bt().sessao_media_estado())
            .await
            .ok()?
            .ok()?;

        let estado: EstadoLocal = serde_json::from_str(&bruto).ok()?;
        if !estado.conectado {
            if !self.ja_reclamou {
                self.ja_reclamou = true;
                tracing::info!(
                    motivo = %estado.motivo,
                    "sem sessão local do Spotify; seguindo pela Web API"
                );
            }
            return None;
        }
        // Reconectou depois de ter falhado: vale dizer, senão o diário só
        // registra a queda e nunca a volta.
        if self.ja_reclamou {
            self.ja_reclamou = false;
            tracing::info!("sessão local do Spotify de volta");
        }
        Some(estado)
    }

    /// Manda um toque para a sessão local. `false` = não atendeu.
    ///
    /// Recebe o `AppHandle` em vez de usar o `self` de propósito: `MusicSource`
    /// é `Send` mas não `Sync`, então segurar um `&self` através do `await`
    /// tornaria esta future não-`Send` — e o trait exige que ela seja. Sem
    /// empréstimo do `self`, o problema não existe.
    async fn mandar(app: tauri::AppHandle, acao: &'static str, valor: i64) -> bool {
        tokio::task::spawn_blocking(move || app.obd_bt().sessao_media_comando(acao, valor))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(false)
    }
}

/// Converte a leitura crua em [`NowPlaying`], lembrando a capa.
///
/// Fora do método de propósito: é a única lógica aqui que não depende do
/// Android, e é a que quebra calada. Se o cache da capa falhar, a capa aparece
/// por um segundo e SOME — e some sem erro nenhum, porque tecnicamente tudo
/// funcionou. Isolada, dá para testá-la de verdade.
fn montar(estado: EstadoLocal, guardada: &mut Option<String>) -> Option<NowPlaying> {
    // Conectado e sem faixa é uma resposta de verdade — o Spotify está aberto e
    // parado. Perguntar à nuvem não mudaria nada e custaria a ida.
    if !estado.tem {
        return None;
    }

    // O Kotlin só manda a capa quando a FAIXA muda (ela custa uns 30 KB de
    // base64, e repeti-la a cada segundo seria desperdício). Então "veio sem
    // capa" quer dizer "a mesma de antes", e não "não tem capa".
    if let Some(capa) = estado.capa {
        *guardada = Some(capa);
    }

    Some(NowPlaying {
        track: estado.faixa,
        artist: estado.artista,
        is_playing: estado.tocando,
        album_art: guardada.clone(),
        // `max(0)` + `try_from`: a posição vem do relógio do Android somado à
        // velocidade de reprodução, e um valor negativo ou absurdo não deve
        // virar barra de progresso maluca — vira ausência.
        progress_ms: u32::try_from(estado.posicao_ms.max(0)).ok(),
        duration_ms: u32::try_from(estado.duracao_ms.max(0))
            .ok()
            .filter(|d| *d > 0),
    })
}

/// Quem apareceu na lista entre as duas fotografias, ignorando computadores.
///
/// Isolada do resto porque é a regra que decide de onde sai o som, e errar
/// aqui não dá erro nenhum: dá música tocando na casa vazia do dono.
fn quem_nasceu_agora(antes: &[String], depois: &[(String, bool)]) -> Vec<String> {
    depois
        .iter()
        // Um computador não nasce de um bind local. Se apareceu no meio, é
        // coincidência — e coincidência não pode mandar som para outra casa.
        .filter(|(_, e_computador)| !e_computador)
        .map(|(nome, _)| nome.clone())
        .filter(|nome| !antes.iter().any(|a| a.eq_ignore_ascii_case(nome)))
        .collect()
}

/// O nome do Spotify da central, se já tiver sido descoberto.
fn aprendido(dir: &std::path::Path) -> Option<String> {
    Arquivo::<Option<String>>::load(dir.join(APRENDIDO_JSON)).dados
}

/// Guarda o que foi descoberto, para nunca mais precisar descobrir.
fn guardar_aprendido(dir: &std::path::Path, nome: &str) {
    let mut arq = Arquivo::<Option<String>>::load(dir.join(APRENDIDO_JSON));
    arq.dados = Some(nome.to_string());
    if let Err(err) = arq.salvar() {
        tracing::warn!(%err, "não consegui guardar qual é o Spotify da central");
    }
}

/// Cutuca o app do Spotify para ele existir.
///
/// # Por que isto acorda o Spotify
///
/// Ligar no `MediaBrowserService` dele é um `bindService`, e o Android inicia o
/// serviço — logo, o PROCESSO do app — para atender. O Spotify então recusa a
/// navegação (ele só libera para Android Auto e afins), mas a essa altura já
/// acordou: o app vivo se anuncia sozinho como dispositivo do Spotify Connect.
///
/// Não é teoria. O diário de 24/09 mostra a sequência:
///
/// ```text
/// 01:43:30  candidatos: ["DESKTOP-ATVE3IV"]                  <- só o PC
/// 01:43:33  musica_local: "o Spotify recusou a conexão"      <- o bind aconteceu
/// 01:44:01  candidatos: ["HT-9960CA", "DESKTOP-ATVE3IV"]     <- a central apareceu
/// ```
///
/// A recusa que parecia o fim da linha é, na prática, o despertador.
///
/// Sem barulho e sem esperar: se falhar, o pior caso é o de hoje.
fn despertar(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || app.obd_bt().sessao_media_estado()).await;
    });
}

#[async_trait]
impl MusicSource for SessaoLocal {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
        let Some(estado) = self.ler().await else {
            return self.nuvem.now_playing().await;
        };

        Ok(montar(estado, &mut self.capa))
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        if self.transporte_no_app_da_central("alternar", 0).await {
            return Ok(());
        }
        if Self::mandar(self.app.clone(), "alternar", 0).await {
            return Ok(());
        }
        self.nuvem.toggle().await
    }

    async fn next(&mut self) -> Result<(), MusicError> {
        if self.transporte_no_app_da_central("proxima", 0).await {
            return Ok(());
        }
        if Self::mandar(self.app.clone(), "proxima", 0).await {
            return Ok(());
        }
        self.nuvem.next().await
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        if self.transporte_no_app_da_central("anterior", 0).await {
            return Ok(());
        }
        if Self::mandar(self.app.clone(), "anterior", 0).await {
            return Ok(());
        }
        self.nuvem.previous().await
    }

    async fn seek(&mut self, posicao_ms: u32) -> Result<(), MusicError> {
        if self
            .transporte_no_app_da_central("saltar", i64::from(posicao_ms))
            .await
        {
            return Ok(());
        }
        if Self::mandar(self.app.clone(), "saltar", i64::from(posicao_ms)).await {
            return Ok(());
        }
        self.nuvem.seek(posicao_ms).await
    }

    // A biblioteca inteira segue na Web API: a sessão local sabe controlar o
    // que já está tocando, não sabe procurar nem montar fila.
    async fn buscar(&mut self, termo: &str) -> Result<Busca, MusicError> {
        self.nuvem.buscar(termo).await
    }

    async fn abrir(&mut self, uri: &str) -> Result<Contexto, MusicError> {
        self.nuvem.abrir(uri).await
    }

    async fn tocar(
        &mut self,
        faixa: Option<&str>,
        contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        // O App Remote PRIMEIRO. Ele fala com o app do Spotify deste aparelho e
        // inicia o processo dele sozinho — não depende de o dispositivo estar
        // anunciado no Spotify Connect, que é o muro em que as três tentativas
        // anteriores bateram.
        if self.tocar_no_app_da_central(faixa, contexto).await {
            return Ok(());
        }

        match self.nuvem.tocar(faixa, contexto).await {
            // Sem App Remote e sem dispositivo: ainda vale acordar pelo
            // caminho antigo e tentar mais uma vez. Custa pouco e às vezes
            // pega — mas não é mais a aposta principal.
            Err(MusicError::SoDispositivoDeFora) | Err(MusicError::NoActiveDevice) => {
                tracing::info!("nenhum Spotify no carro; acordando o app e descobrindo qual é");
                self.aprender_qual_e_o_carro().await;
                self.nuvem.tocar(faixa, contexto).await
            }
            outro => outro,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tocando(faixa: &str, capa: Option<&str>) -> EstadoLocal {
        EstadoLocal {
            conectado: true,
            tem: true,
            faixa: faixa.into(),
            artista: "Alguém".into(),
            tocando: true,
            posicao_ms: 1_000,
            duracao_ms: 200_000,
            capa: capa.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn a_capa_sobrevive_as_leituras_seguintes() {
        // O caso que quebraria calado: o Kotlin manda a capa UMA vez, quando a
        // faixa muda. Se ela não fosse guardada, apareceria por um segundo e
        // sumiria — sem erro nenhum, porque tecnicamente tudo funcionou.
        let mut capa = None;

        let primeira = montar(
            tocando("Faixa A", Some("data:image/jpeg;base64,AAA")),
            &mut capa,
        )
        .expect("tocando");
        assert_eq!(
            primeira.album_art.as_deref(),
            Some("data:image/jpeg;base64,AAA")
        );

        // Segunda leitura da MESMA faixa: vem sem capa, e mesmo assim a capa
        // tem que continuar na tela.
        let segunda = montar(tocando("Faixa A", None), &mut capa).expect("tocando");
        assert_eq!(
            segunda.album_art.as_deref(),
            Some("data:image/jpeg;base64,AAA"),
            "vir sem capa quer dizer 'a mesma de antes', não 'não tem capa'"
        );
    }

    #[test]
    fn trocar_de_faixa_troca_a_capa() {
        let mut capa = None;
        montar(tocando("Faixa A", Some("capa-A")), &mut capa);
        let nova = montar(tocando("Faixa B", Some("capa-B")), &mut capa).expect("tocando");
        assert_eq!(nova.album_art.as_deref(), Some("capa-B"));
    }

    #[test]
    fn parado_nao_e_erro_e_nao_inventa_faixa() {
        // Spotify aberto e parado: `tem: false`. Isso é resposta, não falha —
        // e não pode virar uma faixa vazia na tela.
        let mut capa = Some("capa-velha".to_string());
        let parado = EstadoLocal {
            conectado: true,
            tem: false,
            ..Default::default()
        };
        assert!(montar(parado, &mut capa).is_none());
    }

    #[test]
    fn duracao_zero_vira_ausencia_e_nao_barra_vazia() {
        // Nem toda sessão informa duração. Zero significa "não sei", e como
        // número viraria uma barra de progresso que nunca anda.
        let mut capa = None;
        let sem_duracao = EstadoLocal {
            duracao_ms: 0,
            ..tocando("Faixa", None)
        };
        assert_eq!(montar(sem_duracao, &mut capa).unwrap().duration_ms, None);
    }

    #[test]
    fn posicao_negativa_nao_vira_numero_gigante() {
        // A posição é calculada somando o relógio do Android; se vier negativa,
        // um `as u32` daria 4 bilhões de milissegundos de progresso.
        let mut capa = None;
        let torta = EstadoLocal {
            posicao_ms: -5,
            ..tocando("Faixa", None)
        };
        assert_eq!(montar(torta, &mut capa).unwrap().progress_ms, Some(0));
    }
}

#[cfg(test)]
mod tests_aprender {
    use super::*;

    fn lista(v: &[(&str, bool)]) -> Vec<(String, bool)> {
        v.iter().map(|(n, c)| (n.to_string(), *c)).collect()
    }

    fn nomes(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn o_caso_real_do_carro() {
        // Diário de 24/09, a sequência exata:
        //   17:37:20  candidatos: []
        //   17:38:11  candidatos: ["HT-9960CA"]
        // O app da central nasceu porque o Eclipse o acordou.
        let novos = quem_nasceu_agora(&[], &lista(&[("HT-9960CA", false)]));
        assert_eq!(novos, vec!["HT-9960CA"]);
    }

    #[test]
    fn o_computador_de_casa_nao_e_aprendido_nem_se_aparecer_agora() {
        // Um PC não nasce de um bind local. Se apareceu no meio, foi o dono
        // abrindo o Spotify em casa — e aprender isso mandaria TODA música
        // futura para lá.
        let novos = quem_nasceu_agora(&[], &lista(&[("DESKTOP-ATVE3IV", true)]));
        assert!(novos.is_empty());
    }

    #[test]
    fn quem_ja_estava_na_lista_nao_conta() {
        // O celular do dono já estava ligado antes do despertador; ele não
        // nasceu de nada que o Eclipse fez.
        let antes = nomes(&["iPhone do Vinicius"]);
        let depois = lista(&[("iPhone do Vinicius", false), ("HT-9960CA", false)]);
        assert_eq!(quem_nasceu_agora(&antes, &depois), vec!["HT-9960CA"]);
    }

    #[test]
    fn dois_nascendo_juntos_nao_ensinam_nada() {
        // Ambiguidade não vira palpite. Melhor não tocar do que tocar longe.
        let novos = quem_nasceu_agora(&[], &lista(&[("HT-9960CA", false), ("Outro", false)]));
        assert_eq!(novos.len(), 2, "quem chama precisa ver a ambiguidade");
    }

    #[test]
    fn a_comparacao_ignora_maiuscula() {
        let antes = nomes(&["ht-9960ca"]);
        let depois = lista(&[("HT-9960CA", false)]);
        assert!(
            quem_nasceu_agora(&antes, &depois).is_empty(),
            "é o mesmo aparelho escrito diferente, não um recém-nascido"
        );
    }
}
