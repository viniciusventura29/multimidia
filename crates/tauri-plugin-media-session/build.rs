const COMMANDS: &[&str] = &[
    "now_playing",
    "play",
    "pause",
    "next",
    "previous",
    "has_notification_access",
    "request_notification_access",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
