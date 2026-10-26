//! Memory's record commands (#31, family_commands.rs) through the real
//! command handlers and window permissions, on a mock runtime. The picker
//! command `start_memory_record` is not called (the mock has no picker);
//! its core part, `start_memory_at`, is tested in the core.

use std::sync::Arc;

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};
use traeva_core::OrganizeService;

const UNKNOWN: &str = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";

fn app(service: Arc<OrganizeService>) -> App<MockRuntime> {
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

fn code(error: &Value) -> &str {
    error["code"].as_str().unwrap_or_default()
}

#[test]
fn every_record_command_reaches_the_core_from_the_main_window_only() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
    let app = app(service);
    let main = window(&app, "main");

    assert_eq!(
        invoke(&main, "list_memory_records", json!({})).unwrap(),
        json!([])
    );
    for (cmd, args, unknown_code) in [
        ("start_memory_for_file", "file_id", "unknown_file"),
        ("stop_memory_record", "chain_id", "unknown_record"),
        ("get_memory_family", "family_id", "unknown_family"),
        ("get_candidate_capture", "chain_id", "unknown_record"),
        ("get_pending_candidates", "chain_id", "unknown_record"),
    ] {
        let bad = invoke(&main, cmd, json!({ args: "../../etc" })).unwrap_err();
        assert_eq!(code(&bad), "invalid_input", "{cmd}: {bad}");
        let unknown = invoke(&main, cmd, json!({ args: UNKNOWN })).unwrap_err();
        assert_eq!(code(&unknown), unknown_code, "{cmd}: {unknown}");
        let missing = invoke(&main, cmd, json!({})).unwrap_err();
        assert!(
            missing
                .as_str()
                .unwrap_or_default()
                .contains("missing required key"),
            "{cmd}: {missing}"
        );
    }

    // Requests with fields the core does not know are refused.
    let extra = invoke(
        &main,
        "set_candidate_capture",
        json!({"request": {"chain_id": UNKNOWN, "on": false, "path": "/etc"}}),
    )
    .unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );
    let unknown = invoke(
        &main,
        "set_candidate_capture",
        json!({"request": {"chain_id": UNKNOWN, "on": false}}),
    )
    .unwrap_err();
    assert_eq!(code(&unknown), "unknown_record");
    let decide = invoke(
        &main,
        "decide_pending_candidates",
        json!({"request": {"chain_id": UNKNOWN, "page_token": "a".repeat(64),
                           "current_object": {"kind": "independent"}}}),
    )
    .unwrap_err();
    assert_eq!(code(&decide), "unknown_record");
    let relocate = |request: Value| {
        invoke(
            &main,
            "relocate_memory_record",
            json!({ "request": request }),
        )
    };
    let request = json!({"chain_id": UNKNOWN, "root_id": UNKNOWN, "relative_path": "todo.txt",
                         "page_token": "a".repeat(64)});
    assert_eq!(
        code(&relocate(request.clone()).unwrap_err()),
        "unknown_record"
    );
    let mut bad = request.clone();
    bad["chain_id"] = json!("../../etc");
    assert_eq!(code(&relocate(bad).unwrap_err()), "invalid_input");
    let mut extra = request;
    extra["path"] = json!("/etc/hosts");
    let extra = relocate(extra).unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );

    assert_eq!(
        invoke(&main, "list_memory_works", json!({})).unwrap(),
        json!([])
    );
    let merge = invoke(
        &main,
        "preview_memory_merge",
        json!({"family_id": UNKNOWN, "other_family_id": "../x"}),
    )
    .unwrap_err();
    assert_eq!(code(&merge), "invalid_input");
    let merge = invoke(
        &main,
        "preview_memory_merge",
        json!({"family_id": UNKNOWN, "other_family_id": UNKNOWN}),
    )
    .unwrap_err();
    assert_eq!(code(&merge), "unknown_family");
    let extra = invoke(
        &main,
        "merge_memory_works",
        json!({"request": {"family_id": UNKNOWN, "other_family_id": UNKNOWN,
                           "page_token": "a".repeat(64), "keep": true}}),
    )
    .unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );
    let moved = invoke(
        &main,
        "preview_memory_move",
        json!({"query": {"family_id": UNKNOWN, "file_ids": [UNKNOWN],
                         "target": {"kind": "new_work"}}}),
    )
    .unwrap_err();
    assert_eq!(code(&moved), "unknown_family");
    let extra = invoke(
        &main,
        "move_memory_members",
        json!({"request": {"family_id": UNKNOWN, "file_ids": [UNKNOWN],
                           "target": {"kind": "new_work"}, "page_token": "a".repeat(64),
                           "why": 1}}),
    )
    .unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );
    let relation = invoke(
        &main,
        "preview_memory_relation",
        json!({"edge_id": UNKNOWN}),
    )
    .unwrap_err();
    assert_eq!(code(&relation), "unknown_relation");
    let extra = invoke(
        &main,
        "correct_memory_relation",
        json!({"request": {"edge_id": UNKNOWN, "action": "revoke",
                           "page_token": "a".repeat(64), "why": "x"}}),
    )
    .unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );
    let association = invoke(
        &main,
        "preview_memory_association_for_file",
        json!({"family_id": UNKNOWN, "file_id": UNKNOWN}),
    )
    .unwrap_err();
    assert_eq!(code(&association), "unknown_file");
    let extra = invoke(
        &main,
        "associate_memory_file",
        json!({"request": {"family_id": UNKNOWN, "page_token": "a".repeat(64),
                           "subject": {"kind": "file", "file_id": UNKNOWN},
                           "choice": {"kind": "unknown", "why": "x"}}}),
    )
    .unwrap_err();
    assert!(
        extra.as_str().unwrap_or_default().contains("unknown field"),
        "{extra}"
    );

    let other = window(&app, "other");
    for cmd in [
        "list_memory_records",
        "start_memory_record",
        "start_memory_for_file",
        "stop_memory_record",
        "get_memory_family",
        "get_candidate_capture",
        "set_candidate_capture",
        "get_pending_candidates",
        "decide_pending_candidates",
        "preview_memory_relocation",
        "relocate_memory_record",
        "list_memory_works",
        "preview_memory_merge",
        "merge_memory_works",
        "preview_memory_association",
        "preview_memory_association_for_file",
        "associate_memory_file",
        "preview_memory_relation",
        "correct_memory_relation",
        "preview_memory_move",
        "move_memory_members",
    ] {
        let refused = invoke(&other, cmd, json!({})).unwrap_err();
        assert!(
            refused.as_str().unwrap_or_default().contains("not allowed"),
            "{cmd}: {refused}"
        );
    }
}

#[test]
fn a_record_turned_on_in_the_core_is_listed_with_its_fields() {
    // Not ".tmp…": a hidden folder cannot be authorized.
    let temp = tempfile::Builder::new()
        .prefix("traeva-family-ipc-")
        .tempdir()
        .unwrap();
    let root = temp.path().canonicalize().unwrap().join("notes");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("todo.txt"), b"synthetic\n").unwrap();
    let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
    let root_id = service.authorize_root(&root).unwrap().root_id;
    let file = service
        .register_file(&root_id, &root.join("todo.txt"))
        .unwrap()
        .file;
    let app = app(Arc::clone(&service));
    let main = window(&app, "main");

    let record = invoke(
        &main,
        "start_memory_for_file",
        json!({"file_id": file.file_id}),
    )
    .unwrap();
    let keys: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "candidate_capture",
            "chain_id",
            "family_id",
            "hold",
            "line_versions",
            "open_gaps",
            "origin_file_id",
            "pending",
            "permission_file_id",
            "place",
            "root_active",
            "same_content",
            "state",
            "waiting_for_relocation",
            "watch"
        ]
    );
    assert_eq!(record["state"], "active");
    assert_eq!(record["candidate_capture"], "off");
    assert_eq!(
        record["pending"],
        json!({"objects": 0, "snapshots": 0, "limit": 20})
    );
    assert_eq!(record["place"]["relative_path"], "todo.txt");
    assert_eq!(record["watch"]["versions"], 1);
    let listed = invoke(&main, "list_memory_records", json!({})).unwrap();
    assert_eq!(listed, json!([record.clone()]));

    let family = invoke(
        &main,
        "get_memory_family",
        json!({"family_id": record["family_id"]}),
    )
    .unwrap();
    assert_eq!(family["members"][0]["status"], "confirmed_system");
    assert_eq!(family["members"][0]["basis"], "memory_on");

    let stopped = invoke(
        &main,
        "stop_memory_record",
        json!({"chain_id": record["chain_id"]}),
    )
    .unwrap();
    assert_eq!(stopped["state"], "stopped");
}
