fn main() {
    // Credenciais de DEV embutidas no binário para aparelho físico sem root —
    // lidas de arquivos ao lado deste Cargo.toml (fora do git) e entregues ao
    // option_env! de src/lib.rs. Arquivo, e não variável de ambiente, porque o
    // Gradle invoca o cargo com ambiente próprio: export no shell não chega.
    //
    // `versao.txt` entra na mesma tabela mas por outro motivo, e vale escrever
    // porque o motivo acima está incompleto para ele. A env CHEGARIA ao cargo:
    // o `BuildTask.kt` chama `project.exec` sem `environment(...)`, herdando o
    // ambiente do daemon do Gradle — que na CI é sempre novo. O que impede é o
    // CACHE: `option_env!` é expandido pelo rustc, e o cargo não põe env var no
    // fingerprint sem um `rerun-if-env-changed`. Com o `Swatinem/rust-cache`
    // restaurando o `target/`, o run 43 reaproveitaria o binário do run 42 e o
    // APK sairia com 43 no manifesto (o gradle lê a env de verdade) e 42 por
    // dentro. Duas verdades, e a errada é justamente a que o aviso de
    // atualização leria. Com arquivo, o `rerun-if-changed` invalida direito.
    //
    // Sem `versao.txt` — todo build local — o `option_env!` devolve `None`, o
    // app entende zero, e zero quer dizer "não sei em que versão estou".
    //
    // `maps_map_id.txt` continua aqui e continua não chegando ao binário:
    // nenhum `option_env!` o lê desde que o mapa saiu do Google Maps JS para o
    // MapLibre. Variável que ninguém consome não entra no `.so` — não perca
    // tempo caçando, e não a inclua na checagem de APK mudo do `apk.yml`.
    for (arquivo, var) in [
        ("maps_api_key.txt", "ECLIPSE_MAPS_API_KEY"),
        ("maps_map_id.txt", "ECLIPSE_MAPS_MAP_ID"),
        ("spotify_client_id.txt", "ECLIPSE_SPOTIFY_CLIENT_ID"),
        ("anthropic_api_key.txt", "ECLIPSE_ANTHROPIC_API_KEY"),
        ("openrouter_api_key.txt", "ECLIPSE_OPENROUTER_API_KEY"),
        ("versao.txt", "ECLIPSE_VERSION_CODE"),
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
