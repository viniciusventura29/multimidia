fn main() {
    // ⚠️ Se você acabou de CRIAR um dos .txt abaixo, rode `touch src/lib.rs`
    // antes de compilar.
    //
    // Este script roda e emite os `rustc-env` certinho — dá para conferir em
    // `target/<alvo>/debug/build/eclipse-os-*/output`. O que não acontece é o
    // cargo marcar o crate como sujo por causa disso: sem nenhum fonte alterado
    // ele não reinvoca o rustc, e o `option_env!` do `lib.rs` continua sendo o
    // da compilação anterior, quando a variável não existia.
    //
    // O resultado é o pior tipo de falha: compila, empacota, não avisa nada, e
    // o APK sai sem mapa, sem Spotify e sem assistente. Aconteceu em 02/08/2026
    // e custou três builds até alguém olhar os bytes do `.so`.
    //
    // `touch build.rs` NÃO resolve — refaz o script, não o crate.
    // Credenciais de DEV embutidas no binário para aparelho físico sem root —
    // lidas de arquivos ao lado deste Cargo.toml (fora do git) e entregues ao
    // option_env! de src/lib.rs. Arquivo, e não variável de ambiente, porque o
    // Gradle invoca o cargo com ambiente próprio: export no shell não chega.
    for (arquivo, var) in [
        ("maps_api_key.txt", "ECLIPSE_MAPS_API_KEY"),
        ("maps_map_id.txt", "ECLIPSE_MAPS_MAP_ID"),
        ("spotify_client_id.txt", "ECLIPSE_SPOTIFY_CLIENT_ID"),
        ("anthropic_api_key.txt", "ECLIPSE_ANTHROPIC_API_KEY"),
        ("openrouter_api_key.txt", "ECLIPSE_OPENROUTER_API_KEY"),
    ] {
        println!("cargo:rerun-if-changed={arquivo}");
        if let Ok(valor) = std::fs::read_to_string(arquivo) {
            let valor = valor.trim();
            if !valor.is_empty() {
                println!("cargo:rustc-env={var}={valor}");
            }
        }
    }

    tauri_build::build()
}
