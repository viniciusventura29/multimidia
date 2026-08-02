//! O shell do Eclipse OS.
//!
//! Não tem regra de negócio aqui: este crate liga o supervisor do `eclipse-core`
//! na janela do Tauri. O Rust é dono do estado; a UI é uma projeção dele.

mod assistente;
mod modules;
mod obd_bt;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eclipse_core::{
    factory, ModuleCommand, ModuleId, Profile, ProfileStore, StateEnvelope, Supervisor,
};
use eclipse_music::TokenStore;
use serde_json::Value;
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

/// Evento que carrega mudança de estado de módulo até a UI.
const EVENT_MODULE_STATE: &str = "module-state";
/// Evento disparado quando o perfil ativo muda.
const EVENT_PROFILE: &str = "profile-changed";

/// Os perfis em disco, atrás de um mutex porque comandos chegam de várias threads.
struct Perfis(Mutex<ProfileStore>);

impl Perfis {
    fn lock(&self) -> std::sync::MutexGuard<'_, ProfileStore> {
        // Um mutex envenenado não pode derrubar o painel: o pior caso é um perfil
        // meio gravado, e o `load` já sabe lidar com arquivo quebrado.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/* ------------------------------------------------------------------ */
/* Módulos                                                             */
/* ------------------------------------------------------------------ */

/// O estado atual de todos os módulos.
///
/// Quem assina o barramento não recebe o que já passou, então a UI chama isto
/// ao montar para pintar a tela antes do primeiro evento chegar.
#[tauri::command]
fn get_snapshot(supervisor: tauri::State<'_, Supervisor>) -> Vec<StateEnvelope> {
    supervisor.snapshot()
}

/// Entrega uma ação da UI ao módulo dono dela.
///
/// Não devolve resultado de propósito: quem responde é o próximo estado que o
/// módulo publicar. A tela nunca inventa o efeito de um toque — ela espera o Rust.
#[tauri::command]
fn dispatch_action(supervisor: tauri::State<'_, Supervisor>, module: String, payload: Value) {
    supervisor.dispatch(ModuleCommand::Action {
        target: ModuleId(module.into()),
        payload,
    });
}

/* ------------------------------------------------------------------ */
/* Perfis                                                              */
/* ------------------------------------------------------------------ */

#[tauri::command]
fn list_profiles(perfis: tauri::State<'_, Perfis>) -> Vec<Profile> {
    perfis.lock().profiles().to_vec()
}

#[tauri::command]
fn active_profile(perfis: tauri::State<'_, Perfis>) -> Option<Profile> {
    perfis.lock().active().cloned()
}

#[tauri::command]
fn create_profile(
    app: tauri::AppHandle,
    perfis: tauri::State<'_, Perfis>,
    supervisor: tauri::State<'_, Supervisor>,
    name: String,
    color: String,
) -> Result<Profile, String> {
    let (profile, virou_ativo) = {
        let mut store = perfis.lock();
        let profile = store.create(name, color).map_err(|e| e.to_string())?;
        let virou_ativo = store.active().map(|p| p.id) == Some(profile.id);
        (profile, virou_ativo)
    };

    if virou_ativo {
        anunciar_perfil(&app, &supervisor, Some(&profile));
    }
    Ok(profile)
}

#[tauri::command]
fn select_profile(
    app: tauri::AppHandle,
    perfis: tauri::State<'_, Perfis>,
    supervisor: tauri::State<'_, Supervisor>,
    id: Uuid,
) -> Result<Profile, String> {
    let profile = perfis.lock().select(id).map_err(|e| e.to_string())?;
    anunciar_perfil(&app, &supervisor, Some(&profile));
    Ok(profile)
}

#[tauri::command]
fn delete_profile(
    app: tauri::AppHandle,
    perfis: tauri::State<'_, Perfis>,
    supervisor: tauri::State<'_, Supervisor>,
    id: Uuid,
) -> Result<Option<Profile>, String> {
    let ativo = {
        let mut store = perfis.lock();
        store.remove(id).map_err(|e| e.to_string())?;
        store.active().cloned()
    };

    anunciar_perfil(&app, &supervisor, ativo.as_ref());
    Ok(ativo)
}

/// Conta a troca de perfil para os dois lados: módulos e tela.
///
/// Os módulos recebem via barramento — o de música derruba a sessão e reconecta
/// com o token do novo perfil. A UI recebe via evento, para repintar o tema.
fn anunciar_perfil(app: &tauri::AppHandle, supervisor: &Supervisor, profile: Option<&Profile>) {
    if let Some(profile) = profile {
        supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(profile.clone())));
    }
    if let Err(err) = app.emit(EVENT_PROFILE, profile) {
        tracing::warn!(%err, "não consegui avisar a UI sobre o perfil");
    }
}

/* ------------------------------------------------------------------ */
/* Spotify                                                             */
/* ------------------------------------------------------------------ */

/// O cofre de refresh tokens, compartilhado com o módulo de música.
struct Cofre(Arc<Mutex<TokenStore>>);

/// Login do Spotify em curso: o cliente PKCE (com o `code_verifier`) e o perfil
/// que iniciou, esperando o `code` chegar pelo deep link. Só existe no mobile,
/// onde o callback vem por `eclipseos://callback` em vez de servidor loopback.
#[cfg(mobile)]
struct SpotifyPendingAuth(Mutex<Option<(Uuid, eclipse_music::spotify::PkcePendente)>>);

/// Lê uma credencial.
///
/// Variável de ambiente para desenvolver, arquivo para a head unit — lá não há
/// shell para exportar nada antes de o launcher subir.
fn credencial(dir_dados: &std::path::Path, env: &str, arquivo: &str) -> Option<String> {
    std::env::var(env)
        .ok()
        .or_else(|| std::fs::read_to_string(dir_dados.join(arquivo)).ok())
        .map(|valor| valor.trim().to_string())
        .filter(|valor| !valor.is_empty())
}

fn client_id(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_SPOTIFY_CLIENT_ID", "spotify_client_id.txt")
        .or_else(|| embutida(option_env!("ECLIPSE_SPOTIFY_CLIENT_ID")))
}

/// Fallback embutido em tempo de compilação, para builds de teste em aparelho
/// físico: sem root não há como criar o arquivo no diretório de dados, e não há
/// shell para exportar variável. A chave do Maps já é pública por natureza (vai
/// ao WebView de qualquer jeito — ver `modules/nav.rs`); a proteção é a cota.
fn embutida(valor: Option<&'static str>) -> Option<String> {
    valor
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn maps_api_key(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_MAPS_API_KEY", "maps_api_key.txt")
        .or_else(|| embutida(option_env!("ECLIPSE_MAPS_API_KEY")))
}

/// Sem Map ID o mapa é raster, e raster ignora inclinação e rotação.
fn maps_map_id(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_MAPS_MAP_ID", "maps_map_id.txt")
        .or_else(|| embutida(option_env!("ECLIPSE_MAPS_MAP_ID")))
}

/// A chave da API da Anthropic, que move o assistente.
///
/// ⚠️ Diferente da chave do Maps, esta **não** é pública por natureza: quem a
/// tiver gasta dinheiro em nome do dono. O fallback embutido existe porque a
/// head unit não tem root para criar o arquivo nem shell para exportar variável —
/// mas ele grava a chave no APK. Só compile com `anthropic_api_key.txt` presente
/// se o APK for ficar com você. A defesa real é o teto de gasto no console da
/// Anthropic, não o segredo.
fn anthropic_api_key(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_ANTHROPIC_API_KEY", "anthropic_api_key.txt")
        .or_else(|| embutida(option_env!("ECLIPSE_ANTHROPIC_API_KEY")))
}

/// A chave do OpenRouter, usada só para gerar imagem — a Anthropic não gera.
///
/// Vale a mesma ressalva de `anthropic_api_key`: é chave que gasta.
fn openrouter_api_key(dir_dados: &std::path::Path) -> Option<String> {
    credencial(
        dir_dados,
        "ECLIPSE_OPENROUTER_API_KEY",
        "openrouter_api_key.txt",
    )
    .or_else(|| embutida(option_env!("ECLIPSE_OPENROUTER_API_KEY")))
}

/// Um access token fresco do Spotify, para o Web Playback SDK.
///
/// É o que permite o Eclipse **tocar o áudio ele mesmo**, dentro do WebView, em
/// vez de comandar o app oficial do Spotify no aparelho. O token é curto (~1h) e
/// o SDK pede outro pelo callback dele quando vence — por isso um comando, e não
/// um valor no estado do módulo.
#[tauri::command]
async fn spotify_access_token(app: tauri::AppHandle, id: Uuid) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("sem diretório de dados: {e}"))?;
    let client_id = client_id(&dir).ok_or("falta o Client ID do Spotify")?;
    let cofre = app.state::<Cofre>().0.clone();

    let fonte = eclipse_music::SpotifySource::conectar(&client_id, id, cofre)
        .await
        .map_err(|e| e.to_string())?;
    fonte.access_token().await.map_err(|e| e.to_string())
}

/// Conecta o Spotify de um perfil: abre o navegador, espera o redirect de volta
/// e guarda o refresh token.
#[tauri::command]
async fn connect_spotify(
    app: tauri::AppHandle,
    id: Uuid,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("sem diretório de dados: {e}"))?;

    let client_id = client_id(&dir).ok_or_else(|| {
        "falta o Client ID: defina ECLIPSE_SPOTIFY_CLIENT_ID ou crie spotify_client_id.txt \
         no diretório de dados do app"
            .to_string()
    })?;

    let (pendente, url) =
        eclipse_music::spotify::iniciar_autorizacao(&client_id).map_err(|e| e.to_string())?;

    // `app.opener().open_url` (método do manager), NÃO a função livre
    // `tauri_plugin_opener::open_url`: a função livre cai no crate `open`, que
    // tenta *exec* de um helper (estilo xdg-open) — o Android nega com EACCES
    // ("Permission denied, os error 13") e o navegador nem abre. O método do
    // manager tem o branch `#[cfg(mobile)]` que dispara um ACTION_VIEW nativo.

    // No mobile o `code` volta por deep link (`eclipseos://callback`), não por
    // servidor loopback (o Android congela o app em segundo plano e o servidor
    // trava). Guarda o cliente PKCE — que carrega o `code_verifier` — e deixa o
    // handler `on_open_url` concluir quando o sistema entregar o deep link.
    #[cfg(mobile)]
    {
        *app.state::<SpotifyPendingAuth>()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((id, pendente));
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // No desktop o loopback funciona: abre o navegador e espera o redirect.
    #[cfg(not(mobile))]
    {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())?;
        let autorizacao = eclipse_music::spotify::concluir_autorizacao(pendente)
            .await
            .map_err(|e| e.to_string())?;
        finalizar_spotify(&app, id, autorizacao)?;
        Ok(())
    }
}

/// Guarda o refresh token no cofre e faz o módulo reconectar na hora.
///
/// Compartilhado pelos dois caminhos de login: o loopback (desktop, inline) e o
/// deep link (mobile, pelo `on_open_url`).
fn finalizar_spotify(
    app: &tauri::AppHandle,
    id: Uuid,
    autorizacao: eclipse_music::spotify::Autorizacao,
) -> Result<(), String> {
    {
        let cofre = app.state::<Cofre>();
        let mut cofre = cofre.0.lock().unwrap_or_else(|e| e.into_inner());
        cofre
            .autorizou(id, autorizacao.refresh_token, autorizacao.quando)
            .map_err(|e| e.to_string())?;
    }

    // O módulo só descobre que agora há token quando tentar de novo; avisar via
    // troca de perfil faz ele reconectar na hora.
    let perfis = app.state::<Perfis>();
    let ativo = perfis.lock().active().cloned();
    if let Some(profile) = ativo.filter(|p| p.id == id) {
        app.state::<Supervisor>()
            .dispatch(ModuleCommand::ProfileChanged(Arc::new(profile)));
    }

    Ok(())
}

/* ------------------------------------------------------------------ */
/* Assistente                                                          */
/* ------------------------------------------------------------------ */

/// Aceita um nome de arquivo, e só isso.
///
/// O nome chega até aqui vindo do **modelo**, que é entrada não confiável por
/// definição: ele inventa o conteúdo do cartão, e o cartão carrega o nome. Um
/// `../../` aí dentro leria qualquer arquivo do diretório do app — inclusive
/// `spotify_tokens.json` e as chaves de API — e devolveria os bytes para o
/// WebView.
///
/// A checagem é comparar o nome com o próprio `file_name()`: se sobrou barra ou
/// `..`, os dois diferem.
fn nome_de_imagem_seguro(nome: &str) -> Option<&str> {
    std::path::Path::new(nome)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| *n == nome)
}

/// Os bytes de uma imagem que o assistente baixou ou gerou.
///
/// Um comando, e não o protocolo de asset do Tauri: assim funciona igual no Mac
/// e no Android sem mexer em capability nenhuma. A tela monta um object URL com
/// o que vier.
///
/// `async` + `spawn_blocking` para o disco não segurar o thread de comandos, e
/// `ipc::Response` para os bytes atravessarem a IPC crus — devolver `Vec<u8>`
/// os serializava como um array JSON com um número por byte.
#[tauri::command]
async fn imagem_ia(
    app: tauri::AppHandle,
    nome: String,
) -> Result<tauri::ipc::Response, String> {
    let seguro = nome_de_imagem_seguro(&nome)
        .ok_or("nome de imagem inválido")?
        .to_owned();

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("sem diretório de dados: {e}"))?;

    let caminho = dir.join("ia_imagens").join(seguro);
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(caminho))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
mod tests_imagem {
    use super::nome_de_imagem_seguro;

    #[test]
    fn nome_simples_passa() {
        assert_eq!(nome_de_imagem_seguro("abc.png"), Some("abc.png"));
        assert_eq!(nome_de_imagem_seguro("demonstracao.svg"), Some("demonstracao.svg"));
    }

    /// O diretório de dados guarda `spotify_tokens.json` e as chaves de API. Um
    /// nome com caminho não pode chegar perto deles.
    #[test]
    fn caminho_e_travessia_sao_recusados() {
        for tentativa in [
            "../spotify_tokens.json",
            "../../anthropic_api_key.txt",
            "sub/abc.png",
            "/etc/passwd",
            "..",
            ".",
            "",
            "./abc.png",
        ] {
            assert_eq!(
                nome_de_imagem_seguro(tentativa),
                None,
                "`{tentativa}` passou pelo filtro"
            );
        }
    }
}

/* ------------------------------------------------------------------ */
/* Localização                                                         */
/* ------------------------------------------------------------------ */

/// O lado do canal que os comandos abaixo usam para repassar ao módulo `nav`
/// o que o `navigator.geolocation` do navegador relatou.
struct Localizacao(eclipse_gps::Emissor);

/// Uma posição nova, vinda do `navigator.geolocation.watchPosition` do JS.
///
/// O Rust não tem como chamar essa API sozinho — só o navegador fala com o
/// sistema operacional para isso. Por isso a posição entra pelo caminho
/// inverso dos outros sensores: o JS empurra, o módulo só escuta.
#[tauri::command]
fn push_location(
    canal: tauri::State<'_, Localizacao>,
    lat: f64,
    lon: f64,
    heading: f32,
    speed_kmh: f32,
    accuracy_m: f32,
) {
    let _ = canal.0.send(Ok(eclipse_gps::Fix {
        lat,
        lon,
        heading,
        speed_kmh,
        accuracy_m,
    }));
}

/// O navegador não conseguiu (ou não quis) dar a localização.
///
/// Chega pelo mesmo canal que as posições boas, como erro em vez de sucesso —
/// é o que faz o módulo `nav` degradar com um motivo certo, em vez de ficar
/// esperando para sempre uma posição que não vem.
#[tauri::command]
fn push_location_error(canal: tauri::State<'_, Localizacao>, permissao_negada: bool) {
    let erro = if permissao_negada {
        eclipse_gps::GpsError::SemPermissao
    } else {
        eclipse_gps::GpsError::SemSinal
    };
    let _ = canal.0.send(Err(erro));
}

/* ------------------------------------------------------------------ */
/* Fiação                                                              */
/* ------------------------------------------------------------------ */

/// Repassa cada mudança de estado para a janela, conflando rajadas.
///
/// Cada emit é uma serialização JSON e uma travessia de IPC até o WebView, e o
/// OBD publica a cada leitura de PID. A janela de 250 ms por módulo deixa o
/// primeiro evento passar na hora (latência zero percebida) e, dentro da
/// janela, guarda só o mais novo para emitir no tick seguinte — teto de
/// 4 emits/s por módulo. Descartar intermediários é seguro por construção:
/// cada envelope carrega o estado **inteiro** do módulo.
fn forward_states(app: tauri::AppHandle, supervisor: &Supervisor) {
    const JANELA: Duration = Duration::from_millis(250);

    let mut rx = supervisor.subscribe();
    tokio::spawn(async move {
        let mut ultimo_emit: HashMap<ModuleId, Instant> = HashMap::new();
        let mut pendentes: HashMap<ModuleId, StateEnvelope> = HashMap::new();
        let mut tick = tokio::time::interval(JANELA);

        let emitir = |envelope: &StateEnvelope| {
            if let Err(err) = app.emit(EVENT_MODULE_STATE, envelope) {
                tracing::warn!(%err, "não consegui emitir estado para a UI");
            }
        };

        loop {
            tokio::select! {
                recebido = rx.recv() => match recebido {
                    Ok(envelope) => {
                        let agora = Instant::now();
                        let na_janela = ultimo_emit
                            .get(&envelope.module)
                            .is_some_and(|t| agora.duration_since(*t) < JANELA);
                        if na_janela {
                            // O mais novo ganha; o intermediário nunca fez falta.
                            pendentes.insert(envelope.module.clone(), envelope);
                        } else {
                            ultimo_emit.insert(envelope.module.clone(), agora);
                            emitir(&envelope);
                        }
                    }
                    // A UI ficou pra trás. Ela se corrige sozinha no próximo
                    // evento, porque cada envelope carrega o estado inteiro.
                    Err(RecvError::Lagged(perdidos)) => {
                        tracing::warn!(perdidos, "UI ficou pra trás no barramento");
                    }
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    for (module, envelope) in pendentes.drain() {
                        ultimo_emit.insert(module, Instant::now());
                        emitir(&envelope);
                    }
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Liga o `tracing`.
///
/// Sem isto, **todo** `tracing::info!`/`warn!`/`error!` do Eclipse cai no vazio —
/// os do supervisor quando um módulo entra em pânico, os do OBD quando o
/// adaptador solta, e os do assistente com o consumo de token de cada turno.
/// O `tracing` só escreve em algum lugar depois que alguém instala um
/// subscriber, e ninguém instalava.
///
/// O padrão mostra os nossos crates em `info` e cala o barulho das
/// dependências. `RUST_LOG` sobrepõe quando se quer investigar algo:
/// `RUST_LOG=debug npm run tauri dev`.
fn ligar_log() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filtro = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,eclipse_os_lib=info,eclipse_ia=info,eclipse_core=info,eclipse_music=info,eclipse_obd=info,eclipse_gps=info"));

    // `try_init` e não `init`: entrar em pânico por causa do log seria pior que
    // ficar sem log — e num teste que chame `run()` duas vezes o segundo falha.
    let _ = fmt().with_env_filter(filtro).with_target(true).try_init();
}

pub fn run() {
    ligar_log();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_obd_bt::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            dispatch_action,
            list_profiles,
            active_profile,
            create_profile,
            select_profile,
            delete_profile,
            connect_spotify,
            spotify_access_token,
            push_location,
            push_location_error,
            imagem_ia,
        ])
        .setup(|app| {
            let dir = app
                .path()
                .app_data_dir()
                .expect("sem diretório de dados do app");

            let store = ProfileStore::load(dir.join("profiles.json"));
            let ativo = store.active().cloned();

            let handle = app.handle().clone();
            // O módulo OBD precisa do handle para alcançar o plugin de Bluetooth,
            // e o supervisor o reconstrói a cada reconexão — daí um clone à parte.
            let handle_obd = app.handle().clone();
            let dir_obd = dir.clone();
            // O assistente lê o painel inteiro e grava imagens no diretório de
            // dados, então precisa do handle pelos mesmos motivos.
            let handle_ia = app.handle().clone();

            // O cofre de tokens continua existindo nas duas plataformas — o
            // comando `connect_spotify` e a Web API seguem úteis mesmo no
            // Android para busca e playlists, ainda que a reprodução em si não
            // passe mais por eles.
            let cofre = Arc::new(Mutex::new(TokenStore::load(dir.join("spotify_tokens.json"))));

            // Spotify pela Web API nas DUAS plataformas: é o que dá busca,
            // playlists e "escolher a música dentro do Eclipse" sem abrir o app
            // do Spotify. No Android o app oficial do Spotify serve só de device
            // (Spotify Connect) em segundo plano — a UI é toda aqui. Isso
            // abandona o caminho da sessão de mídia (`AndroidConector`), que só
            // controlava o que já estivesse tocando e não sabia iniciar nada.
            let conector: Arc<dyn modules::music::Conector> =
                Arc::new(modules::music::SpotifyConector {
                    client_id: client_id(&dir),
                    cofre: Arc::clone(&cofre),
                    demo: std::env::var("ECLIPSE_MUSIC_DEMO").is_ok_and(|v| v == "1"),
                });

            let chave_mapa = maps_api_key(&dir);
            let id_mapa = maps_map_id(&dir);

            // Criado uma vez só, fora da closure do `factory`: um
            // `watch::Receiver` clona, e é isso que permite o supervisor
            // reconstruir o módulo depois de um pânico sem perder a última
            // posição conhecida — um canal `mpsc` não teria como existir de
            // novo depois de reconstruído.
            let (emissor_local, receptor_local) = eclipse_gps::PushedLocation::canal();

            // `block_on` serve só para entrar no runtime do Tauri, para que os
            // `tokio::spawn` lá dentro tenham contexto; as tasks seguem vivas depois.
            let supervisor = tauri::async_runtime::block_on(async move {
                let mut supervisor = Supervisor::new();
                forward_states(handle, &supervisor);

                supervisor.spawn(factory(modules::obd::OBD, move || {
                    // O diretório vai junto porque o tanque e a configuração do carro
                    // moram em disco: o supervisor reconstrói este módulo a cada
                    // reconexão do adaptador, e cada tentativa relê o que já se sabia.
                    modules::obd::ObdModule::new(handle_obd.clone(), dir_obd.clone())
                }));
                supervisor.spawn(factory(modules::music::MUSIC, move || {
                    modules::music::MusicModule::new(Arc::clone(&conector))
                }));
                supervisor.spawn(factory(
                    modules::messaging::MESSAGING,
                    modules::messaging::MessagingModule::default,
                ));
                supervisor.spawn(factory(modules::nav::NAV, move || {
                    modules::nav::NavModule::new(
                        chave_mapa.clone(),
                        id_mapa.clone(),
                        Box::new(eclipse_gps::PushedLocation::nova(receptor_local.clone())),
                    )
                }));
                supervisor.spawn(factory(modules::assistente::ASSISTENTE, move || {
                    modules::assistente::AssistenteModule::new(handle_ia.clone())
                }));

                supervisor
            });

            // Os módulos precisam saber quem está dirigindo antes do primeiro
            // toque na tela, senão a música subiria com a conta do perfil errado.
            if let Some(profile) = &ativo {
                supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(profile.clone())));
            }

            app.manage(supervisor);
            app.manage(Localizacao(emissor_local));
            app.manage(Perfis(Mutex::new(store)));
            app.manage(Cofre(cofre));

            // Login do Spotify no mobile: o `code` chega por deep link
            // (`eclipseos://callback?code=...`). O `connect_spotify` guardou o
            // cliente PKCE aqui; ao chegar o deep link, troca o code pelo token,
            // finaliza no cofre e faz o módulo reconectar.
            #[cfg(mobile)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.manage(SpotifyPendingAuth(Mutex::new(None)));
                let alvo = app.handle().clone();
                app.deep_link().on_open_url(move |evento| {
                    for url in evento.urls() {
                        println!("[eclipse] deep link recebido: {}", url.as_str());
                        let Some(codigo) =
                            eclipse_music::spotify::codigo_de_url(url.as_str())
                        else {
                            println!("[eclipse] deep link sem code, ignorando");
                            continue;
                        };
                        let pendente = alvo
                            .state::<SpotifyPendingAuth>()
                            .0
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .take();
                        let Some((id, pkce)) = pendente else {
                            println!("[eclipse] deep link com code, mas sem login pendente");
                            continue;
                        };
                        println!("[eclipse] code recebido, trocando pelo token…");
                        let app = alvo.clone();
                        tauri::async_runtime::spawn(async move {
                            match eclipse_music::spotify::trocar_codigo(pkce, &codigo).await {
                                Ok(autorizacao) => {
                                    println!("[eclipse] Spotify conectado com sucesso");
                                    if let Err(err) = finalizar_spotify(&app, id, autorizacao) {
                                        eprintln!("[eclipse] falha ao finalizar login do Spotify: {err}");
                                    }
                                }
                                Err(err) => {
                                    eprintln!("[eclipse] troca do code do Spotify falhou: {err}");
                                }
                            }
                        });
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar o Eclipse OS");
}
