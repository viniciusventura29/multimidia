// Nenhum comando é exposto ao JS: o plugin é chamado só pelo Rust (o módulo OBD),
// via `run_mobile_plugin`. Mesmo assim o build precisa rodar para registrar o
// projeto Android (`android/`) junto ao app.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
