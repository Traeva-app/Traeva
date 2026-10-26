//! The Source follow-up in `register_file` (lib.rs, #27 integration): a
//! source confirmed for a file follows it when the same physical file is
//! chosen again under a new record. If Source cannot save that, choosing the
//! file still works exactly as before and Source's records do not change.

use std::fs;

use crate::test_fixture::Fixture;
use crate::{SourceDownloadEvent, SourceDownloadState, SourceRecordView, SourceRelation};

fn download_of(f: &Fixture, name: &str) -> SourceDownloadEvent {
    let file = f.root.path().canonicalize().unwrap().join(name);
    SourceDownloadEvent {
        profile_id: "77777777-7777-4777-8777-777777777777".to_string(),
        download_id: "1".to_string(),
        started_at_ms: Some(1_700_000_000_000),
        completed_at_ms: Some(1_700_000_000_100),
        state: SourceDownloadState::Complete,
        filename: file.display().to_string(),
        initial_url: Some("https://example.test/synthetic".to_string()),
        final_url: None,
        referrer: None,
        file_size: Some(9),
        incognito: false,
        explicit_history: true,
    }
}

fn record(f: &Fixture, source_id: &str) -> SourceRecordView {
    f.service()
        .source_state()
        .unwrap()
        .records
        .into_iter()
        .find(|r| r.source_id == source_id)
        .expect("the record")
}

#[test]
fn a_failed_source_follow_up_never_fails_choosing_the_file() {
    let f = Fixture::new();
    f.service().set_source_enabled(true).unwrap();
    let saved = f
        .service()
        .ingest_source_event(download_of(&f, "todo.txt"))
        .unwrap();
    let source_id = saved.source_id.expect("saved");
    let saved_record = record(&f, &source_id);
    let confirmed = f
        .service()
        .confirm_source(
            &source_id,
            &saved_record.candidates[0].file_id,
            SourceRelation::Download,
            "Synthetic test download.",
        )
        .unwrap();
    let old_file_id = confirmed.association.unwrap().file_id;
    let history = f.count("source_history");

    // From now on Source cannot save anything new.
    f.db()
        .execute_batch(
            "CREATE TRIGGER test_source_refuses BEFORE INSERT ON source_history
             BEGIN SELECT RAISE(ABORT, 'refused for the test'); END;",
        )
        .unwrap();
    let root = f.root.path().canonicalize().unwrap();
    fs::rename(root.join("todo.txt"), root.join("renamed.txt")).unwrap();

    // Choosing the renamed file works (`register` unwraps) and gives a new record.
    let chosen = f.register("renamed.txt");
    assert_ne!(chosen.file_id, old_file_id);
    // The source stays on the older record; nothing of Source changed.
    let after = record(&f, &source_id);
    assert_eq!(after.association.unwrap().file_id, old_file_id);
    assert_eq!(f.count("source_history"), history);
}
