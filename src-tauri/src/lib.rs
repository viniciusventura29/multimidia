//! O shell do Eclipse OS.
//!
//! Não tem regra de negócio aqui: este crate liga o supervisor do `eclipse-core`
//! na janela do Tauri. O Rust é dono do estado; a UI é uma projeção dele.

mod modules;

use std::sync::{Arc, Mutex};

use eclipse_core::{
    factory, ModuleCommand, ModuleId, Profile, ProfileStore, StateEnvelope, Supervisor,
};
use eclipse_music::TokenStore;
use serde_json::Value;
use tauri::{Emitter, Manager};
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
}

fn maps_api_key(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_MAPS_API_KEY", "maps_api_key.txt")
}

/// Sem Map ID o mapa é raster, e raster ignora inclinação e rotação.
fn maps_map_id(dir_dados: &std::path::Path) -> Option<String> {
    credencial(dir_dados, "ECLIPSE_MAPS_MAP_ID", "maps_map_id.txt")
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

    let (client, url) =
        eclipse_music::spotify::iniciar_autorizacao(&client_id).map_err(|e| e.to_string())?;

    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string())?;

    let autorizacao = eclipse_music::spotify::concluir_autorizacao(client)
        .await
        .map_err(|e| e.to_string())?;

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
/* Navegação                                                           */
/* ------------------------------------------------------------------ */

/// Entrega um destino ao app do Google Maps.
///
/// O painel não navega: ele passa a bola para quem faz isso bem. No Android o
/// esquema `google.navigation:` cai direto no turn-by-turn, sem tela
/// intermediária — que é o que se quer com o carro andando. Fora do Android não
/// existe app para abrir, então vai pela URL universal, que resolve no navegador
/// e serve para desenvolver.
#[tauri::command]
fn open_navigation(destino: String) -> Result<(), String> {
    let destino = destino.trim();
    if destino.is_empty() {
        return Err("destino vazio".into());
    }

    let alvo = urlencoding(destino);
    let url = if cfg!(target_os = "android") {
        format!("google.navigation:q={alvo}&mode=d")
    } else {
        format!("https://www.google.com/maps/dir/?api=1&destination={alvo}&travelmode=driving")
    };

    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string())
}

/// Percent-encoding do que vai na query.
///
/// Endereço brasileiro tem acento, vírgula e espaço; mandar cru quebra a URL.
fn urlencoding(texto: &str) -> String {
    texto
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            outro => format!("%{outro:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::urlencoding;

    /// Endereço brasileiro tem acento, vírgula e espaço. Mandar cru quebra a URL
    /// e o Maps abre no lugar errado — ou não abre.
    #[test]
    fn codifica_endereco_brasileiro() {
        assert_eq!(
            urlencoding("Praça da Sé, São Paulo"),
            "Pra%C3%A7a+da+S%C3%A9%2C+S%C3%A3o+Paulo"
        );
    }

    #[test]
    fn nao_mexe_no_que_ja_e_seguro() {
        assert_eq!(urlencoding("Av-Paulista_1000.5~x"), "Av-Paulista_1000.5~x");
    }

    #[test]
    fn escapa_o_que_quebraria_a_query() {
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
    }
}

/* ------------------------------------------------------------------ */
/* Fiação                                                              */
/* ------------------------------------------------------------------ */

/// Repassa cada mudança de estado para a janela.
fn forward_states(app: tauri::AppHandle, supervisor: &Supervisor) {
    let mut rx = supervisor.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    if let Err(err) = app.emit(EVENT_MODULE_STATE, &envelope) {
                        tracing::warn!(%err, "não consegui emitir estado para a UI");
                    }
                }
                // A UI ficou pra trás. Ela se corrige sozinha no próximo evento,
                // porque cada envelope carrega o estado inteiro do módulo.
                Err(RecvError::Lagged(perdidos)) => {
                    tracing::warn!(perdidos, "UI ficou pra trás no barramento");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            dispatch_action,
            list_profiles,
            active_profile,
            create_profile,
            select_profile,
            delete_profile,
            connect_spotify,
            open_navigation,
        ])
        .setup(|app| {
            let dir = app
                .path()
                .app_data_dir()
                .expect("sem diretório de dados do app");

            let store = ProfileStore::load(dir.join("profiles.json"));
            let ativo = store.active().cloned();

            let cofre = Arc::new(Mutex::new(TokenStore::load(dir.join("spotify_tokens.json"))));
            let conector: Arc<dyn modules::music::Conector> =
                Arc::new(modules::music::SpotifyConector {
                    client_id: client_id(&dir),
                    cofre: Arc::clone(&cofre),
                    demo: std::env::var("ECLIPSE_MUSIC_DEMO").is_ok_and(|v| v == "1"),
                });

            let chave_mapa = maps_api_key(&dir);
            let id_mapa = maps_map_id(&dir);
            let handle = app.handle().clone();

            // `block_on` serve só para entrar no runtime do Tauri, para que os
            // `tokio::spawn` lá dentro tenham contexto; as tasks seguem vivas depois.
            let supervisor = tauri::async_runtime::block_on(async move {
                let mut supervisor = Supervisor::new();
                forward_states(handle, &supervisor);

                supervisor.spawn(factory(modules::obd::OBD, modules::obd::ObdModule::default));
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
                        Box::new(eclipse_gps::SimulatedLocation::default()),
                    )
                }));

                supervisor
            });

            // Os módulos precisam saber quem está dirigindo antes do primeiro
            // toque na tela, senão a música subiria com a conta do perfil errado.
            if let Some(profile) = &ativo {
                supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(profile.clone())));
            }

            app.manage(supervisor);
            app.manage(Perfis(Mutex::new(store)));
            app.manage(Cofre(cofre));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar o Eclipse OS");
}
