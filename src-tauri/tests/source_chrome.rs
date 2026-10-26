//! Opt-in real Chrome → production receiver/core → mock Tauri IPC test.
//! This does not exercise a real desktop WebView.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};
use traeva_core::{OrganizeService, SourceDownloadEvent};

fn context() -> tauri::Context<MockRuntime> {
    tauri::generate_context!()
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Value {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.to_string(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .unwrap()
    .deserialize::<Value>()
    .unwrap()
}

// Private pipe driver for a real receiver/core restart with no desktop instance.
#[test]
#[ignore = "only started by the isolated browser-recovery helper"]
fn source_recovery_receiver_driver() {
    assert_eq!(std::env::var("TRAEVA_SOURCE_RECOVERY").as_deref(), Ok("1"));
    let base = PathBuf::from(std::env::var_os("TRAEVA_SOURCE_RECOVERY_BASE").unwrap())
        .canonicalize()
        .unwrap();
    assert!(
        base.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("traeva-source-recovery-")
    );
    let root = base.join("downloads");
    let app_data = base.join("app-data");
    assert!(root.is_dir());
    let service = Arc::new(OrganizeService::start(app_data));
    if service.list_roots().unwrap().is_empty() {
        service.authorize_root(&root).unwrap();
        service.set_source_enabled(true).unwrap();
    }
    let app = traeva_app::app_builder(mock_builder(), service)
        .build(context())
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    println!(
        "SOURCE_TEST {}",
        invoke(&window, "source_connection", json!({}))
    );
    std::io::stdout().flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let cmd = request["cmd"].as_str().unwrap();
        if cmd == "quit" {
            break;
        }
        assert!(matches!(
            cmd,
            "source_state" | "source_connection" | "source_set_enabled" | "source_confirm"
        ));
        println!(
            "SOURCE_TEST {}",
            invoke(&window, cmd, request["args"].clone())
        );
        std::io::stdout().flush().unwrap();
    }
    // MockRuntime retains managed handles; exiting releases this process's lock.
    std::process::exit(0);
}

fn capture(base: &Path) {
    let root = base.join("downloads");
    let app_data = base.join("app-data");
    std::fs::create_dir(&root).unwrap();
    let service = Arc::new(OrganizeService::start(app_data.clone()));
    service.authorize_root(&root).unwrap();
    let app = traeva_app::app_builder(mock_builder(), Arc::clone(&service))
        .build(context())
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    invoke(&window, "source_set_enabled", json!({"enabled":true}));
    let connection = invoke(&window, "source_connection", json!({}));
    assert_eq!(connection["active"], true);
    let configuration = json!({
        "endpoint":connection["endpoint"],"pairing_token":connection["pairing_token"],
        "root":root,"app_data":app_data,
    });
    let config_path = app_data.join("chrome-test-connection.json");
    let mut config = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&config_path)
        .unwrap();
    config
        .write_all(&serde_json::to_vec(&configuration).unwrap())
        .unwrap();
    drop(config);
    let helper = std::env::var_os("TRAEVA_SOURCE_CHROME_HELPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../extensions/chrome/test/browser-core.cjs")
        });
    let status = Command::new("node")
        .arg(helper)
        .env("TRAEVA_SOURCE_TEST_CONFIG", &config_path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    std::fs::remove_file(config_path).unwrap();
    assert!(status.success(), "The real Chrome helper failed.");
    let delivery: Value =
        serde_json::from_slice(&std::fs::read(app_data.join("chrome-delivery.json")).unwrap())
            .unwrap();
    let browser_environment: Value = serde_json::from_slice(
        &std::fs::read(app_data.join("chrome-adapter/environment.json")).unwrap(),
    )
    .unwrap();
    let state = invoke(&window, "source_state", json!({}));
    let records = state["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    let downloaded = PathBuf::from(record["filename"].as_str().unwrap());
    assert!(downloaded.starts_with(&root));
    let original_bytes = std::fs::read(&downloaded).unwrap();
    let metadata = std::fs::metadata(&downloaded).unwrap();
    assert_eq!(record["download_state"], "complete");
    assert_eq!(record["status"], "likely");
    assert!(record["title"].is_null());
    assert!(record["association"].is_null());
    assert_ne!(record["initial_url"], record["final_url"]);
    assert_eq!(record["candidates"].as_array().unwrap().len(), 1);
    let source_id = record["source_id"].as_str().unwrap();
    let file_id = record["candidates"][0]["file_id"].as_str().unwrap();
    assert_eq!(record["candidates"][0]["can_confirm"], true);
    let confirmed = invoke(
        &window,
        "source_confirm",
        json!({
            "source_id":source_id,"file_id":file_id,"relation":"download","note":"Confirmed in isolated real Chrome test"
        }),
    );
    assert_eq!(confirmed["status"], "user_confirmed");
    assert_eq!(confirmed["association"]["basis"], "user_confirmed");
    invoke(&window, "source_set_enabled", json!({"enabled":false}));
    std::fs::write(app_data.join("expected-bytes.bin"), original_bytes).unwrap();
    std::fs::write(app_data.join("expected.json"), serde_json::to_vec(&json!({
        "record":record,"confirmed":confirmed,"delivery":delivery,"environment":browser_environment,
        "object":{"dev":metadata.dev(),"ino":metadata.ino(),"links":metadata.nlink(),"size":metadata.len()}
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "explicitly opt in with TRAEVA_SOURCE_CHROME=1; launches an isolated real Chrome Profile"]
fn real_chrome_download_reaches_candidates_confirmation_and_durable_storage() {
    assert_eq!(std::env::var("TRAEVA_SOURCE_CHROME").as_deref(), Ok("1"));
    if let Some(base) = std::env::var_os("TRAEVA_SOURCE_CHROME_CAPTURE") {
        capture(Path::new(&base));
        return;
    }
    let temp = tempfile::Builder::new()
        .prefix("traeva-source-chrome-")
        .tempdir()
        .unwrap();
    let base = temp.path().canonicalize().unwrap();
    // MockRuntime retains managed handles. A child process gives a real restart
    // without unmanaging live state or bypassing the app-data lock.
    let captured = Command::new(std::env::current_exe().unwrap())
        .args([
            "real_chrome_download_reaches_candidates_confirmation_and_durable_storage",
            "--exact",
            "--ignored",
            "--nocapture",
        ])
        .env("TRAEVA_SOURCE_CHROME_CAPTURE", &base)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(
        captured.success(),
        "Real Chrome capture and mock IPC confirmation must pass before restart."
    );
    let root = base.join("downloads");
    let app_data = base.join("app-data");
    let expected: Value =
        serde_json::from_slice(&std::fs::read(app_data.join("expected.json")).unwrap()).unwrap();
    let record = &expected["record"];
    let source_id = record["source_id"].as_str().unwrap();
    let file_id = expected["confirmed"]["association"]["file_id"]
        .as_str()
        .unwrap();
    let downloaded = PathBuf::from(record["filename"].as_str().unwrap());
    let original_bytes = std::fs::read(app_data.join("expected-bytes.bin")).unwrap();
    let reopened = OrganizeService::start(app_data);
    let state = serde_json::to_value(reopened.source_state().unwrap()).unwrap();
    let saved = &state["records"][0];
    assert_eq!(saved["source_id"], source_id);
    assert_eq!(saved["status"], "user_confirmed");
    assert_eq!(saved["association"]["file_id"], file_id);
    assert_eq!(saved["imported_at_ms"], record["imported_at_ms"]);
    let mut event: SourceDownloadEvent =
        serde_json::from_value(expected["delivery"]["event"].clone()).unwrap();
    // Capture process explicitly disabled the collector before exiting.
    event.explicit_history = true;
    reopened.set_source_enabled(true).unwrap();
    assert_eq!(
        reopened
            .ingest_source_event(event)
            .unwrap()
            .source_id
            .as_deref(),
        Some(source_id)
    );
    let after = serde_json::to_value(reopened.source_state().unwrap()).unwrap();
    assert_eq!(after["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["records"][0]["imported_at_ms"],
        record["imported_at_ms"]
    );
    assert_eq!(after["records"][0]["status"], "user_confirmed");
    reopened.set_source_enabled(false).unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(std::fs::read(&downloaded).unwrap(), original_bytes);
    assert_eq!(
        std::fs::metadata(&downloaded).unwrap().ino(),
        expected["object"]["ino"].as_u64().unwrap()
    );
    if let Some(report) = std::env::var_os("TRAEVA_SOURCE_CHROME_REPORT") {
        std::fs::write(report, serde_json::to_vec_pretty(&json!({
            "result":"PASS","scope":"real Chrome to production receiver/core to mock Tauri IPC, then process restart",
            "download_id":saved["download_id"],"source_id":source_id,"file_id":file_id,
            "browser":expected["environment"]["browser"],"download_sha256":expected["delivery"]["sha256"],
            "observed_file":expected["object"],
            "first_state":"likely","confirmed_state":"user_confirmed",
            "restart_retained":true,"import_time_retained":true,
            "duplicate_replay_retained_one_record":true,"user_file_unchanged":true,
            "title_unknown":true,"redirect_urls_distinct":true,
            "automatic_verified":"not implemented; experimental FAIL retained",
            "desktop_webview":"not tested by this harness",
        })).unwrap()).unwrap();
    }
}
