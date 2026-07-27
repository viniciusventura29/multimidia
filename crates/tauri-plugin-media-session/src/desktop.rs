//! Fallback para desenvolver no Mac.
//!
//! `MediaSessionManager` é API do Android — não existe equivalente aqui. Este
//! arquivo só existe para o crate compilar no desktop (é a metade que dá pra
//! verificar de verdade neste ambiente); o app continua usando o Spotify via
//! Web API para desenvolver, como já fazia.

use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::models::AndroidNowPlaying;
use crate::Result;

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<MediaSession<R>> {
    Ok(MediaSession(std::marker::PhantomData))
}

// `fn() -> R` em vez de `R` puro: `PhantomData<R>` herdaria os bounds de auto
// trait de `R`, e nada garante que todo `Runtime` seja `Send + Sync` só por
// ser `Runtime`. Um ponteiro de função é sempre `Send + Sync`, independente de
// `R` — e como não guardamos nenhum `R` de verdade aqui, é seguro.
pub struct MediaSession<R: Runtime>(std::marker::PhantomData<fn() -> R>);

impl<R: Runtime> MediaSession<R> {
    pub fn now_playing(&self) -> Result<Option<AndroidNowPlaying>> {
        Ok(None)
    }

    pub fn play(&self) -> Result<()> {
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        Ok(())
    }

    pub fn next(&self) -> Result<()> {
        Ok(())
    }

    pub fn previous(&self) -> Result<()> {
        Ok(())
    }

    pub fn has_notification_access(&self) -> Result<bool> {
        Ok(false)
    }

    pub fn request_notification_access(&self) -> Result<()> {
        Ok(())
    }
}
