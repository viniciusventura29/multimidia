fn main() {
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
