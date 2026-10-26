//! The windows' limits, checked from the real config files: the main window
//! may call exactly Traeva's own commands, nothing from any plugin (no fs,
//! sql, shell or dialog); the desktop pet (#208) only three things; and the
//! pages may load only local content.

use std::path::Path;

use serde_json::Value;
use traeva_app::COMMANDS;

fn read_json(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_main_window_may_call_exactly_the_listed_commands() {
    let capability = read_json("capabilities/main-window.json");
    assert_eq!(strings(&capability["windows"]), ["main"]);
    let granted = strings(&capability["permissions"]);
    let expected: Vec<String> = COMMANDS
        .iter()
        .map(|name| format!("allow-{}", name.replace('_', "-")))
        .collect();
    assert_eq!(granted, expected);
    // A plugin permission always has a "plugin:" prefix, e.g. "fs:default".
    assert!(granted.iter().all(|permission| !permission.contains(':')));
}

#[test]
fn the_pet_may_read_its_pathless_state_show_the_main_window_and_be_dragged_only() {
    let capability = read_json("capabilities/pet-window.json");
    assert_eq!(strings(&capability["windows"]), ["pet"]);
    assert_eq!(
        strings(&capability["permissions"]),
        [
            "allow-get-pet-state",
            "allow-get-pet-signal",
            "allow-show-main-window",
            "allow-show-quick-search",
            "allow-show-drop-popup",
            // The moved note, by the drop's ID (10/17): no path either.
            "allow-show-moved-note",
            "core:window:allow-start-dragging"
        ],
        "no other command (not the drop queue, which holds paths), and no event listening"
    );
}

#[test]
fn quick_search_may_use_find_and_its_own_size_only() {
    let capability = read_json("capabilities/quick-window.json");
    assert_eq!(strings(&capability["windows"]), ["quick"]);
    assert_eq!(
        strings(&capability["permissions"]),
        [
            "allow-get-find-setup",
            "allow-start-find",
            "allow-get-find",
            "allow-cancel-find",
            "allow-reveal-find-result",
            "allow-open-find-result",
            "allow-resize-quick-search",
            "allow-hide-quick-search",
            "allow-show-main-window"
        ],
        "Find and its own window only: not even agreeing to send"
    );
}

#[test]
fn the_pet_s_popup_may_finish_one_dropped_file_and_nothing_else() {
    let capability = read_json("capabilities/ask-window.json");
    assert_eq!(strings(&capability["windows"]), ["ask"]);
    assert_eq!(
        strings(&capability["permissions"]),
        [
            "allow-get-app-state",
            "allow-get-drop-queue",
            "allow-get-operation",
            "allow-start-analysis",
            "allow-answer-clarification",
            "allow-choose-manual-location",
            "allow-continue-drops",
            "allow-start-drop-operation",
            "allow-cancel-drop",
            "allow-cancel-operation",
            "allow-list-folders",
            "allow-build-plan",
            "allow-get-plan",
            "allow-cancel-plan",
            "allow-apply-plan",
            "allow-get-action",
            "allow-show-main-window",
            "allow-resize-drop-popup",
            "allow-hide-drop-popup",
            // The moved note (10/17): its drop's ID, and Undo only after the
            // preview of where the file goes back (organize-spec §8).
            "allow-preview-undo",
            "allow-take-moved-note",
            "core:window:allow-start-dragging"
        ],
        "no agreeing to send, no source authorization, no pickers, Undo only after its preview, no settings, no event listening"
    );
}

#[test]
fn there_are_four_windows_and_their_pages_load_only_local_content() {
    let config = read_json("tauri.conf.json");
    let windows = config["app"]["windows"].as_array().unwrap();
    let labels: Vec<&str> = windows
        .iter()
        .map(|window| window["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["main", "pet", "quick", "ask"]);
    for window in windows {
        assert!(window.get("url").is_none(), "no remote page");
    }
    // The pet starts hidden; the main window shows it as the user chose.
    assert_eq!(windows[1]["visible"], false);
    assert_eq!(windows[1]["transparent"], true);
    // The pet is almost never the key window: the first click must reach
    // its page (#208).
    assert_eq!(windows[1]["acceptFirstMouse"], true);
    assert_eq!(
        strings(&config["app"]["security"]["capabilities"]),
        ["main-window", "pet-window", "quick-window", "ask-window"]
    );
    // The pet's popup and quick search start hidden. Their windows are
    // transparent with a shadow: the page paints an opaque background with
    // round corners, like a macOS window (popup.css, quick.css; 10/15).
    for window in [&windows[2], &windows[3]] {
        assert_eq!(window["visible"], false);
        assert_eq!(window["transparent"], true);
        assert_eq!(window["shadow"], true);
    }
    let csp = config["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.starts_with("default-src 'self';"));
    for part in csp.split(';') {
        let part = part.trim();
        // The only network address is Tauri's own IPC endpoint.
        for source in part.split_whitespace().skip(1) {
            let local =
                source.starts_with('\'') || source == "ipc:" || source == "http://ipc.localhost";
            assert!(local, "unexpected source {source} in `{part}`");
        }
    }
    assert!(!csp.contains("unsafe-eval"));
}
