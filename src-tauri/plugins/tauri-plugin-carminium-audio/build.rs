const COMMANDS: &[&str] = &["open_airplay"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .build();
}
