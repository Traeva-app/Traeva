//! The automatic mode for dropped files (#168) through the IPC boundary, on
//! Tauri's mock runtime with the app's real config and capability. No model
//! settings are saved, so nothing is ever sent; the core's tests cover the
//! moves themselves.

use std::fs;
use std::sync::Arc;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::{AUTO_DROP_NOTICE_VERSION, OrganizeService};

const UNKNOWN: &str = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";

struct Ipc {
    _app_data: tempfile::TempDir,
    root: tempfile::TempDir,
    app: App<MockRuntime>,
    service: Arc<OrganizeService>,
    root_id: String,
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
        fs::create_dir(root.path().join("Travel")).unwrap();
        fs::write(root.path().join("trip.md"), "Trip to Kyoto.\n").unwrap();
        let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
        let root_id = service.authorize_root(root.path()).unwrap().root_id;
        let app = traeva_app::app_builder(mock_builder(), Arc::clone(&service))
            .build(tauri::generate_context!())
            .unwrap();
        Ipc {
            _app_data: app_data,
            root,
            app,
            service,
            root_id,
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

fn refusal(error: &Value) -> &str {
    error.as_str().unwrap_or("")
}

#[test]
fn the_settings_reach_the_core_and_refuse_what_does_not_fit() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    let off = invoke(&main, "get_auto_drop_settings", json!({})).unwrap();
    assert_eq!(off["enabled"], json!(false));
    assert_eq!(off["notice_version"], json!(AUTO_DROP_NOTICE_VERSION));

    let request = json!({
        "enable": true,
        "notice_version": AUTO_DROP_NOTICE_VERSION,
        "target_root_id": ipc.root_id,
        "scopes": ["Travel"]
    });
    let on = invoke(
        &main,
        "set_auto_drop_settings",
        json!({ "request": request }),
    )
    .unwrap();
    assert_eq!(on["enabled"], json!(true));
    assert_eq!(on["scopes"][0]["relative_path"], json!("Travel"));

    // An unknown field inside the request is refused; nothing changes.
    let mut extra = request.clone();
    extra["move_without_asking_everything"] = json!(true);
    let error = invoke(&main, "set_auto_drop_settings", json!({ "request": extra })).unwrap_err();
    assert!(refusal(&error).contains("unknown field"), "{error}");
    // A folder that does not exist is refused by the core.
    let mut nowhere = request.clone();
    nowhere["scopes"] = json!(["Nowhere"]);
    let error = invoke(
        &main,
        "set_auto_drop_settings",
        json!({ "request": nowhere }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "invalid_input");
}

#[test]
fn the_queue_commands_check_their_ids_and_only_the_main_window_may_call_them() {
    let ipc = Ipc::new();
    let main = ipc.window("main");
    let queue = invoke(&main, "get_drop_queue", json!({})).unwrap();
    assert_eq!(queue["items"], json!([]));
    for (cmd, args) in [
        ("authorize_drop_source", json!({ "drop_id": "not-an-id" })),
        ("cancel_drop", json!({ "drop_id": UNKNOWN })),
        (
            "start_drop_operation",
            json!({ "drop_id": UNKNOWN, "target_root_id": ipc.root_id }),
        ),
    ] {
        let error = invoke(&main, cmd, args).unwrap_err();
        assert_eq!(code(&error), "invalid_input", "{cmd}: {error}");
    }
    assert!(invoke(&main, "continue_drops", json!({})).is_ok());

    // A file the system handed over shows in the queue (the mode is off:
    // the normal flow), and the window may start its operation.
    let queue = ipc
        .service
        .receive_drops(&[ipc.root.path().join("trip.md")])
        .unwrap();
    let drop_id = queue.items[0].drop_id.clone();
    ipc.service.run_drops();
    let queue = invoke(&main, "get_drop_queue", json!({})).unwrap();
    assert_eq!(queue["items"][0]["state"], json!("handed_over"));
    let operation = invoke(
        &main,
        "start_drop_operation",
        json!({ "drop_id": drop_id, "target_root_id": ipc.root_id }),
    )
    .unwrap();
    assert_eq!(operation["stage"], json!("observing"));

    // Another window may call none of them.
    let other = ipc.window("other");
    for cmd in ["get_auto_drop_settings", "get_drop_queue", "continue_drops"] {
        let error = invoke(&other, cmd, json!({})).unwrap_err();
        assert!(refusal(&error).contains("not allowed"), "{cmd}: {error}");
    }
}
