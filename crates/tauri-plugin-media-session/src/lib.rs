//! Lê e controla a sessão de mídia ativa do Android.
//!
//! O caminho é `MediaSessionManager.getActiveSessions()`, o mesmo mecanismo que
//! o Android Auto usa — API pública, não é engenharia reversa. Funciona com
//! **qualquer** app que publique uma `MediaSession` (Spotify, YouTube Music,
//! podcast), não só um. Custa uma permissão manual: "acesso a notificações",
//! concedida em Ajustes, porque é o mesmo emprestado de identidade que dá acesso
//! às notificações — o Android não tem uma permissão específica só para sessão
//! de mídia.
//!
//! ⚠️ **O lado Android (`android/`, `src/mobile.rs`) nunca foi compilado.** Este
//! projeto é desenvolvido num Mac sem SDK do Android, `adb` ou Java instalados.
//! O que dá para verificar aqui é só `src/desktop.rs` e este arquivo — a ponte
//! JNI de verdade só se prova rodando `npx tauri android dev` com o SDK
//! instalado e um aparelho conectado.

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

mod error;
mod models;

pub use error::{Error, Result};
pub use models::AndroidNowPlaying;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

#[cfg(desktop)]
use desktop::MediaSession;
#[cfg(mobile)]
use mobile::MediaSession;

/// Acesso à sessão de mídia a partir de qualquer `AppHandle`/`App`.
pub trait MediaSessionExt<R: Runtime> {
    fn media_session(&self) -> &MediaSession<R>;
}

impl<R: Runtime, T: Manager<R>> MediaSessionExt<R> for T {
    fn media_session(&self) -> &MediaSession<R> {
        self.state::<MediaSession<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("media-session")
        .setup(|app, api| {
            #[cfg(mobile)]
            let media_session = mobile::init(app, api)?;
            #[cfg(desktop)]
            let media_session = desktop::init(app, api)?;

            app.manage(media_session);
            Ok(())
        })
        .build()
}
