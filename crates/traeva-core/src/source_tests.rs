use std::fs;
use std::os::unix::fs::symlink;

use super::*;
use crate::test_fixture::Fixture;

fn path(f: &Fixture, relative: &str) -> PathBuf {
    f.root.path().canonicalize().unwrap().join(relative)
}

fn event(f: &Fixture, id: &str) -> SourceDownloadEvent {
    SourceDownloadEvent {
        profile_id: "77777777-7777-4777-8777-777777777777".to_string(),
        download_id: id.to_string(),
        started_at_ms: Some(1_700_000_000_000),
        completed_at_ms: Some(1_700_000_000_100),
        state: SourceDownloadState::Complete,
        filename: path(f, "todo.txt").display().to_string(),
        initial_url: Some("https://example.test/start?signature=synthetic".to_string()),
        final_url: Some("https://example.test/final".to_string()),
        referrer: None,
        file_size: Some(9),
        incognito: false,
        explicit_history: true,
    }
}

fn enabled() -> Fixture {
    let f = Fixture::new();
    f.service().set_source_enabled(true).unwrap();
    f
}

fn ingest(f: &Fixture, e: SourceDownloadEvent) -> SourceRecordView {
    let result = f.service().ingest_source_event(e).unwrap();
    assert_eq!(result.outcome, "saved", "{:?}", result.reason);
    f.service().source_view(&result.source_id.unwrap()).unwrap()
}

fn confirmed(f: &Fixture) -> SourceRecordView {
    let r = ingest(f, event(f, "1"));
    f.service()
        .confirm_source(
            &r.source_id,
            &r.candidates[0].file_id,
            SourceRelation::Download,
            "I downloaded this file.",
        )
        .unwrap()
}

#[test]
fn collection_is_explicit_and_incognito_outside_roots_are_not_saved() {
    let f = Fixture::new();
    assert!(!f.service().source_state().unwrap().enabled);
    assert_eq!(
        f.service()
            .ingest_source_event(event(&f, "1"))
            .unwrap()
            .outcome,
        "ignored"
    );
    f.service().set_source_enabled(true).unwrap();
    let mut e = event(&f, "1");
    e.incognito = true;
    assert_eq!(
        f.service().ingest_source_event(e).unwrap().outcome,
        "ignored"
    );
    let mut e = event(&f, "2");
    e.filename = f.app.path().join("secret.txt").display().to_string();
    assert_eq!(
        f.service().ingest_source_event(e).unwrap().outcome,
        "ignored"
    );
    assert_eq!(f.count("source_records"), 0);
}

#[test]
fn automatic_capture_excludes_disabled_downloads_after_reenable_and_restart() {
    let mut f = enabled();
    let first_since = f
        .service()
        .source_capture_state()
        .unwrap()
        .enabled_since_ms
        .unwrap();
    let mut current = event(&f, "1");
    current.explicit_history = false;
    current.started_at_ms = Some(first_since);
    current.completed_at_ms = Some(first_since);
    let saved = ingest(&f, current.clone());
    f.advance(100);
    f.service().set_source_enabled(false).unwrap();
    f.advance(100);
    let mut disabled = current.clone();
    disabled.download_id = "2".to_string();
    disabled.started_at_ms = Some(first_since + 200);
    disabled.completed_at_ms = Some(first_since + 200);
    assert_eq!(
        f.service()
            .ingest_source_event(disabled.clone())
            .unwrap()
            .outcome,
        "ignored"
    );
    f.advance(100);
    f.service().set_source_enabled(true).unwrap();
    assert_eq!(
        f.service()
            .ingest_source_event(disabled.clone())
            .unwrap()
            .outcome,
        "ignored"
    );
    f.restart();
    f.service().set_source_enabled(true).unwrap();
    assert_eq!(
        f.service().source_capture_state().unwrap().enabled_since_ms,
        Some(first_since + 300)
    );
    assert_eq!(
        f.service()
            .ingest_source_event(disabled.clone())
            .unwrap()
            .outcome,
        "ignored"
    );
    // A lost old acknowledgement cannot turn re-enabling into a history import.
    assert_eq!(
        f.service().ingest_source_event(current).unwrap().outcome,
        "ignored"
    );
    assert_eq!(f.count("source_records"), 1);
    assert_eq!(
        f.service()
            .source_view(&saved.source_id)
            .unwrap()
            .imported_at_ms,
        saved.imported_at_ms
    );
    disabled.explicit_history = true;
    ingest(&f, disabled);
    let mut fresh = event(&f, "3");
    fresh.explicit_history = false;
    fresh.started_at_ms = Some(first_since + 300);
    fresh.completed_at_ms = fresh.started_at_ms;
    ingest(&f, fresh);
    assert_eq!(f.count("source_records"), 3);
}

#[test]
fn automatic_capture_requires_known_start_and_preserves_window_on_process_restart() {
    let mut f = enabled();
    let since = f.service().source_capture_state().unwrap().enabled_since_ms;
    let mut e = event(&f, "1");
    e.explicit_history = false;
    e.started_at_ms = None;
    e.completed_at_ms = None;
    assert_eq!(
        f.service().ingest_source_event(e.clone()).unwrap().outcome,
        "ignored"
    );
    e.started_at_ms = Some(f.service().now() + 1);
    assert_eq!(
        f.service().ingest_source_event(e.clone()).unwrap().outcome,
        "ignored"
    );
    e.started_at_ms = since;
    f.advance(1000);
    f.restart();
    f.service().set_source_enabled(true).unwrap();
    assert_eq!(
        f.service().source_capture_state().unwrap().enabled_since_ms,
        since
    );
    ingest(&f, e);
}

#[test]
fn enabling_storage_failure_does_not_change_capture_window_or_permission() {
    let f = enabled();
    f.service().set_source_enabled(false).unwrap();
    let before = f.service().source_capture_state().unwrap();
    f.db().execute_batch("CREATE TRIGGER fail_enable BEFORE UPDATE ON source_settings BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(f.service().set_source_enabled(true).is_err());
    let after = f.service().source_capture_state().unwrap();
    assert!(!after.enabled);
    assert_eq!(after.enabled_since_ms, before.enabled_since_ms);
    assert_eq!(f.count("source_records"), 0);
}

#[test]
fn redirected_source_preserves_unknown_title_and_separate_times() {
    let f = enabled();
    let r = ingest(&f, event(&f, "1"));
    assert_ne!(r.initial_url, r.final_url);
    assert_eq!(r.title, None);
    assert_eq!(r.referrer, None);
    assert_eq!(r.started_at_ms, Some(1_700_000_000_000));
    assert_eq!(r.completed_at_ms, Some(1_700_000_000_100));
    assert!(r.imported_at_ms > r.completed_at_ms.unwrap());
    assert_eq!(r.status, SourceStatus::Likely);
    assert!(r.association.is_none());
    assert!(r.candidates[0].can_confirm);
}

#[test]
fn missing_fields_remain_unknown_and_in_progress_does_not_bind() {
    let f = enabled();
    let mut e = event(&f, "1");
    e.initial_url = None;
    e.final_url = None;
    e.started_at_ms = None;
    e.state = SourceDownloadState::InProgress;
    let r = ingest(&f, e);
    assert_eq!(r.initial_url, None);
    assert_eq!(r.final_url, None);
    assert_eq!(r.started_at_ms, None);
    assert_eq!(r.completed_at_ms, None);
    assert_eq!(r.status, SourceStatus::Unknown);
    assert!(r.candidates.is_empty());
}

#[test]
fn replay_restart_and_late_progress_do_not_duplicate_or_reset_import() {
    let mut f = enabled();
    let original = event(&f, "1");
    let r = ingest(&f, original.clone());
    f.advance(10000);
    let again = ingest(&f, original.clone());
    assert_eq!(r.source_id, again.source_id);
    assert_eq!(r.imported_at_ms, again.imported_at_ms);
    f.restart();
    let again = ingest(&f, original.clone());
    assert_eq!(r.imported_at_ms, again.imported_at_ms);
    let mut late = original;
    late.state = SourceDownloadState::InProgress;
    late.final_url = None;
    let late = ingest(&f, late);
    assert_eq!(late.download_state, SourceDownloadState::Complete);
    assert_eq!(late.final_url, r.final_url);
    assert_eq!(f.count("source_records"), 1);
    assert_eq!(f.count("source_candidates"), 1);
}

#[test]
fn confirmation_is_durable_idempotent_and_never_verified() {
    let mut f = enabled();
    let r = confirmed(&f);
    let a = r.association.unwrap();
    assert_eq!(r.status, SourceStatus::UserConfirmed);
    assert_eq!(a.basis, "user_confirmed");
    f.service()
        .confirm_source(&r.source_id, &a.file_id, SourceRelation::Download, &a.note)
        .unwrap();
    assert_eq!(f.count("source_history"), 1);
    f.restart();
    let restored = f.service().source_view(&r.source_id).unwrap();
    assert_eq!(restored.status, SourceStatus::UserConfirmed);
    assert_eq!(restored.association.unwrap().basis, "user_confirmed");
}

#[test]
fn rejection_survives_replay_and_manual_reselection_allows_later_confirmation() {
    let f = enabled();
    let r = ingest(&f, event(&f, "1"));
    let file = &r.candidates[0].file_id;
    let r = f.service().reject_source(&r.source_id, file).unwrap();
    assert!(r.association.is_none());
    assert!(r.candidates.is_empty());
    assert!(ingest(&f, event(&f, "1")).candidates.is_empty());
    let selected = f
        .service()
        .suggest_source_file(&r.source_id, &path(&f, "todo.txt"))
        .unwrap();
    assert_eq!(selected.candidates.len(), 1);
    assert!(
        f.service()
            .confirm_source(&r.source_id, file, SourceRelation::Download, "")
            .unwrap()
            .association
            .is_some()
    );
}

#[test]
fn replacement_between_recommendation_and_confirmation_is_refused() {
    let f = enabled();
    let r = ingest(&f, event(&f, "1"));
    fs::rename(path(&f, "todo.txt"), path(&f, "original.txt")).unwrap();
    fs::copy(path(&f, "original.txt"), path(&f, "todo.txt")).unwrap();
    assert!(!f.service().source_view(&r.source_id).unwrap().candidates[0].can_confirm);
    let error = f
        .service()
        .confirm_source(
            &r.source_id,
            &r.candidates[0].file_id,
            SourceRelation::Download,
            "",
        )
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::FileChanged);
    assert_eq!(f.count("source_associations"), 0);
}

#[test]
fn rename_extends_user_confirmed_basis_without_merging_file_ids_or_starting_memory() {
    let f = enabled();
    let r = confirmed(&f);
    let old = r.association.unwrap().file_id;
    fs::rename(path(&f, "todo.txt"), path(&f, "renamed.txt")).unwrap();
    let gone = f.service().source_view(&r.source_id).unwrap();
    assert!(!gone.association.unwrap().current_location_confirmed);
    let current = f
        .service()
        .suggest_source_file(&r.source_id, &path(&f, "renamed.txt"))
        .unwrap();
    let a = current.association.unwrap();
    assert_ne!(a.file_id, old);
    assert_eq!(a.basis, "user_confirmed");
    assert!(a.current_location_confirmed);
    assert_eq!(current.imported_at_ms, r.imported_at_ms);
    assert_eq!(f.count("memory_watches"), 0);
    let original: String = f
        .service()
        .store()
        .unwrap()
        .connection()
        .unwrap()
        .query_row(
            "SELECT original_file_id FROM source_associations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, old);
}

#[test]
fn copy_delete_cannot_extend_original_but_user_can_confirm_copy_and_unlink() {
    let f = enabled();
    let r = confirmed(&f);
    let old = r.association.unwrap().file_id;
    fs::copy(path(&f, "todo.txt"), path(&f, "copy.txt")).unwrap();
    fs::remove_file(path(&f, "todo.txt")).unwrap();
    let suggested = f
        .service()
        .suggest_source_file(&r.source_id, &path(&f, "copy.txt"))
        .unwrap();
    assert_eq!(suggested.association.unwrap().file_id, old);
    let copy = suggested
        .candidates
        .iter()
        .find(|c| c.file_id != old)
        .unwrap();
    let r = f
        .service()
        .confirm_source(
            &r.source_id,
            &copy.file_id,
            SourceRelation::Copy,
            "This is my copy.",
        )
        .unwrap();
    assert_eq!(r.association.unwrap().relation, SourceRelation::Copy);
    let r = f
        .service()
        .unlink_source(&r.source_id, "Wrong selection.")
        .unwrap();
    assert!(r.association.is_none());
    assert_eq!(f.count("source_history"), 3);
}

#[test]
fn two_downloads_and_browser_profiles_never_merge_on_identical_content() {
    let f = enabled();
    let first = ingest(&f, event(&f, "1"));
    fs::copy(path(&f, "todo.txt"), path(&f, "copy.txt")).unwrap();
    let mut second = event(&f, "2");
    second.filename = path(&f, "copy.txt").display().to_string();
    let second = ingest(&f, second);
    assert_ne!(first.source_id, second.source_id);
    assert_ne!(first.candidates[0].file_id, second.candidates[0].file_id);
    let mut third = event(&f, "1");
    third.profile_id = uuid::Uuid::new_v4().to_string();
    ingest(&f, third);
    assert_eq!(f.count("source_records"), 3);
}

#[test]
fn registration_failures_keep_source_fields_and_actual_reason() {
    let f = enabled();
    let large = path(&f, "large.txt");
    fs::File::create(&large)
        .unwrap()
        .set_len(21 * 1024 * 1024)
        .unwrap();
    let mut e = event(&f, "1");
    e.filename = large.display().to_string();
    let r = ingest(&f, e);
    assert!(r.reason.unwrap().contains("20 MiB"));
    assert!(r.initial_url.is_some());
    assert!(r.candidates.is_empty());
    fs::write(path(&f, ".hidden.txt"), "synthetic").unwrap();
    let mut e = event(&f, "2");
    e.filename = path(&f, ".hidden.txt").display().to_string();
    let r = ingest(&f, e);
    assert!(r.reason.unwrap().contains("hidden"));
    assert!(r.initial_url.is_some());
    symlink(path(&f, "todo.txt"), path(&f, "link.txt")).unwrap();
    let mut e = event(&f, "3");
    e.filename = path(&f, "link.txt").display().to_string();
    assert!(ingest(&f, e).reason.unwrap().contains("symbolic link"));
}

#[test]
fn parent_symlink_outside_root_and_revoked_root_are_not_ingested() {
    let f = enabled();
    symlink(f.app.path(), path(&f, "escape")).unwrap();
    let mut e = event(&f, "1");
    e.filename = path(&f, "escape/x.txt").display().to_string();
    assert_eq!(
        f.service().ingest_source_event(e).unwrap().outcome,
        "ignored"
    );
    f.service().revoke_root(&f.root_id).unwrap();
    assert_eq!(
        f.service()
            .ingest_source_event(event(&f, "2"))
            .unwrap()
            .outcome,
        "ignored"
    );
    assert_eq!(f.count("source_records"), 0);
}

#[test]
fn save_failure_is_an_error_and_retry_does_not_duplicate() {
    let f = enabled();
    f.service().store().unwrap().connection().unwrap().execute_batch("CREATE TRIGGER fail_source BEFORE INSERT ON source_records BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(f.service().ingest_source_event(event(&f, "1")).is_err());
    assert_eq!(f.count("source_records"), 0);
    f.service()
        .store()
        .unwrap()
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_source")
        .unwrap();
    let r = ingest(&f, event(&f, "1"));
    assert_eq!(ingest(&f, event(&f, "1")).source_id, r.source_id);
    assert_eq!(f.count("source_records"), 1);
}

#[test]
fn association_failure_rolls_back_relation_and_history() {
    let f = enabled();
    let r = ingest(&f, event(&f, "1"));
    f.service().store().unwrap().connection().unwrap().execute_batch("CREATE TRIGGER fail_history BEFORE INSERT ON source_history BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(
        f.service()
            .confirm_source(
                &r.source_id,
                &r.candidates[0].file_id,
                SourceRelation::Download,
                ""
            )
            .is_err()
    );
    assert_eq!(f.count("source_associations"), 0);
    assert_eq!(f.count("source_history"), 0);
}

#[test]
fn links_are_explicit_http_only_and_titles_are_never_fabricated() {
    let f = enabled();
    for (n, url) in [
        "file:///tmp/x",
        "javascript:alert(1)",
        "data:text/plain,x",
        "blob:https://example.test/x",
        "custom://x",
        "https://user:password@example.test/x",
    ]
    .iter()
    .enumerate()
    {
        let mut e = event(&f, &n.to_string());
        e.initial_url = Some(url.to_string());
        let r = ingest(&f, e);
        assert!(
            f.service()
                .source_link(&r.source_id, SourceLinkKind::Initial)
                .is_err()
        );
        assert_eq!(r.title, None);
    }
    let r = ingest(&f, event(&f, "99"));
    assert_eq!(
        f.service()
            .source_link(&r.source_id, SourceLinkKind::Final)
            .unwrap(),
        "https://example.test/final"
    );
}

#[test]
fn pairing_token_and_port_survive_restart_but_are_not_in_source_state() {
    let mut f = enabled();
    let token = f.service().source_pairing_token().unwrap();
    assert_eq!(token.len(), 64);
    f.service().set_source_receiver_port(49152).unwrap();
    f.restart();
    assert_eq!(token, f.service().source_pairing_token().unwrap());
    assert_eq!(f.service().source_receiver_port().unwrap(), Some(49152));
    assert!(
        !serde_json::to_string(&f.service().source_state().unwrap())
            .unwrap()
            .contains(&token)
    );
}

#[test]
fn ordinary_registration_continues_only_live_same_objects() {
    let f = enabled();
    let first = confirmed(&f);
    let old = first.association.unwrap().file_id;
    fs::rename(path(&f, "todo.txt"), path(&f, "relocated.txt")).unwrap();
    let new = f.register("relocated.txt");
    let current = f.service().source_view(&first.source_id).unwrap();
    assert_ne!(old, new.file_id);
    assert_eq!(current.association.unwrap().file_id, new.file_id);
    assert_eq!(current.status, SourceStatus::UserConfirmed);
    assert_eq!(current.imported_at_ms, first.imported_at_ms);
    assert!(current.candidates.iter().all(|c| c.file_id != new.file_id));
    assert_eq!(f.count("memory_watches"), 0);
    let count = f.count("source_history");
    f.register("relocated.txt");
    assert_eq!(f.count("source_history"), count);

    // The old database row remains 'present'; it must not authorize a replacement.
    fs::copy(path(&f, "relocated.txt"), path(&f, "replacement.txt")).unwrap();
    fs::rename(path(&f, "replacement.txt"), path(&f, "relocated.txt")).unwrap();
    let replacement = f.register("relocated.txt");
    assert_ne!(replacement.file_id, new.file_id);
    let kept = f.service().source_view(&first.source_id).unwrap();
    let association = kept.association.unwrap();
    assert_eq!(association.file_id, new.file_id);
    assert!(!association.current_location_confirmed);
}

#[test]
fn confirmed_file_is_not_offered_as_a_candidate_and_revocation_has_a_reason() {
    let f = enabled();
    let r = confirmed(&f);
    assert!(r.candidates.is_empty());
    f.service().revoke_root(&f.root_id).unwrap();
    let r = f.service().source_view(&r.source_id).unwrap();
    assert_eq!(r.status, SourceStatus::UserConfirmed);
    assert!(r.initial_url.is_some());
    assert!(!r.association.unwrap().current_location_confirmed);
    assert!(r.reason.unwrap().contains("removed from Traeva"));
}

#[test]
fn turning_collection_off_does_not_disable_explicit_file_checks() {
    let f = enabled();
    let r = ingest(&f, event(&f, "1"));
    f.service().set_source_enabled(false).unwrap();
    fs::rename(path(&f, "todo.txt"), path(&f, "old.txt")).unwrap();
    let r = f.service().refresh_source(&r.source_id).unwrap();
    assert!(r.reason.unwrap().contains("does not exist"));
    assert!(r.candidates.iter().all(|c| !c.can_confirm));
    assert!(r.initial_url.is_some());
}

#[test]
fn old_records_remain_accessible_after_more_than_five_hundred_imports() {
    let f = enabled();
    for id in 0..501 {
        let mut e = event(&f, &id.to_string());
        e.state = SourceDownloadState::InProgress;
        ingest(&f, e);
    }
    let records = f.service().source_state().unwrap().records;
    assert_eq!(records.len(), 501);
    assert!(records.iter().any(|r| r.download_id == "0"));
}

#[test]
fn duplicate_identity_conflicts_preserve_saved_evidence() {
    let f = enabled();
    let original = ingest(&f, event(&f, "1"));
    let mut conflicting = event(&f, "1");
    conflicting.started_at_ms = Some(20);
    assert!(f.service().ingest_source_event(conflicting).is_err());
    let saved = f.service().source_view(&original.source_id).unwrap();
    assert_eq!(saved.started_at_ms, original.started_at_ms);
    assert_eq!(saved.imported_at_ms, original.imported_at_ms);
    assert_eq!(f.count("source_records"), 1);
}

#[test]
fn valid_link_at_sign_in_query_is_not_confused_with_authority_credentials() {
    let f = enabled();
    let mut e = event(&f, "1");
    e.initial_url = Some("https://example.test/file?tag=@synthetic".to_string());
    let r = ingest(&f, e);
    assert!(
        f.service()
            .source_link(&r.source_id, SourceLinkKind::Initial)
            .is_ok()
    );
}

#[test]
fn source_capture_decisions_and_display_leave_user_tree_unchanged() {
    let f = enabled();
    let before = crate::test_snapshot::snapshot(f.root.path());
    let r = ingest(&f, event(&f, "1"));
    f.service()
        .reject_source(&r.source_id, &r.candidates[0].file_id)
        .unwrap();
    let r = f
        .service()
        .suggest_source_file(&r.source_id, &path(&f, "todo.txt"))
        .unwrap();
    f.service()
        .confirm_source(
            &r.source_id,
            &r.candidates[0].file_id,
            SourceRelation::Download,
            "Synthetic confirmation",
        )
        .unwrap();
    f.service().refresh_source(&r.source_id).unwrap();
    f.service().source_state().unwrap();
    f.service()
        .unlink_source(&r.source_id, "Synthetic correction")
        .unwrap();
    assert_eq!(crate::test_snapshot::snapshot(f.root.path()), before);
}

#[test]
fn committed_organize_move_retains_source_evidence_at_its_current_location() {
    let f = enabled();
    let r = confirmed(&f);
    let file = f.register("todo.txt");
    let plan = f.plan(&file, "Taxes", &[]);
    f.apply(&plan.plan_id).unwrap();
    let r = f.service().source_view(&r.source_id).unwrap();
    let association = r.association.unwrap();
    assert_eq!(association.file_id, file.file_id);
    assert!(association.display_path.ends_with("Taxes/todo.txt"));
    assert!(association.current_location_confirmed);
    assert_eq!(association.basis, "user_confirmed");
}

#[test]
fn previous_schema_upgrades_in_place_with_capture_off() {
    let temp = tempfile::Builder::new()
        .prefix("traeva-source-upgrade-")
        .tempdir()
        .unwrap();
    let app = temp.path().join("app-data");
    fs::create_dir(&app).unwrap();
    let db_path = app.join(crate::DATABASE_FILE_NAME);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0002_snapshots.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0003_memory_watches.sql"))
        .unwrap();
    conn.execute("INSERT INTO authorized_roots (root_id,display_path,evidence_json,status,granted_at_ms,revoked_at_ms) VALUES ('77777777-7777-4777-8777-777777777777','/synthetic/old-root','{\"dev\":1,\"ino\":2,\"birthtime_ns\":3,\"fs_type\":\"apfs\"}','revoked',12,13)", []).unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
    drop(conn);
    let service = OrganizeService::start(app);
    let state = service.source_state().unwrap();
    assert!(!state.enabled);
    assert!(state.records.is_empty());
    let conn = service.store().unwrap().connection().unwrap();
    let preserved: (String, i64) = conn
        .query_row(
            "SELECT display_path,granted_at_ms FROM authorized_roots",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(preserved, ("/synthetic/old-root".to_string(), 12));
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::LATEST_SCHEMA_VERSION);
}
