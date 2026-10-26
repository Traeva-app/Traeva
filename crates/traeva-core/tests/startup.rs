//! Startup: lock → open → PRAGMA → integrity → migrate → ready, or a clear blocked
//! state. A blocked start never deletes, recreates or rewrites the database.

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use common::*;
use rusqlite::{Connection, OpenFlags};
use traeva_core::{
    BuildOutcome, ErrorCode, ErrorStage, LATEST_SCHEMA_VERSION, NextStep, OrganizeService,
    PlanRequest, PlanTarget,
};

#[test]
fn restart_keeps_data_and_does_not_migrate_again() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));

    let conn = open_for_inspection(&dir);
    // Valid evidence: the window lists roots now, and reads each one back.
    conn.execute(
        "INSERT INTO authorized_roots (root_id, display_path, evidence_json, status, granted_at_ms)
         VALUES ('root-1', '/synthetic/root',
                 '{\"dev\":1,\"ino\":2,\"birthtime_ns\":3,\"fs_type\":\"apfs\"}', 'active', 0)",
        [],
    )
    .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);

    let conn = open_for_inspection(&dir);
    let roots: i64 = conn
        .query_row("SELECT count(*) FROM authorized_roots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        roots, 1,
        "the row written before the restart must still be there"
    );
}

#[test]
fn second_instance_is_blocked_and_does_not_touch_the_database() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);

    let first = OrganizeService::start(dir.clone());
    expect_ready(&first);
    let bytes_before = fs::read(database_path(&dir)).unwrap();

    let second = OrganizeService::start(dir.clone());
    let error = expect_blocked(&second);
    assert_eq!(error.code, ErrorCode::AnotherInstanceRunning);
    assert_eq!(error.stage, ErrorStage::Storage);
    assert!(error.retryable);
    assert_eq!(error.allowed_next, vec![NextStep::RestartApp]);
    assert_eq!(fs::read(database_path(&dir)).unwrap(), bytes_before);

    // When the first instance exits, the lock is released and a new start works.
    drop(first);
    drop(second);
    let third = OrganizeService::start(dir);
    expect_ready(&third);
}

#[test]
fn newer_schema_is_refused_and_the_file_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(database_path(&dir)).unwrap();
    conn.execute_batch("CREATE TABLE from_the_future (x); PRAGMA user_version = 99;")
        .unwrap();
    drop(conn);
    let bytes_before = fs::read(database_path(&dir)).unwrap();

    let service = OrganizeService::start(dir.clone());
    let error = expect_blocked(&service);
    assert_eq!(error.code, ErrorCode::UnsupportedSchemaVersion);
    assert!(!error.retryable);
    drop(service);

    assert_eq!(fs::read(database_path(&dir)).unwrap(), bytes_before);
}

#[test]
fn corrupt_database_is_refused_and_the_file_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    fs::create_dir_all(&dir).unwrap();
    let garbage = b"this is not a SQLite database, only synthetic test bytes. ".repeat(100);
    fs::write(database_path(&dir), &garbage).unwrap();

    let service = OrganizeService::start(dir.clone());
    let error = expect_blocked(&service);
    assert_eq!(error.code, ErrorCode::DatabaseCorrupt);
    drop(service);

    assert_eq!(fs::read(database_path(&dir)).unwrap(), garbage);
}

#[test]
fn failed_migration_rolls_back_and_keeps_the_database() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    fs::create_dir_all(&dir).unwrap();
    // A version-0 database that already has a table named `files`, so migration 1
    // fails part-way through (after it has created `authorized_roots`).
    let conn = Connection::open(database_path(&dir)).unwrap();
    conn.execute_batch("CREATE TABLE files (note TEXT); INSERT INTO files VALUES ('keep me');")
        .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    let error = expect_blocked(&service);
    assert_eq!(error.code, ErrorCode::MigrationFailed);
    drop(service);

    let conn = open_for_inspection(&dir);
    assert_eq!(user_version(&conn), 0);
    assert_eq!(
        user_tables(&conn),
        ["files"],
        "migration 1 must be fully rolled back"
    );
    let note: String = conn
        .query_row("SELECT note FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(note, "keep me");
}

#[test]
fn unwritable_app_data_location_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("no-write");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();

    let service = OrganizeService::start(parent.join("Traeva"));
    let error = expect_blocked(&service);

    // Restore permissions so the temporary folder can be cleaned up.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.code, ErrorCode::AppDataUnavailable);
    assert!(error.retryable);
}

#[test]
fn app_data_path_that_is_a_file_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    fs::write(&dir, b"a regular file where the folder should be").unwrap();

    let service = OrganizeService::start(dir);
    assert_eq!(expect_blocked(&service).code, ErrorCode::AppDataUnavailable);
}

#[test]
fn read_only_database_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));
    let db = database_path(&dir);
    fs::set_permissions(&db, fs::Permissions::from_mode(0o444)).unwrap();

    let service = OrganizeService::start(dir);
    let error = expect_blocked(&service);
    drop(service);

    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(error.code, ErrorCode::DatabaseUnavailable);
}

#[test]
fn app_state_uses_the_shape_the_window_expects() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);

    let first = OrganizeService::start(dir.clone());
    let ready = serde_json::to_value(first.get_app_state().unwrap()).unwrap();
    assert_eq!(ready["storage"]["status"], "ready");
    assert_eq!(ready["storage"]["schema_version"], LATEST_SCHEMA_VERSION);
    assert_eq!(ready["storage"]["pragmas"]["journal_mode"], "delete");
    assert_eq!(ready["model"], "not_configured");

    let second = OrganizeService::start(dir);
    let blocked = serde_json::to_value(second.get_app_state().unwrap()).unwrap();
    let error = &blocked["storage"]["error"];
    assert_eq!(blocked["storage"]["status"], "blocked");
    assert_eq!(error["code"], "another_instance_running");
    assert_eq!(error["stage"], "storage");
    assert_eq!(error["retryable"], true);
    assert_eq!(error["allowed_next"], serde_json::json!(["restart_app"]));
    assert!(
        error["file_id"].is_null() && error["plan_id"].is_null() && error["action_id"].is_null()
    );
}

// ---------------------------------------------------------------------------
// The app-data folder moved into an authorized folder while Traeva was closed
// (#47, organize-spec.md §2.2): the start must notice before any write.
// ---------------------------------------------------------------------------

/// `<home>/data/Traeva` as app data, and `<home>/root` authorized, holding
/// one synthetic note and a "Notes" folder, with one preview that was never
/// confirmed. Returns the home folder, the root, the app data and the plan.
fn data_beside_an_authorized_folder() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let home = tempfile::Builder::new()
        .prefix("traeva-home-")
        .tempdir()
        .unwrap();
    let root = home.path().join("root");
    fs::create_dir_all(root.join("Notes")).unwrap();
    fs::write(root.join("note.txt"), "synthetic note\n").unwrap();
    let app_data = home.path().join("data").join("Traeva");

    let service = OrganizeService::start(app_data.clone());
    let root_id = service.authorize_root(&root).unwrap().root_id;
    let file = service
        .register_file(&root_id, &root.join("note.txt"))
        .unwrap()
        .file;
    let catalog = service.list_folders(&root_id).unwrap();
    let notes = catalog
        .folders
        .iter()
        .find(|f| f.relative_path == "Notes")
        .unwrap()
        .folder_id
        .clone();
    let outcome = service
        .build_plan(&PlanRequest {
            file_id: file.file_id,
            file_revision: file.file_revision,
            target_root_id: root_id,
            operation_id: None,
            target: PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id: notes,
                new_folders: Vec::new(),
            },
        })
        .unwrap();
    let BuildOutcome::Planned { plan } = outcome else {
        panic!("expected a plan");
    };
    (home, root, app_data, plan.plan_id)
}

/// Read only, after the start under test.
fn plan_lifecycle(app_data: &Path, plan_id: &str) -> String {
    let conn =
        Connection::open_with_flags(database_path(app_data), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    conn.query_row(
        "SELECT lifecycle FROM plans WHERE plan_id = ?1",
        [plan_id],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn a_data_folder_moved_into_an_authorized_folder_blocks_the_start_before_any_write() {
    let (home, root, app_data, plan_id) = data_beside_an_authorized_folder();
    // While Traeva is closed, the folder holding its data folder is moved
    // into the authorized folder, and a symbolic link takes its old place.
    let moved = root.join("internal-data");
    fs::rename(home.path().join("data"), &moved).unwrap();
    symlink(&moved, home.path().join("data")).unwrap();
    let before = snapshot::snapshot(&root);

    // Started with the old path, through the link.
    let service = OrganizeService::start(app_data.clone());
    let error = expect_blocked(&service);
    drop(service);

    assert_eq!(error.code, ErrorCode::OverlapsAppData);
    assert_eq!(error.stage, ErrorStage::Storage);
    assert_eq!(error.allowed_next, vec![NextStep::RestartApp]);
    // Contents, permissions, xattrs, mtime and ctime, including the
    // database and the lock file now inside the authorized folder.
    assert_eq!(snapshot::snapshot(&root), before);
    assert_eq!(plan_lifecycle(&moved.join("Traeva"), &plan_id), "open");

    // Started with the new path inside the authorized folder: the same.
    let service = OrganizeService::start(moved.join("Traeva"));
    assert_eq!(expect_blocked(&service).code, ErrorCode::OverlapsAppData);
    drop(service);
    assert_eq!(snapshot::snapshot(&root), before);
}

#[test]
fn a_data_folder_moved_into_an_authorized_folder_is_noticed_after_the_volume_was_renumbered() {
    let (home, root, app_data, _) = data_beside_an_authorized_folder();
    // A restart gave the volume another device number (10/15: 16777232
    // became 16777234), and the root was saved before volume UUIDs were.
    let conn = open_for_inspection(&app_data);
    conn.execute(
        "UPDATE authorized_roots SET evidence_json = json_remove(
             json_set(evidence_json, '$.dev', json_extract(evidence_json, '$.dev') + 2),
             '$.volume')",
        [],
    )
    .unwrap();
    drop(conn);
    let moved = root.join("internal-data");
    fs::rename(home.path().join("data"), &moved).unwrap();
    let before = snapshot::snapshot(&root);

    let service = OrganizeService::start(moved.join("Traeva"));
    assert_eq!(expect_blocked(&service).code, ErrorCode::OverlapsAppData);
    drop(service);
    assert_eq!(snapshot::snapshot(&root), before);
}

#[test]
fn a_data_folder_beside_the_authorized_folder_starts_normally() {
    // The control: nothing moved, so the start goes on and, as always,
    // ends the preview that was never confirmed.
    let (_home, _root, app_data, plan_id) = data_beside_an_authorized_folder();
    let service = OrganizeService::start(app_data.clone());
    expect_ready(&service);
    drop(service);
    assert_eq!(plan_lifecycle(&app_data, &plan_id), "invalidated");
}

#[test]
fn a_revoked_folder_does_not_count_unless_a_journal_is_left_over() {
    let (home, root, app_data, _plan) = data_beside_an_authorized_folder();
    let service = OrganizeService::start(app_data.clone());
    let root_id = service.list_roots().unwrap()[0].root_id.clone();
    service.revoke_root(&root_id).unwrap();
    drop(service);
    let moved = root.join("internal-data");
    fs::rename(home.path().join("data"), &moved).unwrap();
    symlink(&moved, home.path().join("data")).unwrap();
    let data = moved.join("Traeva");

    // A rollback journal left over from a crash could still undo the
    // revocation, so then every saved folder counts.
    let journal = data.join(format!("{}-journal", traeva_core::DATABASE_FILE_NAME));
    fs::write(&journal, vec![0u8; 512]).unwrap();
    let before = snapshot::snapshot(&root);
    let service = OrganizeService::start(app_data.clone());
    assert_eq!(expect_blocked(&service).code, ErrorCode::OverlapsAppData);
    drop(service);
    assert_eq!(snapshot::snapshot(&root), before);

    // Without it, a revoked folder is no longer authorized: Traeva starts.
    fs::remove_file(&journal).unwrap();
    let service = OrganizeService::start(app_data);
    expect_ready(&service);
}

#[test]
fn a_data_folder_that_cannot_be_checked_blocks_the_start() {
    // Readable database, but the folder itself cannot be opened (write and
    // search permission only): Traeva cannot tell where it is, and storage
    // could still have written into it. So it does not start.
    let (_home, root, app_data, _plan) = data_beside_an_authorized_folder();
    fs::set_permissions(&app_data, fs::Permissions::from_mode(0o300)).unwrap();
    let before = snapshot::snapshot(&root);

    let service = OrganizeService::start(app_data.clone());
    let error = expect_blocked(&service);
    drop(service);

    fs::set_permissions(&app_data, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.code, ErrorCode::OverlapsAppData);
    assert!(error.message.contains("could not check"));
    assert_eq!(snapshot::snapshot(&root), before);
}

/// While storage is blocked, every operation is refused with the storage
/// error before it reads or writes anything (organize-spec.md §9, O34).
fn every_operation_is_refused(service: &OrganizeService, code: ErrorCode) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("todo.txt"), "buy milk\n").unwrap();
    let before = snapshot::snapshot(root.path());
    let id = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    let codes = [
        service.authorize_root(root.path()).map(|_| ()),
        service.list_roots().map(|_| ()),
        service.apply_plan(id).map(|_| ()),
        service.get_plan(id).map(|_| ()),
        service.get_action(id).map(|_| ()),
        service.reconcile_action(id).map(|_| ()),
        service.preview_undo(id).map(|_| ()),
        // Memory's records, works and pairs (#31).
        service.start_memory_for_file(id).map(|_| ()),
        service
            .start_memory_at(id, &root.path().join("todo.txt"))
            .map(|_| ()),
        service.stop_memory_record(id).map(|_| ()),
        service.memory_records().map(|_| ()),
        service.memory_record(id).map(|_| ()),
        service.memory_family(id).map(|_| ()),
        service.memory_watches_of(id).map(|_| ()),
        service.all_usable_pairs().map(|_| ()),
        service.usable_pairs_of_file(id).map(|_| ()),
        service.usable_pairs_of_family(id).map(|_| ()),
        service.usable_pairs_of_line(id).map(|_| ()),
        service.memory_works().map(|_| ()),
        service.merge_preview(id, id).map(|_| ()),
        service
            .merge_works(&traeva_core::MergeRequest {
                family_id: id.to_string(),
                other_family_id: id.to_string(),
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        service
            .association_preview(id, id, std::path::Path::new("/tmp/x.txt"))
            .map(|_| ()),
        service.association_preview_for_file(id, id).map(|_| ()),
        service
            .associate_file(&traeva_core::AssociationRequest {
                family_id: id.to_string(),
                subject: traeva_core::AssociationSubject::File {
                    file_id: id.to_string(),
                },
                choice: traeva_core::AssociateChoice::Unknown {},
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        service
            .move_preview(&traeva_core::MoveQuery {
                family_id: id.to_string(),
                file_ids: vec![id.to_string()],
                target: traeva_core::MoveTarget::NewWork {},
            })
            .map(|_| ()),
        service
            .move_members(&traeva_core::MoveRequest {
                family_id: id.to_string(),
                file_ids: vec![id.to_string()],
                target: traeva_core::MoveTarget::NewWork {},
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        service.relation_preview(id).map(|_| ()),
        service
            .correct_relation(&traeva_core::RelationRequest {
                edge_id: id.to_string(),
                action: traeva_core::RelationAction::Revoke,
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        service.check_usable_pair(id, id).map(|_| ()),
        service.candidate_capture(id).map(|_| ()),
        service
            .set_candidate_capture(&traeva_core::CandidateSwitchRequest {
                chain_id: id.to_string(),
                on: false,
                notice_version: None,
                occupant_token: None,
                place_token: None,
            })
            .map(|_| ()),
        service.pending_candidates(id).map(|_| ()),
        service
            .decide_pending_candidates(&traeva_core::CandidateDecisionRequest {
                chain_id: id.to_string(),
                page_token: "a".repeat(64),
                items: vec![],
                current_object: Some(traeva_core::CurrentObjectChoice::Independent {}),
            })
            .map(|_| ()),
        service
            .relocation_preview(id, id, std::path::Path::new("/tmp/x.txt"))
            .map(|_| ()),
        service
            .relocate_record(&traeva_core::RelocationRequest {
                chain_id: id.to_string(),
                root_id: id.to_string(),
                relative_path: "x.txt".to_string(),
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        // Exporting an old version (#35).
        service
            .preview_export(id, id, &root.path().join("todo.txt"))
            .map(|_| ()),
        service
            .export_version(&traeva_core::ExportRequest {
                snapshot_id: id.to_string(),
                root_id: id.to_string(),
                folder: String::new(),
                name: "x.txt".to_string(),
                page_token: "a".repeat(64),
            })
            .map(|_| ()),
        service
            .build_plan(&PlanRequest {
                file_id: id.to_string(),
                file_revision: 1,
                target_root_id: id.to_string(),
                operation_id: None,
                target: PlanTarget::Folder {
                    catalog_id: id.to_string(),
                    folder_id: "F1".to_string(),
                    new_folders: vec![],
                },
            })
            .map(|_| ()),
    ];
    for (n, result) in codes.into_iter().enumerate() {
        assert_eq!(result.unwrap_err().code, code, "operation {n}");
    }
    assert_eq!(snapshot::snapshot(root.path()), before);
}

#[test]
fn blocked_storage_refuses_every_operation_and_keeps_the_file() {
    // Read-only.
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));
    let db = database_path(&dir);
    let bytes = fs::read(&db).unwrap();
    fs::set_permissions(&db, fs::Permissions::from_mode(0o444)).unwrap();
    let service = OrganizeService::start(dir.clone());
    let code = expect_blocked(&service).code;
    every_operation_is_refused(&service, code);
    drop(service);
    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code, ErrorCode::DatabaseUnavailable);
    assert_eq!(fs::read(&db).unwrap(), bytes, "not rewritten");

    // Not a database.
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        database_path(&dir),
        b"synthetic bytes, not SQLite. ".repeat(200),
    )
    .unwrap();
    let service = OrganizeService::start(dir.clone());
    every_operation_is_refused(&service, ErrorCode::DatabaseCorrupt);

    // A newer schema.
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));
    open_for_inspection(&dir)
        .execute_batch(&format!(
            "PRAGMA user_version = {};",
            LATEST_SCHEMA_VERSION + 1
        ))
        .unwrap();
    let service = OrganizeService::start(dir.clone());
    let code = expect_blocked(&service).code;
    every_operation_is_refused(&service, code);
}

/// A real database with one damaged page inside (not the header): the
/// integrity check at the start finds it, storage is blocked, and the file
/// stays exactly as it was (O34).
#[test]
fn a_damaged_page_is_found_at_the_start_and_the_file_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));
    let db = database_path(&dir);
    // The first page of path_history: a table the start itself never reads,
    // so only the integrity check can notice. Never the file header.
    let (page_size, page): (usize, usize) = {
        let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let size: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap();
        let root: i64 = conn
            .query_row(
                "SELECT rootpage FROM sqlite_master WHERE name = 'path_history'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (size as usize, root as usize)
    };
    assert!(page > 1);
    let mut bytes = fs::read(&db).unwrap();
    bytes[(page - 1) * page_size..page * page_size].fill(0xA5);
    fs::write(&db, &bytes).unwrap();

    let service = OrganizeService::start(dir.clone());
    let error = expect_blocked(&service);
    drop(service);

    assert_eq!(error.code, ErrorCode::DatabaseCorrupt, "{error:?}");
    assert_eq!(fs::read(&db).unwrap(), bytes, "not repaired, not replaced");
}
