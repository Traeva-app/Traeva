//! The desktop pet's window (#208) through the IPC boundary, on Tauri's
//! mock runtime with the app's real config and capabilities: the pet may
//! read the drop queue and ask for the main window, and nothing else.

use std::sync::Arc;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::OrganizeService;

fn app() -> (tempfile::TempDir, App<MockRuntime>) {
    let app_data = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
    (app_data, app_with(service))
}

/// The app's real config and capabilities on the mock runtime (the context
/// can be generated once per test binary).
fn app_with(service: Arc<OrganizeService>) -> App<MockRuntime> {
    traeva_app::app_builder(mock_builder(), service)
        .build(tauri::generate_context!())
        .unwrap()
}

fn window(app: &App<MockRuntime>, label: &str) -> WebviewWindow<MockRuntime> {
    WebviewWindowBuilder::new(app, label, Default::default())
        .build()
        .unwrap()
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn the_pet_asks_for_a_page_and_the_main_window_takes_it_once() {
    let (_app_data, app) = app();
    let main = window(&app, "main");
    let pet = window(&app, "pet");

    let state = invoke(&pet, "get_pet_state", json!({})).unwrap();
    assert_eq!(state["items"], json!([]));
    invoke(&pet, "show_main_window", json!({ "page": "find" })).unwrap();
    assert_eq!(
        invoke(&main, "take_page_request", json!({})).unwrap(),
        json!("find")
    );
    assert_eq!(
        invoke(&main, "take_page_request", json!({})).unwrap(),
        Value::Null,
        "taken once"
    );
    // The popup's "Turn on automatic mode…": Settings with that card.
    let ask = window(&app, "ask");
    invoke(&ask, "show_main_window", json!({ "page": "automatic" })).unwrap();
    assert_eq!(
        invoke(&main, "take_page_request", json!({})).unwrap(),
        json!("automatic")
    );
    let error = invoke(&pet, "show_main_window", json!({ "page": "source" })).unwrap_err();
    assert_eq!(error["code"], json!("invalid_input"));
    assert!(invoke(&main, "set_pet_visible", json!({ "visible": false })).is_ok());
}

#[test]
fn the_pet_may_call_nothing_else() {
    let (_app_data, app) = app();
    let pet = window(&app, "pet");
    for (cmd, args) in [
        ("get_app_state", json!({})),
        ("get_drop_queue", json!({})),
        ("start_find", json!({ "question": "kyoto" })),
        ("take_page_request", json!({})),
        ("set_pet_visible", json!({ "visible": true })),
        ("set_auto_drop_settings", json!({ "request": {} })),
        ("continue_drops", json!({})),
    ] {
        let error = invoke(&pet, cmd, args).unwrap_err();
        assert!(
            error.as_str().unwrap_or("").contains("not allowed"),
            "{cmd}: {error}"
        );
    }
}

/// With a file in the queue, what the pet reads holds no path, name,
/// folder or reason: only what its face needs (R1's P03 review of #208).
#[test]
fn the_pet_s_state_holds_no_path_or_name() {
    let app_data = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    let file = outside.path().join("secret-plan.md");
    std::fs::write(&file, "notes\n").unwrap();
    let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
    service.receive_drops(std::slice::from_ref(&file)).unwrap();
    service.run_drops();
    let app = app_with(Arc::clone(&service));
    let pet = window(&app, "pet");

    let state = invoke(&pet, "get_pet_state", json!({})).unwrap();

    let item = &state["items"][0];
    assert_eq!(item["state"], json!("needs_source_authorization"));
    assert_eq!(item["needs_user"], json!(true));
    let mut keys: Vec<&str> = item
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "action_status",
            "drop_id",
            "needs_user",
            "received_at_ms",
            "state"
        ]
    );
    let text = state.to_string();
    for private in [
        "secret-plan",
        "traeva-outside",
        "/Users/",
        "/private/",
        "/var/",
    ] {
        assert!(!text.contains(private), "{private} reached the pet");
    }
}

#[test]
fn quick_search_may_use_find_and_nothing_else() {
    let (_app_data, app) = app();
    let quick = window(&app, "quick");
    let pet = window(&app, "pet");

    assert!(invoke(&pet, "show_quick_search", json!({})).is_ok());
    let started = invoke(&quick, "start_find", json!({ "question": "kyoto" })).unwrap();
    let find_id = started["find_id"].as_str().unwrap().to_string();
    assert!(invoke(&quick, "get_find", json!({ "find_id": find_id })).is_ok());
    assert!(invoke(&quick, "get_find_setup", json!({})).is_ok());
    assert_eq!(
        invoke(&quick, "resize_quick_search", json!({ "height": 9000 })).unwrap(),
        json!(480.0),
        "the height is kept in range"
    );
    assert!(invoke(&quick, "hide_quick_search", json!({})).is_ok());
    assert!(invoke(&quick, "show_main_window", json!({ "page": "find" })).is_ok());
    for (cmd, args) in [
        ("set_find_consent", json!({ "agree": true })),
        ("get_app_state", json!({})),
        ("get_drop_queue", json!({})),
        ("get_pet_state", json!({})),
        ("apply_plan", json!({ "plan_id": "x" })),
        ("set_pet_visible", json!({ "visible": true })),
    ] {
        let error = invoke(&quick, cmd, args).unwrap_err();
        assert!(
            error.as_str().unwrap_or("").contains("not allowed"),
            "{cmd}: {error}"
        );
    }
}
