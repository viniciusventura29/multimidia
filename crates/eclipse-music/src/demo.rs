//! Fonte de mentira, para trabalhar na interface sem Client ID do Spotify.
//!
//! Não é o comportamento padrão: sem Spotify configurado o módulo prefere se
//! declarar degradado, porque música falsa num painel de carro esconde que a
//! integração não está de pé. Ligue com `ECLIPSE_MUSIC_DEMO=1` quando o assunto
//! for layout.

use std::time::Duration;

use async_trait::async_trait;

use crate::source::{Faixa, MusicError, MusicSource, NowPlaying, Playlist};

const PLAYLIST: [(&str, &str); 3] = [
    ("Weightless", "Marconi Union"),
    ("Nightcall", "Kavinsky"),
    ("Bloom", "ODESZA"),
];

/// Duração de mentira de cada faixa (3:30), para a barra de progresso ter o que
/// mostrar quando o assunto é layout.
const DURACAO_MS: u32 = 210_000;

#[derive(Default)]
pub struct DemoSource {
    indice: usize,
    tocando: bool,
    posicao_ms: u32,
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
            progress_ms: Some(self.posicao_ms.min(DURACAO_MS)),
            duration_ms: Some(DURACAO_MS),
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
        self.posicao_ms = 0;
        Ok(())
    }

    async fn previous(&mut self) -> Result<(), MusicError> {
        self.indice = (self.indice + PLAYLIST.len() - 1) % PLAYLIST.len();
        self.tocando = true;
        self.posicao_ms = 0;
        Ok(())
    }

    async fn buscar(&mut self, termo: &str) -> Result<crate::source::Busca, MusicError> {
        rede().await;
        Ok(crate::source::Busca {
            faixas: PLAYLIST
                .iter()
                .map(|(track, artist)| Faixa {
                    uri: format!("spotify:track:demo-{track}"),
                    track: format!("{track} — {termo}"),
                    artist: artist.to_string(),
                    album_art: None,
                })
                .collect(),
            albuns: vec![crate::source::Album {
                uri: "spotify:album:demo-1".into(),
                nome: format!("Álbum de {termo}"),
                artist: "Artista Demo".into(),
                album_art: None,
            }],
        })
    }

    async fn abrir(&mut self, uri: &str) -> Result<crate::source::Contexto, MusicError> {
        rede().await;
        Ok(crate::source::Contexto {
            uri: uri.to_string(),
            nome: if uri.contains(":album:") {
                "Álbum Demo"
            } else {
                "Playlist Demo"
            }
            .into(),
            subtitulo: if uri.contains(":album:") {
                "Artista Demo"
            } else {
                "playlist"
            }
            .into(),
            album_art: None,
            faixas: PLAYLIST
                .iter()
                .map(|(track, artist)| Faixa {
                    uri: format!("spotify:track:demo-{track}"),
                    track: track.to_string(),
                    artist: artist.to_string(),
                    album_art: None,
                })
                .collect(),
        })
    }

    async fn tocar(
        &mut self,
        _faixa: Option<&str>,
        _contexto: Option<&str>,
    ) -> Result<(), MusicError> {
        self.tocando = true;
        self.posicao_ms = 0;
        Ok(())
    }

    async fn seek(&mut self, posicao_ms: u32) -> Result<(), MusicError> {
        self.posicao_ms = posicao_ms.min(DURACAO_MS);
        Ok(())
    }

    async fn playlists(&mut self) -> Result<Vec<Playlist>, MusicError> {
        rede().await;
        Ok(vec![
            Playlist {
                uri: "spotify:playlist:demo-1".into(),
                nome: "Foco".into(),
                album_art: None,
            },
            Playlist {
                uri: "spotify:playlist:demo-2".into(),
                nome: "Estrada".into(),
                album_art: None,
            },
            Playlist {
                uri: "spotify:playlist:demo-3".into(),
                nome: "Domingo".into(),
                album_art: None,
            },
        ])
    }
}

/// A espera que uma chamada à Web API do Spotify custa de verdade.
///
/// O demo existe para desenvolver a tela sem uma conta conectada — e um demo
/// que responde no mesmo instante mente sobre a única coisa que importa aqui:
/// como a tela se comporta **durante** a espera. Foi assim que a navegação sem
/// retorno nenhum ao tocar numa playlist passou despercebida.
async fn rede() {
    tokio::time::sleep(Duration::from_millis(600)).await;
}
