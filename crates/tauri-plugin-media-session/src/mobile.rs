//! A ponte com o lado Kotlin.
//!
//! ⚠️ **Nada neste arquivo foi compilado.** Este Mac não tem SDK do Android, nem
//! `adb`, nem Java — só dá para escrever contra a API documentada do Tauri v2
//! para plugins móveis e conferir de novo quando houver toolchain Android de
//! verdade (`npx tauri android init` com o SDK instalado). O formato de
//! `register_android_plugin` e `run_mobile_plugin` foi conferido direto na
//! documentação oficial; o resto segue o padrão usado por outros plugins
//! oficiais do Tauri, mas não foi verificado linha a linha.

use serde::de::DeserializeOwned;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::models::{AndroidNowPlaying, NotificationAccess};
use crate::Result;

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.eclipseos.mediasession";

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<MediaSession<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "MediaSessionPlugin")?;

    Ok(MediaSession(handle))
}

/// A sessão de mídia do sistema, do lado Rust.
///
/// Cada método é uma chamada de ida e volta ao Kotlin — não existe canal de
/// eventos: o Tauri não dá um jeito do nativo empurrar dado pro Rust sozinho,
/// só documenta o caminho Rust → Kotlin. Por isso o módulo de música (que já
/// faz *polling*) pergunta em vez de esperar avisos, e nada muda no
/// `eclipse-core`.
pub struct MediaSession<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> MediaSession<R> {
    /// A sessão de mídia ativa agora, ou vazio se nada estiver tocando em
    /// lugar nenhum.
    ///
    /// O `resolve(null)` do Kotlin não chega como `null`: a ponte entrega `{}`,
    /// que com o `serde(default)` vira um `AndroidNowPlaying` todo vazio. O
    /// `packageName` distingue os casos — o Kotlin sempre o preenche quando
    /// existe sessão de verdade.
    pub fn now_playing(&self) -> Result<Option<AndroidNowPlaying>> {
        self.0
            .run_mobile_plugin::<Option<AndroidNowPlaying>>("now_playing", ())
            .map(|sessao| sessao.filter(|s| s.package_name.is_some()))
            .map_err(Into::into)
    }

    pub fn play(&self) -> Result<()> {
        self.0.run_mobile_plugin("play", ()).map_err(Into::into)
    }

    pub fn pause(&self) -> Result<()> {
        self.0.run_mobile_plugin("pause", ()).map_err(Into::into)
    }

    pub fn next(&self) -> Result<()> {
        self.0.run_mobile_plugin("next", ()).map_err(Into::into)
    }

    pub fn previous(&self) -> Result<()> {
        self.0
            .run_mobile_plugin("previous", ())
            .map_err(Into::into)
    }

    /// Se o usuário já concedeu "acesso a notificações" ao app.
    ///
    /// É uma permissão manual — vive em Ajustes, não num diálogo de runtime.
    /// Sem ela `getActiveSessions` do Android nem devolve nada; é melhor
    /// perguntar isso primeiro do que interpretar um erro genérico.
    pub fn has_notification_access(&self) -> Result<bool> {
        self.0
            .run_mobile_plugin::<NotificationAccess>("has_notification_access", ())
            .map(|r| r.value)
            .map_err(Into::into)
    }

    /// Abre a tela de Ajustes onde o usuário concede o acesso.
    ///
    /// Não dá para conceder isso programaticamente — é por design do Android,
    /// para impedir apps de se auto-concederem acesso a todas as notificações.
    pub fn request_notification_access(&self) -> Result<()> {
        self.0
            .run_mobile_plugin("request_notification_access", ())
            .map_err(Into::into)
    }
}
