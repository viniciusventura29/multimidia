//! Fonte de mentira, para trabalhar na interface sem Client ID do Spotify.
//!
//! Não é o comportamento padrão: sem Spotify configurado o módulo prefere se
//! declarar degradado, porque música falsa num painel de carro esconde que a
//! integração não está de pé. Ligue com `ECLIPSE_MUSIC_DEMO=1` quando o assunto
//! for layout.

use async_trait::async_trait;

use crate::source::{Faixa, MusicError, MusicSource, NowPlaying, Playlist};

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
            // Capa de mentira (SVG embutido): sem ela o layout do card não podia
            // ser conferido no emulador, e é justo a capa que estourava a altura.
            album_art: Some(
                "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' \
                 height='300'%3E%3Crect width='300' height='300' fill='%234a2b7a'/%3E%3Ccircle \
                 cx='150' cy='150' r='70' fill='%233ddc97'/%3E%3C/svg%3E"
                    .to_string(),
            ),
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

    async fn buscar(&mut self, termo: &str) -> Result<Vec<Faixa>, MusicError> {
        Ok(PLAYLIST
            .iter()
            .map(|(track, artist)| Faixa {
                uri: format!("spotify:track:demo-{track}"),
                track: format!("{track} — {termo}"),
                artist: artist.to_string(),
                album_art: None,
            })
            .collect())
    }

    async fn tocar(&mut self, _uri: &str) -> Result<(), MusicError> {
        self.tocando = true;
        Ok(())
    }

    async fn playlists(&mut self) -> Result<Vec<Playlist>, MusicError> {
        Ok(vec![
            Playlist { uri: "spotify:playlist:demo-1".into(), nome: "Foco".into(), album_art: None },
            Playlist { uri: "spotify:playlist:demo-2".into(), nome: "Estrada".into(), album_art: None },
            Playlist { uri: "spotify:playlist:demo-3".into(), nome: "Domingo".into(), album_art: None },
        ])
    }

    async fn tocar_playlist(&mut self, _uri: &str) -> Result<(), MusicError> {
        self.tocando = true;
        Ok(())
    }
}
