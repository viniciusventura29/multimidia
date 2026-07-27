//! O shell do Eclipse OS.
//!
//! Não tem regra de negócio aqui: este crate liga o supervisor do `eclipse-core`
//! na janela do Tauri. O Rust é dono do estado; a UI é uma projeção dele.

mod modules;

use std::sync::{Arc, Mutex};

use eclipse_core::{
    factory, ModuleCommand, ModuleId, Profile, ProfileStore, StateEnvelope, Supervisor,
};
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
        ])
        .setup(|app| {
            let caminho = app
                .path()
                .app_data_dir()
                .expect("sem diretório de dados do app")
                .join("profiles.json");
            let store = ProfileStore::load(caminho);
            let ativo = store.active().cloned();

            let handle = app.handle().clone();

            // `block_on` serve só para entrar no runtime do Tauri, para que os
            // `tokio::spawn` lá dentro tenham contexto; as tasks seguem vivas depois.
            let supervisor = tauri::async_runtime::block_on(async move {
                let mut supervisor = Supervisor::new();
                forward_states(handle, &supervisor);

                supervisor.spawn(factory(modules::obd::OBD, modules::obd::ObdModule::default));
                supervisor.spawn(factory(
                    modules::music::MUSIC,
                    modules::music::PlaceholderMusic::default,
                ));
                supervisor.spawn(factory(
                    modules::nav::NAV,
                    modules::nav::PlaceholderNav::default,
                ));

                supervisor
            });

            // Os módulos precisam saber quem está dirigindo antes do primeiro
            // toque na tela, senão a música subiria com a conta do perfil errado.
            if let Some(profile) = &ativo {
                supervisor.dispatch(ModuleCommand::ProfileChanged(Arc::new(profile.clone())));
            }

            app.manage(supervisor);
            app.manage(Perfis(Mutex::new(store)));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar o Eclipse OS");
}
