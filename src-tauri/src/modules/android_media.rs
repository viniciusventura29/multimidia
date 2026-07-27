//! O Spotify (ou qualquer player) rodando de verdade na head unit, controlado
//! pela sessão de mídia do sistema Android.
//!
//! ⚠️ **Não testado.** Depende de `tauri-plugin-media-session`, cujo lado Android
//! nunca foi compilado neste Mac — ver o aviso no topo daquele crate. Isto só se
//! prova com `npx tauri android dev`, SDK instalado e aparelho conectado.
//!
//! A troca feita aqui, e que vale registrar: perfil deixa de trocar de conta do
//! Spotify (o app oficial guarda uma só). Em compensação, ganha-se o Spotify
//! inteiro — busca, playlist, offline, podcast — sem depender de impersonar
//! client ID de ninguém.

use async_trait::async_trait;
use eclipse_music::{MusicError, MusicSource, NowPlaying};
use tauri_plugin_media_session::{AndroidNowPlaying, MediaSessionExt};

pub struct AndroidMediaSource {
    app: tauri::AppHandle,
}

impl AndroidMediaSource {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

/// Os dois tipos são de outras crates — a regra do órfão do Rust proíbe um
/// `impl From` entre eles aqui, então a conversão é só uma função.
fn converter(a: AndroidNowPlaying) -> NowPlaying {
    NowPlaying {
        // Track/artist são obrigatórios no nosso tipo; um app que não
        // preencheu os metadados vira string vazia, não pânico.
        track: a.track.unwrap_or_default(),
        artist: a.artist.unwrap_or_default(),
        is_playing: a.is_playing,
        album_art: a.album_art_uri,
        progress_ms: a.position_ms.and_then(|p| u32::try_from(p).ok()),
        duration_ms: a.duration_ms.and_then(|d| u32::try_from(d).ok()),
    }
}

fn traduzir(err: tauri_plugin_media_session::Error) -> MusicError {
    // O plugin só tem um formato de erro hoje (falha de invoke); a distinção
    // "falta permissão" é feita antes, checando `has_notification_access`.
    MusicError::Network(err.to_string())
}

#[async_trait]
impl MusicSource for AndroidMediaSource {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
        let sessao = self.app.media_session();

        // Checa a permissão antes de tentar: sem isso, o Android lançaria uma
        // SecurityException que chegaria aqui como um erro de rede genérico,
        // escondendo que a solução é um toque em Ajustes, não tentar de novo.
        if !sessao.has_notification_access().map_err(traduzir)? {
            return Err(MusicError::PermissionRequired);
        }

        sessao
            .now_playing()
            .map(|opt| opt.map(converter))
            .map_err(traduzir)
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        let sessao = self.app.media_session();

        // O Android não tem "toggle" — só play e pause separados. O estado
        // real mora no player, então perguntamos antes de decidir qual dos
        // dois mandar.
        let tocando = sessao
            .now_playing()
            .map_err(traduzir)?
            .map(|n| n.is_playing)
            .unwrap_or(false);

        if tocando {
            sessao.pause()
        } else {
            sessao.play()
        }
        .map_err(traduzir)
    }

    async fn next(&mut self) -> Result<(), MusicError> {
        self.app.media_session().next().map_err(traduzir)
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        self.app.media_session().previous().map_err(traduzir)
    }
}
