//! The IPC boundary with the app's real config and capability, on Tauri's
//! mock runtime (no real window): the main window gets the core's answers
//! and errors, input that does not fit the contract is refused, and nothing
//! outside the capability can be called. The two commands that open native
//! pickers are not called here; the real window flow covers them.

use std::sync::Arc;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::OrganizeService;

struct Ipc {
    _app_data: tempfile::TempDir,
    app: App<MockRuntime>,
}

impl Ipc {
    fn new() -> Ipc {
        let app_data = tempfile::Builder::new()
            .prefix("traeva-appdata-")
            .tempdir()
            .unwrap();
        let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
        let app = traeva_app::app_builder(mock_builder(), service)
            .build(tauri::generate_context!())
            .unwrap();
        Ipc {
            _app_data: app_data,
            app,
        }
    }

    fn window(&self, label: &str) -> WebviewWindow<MockRuntime> {
        WebviewWindowBuilder::new(&self.app, label, Default::default())
            .build()
            .unwrap()
    }
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    invoke_from(window, "tauri://localhost", cmd, args)
}

/// `invoke`, as if the window showed a page from `url`.
fn invoke_from(
    window: &WebviewWindow<MockRuntime>,
    url: &str,
    cmd: &str,
    args: Value,
) -> Result<Value, Value> {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: url.parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

fn code(error: &Value) -> &str {
    error["code"].as_str().unwrap_or("")
}

/// Tauri's own refusals (permissions, arguments) are plain strings.
fn refusal(error: &Value) -> &str {
    error.as_str().unwrap_or("")
}

#[test]
fn the_main_window_gets_the_cores_answers_and_errors() {
    let ipc = Ipc::new();
    let main = ipc.window("main");

    let state = invoke(&main, "get_app_state", json!({})).unwrap();
    assert_eq!(state["storage"]["status"], "ready");
    assert_eq!(state["roots"], json!([]));

    // Forged or unknown IDs: the core refuses them with its own error.
    let error = invoke(&main, "get_plan", json!({ "plan_id": "../../etc" })).unwrap_err();
    assert_eq!(code(&error), "invalid_input");
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    let error = invoke(&main, "apply_plan", json!({ "plan_id": unknown })).unwrap_err();
    assert_eq!(code(&error), "unknown_plan");
    let error = invoke(&main, "get_operation", json!({ "operation_id": unknown })).unwrap_err();
    assert_eq!(code(&error), "unknown_operation");
    let error = invoke(&main, "revoke_root", json!({ "root_id": unknown })).unwrap_err();
    assert_eq!(code(&error), "unknown_root");
}

#[test]
fn input_that_does_not_fit_the_contract_is_refused() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    let request = json!({
        "file_id": "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b",
        "file_revision": 1,
        "target_root_id": "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b",
        "target": { "kind": "folder", "catalog_id": "x", "folder_id": "F1", "new_folders": [] },
        // Not part of PlanRequest: a path the window tries to slip in.
        "target_path": "/etc"
    });
    let error = invoke(&main, "build_plan", json!({ "request": request })).unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `target_path`"),
        "{error}"
    );
    // Inside the target too: a candidate carries only its ID.
    let request = json!({
        "file_id": "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b",
        "file_revision": 1,
        "target_root_id": "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b",
        "operation_id": "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b",
        "target": { "kind": "candidate", "candidate_id": "C1", "folder_path": "/etc" }
    });
    let error = invoke(&main, "build_plan", json!({ "request": request })).unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `folder_path`"),
        "{error}"
    );
    // A missing argument is refused before the core is reached.
    let error = invoke(&main, "get_action", json!({})).unwrap_err();
    assert!(refusal(&error).contains("missing required key action_id"));
}

#[test]
fn nothing_outside_the_capability_can_be_called() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    // The dialog plugin is loaded for the Rust side, but the window may not
    // open a picker itself and pass back any path it likes.
    let error = invoke(&main, "plugin:dialog|open", json!({ "options": {} })).unwrap_err();
    assert!(
        refusal(&error).starts_with("dialog.open not allowed"),
        "{error}"
    );
    // A plugin Traeva does not load at all.
    let error = invoke(
        &main,
        "plugin:fs|read_file",
        json!({ "path": "/etc/hosts" }),
    )
    .unwrap_err();
    assert!(refusal(&error).contains("not allowed"), "{error}");
    // A window other than "main" may call nothing, not even the app state.
    let other = ipc.window("other");
    let error = invoke(&other, "get_app_state", json!({})).unwrap_err();
    assert!(
        refusal(&error).starts_with("get_app_state not allowed on window \"other\""),
        "{error}"
    );
    assert!(invoke(&main, "get_app_state", json!({})).is_ok());
    // A remote page shown in the main window may call nothing either (O32).
    for url in ["https://example.invalid/", "http://127.0.0.1:8080/"] {
        let error = invoke_from(&main, url, "get_app_state", json!({})).unwrap_err();
        assert!(refusal(&error).contains("URL: local"), "{url}: {error}");
    }
}

/// Text recognition's state (#186, §14.8): a service without a recognizer
/// (as in these tests) reports it off; only the main window may ask.
#[test]
fn the_main_window_reads_the_text_recognition_state() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    assert_eq!(invoke(&main, "get_ocr_state", json!({})).unwrap(), "off");
    let pet = ipc.window("pet");
    let error = invoke(&pet, "get_ocr_state", json!({})).unwrap_err();
    assert!(refusal(&error).contains("not allowed"), "{error}");
}

#[test]
fn analysis_and_model_settings_commands_reach_the_core() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";

    // A fresh app has no settings, so nothing here looks into the Keychain.
    let settings = invoke(&main, "get_model_settings", json!({})).unwrap();
    assert_eq!(settings["status"], "not_configured");
    assert_eq!(settings["key_saved"], false);
    assert!(settings["notice"].as_str().unwrap().contains("OpenAI"));
    assert!(
        settings.get("api_key").is_none(),
        "the key is never sent back"
    );
    let error = invoke(&main, "set_analysis_consent", json!({ "agree": true })).unwrap_err();
    assert_eq!(code(&error), "model_not_configured");
    // Checked by the core before any Keychain access.
    let input = json!({ "provider": "not-openai", "model": "gpt-6-luna" });
    let error = invoke(&main, "save_model_settings", json!({ "input": input })).unwrap_err();
    assert_eq!(code(&error), "invalid_input");
    let input = json!({ "provider": "openai", "model": "gpt-6-luna", "endpoint": "http://x" });
    let error = invoke(&main, "save_model_settings", json!({ "input": input })).unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `endpoint`"),
        "{error}"
    );

    // The analysis commands answer for operations the core knows.
    for command in ["start_analysis", "choose_manual_location"] {
        let error = invoke(&main, command, json!({ "operation_id": unknown })).unwrap_err();
        assert_eq!(code(&error), "unknown_operation", "{command}");
        let error = invoke(&main, command, json!({ "operation_id": "../x" })).unwrap_err();
        assert_eq!(code(&error), "invalid_input", "{command}");
    }
    let answer = json!({ "question_id": unknown, "option_id": "O1" });
    let error = invoke(
        &main,
        "answer_clarification",
        json!({ "operation_id": unknown, "answer": answer }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "unknown_operation");
    let answer = json!({ "question_id": unknown, "text": "Work", "locked": false });
    let error = invoke(
        &main,
        "answer_clarification",
        json!({ "operation_id": unknown, "answer": answer }),
    )
    .unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `locked`"),
        "{error}"
    );
}
