//! Memory's commands (#33 changes, #34 search, #35 opening and exporting an
//! old version)
//! through the IPC boundary, on Tauri's mock runtime with the app's real
//! config and capability.
//!
//! No model settings are ever saved here: this test build uses the app's
//! core, whose key store is the real Keychain. Without settings nothing is
//! sent and the Keychain is never looked at. A copy is never opened here
//! either (that would start an app), and the save panel of an export is not
//! opened; the core's tests cover both.

use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::{CheckOutcome, OrganizeService};

const UNKNOWN: &str = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";

/// An app whose one synthetic file has two saved versions, compared on
/// this Mac and not described (nothing was agreed to).
struct Ipc {
    _app_data: tempfile::TempDir,
    root: tempfile::TempDir,
    app: App<MockRuntime>,
    /// The app's own core, for what the window cannot do here (the save
    /// panel's preview).
    service: Arc<OrganizeService>,
    root_id: String,
    file_id: String,
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
        let essay = root.path().join("essay.txt");
        fs::write(&essay, "Smith 2019 says A.\n").unwrap();
        let service = Arc::new(OrganizeService::start(app_data.path().join("Traeva")));
        let root_id = service.authorize_root(root.path()).unwrap().root_id;
        let file_id = service
            .register_file(&root_id, &essay)
            .unwrap()
            .file
            .file_id;
        service.start_memory(&file_id).unwrap();
        fs::write(&essay, "Smith (2019, p. 4) says A.\n").unwrap();
        let start = Instant::now();
        while !service
            .check_memory()
            .unwrap()
            .iter()
            .any(|check| matches!(check.outcome, CheckOutcome::Saved { .. }))
        {
            assert!(start.elapsed() < Duration::from_secs(10), "no new version");
            thread::sleep(Duration::from_millis(100));
        }
        service.analyze_memory().unwrap();
        let app = traeva_app::app_builder(mock_builder(), Arc::clone(&service))
            .build(tauri::generate_context!())
            .unwrap();
        Ipc {
            _app_data: app_data,
            root,
            app,
            service,
            root_id,
            file_id,
        }
    }

    fn main(&self) -> WebviewWindow<MockRuntime> {
        WebviewWindowBuilder::new(&self.app, "main", Default::default())
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

/// Tauri's own refusals (permissions, arguments) are plain strings.
fn refusal(error: &Value) -> &str {
    error.as_str().unwrap_or("")
}

#[test]
fn the_history_and_the_agreement_reach_the_core() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let file = json!({ "file_id": ipc.file_id });

    let pairs = invoke(&main, "get_memory_changes", file.clone()).unwrap();
    assert_eq!(pairs.as_array().unwrap().len(), 1);
    assert_eq!(pairs[0]["before"]["sequence"], 1);
    assert_eq!(pairs[0]["after"]["sequence"], 2);
    assert_eq!(pairs[0]["state"]["kind"], "not_permitted");
    assert_eq!(pairs[0]["changes"], json!([]));

    let consent = invoke(&main, "get_memory_consent", file.clone()).unwrap();
    assert_eq!(consent["agreed"], false);
    // No model is set up, so there is nothing to agree to yet.
    let error = invoke(
        &main,
        "set_memory_consent",
        json!({ "file_id": ipc.file_id, "agree": true }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "model_not_configured");

    let error = invoke(&main, "get_memory_changes", json!({ "file_id": UNKNOWN })).unwrap_err();
    assert_eq!(code(&error), "unknown_file");
    for change_id in ["../x", UNKNOWN] {
        let error =
            invoke(&main, "get_change_lines", json!({ "change_id": change_id })).unwrap_err();
        assert_eq!(code(&error), "invalid_input", "{change_id}");
    }
}

#[test]
fn a_search_without_an_agreement_ends_at_once_and_sends_nothing() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let view = invoke(
        &main,
        "start_memory_search",
        json!({ "file_id": ipc.file_id, "question": "before I fixed the citation" }),
    )
    .unwrap();
    // Nothing may be sent for this file, so no model is asked at all.
    assert_eq!(view["state"]["kind"], "not_permitted");
    assert!(
        !view.to_string().contains("citation"),
        "the question is never sent back"
    );
    let search = json!({ "search_id": view["search_id"] });
    let view = invoke(&main, "get_memory_search", search.clone()).unwrap();
    assert_eq!(view["state"]["kind"], "not_permitted");

    // Nothing to answer or retry now; a cancel ends it.
    let answer = json!({ "search_id": view["search_id"], "answer": { "kind": "none_of_these" } });
    let error = invoke(&main, "answer_memory_search", answer).unwrap_err();
    assert_eq!(code(&error), "wrong_stage");
    let again = invoke(&main, "retry_memory_search", search.clone()).unwrap();
    assert_eq!(again["state"]["kind"], "not_permitted");
    let cancelled = invoke(&main, "cancel_memory_search", search).unwrap();
    assert_eq!(cancelled["state"]["kind"], "cancelled");
}

#[test]
fn search_and_open_input_that_does_not_fit_is_refused() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let error = invoke(&main, "get_memory_search", json!({ "search_id": UNKNOWN })).unwrap_err();
    assert_eq!(code(&error), "invalid_input");
    // An answer carries only an option, a candidate ID, or "none of these".
    let answer = json!({ "kind": "pick", "candidate_id": "K1", "snapshot_id": UNKNOWN });
    let error = invoke(
        &main,
        "answer_memory_search",
        json!({ "search_id": UNKNOWN, "answer": answer }),
    )
    .unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `snapshot_id`"),
        "{error}"
    );
    // Opening takes a saved version's ID, never a path.
    let error = invoke(
        &main,
        "open_memory_version",
        json!({ "snapshot_id": "/etc/hosts" }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "invalid_input");
    let error = invoke(
        &main,
        "open_memory_version",
        json!({ "snapshot_id": UNKNOWN }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "unknown_snapshot");
    let error = invoke(
        &main,
        "open_memory_version",
        json!({ "path": "/etc/hosts" }),
    )
    .unwrap_err();
    assert!(refusal(&error).contains("missing required key snapshot_id"));
}

#[test]
fn an_export_reaches_the_core_and_never_replaces_a_file() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let pairs = invoke(
        &main,
        "get_memory_changes",
        json!({ "file_id": ipc.file_id }),
    )
    .unwrap();
    let first = pairs[0]["before"]["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    // The save panel cannot open here: its preview comes from the core.
    let written = ipc.root.path().join("essay v1.txt");
    let preview = ipc
        .service
        .preview_export(&first, &ipc.root_id, &written)
        .unwrap();
    let request = json!({ "request": {
        "snapshot_id": first,
        "root_id": ipc.root_id,
        "folder": preview.folder,
        "name": preview.name,
        "page_token": preview.page_token,
    }});
    let done = invoke(&main, "export_memory_version", request.clone()).unwrap();
    // The root's path as authorized (macOS shows /var as /private/var).
    assert!(
        done["display_path"]
            .as_str()
            .unwrap()
            .ends_with("/essay v1.txt")
    );
    assert_eq!(fs::read(&written).unwrap(), b"Smith 2019 says A.\n");
    let error = invoke(&main, "export_memory_version", request).unwrap_err();
    assert_eq!(code(&error), "target_conflict");
    assert_eq!(fs::read(&written).unwrap(), b"Smith 2019 says A.\n");
}

#[test]
fn export_input_that_does_not_fit_is_refused_and_only_the_main_window_may_export() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let fields = json!({
        "snapshot_id": UNKNOWN,
        "root_id": ipc.root_id,
        "folder": "",
        "name": "x.txt",
        "page_token": "a".repeat(64),
    });
    // A field the core does not know (such as "overwrite") is refused.
    let mut extra = fields.clone();
    extra["overwrite"] = json!(true);
    let error = invoke(&main, "export_memory_version", json!({ "request": extra })).unwrap_err();
    assert!(
        refusal(&error).contains("unknown field `overwrite`"),
        "{error}"
    );
    let mut missing = fields.clone();
    missing.as_object_mut().unwrap().remove("page_token");
    let error = invoke(
        &main,
        "export_memory_version",
        json!({ "request": missing }),
    )
    .unwrap_err();
    assert!(
        refusal(&error).contains("missing field `page_token`"),
        "{error}"
    );
    let error = invoke(&main, "export_memory_version", json!({ "request": fields })).unwrap_err();
    assert_eq!(code(&error), "unknown_snapshot");
    assert_eq!(
        fs::read_dir(ipc.root.path()).unwrap().count(),
        1,
        "nothing written"
    );

    let other = WebviewWindowBuilder::new(&ipc.app, "other", Default::default())
        .build()
        .unwrap();
    for cmd in ["choose_memory_export", "export_memory_version"] {
        let refused = invoke(&other, cmd, json!({})).unwrap_err();
        assert!(
            refusal(&refused).contains("not allowed"),
            "{cmd}: {refused}"
        );
    }
}

#[test]
fn a_saved_word_version_is_previewed_and_input_that_does_not_fit_is_refused() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let report = ipc.root.path().join("report.docx");
    fs::write(
        &report,
        include_bytes!("../../crates/traeva-core/tests/fixtures/docx/word-basic-v1.docx"),
    )
    .unwrap();
    let file_id = ipc
        .service
        .register_file(&ipc.root_id, &report)
        .unwrap()
        .file
        .file_id;
    ipc.service.start_memory(&file_id).unwrap();
    let snapshot_id = ipc.service.list_snapshots(&file_id).unwrap()[0]
        .snapshot_id
        .clone();

    let preview = invoke(
        &main,
        "preview_memory_docx",
        json!({ "snapshot_id": snapshot_id }),
    )
    .unwrap();
    assert_eq!(preview["snapshot"]["snapshot_id"], json!(snapshot_id));
    assert_eq!(
        preview["extraction"]["summary"]["reader"],
        json!("docx-cache-local-1")
    );
    let text = preview["extraction"]["text"].as_str().unwrap();
    assert!(text.contains("Traeva DOCX validation"));

    // A text file's version previews as unsupported; it is not an error.
    let essay = ipc.service.list_snapshots(&ipc.file_id).unwrap()[0]
        .snapshot_id
        .clone();
    let other = invoke(
        &main,
        "preview_memory_docx",
        json!({ "snapshot_id": essay }),
    )
    .unwrap();
    assert_eq!(
        other["extraction"]["summary"]["status"],
        json!("unsupported")
    );
    assert_eq!(other["extraction"]["text"], json!(""));

    // A saved version's ID only, never a path.
    let error = invoke(
        &main,
        "preview_memory_docx",
        json!({ "snapshot_id": report.to_str().unwrap() }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "invalid_input");
    let error = invoke(
        &main,
        "preview_memory_docx",
        json!({ "snapshot_id": UNKNOWN }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "unknown_snapshot");
    let error = invoke(
        &main,
        "preview_memory_docx",
        json!({ "path": report.to_str().unwrap() }),
    )
    .unwrap_err();
    assert!(refusal(&error).contains("missing required key snapshot_id"));
}

#[test]
fn a_preview_reads_only_the_saved_version_its_id_names() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let essay = ipc.service.list_snapshots(&ipc.file_id).unwrap()[0]
        .snapshot_id
        .clone();
    let report = ipc.root.path().join("report.docx");
    fs::write(
        &report,
        include_bytes!("../../crates/traeva-core/tests/fixtures/docx/word-basic-v1.docx"),
    )
    .unwrap();
    // Tauri ignores keys a command does not take (contract §4.7, D103-01):
    // a path next to the ID is never read; the ID's saved version is.
    let preview = invoke(
        &main,
        "preview_memory_docx",
        json!({ "snapshot_id": essay, "path": report.to_str().unwrap() }),
    )
    .unwrap();
    assert_eq!(preview["snapshot"]["snapshot_id"], json!(essay));
    assert_eq!(
        preview["extraction"]["summary"]["status"],
        json!("unsupported")
    );
    assert_eq!(preview["extraction"]["text"], json!(""));
}

#[test]
fn any_saved_version_is_previewed_through_the_window() {
    let ipc = Ipc::new();
    let main = ipc.main();
    let essay = ipc.service.list_snapshots(&ipc.file_id).unwrap()[0]
        .snapshot_id
        .clone();
    let preview = invoke(
        &main,
        "preview_memory_text",
        json!({ "snapshot_id": essay, "path": "/etc/hosts" }),
    )
    .unwrap();
    assert_eq!(preview["snapshot"]["snapshot_id"], json!(essay));
    assert_eq!(
        preview["extraction"]["summary"]["reader"],
        json!("txt-md-1")
    );
    assert_eq!(preview["extraction"]["text"], json!("Smith 2019 says A.\n"));
    let error = invoke(
        &main,
        "preview_memory_text",
        json!({ "snapshot_id": UNKNOWN }),
    )
    .unwrap_err();
    assert_eq!(code(&error), "unknown_snapshot");
    let error = invoke(
        &main,
        "preview_memory_text",
        json!({ "path": "/etc/hosts" }),
    )
    .unwrap_err();
    assert!(refusal(&error).contains("missing required key snapshot_id"));
}
