use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::OrganizeService;

fn app(service: Arc<OrganizeService>) -> App<MockRuntime> {
    traeva_app::app_builder(mock_builder(), service)
        .build(tauri::generate_context!())
        .unwrap()
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
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
    .map(|body| body.deserialize::<Value>().unwrap())
}

fn send_event(connection: &Value, body: &Value) -> (String, Value) {
    let address = connection["endpoint"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap();
    let token = connection["pairing_token"].as_str().unwrap();
    let body = serde_json::to_vec(body).unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(stream, "POST /v1/source/events HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
    stream.write_all(&body).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (head.to_string(), serde_json::from_str(body).unwrap())
}

#[test]
fn source_collection_to_user_confirmation_crosses_real_receiver_and_ipc() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-source-ipc-")
        .tempdir()
        .unwrap();
    let root = temp.path().canonicalize().unwrap().join("downloads");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("sample.txt");
    std::fs::write(&file, b"Synthetic source test.\n").unwrap();
    let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
    service.authorize_root(&root).unwrap();
    let app = app(service);
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let off = invoke(&window, "source_connection", json!({})).unwrap();
    assert_eq!(off["enabled"], false);
    assert_eq!(off["active"], false);
    assert!(off["pairing_token"].is_null());
    invoke(&window, "source_set_enabled", json!({"enabled": true})).unwrap();
    let connection = invoke(&window, "source_connection", json!({})).unwrap();
    assert_eq!(connection["active"], true);
    let event = json!({
        "profile_id":"e63b3204-31d1-4bad-981a-289f4c50f728", "download_id":"9", "started_at_ms":10,
        "completed_at_ms":20,"state":"complete","filename":file.to_str().unwrap(),
        "initial_url":"https://example.test/download?private=fixture",
        "final_url":"https://cdn.example.test/sample.txt","referrer":null,
        "file_size":std::fs::metadata(&file).unwrap().len(),"incognito":false,"explicit_history":true
    });
    let mut automatic = event.clone();
    automatic["explicit_history"] = json!(false);
    let (headers, ignored) = send_event(&connection, &automatic);
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert_eq!(ignored["outcome"], "ignored");
    assert!(
        invoke(&window, "source_state", json!({})).unwrap()["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (headers, outcome) = send_event(&connection, &event);
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert_eq!(outcome["outcome"], "saved");
    let source_id = outcome["source_id"].as_str().unwrap();
    let state = invoke(&window, "source_state", json!({})).unwrap();
    let record = &state["records"][0];
    assert_eq!(record["status"], "likely");
    assert!(record["association"].is_null());
    let file_id = record["candidates"][0]["file_id"].as_str().unwrap();
    let confirmed = invoke(
        &window,
        "source_confirm",
        json!({
            "source_id":source_id,"file_id":file_id,"relation":"download","note":"Chosen in test"
        }),
    )
    .unwrap();
    assert_eq!(confirmed["status"], "user_confirmed");
    assert_eq!(confirmed["association"]["basis"], "user_confirmed");
    let (_, repeated) = send_event(&connection, &event);
    assert_eq!(repeated["source_id"], source_id);
    let state = invoke(&window, "source_state", json!({})).unwrap();
    assert_eq!(state["records"].as_array().unwrap().len(), 1);
    assert_eq!(state["records"][0]["status"], "user_confirmed");
    assert_eq!(
        state["records"][0]["imported_at_ms"],
        record["imported_at_ms"]
    );
    invoke(
        &window,
        "source_unlink",
        json!({"source_id":source_id,"note":"Correction"}),
    )
    .unwrap();
    invoke(
        &window,
        "source_reject",
        json!({"source_id":source_id,"file_id":file_id}),
    )
    .unwrap();
    let state = invoke(&window, "source_state", json!({})).unwrap();
    assert!(state["records"][0]["association"].is_null());
    assert_eq!(std::fs::read(&file).unwrap(), b"Synthetic source test.\n");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    invoke(&window, "source_set_enabled", json!({"enabled":false})).unwrap();
}

#[test]
fn pairing_secret_is_unavailable_to_other_windows() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
    let app = app(service);
    let window = WebviewWindowBuilder::new(&app, "other", Default::default())
        .build()
        .unwrap();
    let error = invoke(&window, "source_connection", json!({})).unwrap_err();
    assert!(error.as_str().unwrap().contains("not allowed"));
    let error = invoke(&window, "source_set_enabled", json!({"enabled":true})).unwrap_err();
    assert!(error.as_str().unwrap().contains("not allowed"));
}
