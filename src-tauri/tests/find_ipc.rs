//! Find (#198) through the IPC boundary, on Tauri's mock runtime with the
//! app's real config and capability. No model settings are saved, so the
//! question is never sent; the core's tests cover the matching. Showing or
//! opening a real result is left out here (it would start Finder).

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::OrganizeService;

struct Ipc {
    _app_data: tempfile::TempDir,
    _root: tempfile::TempDir,
    app: App<MockRuntime>,
}

impl Ipc {
    fn new() -> Ipc {
        let app_data = tempfile::Builder::new()
            .prefix("traeva-appdata-")
            .tempdir()
            .unwrap();
        let root = tempfile::Builder::new()
            .prefix("traeva-root-")
            .tempdir()
            .unwrap();
        fs::write(root.path().join("notes_0912.md"), "Trip to Kyoto.\n").unwrap();
        let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
        service.authorize_root(root.path()).unwrap();
        let app = traeva_app::app_builder(mock_builder(), service)
            .build(tauri::generate_context!())
            .unwrap();
        Ipc {
            _app_data: app_data,
            _root: root,
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

fn code(error: &Value) -> &str {
    error["code"].as_str().unwrap_or("")
}

#[test]
fn a_search_runs_in_the_background_and_the_window_polls_it() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    let setup = invoke(&main, "get_find_setup", json!({})).unwrap();
    assert_eq!(setup["model_ready"], json!(false));

    let started = invoke(&main, "start_find", json!({ "question": "kyoto trip" })).unwrap();
    assert_eq!(started["state"], json!("working"));
    let find_id = started["find_id"].as_str().unwrap().to_string();
    let waited = Instant::now();
    let view = loop {
        let view = invoke(&main, "get_find", json!({ "find_id": find_id })).unwrap();
        if view["state"] != json!("working") || waited.elapsed() > Duration::from_secs(10) {
            break view;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(view["state"], json!("done"), "{view}");
    assert_eq!(view["words_by"]["kind"], json!("question"));
    assert_eq!(view["results"][0]["name"], json!("notes_0912.md"));
    assert_eq!(view["results"][0]["matches"][0]["place"], json!("text"));
    assert!(
        view.get("question").is_none(),
        "the view never holds the question"
    );

    // The window names a search and a result, never a path.
    for (cmd, args) in [
        (
            "reveal_find_result",
            json!({ "find_id": find_id, "result_id": "R9" }),
        ),
        (
            "open_find_result",
            json!({ "find_id": "nope", "result_id": "R1" }),
        ),
        ("get_find", json!({ "find_id": "nope" })),
    ] {
        let error = invoke(&main, cmd, args).unwrap_err();
        assert_eq!(code(&error), "invalid_input", "{cmd}: {error}");
    }
    let error = invoke(&main, "start_find", json!({ "question": "" })).unwrap_err();
    assert_eq!(code(&error), "invalid_input");
}

#[test]
fn only_the_main_window_may_search() {
    let ipc = Ipc::new();
    let other = ipc.window("other");
    for (cmd, args) in [
        ("get_find_setup", json!({})),
        ("start_find", json!({ "question": "kyoto" })),
        ("set_find_consent", json!({ "agree": true })),
        ("get_recent_files", json!({})),
    ] {
        let error = invoke(&other, cmd, args).unwrap_err();
        assert!(
            error.as_str().unwrap_or("").contains("not allowed"),
            "{cmd}: {error}"
        );
    }
}

#[test]
fn the_recent_files_come_as_a_finished_search() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    // No file here has a "last opened" record (that is set in the core's
    // tests), so the list is empty, but finished and kept like a search.
    let view = invoke(&main, "get_recent_files", json!({})).unwrap();
    assert_eq!(view["state"], json!("done"), "{view}");
    assert_eq!(view["words"], json!(null));
    assert_eq!(view["results"], json!([]));
    assert_eq!(view["coverage"]["folders"], json!(1));
    let find_id = view["find_id"].as_str().unwrap();
    let again = invoke(&main, "get_find", json!({ "find_id": find_id })).unwrap();
    assert_eq!(again["state"], json!("done"));
}
