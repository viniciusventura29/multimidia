//! Fonte de mentira, para trabalhar na interface sem Client ID do Spotify.
//!
//! Não é o comportamento padrão: sem Spotify configurado o módulo prefere se
//! declarar degradado, porque música falsa num painel de carro esconde que a
//! integração não está de pé. Ligue com `ECLIPSE_MUSIC_DEMO=1` quando o assunto
//! for layout.

use async_trait::async_trait;

use crate::source::{MusicError, MusicSource, NowPlaying};

const PLAYLIST: [(&str, &str); 3] = [
    ("Weightless", "Marconi Union"),
    ("Nightcall", "Kavinsky"),
    ("Bloom", "ODESZA"),
];

#[derive(Default)]
pub struct DemoSource {
    indice: usize,
    tocando: bool,
}

impl DemoSource {
    fn atual(&self) -> NowPlaying {
        let (track, artist) = PLAYLIST[self.indice];
        NowPlaying {
            track: track.to_string(),
            artist: artist.to_string(),
            is_playing: self.tocando,
            album_art: None,
            progress_ms: None,
            duration_ms: None,
        }
    }
}

#[async_trait]
impl MusicSource for DemoSource {
    async fn now_playing(&mut self) -> Result<Option<NowPlaying>, MusicError> {
        Ok(Some(self.atual()))
    }

    async fn toggle(&mut self) -> Result<(), MusicError> {
        self.tocando = !self.tocando;
        Ok(())
    }

    async fn next(&mut self) -> Result<(), MusicError> {
        self.indice = (self.indice + 1) % PLAYLIST.len();
        self.tocando = true;
        Ok(())
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        self.indice = (self.indice + PLAYLIST.len() - 1) % PLAYLIST.len();
        self.tocando = true;
        Ok(())
    }
}
