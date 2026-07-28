use serde::{Deserialize, Serialize};

/// O que o Android relata sobre a sessão de mídia ativa.
///
/// Os campos são o que o `MediaController` do Android entrega:
/// `MediaMetadata` para track/artist/capa, `PlaybackState` para tocando/posição.
/// Track e artist vêm opcionais porque um app pode não preencher os dois — um
/// podcast às vezes só manda o título do episódio.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidNowPlaying {
    pub track: Option<String>,
    pub artist: Option<String>,
    /// URI de conteúdo (`content://…`) ou HTTP, quando o app publica uma.
    /// Muitos não publicam — nem todo player tem capa disponível assim.
    pub album_art_uri: Option<String>,
    /// `default` porque o `invoke.resolve(null)` do Kotlin ("nada tocando")
    /// chega aqui como `{}`, não como `null` — a ponte do Tauri embrulha em
    /// objeto vazio, e sem o default a desserialização morre com
    /// "missing field `isPlaying`" toda vez que não há mídia nenhuma.
    #[serde(default)]
    pub is_playing: bool,
    pub position_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    /// De qual app veio a sessão escolhida — o pacote do Spotify é
    /// `com.spotify.music`. Serve para o Rust saber qual sessão pegou.
    pub package_name: Option<String>,
}

/// O Kotlin resolve com um objeto (`{"value": bool}`), não com um booleano
/// solto — não há garantia de que o `invoke.resolve` aceite um tipo primitivo
/// direto, e o padrão documentado do Tauri é sempre resolver com `JSObject`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct NotificationAccess {
    pub value: bool,
}
