// The list of the app's commands, shared with the permissions test.
include!("src/command_names.rs");

fn main() {
    // Listing the app's commands here makes Tauri generate one `allow-<command>`
    // permission per command. A window can then call only the commands its
    // capability file (capabilities/main-window.json) allows.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
