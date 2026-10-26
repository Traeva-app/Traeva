use rusqlite::Connection;
use traeva_core::{DATABASE_FILE_NAME, LATEST_SCHEMA_VERSION, OrganizeService};

#[test]
fn version_five_upgrade_preserves_settings_without_inventing_enablement_time() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("app-data");
    std::fs::create_dir(&data).unwrap();
    let db_path = data.join(DATABASE_FILE_NAME);
    let connection = Connection::open(&db_path).unwrap();
    for sql in [
        include_str!("../migrations/0001_initial.sql"),
        include_str!("../migrations/0002_snapshots.sql"),
        include_str!("../migrations/0003_memory_watches.sql"),
        include_str!("../migrations/0004_memory_changes.sql"),
        include_str!("../migrations/0005_sources.sql"),
    ] {
        connection.execute_batch(sql).unwrap();
    }
    connection.pragma_update(None, "user_version", 5).unwrap();
    connection.execute("UPDATE source_settings SET enabled = 1, receiver_port = 19227, pairing_token = 'synthetic-pairing'", []).unwrap();
    drop(connection);
    let service = OrganizeService::start(data);
    let window = service.source_capture_state().unwrap();
    assert!(window.enabled);
    assert_eq!(window.enabled_since_ms, None);
    assert_eq!(service.source_receiver_port().unwrap(), Some(19227));
    assert_eq!(service.source_pairing_token().unwrap(), "synthetic-pairing");
    let connection = Connection::open(db_path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, LATEST_SCHEMA_VERSION);
    assert!(
        version > 5,
        "The capture-window migration must be integrated by A."
    );
    service.set_source_enabled(true).unwrap();
    let since = service
        .source_capture_state()
        .unwrap()
        .enabled_since_ms
        .unwrap();
    drop(service);
    let reopened = OrganizeService::start(temp.path().join("app-data"));
    assert_eq!(
        reopened.source_capture_state().unwrap().enabled_since_ms,
        Some(since)
    );
    assert!(
        connection
            .execute("UPDATE source_settings SET enabled_since_ms = -1", [])
            .is_err()
    );
}
