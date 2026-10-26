//! The initial migration creates the seven minimal tables of architecture.md §5,
//! with the declared keys and constraints, and SQLite actually enforces them.
//! Migrations 2 and 3 add Memory's `snapshots` and `memory_watches` tables
//! (#32), migration 4 Memory's change tables (#33), migrations 5 and 6
//! Source's tables (#27), and migration 7 Memory's works, record chains,
//! candidates, relations, gaps and watcher runs (#31); migration 12 marks the
//! links Traeva made by itself (#37); they leave those seven as they are. A
//! version-1 database with data is upgraded in place.

mod common;

use common::*;
use rusqlite::Connection;
use traeva_core::{LATEST_SCHEMA_VERSION, OrganizeService, PragmaSettings};

const SEVEN_TABLES: [&str; 7] = [
    "action_events",
    "actions",
    "analyses",
    "authorized_roots",
    "files",
    "path_history",
    "plans",
];

/// Migration 7's tables (#31).
const FAMILY_TABLES: [&str; 14] = [
    "memory_families",
    "memory_chains",
    "memory_candidates",
    "memory_decisions",
    "memory_chain_watches",
    "memory_candidate_switches",
    "memory_members",
    "memory_edges",
    "memory_gaps",
    "memory_runs",
    "memory_work_decisions",
    "memory_family_merges",
    "memory_work_changes",
    "memory_record_holds",
];

/// Migration 10's tables: the automatic mode for dropped files (#168).
const AUTO_DROP_TABLES: [&str; 2] = ["organize_auto_consents", "organize_drop_events"];

/// Migration 11's tables: keeping versions of files Traeva organizes (#201).
const AUTO_MEMORY_TABLES: [&str; 2] = ["memory_auto_agreements", "memory_auto_starts"];

/// Migration 12's table: links Traeva made by itself after an app saved by
/// replacing a file (#37, the student's decision of 2026-10-14).
const AUTO_LINK_TABLES: [&str; 1] = ["memory_auto_links"];

/// Every table of the latest schema: the seven, plus Memory's (#32, #33,
/// #31), Source's (#27) and the automatic mode's (#168).
fn all_tables() -> Vec<&'static str> {
    let mut tables = SEVEN_TABLES.to_vec();
    tables.push("snapshots");
    tables.push("memory_watches");
    tables.push("memory_consents");
    tables.push("memory_pairs");
    tables.push("memory_change_analyses");
    tables.push("memory_changes");
    tables.extend([
        "source_settings",
        "source_records",
        "source_candidates",
        "source_associations",
        "source_history",
    ]);
    tables.extend(FAMILY_TABLES);
    tables.extend(AUTO_DROP_TABLES);
    tables.extend(AUTO_MEMORY_TABLES);
    tables.extend(AUTO_LINK_TABLES);
    tables.sort();
    tables
}

/// Starts the service on an empty folder, stops it, and returns an inspection connection.
fn fresh_database() -> (tempfile::TempDir, Connection) {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    drop(OrganizeService::start(dir.clone()));
    let conn = open_for_inspection(&dir);
    (temp, conn)
}

#[test]
fn empty_database_gets_the_seven_tables_and_snapshots() {
    let (_temp, conn) = fresh_database();
    assert_eq!(user_tables(&conn), all_tables());
    assert_eq!(user_version(&conn), LATEST_SCHEMA_VERSION);
    assert_eq!(LATEST_SCHEMA_VERSION, 12);
}

/// A database exactly as migration 1 left it, with one root and one file.
fn version_one_database(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let conn = Connection::open(database_path(dir)).unwrap();
    conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    // Valid evidence: the start reads saved roots back before it writes (#47).
    conn.execute(
        "INSERT INTO authorized_roots (root_id, display_path, evidence_json, status, granted_at_ms)
         VALUES ('root-1', '/synthetic/root',
                 '{\"dev\":1,\"ino\":2,\"birthtime_ns\":3,\"fs_type\":\"apfs\"}', 'active', 0)",
        [],
    )
    .unwrap();
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
}

#[test]
fn a_version_one_database_is_upgraded_and_keeps_its_rows() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    let before = open_for_inspection(&dir);
    let organize_schema: Vec<String> = schema_sql(&before);
    drop(before);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);

    let conn = open_for_inspection(&dir);
    assert_eq!(user_tables(&conn), all_tables());
    assert_eq!(user_version(&conn), LATEST_SCHEMA_VERSION);
    let file: (String, String) = conn
        .query_row("SELECT file_id, root_id FROM files", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(file, ("file-1".to_string(), "root-1".to_string()));
    // The seven tables, their indexes and triggers are exactly as before.
    let after: Vec<String> = schema_sql(&conn)
        .into_iter()
        .filter(|sql| {
            // Organize's `plans` has `source_root_id`, so Source's objects are
            // matched by how their SQL starts, not by "source_" anywhere.
            !sql.contains("snapshots")
                && !sql.contains("memory_")
                && !sql.starts_with("CREATE TABLE source_")
                && !sql.starts_with("CREATE INDEX source_")
                && !sql.starts_with("CREATE TRIGGER source_")
                && !sql.starts_with("CREATE TABLE organize_")
        })
        .collect();
    assert_eq!(after, organize_schema);
}

#[test]
fn a_version_two_database_with_snapshots_is_upgraded_and_keeps_them() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    let conn = Connection::open(database_path(&dir)).unwrap();
    conn.execute_batch(include_str!("../migrations/0002_snapshots.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    conn.execute(
        "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
             relative_path, sha256, byte_length, observation_json, captured_at_ms)
         VALUES ('00000000-0000-4000-8000-000000000009', 'file-1', 1, 1, 'root-1', 'a.txt',
             ?1, 1, '{}', 7)",
        ["0".repeat(64)],
    )
    .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_tables(&conn), all_tables());
    let kept: (String, i64) = conn
        .query_row("SELECT file_id, captured_at_ms FROM snapshots", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(kept, ("file-1".to_string(), 7));
    let watches: i64 = conn
        .query_row("SELECT count(*) FROM memory_watches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(watches, 0);
}

#[test]
fn a_version_three_database_with_watches_is_upgraded_and_keeps_them() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    let conn = Connection::open(database_path(&dir)).unwrap();
    conn.execute_batch(include_str!("../migrations/0002_snapshots.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0003_memory_watches.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
    for (id, sequence) in [
        ("00000000-0000-4000-8000-000000000001", 1),
        ("00000000-0000-4000-8000-000000000002", 2),
    ] {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, 'file-1', ?2, 1, 'root-1', 'a.txt', ?3, 1, '{}', 7)",
            rusqlite::params![id, sequence, "0".repeat(64)],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
         VALUES ('00000000-0000-4000-8000-000000000003', 'file-1', 'active', 5)",
        [],
    )
    .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_tables(&conn), all_tables());
    let kept: (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM snapshots), (SELECT count(*) FROM memory_watches)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kept, (2, 1));
    for table in [
        "memory_consents",
        "memory_pairs",
        "memory_change_analyses",
        "memory_changes",
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

#[test]
fn a_version_four_database_is_upgraded_and_source_starts_off_and_empty() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    let conn = Connection::open(database_path(&dir)).unwrap();
    for sql in [
        include_str!("../migrations/0002_snapshots.sql"),
        include_str!("../migrations/0003_memory_watches.sql"),
        include_str!("../migrations/0004_memory_changes.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 4).unwrap();
    conn.execute(
        "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
             relative_path, sha256, byte_length, observation_json, captured_at_ms)
         VALUES ('00000000-0000-4000-8000-000000000001', 'file-1', 1, 1, 'root-1', 'a.txt', ?1,
                 1, '{}', 7)",
        [&"0".repeat(64)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
         VALUES ('00000000-0000-4000-8000-000000000003', 'file-1', 'active', 5)",
        [],
    )
    .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_tables(&conn), all_tables());
    let kept: (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM snapshots), (SELECT count(*) FROM memory_watches)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kept, (1, 1));
    // Source is off until the user turns it on, and has no records yet.
    let settings: (i64, i64, Option<String>, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT count(*), max(enabled), max(pairing_token), max(receiver_port),
                    max(enabled_since_ms)
             FROM source_settings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(settings, (1, 0, None, None, None));
    for table in [
        "source_records",
        "source_candidates",
        "source_associations",
        "source_history",
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

#[test]
fn source_history_and_the_first_import_time_cannot_change() {
    let (_temp, conn) = fresh_database();
    insert_root(&conn, "root-1");
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
    conn.execute(
        "INSERT INTO source_records (source_id, profile_id, download_id, root_id, event_json,
             imported_at_ms, updated_at_ms)
         VALUES ('source-1', 'profile-1', '1', 'root-1', '{}', 10, 10)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO source_history (history_id, source_id, file_id, kind, evidence_json, note,
             recorded_at_ms)
         VALUES ('history-1', 'source-1', 'file-1', 'confirmed', '{}', 'synthetic', 11)",
        [],
    )
    .unwrap();
    for change in [
        "UPDATE source_history SET note = 'changed'",
        "DELETE FROM source_history",
        "UPDATE source_records SET imported_at_ms = 12",
    ] {
        assert!(conn.execute(change, []).is_err(), "{change}");
    }
    // Other fields of a record can change (a later notice of the same download).
    conn.execute("UPDATE source_records SET updated_at_ms = 12", [])
        .unwrap();
    // Only one settings row, and it starts off.
    assert!(
        conn.execute("INSERT INTO source_settings (singleton) VALUES (2)", [])
            .is_err()
    );
    // 0006: when Source was last turned on is unknown until it is, and never
    // a negative time.
    let since: Option<i64> = conn
        .query_row("SELECT enabled_since_ms FROM source_settings", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(since, None);
    assert!(
        conn.execute("UPDATE source_settings SET enabled_since_ms = -1", [])
            .is_err()
    );
    conn.execute("UPDATE source_settings SET enabled_since_ms = 0", [])
        .unwrap();
}

/// The SQL of every table, index and trigger, in a stable order.
fn schema_sql(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type, name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn a_snapshot_row_needs_a_real_file_and_a_lowercase_sha256() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    drop(OrganizeService::start(dir.clone()));
    let conn = open_for_inspection(&dir);
    let insert = |file_id: &str, sequence: i64, sha256: &str| {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, ?2, ?3, 1, 'root-1', 'a.txt', ?4, 1, '{}', 0)",
            (uuid_like(sequence), file_id, sequence, sha256),
        )
    };
    let good = "0".repeat(64);
    insert("file-1", 1, &good).unwrap();
    assert_constraint_error(insert("file-1", 1, &good), "the same sequence twice");
    assert_constraint_error(insert("no-such-file", 2, &good), "an unknown file");
    assert_constraint_error(insert("file-1", 3, &"A".repeat(64)), "uppercase hex");
    assert_constraint_error(
        insert("file-1", 4, &format!("../{}", "0".repeat(61))),
        "a path",
    );
    assert_constraint_error(insert("file-1", 0, &good), "sequence 0");
}

fn uuid_like(n: i64) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

#[test]
fn ready_state_reports_the_pragmas_sqlite_actually_uses() {
    let temp = tempfile::tempdir().unwrap();
    let service = OrganizeService::start(app_data_dir(&temp));
    let info = expect_ready(&service);
    assert_eq!(info.schema_version, LATEST_SCHEMA_VERSION);
    assert_eq!(
        info.pragmas,
        PragmaSettings {
            foreign_keys: true,
            journal_mode: "delete".to_string(),
            synchronous: "extra".to_string(),
            fullfsync: true,
        }
    );
}

#[test]
fn foreign_keys_match_architecture() {
    let (_temp, conn) = fresh_database();
    let mut stmt = conn
        .prepare(
            "SELECT m.name, f.\"from\", f.\"table\", f.\"to\"
             FROM sqlite_schema AS m, pragma_foreign_key_list(m.name) AS f
             WHERE m.type = 'table'
             ORDER BY m.name, f.\"from\"",
        )
        .unwrap();
    let actual: Vec<(String, String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let expected = [
        ("action_events", "action_id", "actions", "action_id"),
        ("actions", "plan_id", "plans", "plan_id"),
        ("actions", "undo_of_action_id", "actions", "action_id"),
        ("analyses", "file_id", "files", "file_id"),
        ("files", "root_id", "authorized_roots", "root_id"),
        (
            "memory_change_analyses",
            "consent_id",
            "memory_consents",
            "consent_id",
        ),
        (
            "memory_change_analyses",
            "pair_id",
            "memory_pairs",
            "pair_id",
        ),
        (
            "memory_changes",
            "analysis_id",
            "memory_change_analyses",
            "analysis_id",
        ),
        (
            "memory_candidate_switches",
            "chain_id",
            "memory_chains",
            "chain_id",
        ),
        (
            "memory_candidate_switches",
            "included_file_id",
            "files",
            "file_id",
        ),
        (
            "memory_candidates",
            "after_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        ("memory_candidates", "chain_id", "memory_chains", "chain_id"),
        ("memory_candidates", "file_id", "files", "file_id"),
        (
            "memory_candidates",
            "first_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_chain_watches",
            "chain_id",
            "memory_chains",
            "chain_id",
        ),
        (
            "memory_chain_watches",
            "decision_id",
            "memory_decisions",
            "decision_id",
        ),
        (
            "memory_chain_watches",
            "previous_watch_id",
            "memory_watches",
            "watch_id",
        ),
        (
            "memory_chain_watches",
            "watch_id",
            "memory_watches",
            "watch_id",
        ),
        ("memory_chains", "family_id", "memory_families", "family_id"),
        ("memory_chains", "origin_file_id", "files", "file_id"),
        (
            "memory_chains",
            "tail_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        ("memory_consents", "file_id", "files", "file_id"),
        (
            "memory_decisions",
            "candidate_id",
            "memory_candidates",
            "candidate_id",
        ),
        ("memory_decisions", "chain_id", "memory_chains", "chain_id"),
        (
            "memory_decisions",
            "parent_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_edges",
            "child_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_edges",
            "decision_id",
            "memory_decisions",
            "decision_id",
        ),
        ("memory_edges", "family_id", "memory_families", "family_id"),
        (
            "memory_edges",
            "parent_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_gaps",
            "after_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        ("memory_gaps", "chain_id", "memory_chains", "chain_id"),
        (
            "memory_members",
            "decision_id",
            "memory_decisions",
            "decision_id",
        ),
        (
            "memory_members",
            "family_id",
            "memory_families",
            "family_id",
        ),
        ("memory_members", "file_id", "files", "file_id"),
        (
            "memory_family_merges",
            "absorbed_family_id",
            "memory_families",
            "family_id",
        ),
        (
            "memory_family_merges",
            "into_family_id",
            "memory_families",
            "family_id",
        ),
        (
            "memory_family_merges",
            "work_decision_id",
            "memory_work_decisions",
            "work_decision_id",
        ),
        (
            "memory_record_holds",
            "chain_id",
            "memory_chains",
            "chain_id",
        ),
        ("memory_record_holds", "edge_id", "memory_edges", "edge_id"),
        (
            "memory_record_holds",
            "watch_id",
            "memory_watches",
            "watch_id",
        ),
        (
            "memory_record_holds",
            "work_decision_id",
            "memory_work_decisions",
            "work_decision_id",
        ),
        ("memory_work_changes", "edge_id", "memory_edges", "edge_id"),
        (
            "memory_work_changes",
            "member_id",
            "memory_members",
            "member_id",
        ),
        (
            "memory_work_changes",
            "work_decision_id",
            "memory_work_decisions",
            "work_decision_id",
        ),
        (
            "memory_work_decisions",
            "edge_id",
            "memory_edges",
            "edge_id",
        ),
        (
            "memory_work_decisions",
            "family_id",
            "memory_families",
            "family_id",
        ),
        ("memory_work_decisions", "file_id", "files", "file_id"),
        (
            "memory_work_decisions",
            "other_family_id",
            "memory_families",
            "family_id",
        ),
        (
            "memory_work_decisions",
            "parent_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_pairs",
            "after_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        (
            "memory_pairs",
            "before_snapshot_id",
            "snapshots",
            "snapshot_id",
        ),
        ("memory_watches", "file_id", "files", "file_id"),
        ("path_history", "action_id", "actions", "action_id"),
        ("path_history", "file_id", "files", "file_id"),
        (
            "path_history",
            "from_root_id",
            "authorized_roots",
            "root_id",
        ),
        ("path_history", "to_root_id", "authorized_roots", "root_id"),
        ("plans", "analysis_request_id", "analyses", "request_id"),
        ("plans", "file_id", "files", "file_id"),
        ("plans", "source_root_id", "authorized_roots", "root_id"),
        ("plans", "target_root_id", "authorized_roots", "root_id"),
        ("plans", "undo_of_action_id", "actions", "action_id"),
        ("snapshots", "file_id", "files", "file_id"),
        ("snapshots", "root_id", "authorized_roots", "root_id"),
        ("source_associations", "file_id", "files", "file_id"),
        (
            "source_associations",
            "original_file_id",
            "files",
            "file_id",
        ),
        (
            "source_associations",
            "source_id",
            "source_records",
            "source_id",
        ),
        ("source_candidates", "file_id", "files", "file_id"),
        (
            "source_candidates",
            "source_id",
            "source_records",
            "source_id",
        ),
        ("source_history", "file_id", "files", "file_id"),
        ("source_history", "source_id", "source_records", "source_id"),
        ("source_records", "root_id", "authorized_roots", "root_id"),
        (
            "organize_auto_consents",
            "target_root_id",
            "authorized_roots",
            "root_id",
        ),
        (
            "organize_drop_events",
            "root_id",
            "authorized_roots",
            "root_id",
        ),
        ("organize_drop_events", "file_id", "files", "file_id"),
        (
            "organize_drop_events",
            "consent_id",
            "organize_auto_consents",
            "consent_id",
        ),
        ("organize_drop_events", "plan_id", "plans", "plan_id"),
        ("organize_drop_events", "action_id", "actions", "action_id"),
        ("memory_auto_starts", "action_id", "actions", "action_id"),
        (
            "memory_auto_starts",
            "agreement_id",
            "memory_auto_agreements",
            "agreement_id",
        ),
        ("memory_auto_starts", "file_id", "files", "file_id"),
        (
            "memory_auto_starts",
            "watch_id",
            "memory_watches",
            "watch_id",
        ),
        (
            "memory_auto_starts",
            "consent_id",
            "memory_consents",
            "consent_id",
        ),
        ("memory_auto_links", "edge_id", "memory_edges", "edge_id"),
        (
            "memory_auto_links",
            "member_id",
            "memory_members",
            "member_id",
        ),
        ("memory_auto_links", "chain_id", "memory_chains", "chain_id"),
        (
            "memory_auto_links",
            "decision_id",
            "memory_decisions",
            "decision_id",
        ),
    ];
    let mut expected: Vec<(String, String, String, String)> = expected
        .iter()
        .map(|(a, b, c, d)| (a.to_string(), b.to_string(), c.to_string(), d.to_string()))
        .collect();
    // The query sorts by table, then column.
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn unique_keys_match_architecture() {
    let (_temp, conn) = fresh_database();

    // Every unique index (including the ones behind PRIMARY KEY / UNIQUE), as
    // "table(columns) [WHERE partial]".
    let mut stmt = conn
        .prepare(
            "SELECT m.name, l.name, l.partial, group_concat(i.name, ',')
             FROM sqlite_schema AS m,
                  pragma_index_list(m.name) AS l,
                  pragma_index_info(l.name) AS i
             WHERE m.type = 'table' AND l.\"unique\" = 1
             GROUP BY m.name, l.name
             ORDER BY m.name, l.name",
        )
        .unwrap();
    let actual: Vec<String> = stmt
        .query_map([], |r| {
            let table: String = r.get(0)?;
            let partial: bool = r.get(2)?;
            let columns: String = r.get(3)?;
            Ok(format!(
                "{table}({columns}){}",
                if partial { " partial" } else { "" }
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut actual = actual;
    actual.sort();

    let mut expected = vec![
        "action_events(action_id,sequence)",
        "actions(action_id)",
        "actions(plan_id)",
        "actions(undo_of_action_id) partial",
        "analyses(request_id)",
        "authorized_roots(root_id)",
        "files(file_id)",
        "files(root_id,relative_path) partial",
        "path_history(action_id,reason)",
        "path_history(path_event_id)",
        "plans(plan_id)",
        "memory_watches(file_id) partial",
        "memory_watches(watch_id)",
        "memory_consents(consent_id)",
        "memory_consents(file_id) partial",
        "memory_pairs(pair_id)",
        "memory_pairs(before_snapshot_id,after_snapshot_id,differ_version,before_reader,after_reader)",
        "memory_change_analyses(analysis_id)",
        "memory_change_analyses(pair_id) partial",
        "memory_change_analyses(pair_id) partial",
        "memory_changes(change_id)",
        "memory_changes(analysis_id,position)",
        "snapshots(file_id,sequence)",
        "snapshots(snapshot_id)",
        "source_associations(source_id)",
        "source_candidates(source_id,file_id)",
        "source_history(history_id)",
        "source_records(profile_id,download_id)",
        "source_records(source_id)",
        "organize_auto_consents(consent_id)",
        "organize_drop_events(drop_id)",
        "organize_drop_events(batch_id,position)",
        "memory_families(family_id)",
        "memory_chains(chain_id)",
        "memory_candidates(candidate_id)",
        "memory_candidates(chain_id,seq)",
        "memory_candidates(chain_id,file_id)",
        "memory_decisions(decision_id)",
        "memory_decisions(batch_id,candidate_id) partial",
        "memory_chain_watches(watch_id)",
        "memory_chain_watches(previous_watch_id) partial",
        "memory_candidate_switches(switch_id)",
        "memory_candidate_switches(chain_id) partial",
        "memory_members(member_id)",
        "memory_members(family_id,file_id) partial",
        "memory_members(file_id) partial",
        "memory_edges(edge_id)",
        "memory_edges(child_snapshot_id) partial",
        "memory_gaps(gap_id)",
        "memory_gaps(chain_id,kind) partial",
        "memory_runs(run_id)",
        "memory_work_decisions(work_decision_id)",
        "memory_work_decisions(page_token)",
        "memory_family_merges(absorbed_family_id)",
        "memory_family_merges(work_decision_id)",
        "memory_work_changes(change_id)",
        "memory_work_changes(member_id) partial",
        "memory_work_changes(edge_id) partial",
        "memory_record_holds(hold_id)",
        "memory_record_holds(chain_id) partial",
        "memory_auto_agreements(agreement_id)",
        "memory_auto_starts(action_id)",
        "memory_auto_starts(consent_id) partial",
        "memory_auto_links(edge_id)",
        "memory_auto_links(decision_id)",
    ];
    expected.sort();
    assert_eq!(actual, expected);

    let plan_id_not_null: bool = conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('actions') WHERE name = 'plan_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(plan_id_not_null, "actions.plan_id must be NOT NULL");
}

// ---- Behaviour: SQLite really enforces the rules ----

fn insert_root(conn: &Connection, root_id: &str) {
    conn.execute(
        "INSERT INTO authorized_roots (root_id, display_path, evidence_json, status, granted_at_ms)
         VALUES (?1, '/synthetic/root', '{}', 'active', 0)",
        [root_id],
    )
    .unwrap();
}

fn insert_file(
    conn: &Connection,
    file_id: &str,
    root_id: &str,
    path: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO files (file_id, root_id, relative_path, basename, presence, sha256,
                            byte_length, observation_json, observed_at_ms, record_revision,
                            created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, 'a.txt', 'present', ?4, 1, '{}', 0, 1, 0, 0)",
        (file_id, root_id, path, "0".repeat(64)),
    )
}

fn insert_plan(conn: &Connection, plan_id: &str, file_id: &str, root_id: &str) {
    conn.execute(
        "INSERT INTO plans (plan_id, kind, file_id, source_root_id, source_relative_path,
                            expected_source_json, target_root_id, target_segments_json, basename,
                            folder_chain_json, folders_to_create_json, user_choice, created_at_ms)
         VALUES (?1, 'organize', ?2, ?3, 'a.txt', '{}', ?3, '[\"Notes\"]', 'a.txt',
                 '[]', '[\"Notes\"]', 'manual', 0)",
        (plan_id, file_id, root_id),
    )
    .unwrap();
}

fn insert_undo_plan(conn: &Connection, plan_id: &str, file_id: &str, root_id: &str, undo_of: &str) {
    conn.execute(
        "INSERT INTO plans (plan_id, kind, file_id, source_root_id, source_relative_path,
                            expected_source_json, target_root_id, target_segments_json, basename,
                            folder_chain_json, folders_to_create_json, undo_of_action_id,
                            created_at_ms)
         VALUES (?1, 'undo', ?2, ?3, 'Notes/a.txt', '{}', ?3, '[]', 'a.txt', '[]', '[]', ?4, 0)",
        (plan_id, file_id, root_id, undo_of),
    )
    .unwrap();
}

fn insert_action(
    conn: &Connection,
    action_id: &str,
    plan_id: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO actions (action_id, plan_id, kind, status, preconditions_json, created_at_ms)
         VALUES (?1, ?2, 'organize', 'prepared', '{}', 0)",
        (action_id, plan_id),
    )
}

fn insert_committed_undo(
    conn: &Connection,
    action_id: &str,
    plan_id: &str,
    undo_of: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO actions (action_id, plan_id, kind, undo_of_action_id, status,
                              preconditions_json, created_at_ms, committed_at_ms)
         VALUES (?1, ?2, 'undo', ?3, 'committed', '{}', 0, 0)",
        (action_id, plan_id, undo_of),
    )
}

fn insert_event(conn: &Connection, action_id: &str, sequence: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO action_events (action_id, sequence, step, phase, detail_json, recorded_at_ms)
         VALUES (?1, ?2, 'move', 'intent', '{}', 0)",
        (action_id, sequence),
    )
}

fn insert_path_event(
    conn: &Connection,
    id: &str,
    file_id: &str,
    root_id: &str,
    action_id: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO path_history (path_event_id, file_id, from_root_id, from_relative_path,
                                   to_root_id, to_relative_path, reason, action_id,
                                   evidence_json, recorded_at_ms)
         VALUES (?1, ?2, ?3, 'a.txt', ?3, 'Notes/a.txt', 'organize', ?4, '{}', 0)",
        (id, file_id, root_id, action_id),
    )
}

/// One root, one file, one organize plan with one action.
fn database_with_one_action() -> (tempfile::TempDir, Connection) {
    let (temp, conn) = fresh_database();
    insert_root(&conn, "root-1");
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
    insert_plan(&conn, "plan-1", "file-1", "root-1");
    insert_action(&conn, "action-1", Some("plan-1")).unwrap();
    (temp, conn)
}

fn assert_constraint_error(result: rusqlite::Result<usize>, what: &str) {
    let err = result.expect_err(what);
    assert_eq!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation),
        "{what}: {err}"
    );
}

#[test]
fn one_plan_has_at_most_one_action() {
    let (_temp, conn) = database_with_one_action();
    assert_constraint_error(
        insert_action(&conn, "action-2", Some("plan-1")),
        "a second action for the same plan must fail",
    );
    assert_constraint_error(
        insert_action(&conn, "action-3", None),
        "an action without a plan must fail",
    );
    assert_constraint_error(
        insert_action(&conn, "action-4", Some("no-such-plan")),
        "an action for an unknown plan must fail (foreign key)",
    );
}

#[test]
fn journal_sequence_and_path_history_reason_are_unique_per_action() {
    let (_temp, conn) = database_with_one_action();
    insert_event(&conn, "action-1", 1).unwrap();
    insert_event(&conn, "action-1", 2).unwrap();
    assert_constraint_error(
        insert_event(&conn, "action-1", 2),
        "duplicate journal sequence",
    );

    insert_path_event(&conn, "path-1", "file-1", "root-1", "action-1").unwrap();
    assert_constraint_error(
        insert_path_event(&conn, "path-2", "file-1", "root-1", "action-1"),
        "a second organize path event for the same action",
    );
}

#[test]
fn a_location_holds_at_most_one_present_file() {
    let (_temp, conn) = database_with_one_action();
    assert_constraint_error(
        insert_file(&conn, "file-2", "root-1", "a.txt"),
        "a second present file at the same path",
    );
    // Once the old record is marked missing, a new file may be registered there.
    conn.execute(
        "UPDATE files SET presence = 'missing' WHERE file_id = 'file-1'",
        [],
    )
    .unwrap();
    insert_file(&conn, "file-2", "root-1", "a.txt").unwrap();
}

#[test]
fn an_action_has_at_most_one_committed_undo() {
    let (_temp, conn) = database_with_one_action();
    insert_undo_plan(&conn, "undo-plan-1", "file-1", "root-1", "action-1");
    insert_undo_plan(&conn, "undo-plan-2", "file-1", "root-1", "action-1");
    insert_committed_undo(&conn, "undo-1", "undo-plan-1", "action-1").unwrap();
    assert_constraint_error(
        insert_committed_undo(&conn, "undo-2", "undo-plan-2", "action-1"),
        "a second committed undo of the same action",
    );
}

#[test]
fn plan_payload_cannot_change_but_lifecycle_can() {
    let (_temp, conn) = database_with_one_action();
    assert_constraint_error(
        conn.execute(
            "UPDATE plans SET target_segments_json = '[\"Other\"]' WHERE plan_id = 'plan-1'",
            [],
        ),
        "changing a plan's target must fail",
    );
    conn.execute(
        "UPDATE plans SET lifecycle = 'cancelled', lifecycle_changed_at_ms = 1
         WHERE plan_id = 'plan-1'",
        [],
    )
    .unwrap();
}

#[test]
fn journal_and_path_history_are_append_only() {
    let (_temp, conn) = database_with_one_action();
    insert_event(&conn, "action-1", 1).unwrap();
    insert_path_event(&conn, "path-1", "file-1", "root-1", "action-1").unwrap();

    assert_constraint_error(
        conn.execute("UPDATE action_events SET step = 'other'", []),
        "journal update",
    );
    assert_constraint_error(
        conn.execute("DELETE FROM action_events", []),
        "journal delete",
    );
    assert_constraint_error(
        conn.execute("UPDATE path_history SET to_relative_path = 'x'", []),
        "path history update",
    );
    assert_constraint_error(
        conn.execute("DELETE FROM path_history", []),
        "path history delete",
    );
}

// ---- Migration 7: works, record chains and watcher runs (#31) ----

const W_A: &str = "00000000-0000-4000-8000-0000000000a1";
const W_B: &str = "00000000-0000-4000-8000-0000000000b2";
const W_C: &str = "00000000-0000-4000-8000-0000000000c3";
const W_D: &str = "00000000-0000-4000-8000-0000000000d4";
const S_1: &str = "00000000-0000-4000-8000-000000000011";
const S_2: &str = "00000000-0000-4000-8000-000000000012";
const S_3: &str = "00000000-0000-4000-8000-000000000013";

#[test]
fn a_version_six_database_with_watches_and_sources_is_upgraded_and_linked() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_one_database(&dir);
    let conn = Connection::open(database_path(&dir)).unwrap();
    for sql in [
        include_str!("../migrations/0002_snapshots.sql"),
        include_str!("../migrations/0003_memory_watches.sql"),
        include_str!("../migrations/0004_memory_changes.sql"),
        include_str!("../migrations/0005_sources.sql"),
        include_str!("../migrations/0006_source_capture_window.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 6).unwrap();
    insert_file(&conn, "file-2", "root-1", "b.txt").unwrap();
    for (id, sequence, at) in [(S_1, 1, 6), (S_2, 2, 7)] {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, 'file-1', ?2, 1, 'root-1', 'a.txt', ?3, 1, '{}', ?4)",
            rusqlite::params![id, sequence, "0".repeat(64), at],
        )
        .unwrap();
    }
    // file-1: on at 5, off at 8, on again at 10. file-2: on at 6, off at
    // 20, on again "at 12": the clock was set back in between.
    for (watch, file, status, started, stopped) in [
        (W_A, "file-1", "stopped", 5, Some(8)),
        (W_C, "file-2", "stopped", 6, Some(20)),
        (W_B, "file-1", "active", 10, None),
        (W_D, "file-2", "active", 12, None),
    ] {
        conn.execute(
            "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms, stopped_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![watch, file, status, started, stopped],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE source_settings SET enabled = 1, enabled_since_ms = 4",
        [],
    )
    .unwrap();
    drop(conn);

    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_tables(&conn), all_tables());
    let rows = |sql: &str| -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    // Each file Memory was on for is its own work with one chain, named
    // after its first watch.
    assert_eq!(
        rows("SELECT family_id || ' ' || created_at_ms FROM memory_families ORDER BY family_id"),
        vec![format!("{W_A} 5"), format!("{W_C} 6")]
    );
    assert_eq!(
        rows(
            "SELECT chain_id || ' ' || family_id || ' ' || origin_file_id || ' '
                    || coalesce(tail_snapshot_id, '-')
             FROM memory_chains ORDER BY chain_id"
        ),
        vec![
            format!("{W_A} {W_A} file-1 {S_2}"),
            format!("{W_C} {W_C} file-2 -")
        ]
    );
    assert_eq!(
        rows(
            "SELECT file_id || ' ' || status || ' ' || basis FROM memory_members
             ORDER BY file_id"
        ),
        vec![
            "file-1 confirmed_system from_before",
            "file-2 confirmed_system from_before"
        ]
    );
    assert_eq!(
        rows(
            "SELECT watch_id || ' ' || chain_id || ' ' || reason || ' '
                    || coalesce(previous_watch_id, '-')
             FROM memory_chain_watches ORDER BY linked_at_ms, watch_id"
        ),
        vec![
            format!("{W_A} {W_A} from_before -"),
            format!("{W_C} {W_C} from_before -"),
            format!("{W_B} {W_A} restarted {W_A}"),
            format!("{W_D} {W_C} restarted {W_C}"),
        ]
    );
    // The time Memory was off for file-1 is a gap after its newest version;
    // file-2's never ends before it starts.
    assert_eq!(
        rows(
            "SELECT gap_id || ' ' || chain_id || ' ' || kind || ' '
                    || coalesce(after_snapshot_id, '-') || ' ' || started_at_ms || ' '
                    || ended_at_ms || ' ' || end_reason
             FROM memory_gaps ORDER BY gap_id"
        ),
        vec![
            format!("{W_B} {W_A} memory_off {S_2} 8 10 resumed"),
            format!("{W_D} {W_C} memory_off - 12 12 resumed"),
        ]
    );
    // No chain gets candidate capture; no run was recorded yet; Source and
    // #33's tables are kept.
    for (table, count) in [
        ("memory_candidate_switches", 0),
        ("memory_candidates", 0),
        ("memory_edges", 0),
        ("memory_runs", 0),
        ("memory_watches", 4),
        ("snapshots", 2),
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, count, "{table}");
    }
    let source: (i64, i64) = conn
        .query_row(
            "SELECT enabled, enabled_since_ms FROM source_settings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(source, (1, 4));
}

#[test]
fn migration_files_are_numbered_without_gaps() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    names.sort();
    for (index, name) in names.iter().enumerate() {
        assert!(
            name.starts_with(&format!("{:04}_", index + 1)),
            "migration {name} is out of order"
        );
    }
    assert_eq!(names.len() as i64, LATEST_SCHEMA_VERSION);
}

#[test]
fn works_chains_gaps_and_runs_keep_their_rules() {
    let (_temp, conn) = fresh_database();
    insert_root(&conn, "root-1");
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
    insert_file(&conn, "file-2", "root-1", "b.txt").unwrap();
    for (id, file, sequence) in [(S_1, "file-1", 1), (S_2, "file-1", 2), (S_3, "file-2", 1)] {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, ?2, ?3, 1, 'root-1', 'a.txt', ?4, 1, '{}', 0)",
            rusqlite::params![id, file, sequence, "0".repeat(64)],
        )
        .unwrap();
    }
    let ok = |sql: &str| {
        conn.execute(sql, [])
            .unwrap_or_else(|err| panic!("{sql}: {err}"))
    };
    let refused = |sql: &str| assert!(conn.execute(sql, []).is_err(), "{sql}");
    ok("INSERT INTO memory_families VALUES ('fam-1', 0), ('fam-2', 0)");
    refused("UPDATE memory_families SET created_at_ms = 1");
    refused("DELETE FROM memory_families");
    ok(&format!(
        "INSERT INTO memory_chains VALUES ('chain-1', 'fam-1', 'file-1', '{S_2}', 0)"
    ));
    // The newest version only moves forward.
    refused(&format!(
        "UPDATE memory_chains SET tail_snapshot_id = '{S_1}'"
    ));
    refused("UPDATE memory_chains SET tail_snapshot_id = NULL");
    refused("UPDATE memory_chains SET origin_file_id = 'file-2'");

    // A file is a confirmed member of one work at a time; statuses move only
    // forward.
    ok(
        "INSERT INTO memory_members VALUES ('m-1', 'fam-1', 'file-1', 'confirmed_system',
        'memory_on', NULL, 0, 0)",
    );
    refused(
        "INSERT INTO memory_members VALUES ('m-2', 'fam-2', 'file-1', 'confirmed_user',
             'user_decision', NULL, 0, 0)",
    );
    ok(
        "INSERT INTO memory_members VALUES ('m-3', 'fam-1', 'file-2', 'candidate',
        'candidate_capture', NULL, 0, 0)",
    );
    refused("UPDATE memory_members SET status = 'candidate' WHERE member_id = 'm-1'");
    ok("UPDATE memory_members SET status = 'rejected' WHERE member_id = 'm-3'");
    refused("UPDATE memory_members SET status = 'confirmed_user' WHERE member_id = 'm-3'");
    refused("DELETE FROM memory_members");

    // Relations: a confirmed one needs its parent, which is saved first.
    refused(&format!(
        "INSERT INTO memory_edges VALUES ('e-1', 'fam-1', 'branch', NULL, '{S_2}',
         'confirmed_user', 0, NULL, 0, 0)"
    ));
    refused(&format!(
        "INSERT INTO memory_edges VALUES ('e-2', 'fam-1', 'continuation', '{S_2}', '{S_1}',
         'confirmed_user', 0, NULL, 0, 0)"
    ));
    ok(&format!(
        "INSERT INTO memory_edges VALUES ('e-3', 'fam-1', 'unplaced', NULL, '{S_2}',
         'unknown', 0, NULL, 0, 0)"
    ));
    // A second placement of the same version is refused.
    refused(&format!(
        "INSERT INTO memory_edges VALUES ('e-4', 'fam-1', 'continuation', '{S_1}', '{S_2}',
         'confirmed_user', 0, NULL, 0, 0)"
    ));
    ok(&format!(
        "INSERT INTO memory_edges VALUES ('e-5', 'fam-1', 'branch', '{S_1}', '{S_3}',
         'unknown', 0, NULL, 0, 0)"
    ));
    // An unknown relation never becomes confirmed (the status trigger: e-5
    // has its parent, so no CHECK refuses it).
    refused("UPDATE memory_edges SET status = 'confirmed_user' WHERE edge_id = 'e-5'");
    ok("UPDATE memory_edges SET status = 'rejected' WHERE edge_id = 'e-5'");
    refused("UPDATE memory_edges SET skipped_items = 2");
    refused("DELETE FROM memory_edges");

    // Gaps: Memory being off is always over when written; an end is final.
    refused(
        "INSERT INTO memory_gaps VALUES ('g-1', 'chain-1', 'memory_off', NULL, NULL, 5,
             NULL, NULL)",
    );
    refused(
        "INSERT INTO memory_gaps VALUES ('g-2', 'chain-1', 'capture_failed', NULL, NULL, 5,
             NULL, NULL)",
    );
    ok(
        "INSERT INTO memory_gaps VALUES ('g-3', 'chain-1', 'capture_failed', NULL,
        'file_not_found', 5, NULL, NULL)",
    );
    refused(
        "INSERT INTO memory_gaps VALUES ('g-4', 'chain-1', 'capture_failed', NULL,
             'root_revoked', 6, NULL, NULL)",
    );
    ok("UPDATE memory_gaps SET ended_at_ms = 7, end_reason = 'recovered'");
    refused("UPDATE memory_gaps SET ended_at_ms = 8");
    refused("DELETE FROM memory_gaps");

    // Runs: the last sign of life only moves forward; rows stay.
    ok("INSERT INTO memory_runs VALUES ('run-1', 10, 10)");
    ok("UPDATE memory_runs SET last_seen_at_ms = 40");
    refused("UPDATE memory_runs SET last_seen_at_ms = 39");
    refused("UPDATE memory_runs SET started_at_ms = 11");
    refused("DELETE FROM memory_runs");

    // A file's watches belong to one chain; a chain has one watch on.
    ok("INSERT INTO memory_chains VALUES ('chain-2', 'fam-2', 'file-2', NULL, 0)");
    ok("INSERT INTO memory_watches VALUES ('w-1', 'file-1', 'active', 0, NULL)");
    ok("INSERT INTO memory_watches VALUES ('w-2', 'file-2', 'active', 0, NULL)");
    ok("INSERT INTO memory_chain_watches VALUES ('w-1', 'chain-1', 'started', NULL, NULL, 0)");
    refused("INSERT INTO memory_chain_watches VALUES ('w-2', 'chain-1', 'started', NULL, NULL, 0)");
    ok("UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1 WHERE watch_id = 'w-1'");
    ok("INSERT INTO memory_watches VALUES ('w-3', 'file-1', 'active', 2, NULL)");
    refused("INSERT INTO memory_chain_watches VALUES ('w-3', 'chain-2', 'started', NULL, NULL, 2)");
    ok("INSERT INTO memory_chain_watches VALUES ('w-3', 'chain-1', 'restarted', 'w-1', NULL, 2)");
    refused("UPDATE memory_chain_watches SET reason = 'started'");
    refused("DELETE FROM memory_chain_watches");
    refused("DELETE FROM memory_chains");

    // M1b's tables: a candidate keeps what was seen and a decision is final;
    // a decision never changes; a switch turned off stays off.
    let candidate = |id: &str, seq: i64, reason: &str, state: &str, decided: &str| {
        format!(
            "INSERT INTO memory_candidates VALUES ('{id}', 'chain-1', 'file-2', {seq}, '{reason}',
             '{S_2}', '{S_3}', '{state}', 0, {decided})"
        )
    };
    refused(&candidate("c-0", 1, "read_on_decision", "pending", "NULL"));
    refused(&candidate("c-0", 1, "replaced", "continued", "NULL"));
    ok(&candidate("c-1", 1, "replaced", "pending", "NULL"));
    refused(&candidate("c-2", 2, "replaced", "pending", "NULL"));
    refused("UPDATE memory_candidates SET after_snapshot_id = NULL");
    ok("UPDATE memory_candidates SET state = 'branch', decided_at_ms = 1");
    refused("UPDATE memory_candidates SET state = 'independent', decided_at_ms = 2");
    refused("DELETE FROM memory_candidates");
    let decision = |id: &str, choice: &str, parent: &str| {
        format!(
            "INSERT INTO memory_decisions VALUES ('{id}', 'chain-1', 'batch-1', '{choice}', 'c-1',
             {parent}, '{}', 0)",
            "a".repeat(64)
        )
    };
    refused(&decision("d-0", "continue", "NULL"));
    refused(&decision("d-0", "independent", &format!("'{S_2}'")));
    ok(&decision("d-1", "branch", &format!("'{S_2}'")));
    refused(&decision("d-2", "defer", "NULL"));
    refused("UPDATE memory_decisions SET choice = 'unknown'");
    refused("DELETE FROM memory_decisions");
    insert_file(&conn, "file-3", "root-1", "c.txt").unwrap();
    ok("INSERT INTO memory_watches VALUES ('w-4', 'file-3', 'active', 3, NULL)");
    refused(
        "INSERT INTO memory_chain_watches VALUES ('w-4', 'chain-2', 'continued', NULL, NULL, 3)",
    );
    ok("INSERT INTO memory_candidate_switches VALUES ('s-1', 'chain-1', 1, 0, NULL, NULL, NULL)");
    refused(
        "INSERT INTO memory_candidate_switches VALUES ('s-2', 'chain-1', 1, 0, NULL, NULL, NULL)",
    );
    refused("UPDATE memory_candidate_switches SET disabled_at_ms = 5");
    ok("UPDATE memory_candidate_switches SET disabled_at_ms = 5, disabled_reason = 'user'");
    refused("UPDATE memory_candidate_switches SET disabled_reason = 'stopped'");
    refused("DELETE FROM memory_candidate_switches");
}

/// A version 7 database with one root, one file and one work (#31 M1).
fn version_seven_database(dir: &std::path::Path) {
    version_one_database(dir);
    let conn = Connection::open(database_path(dir)).unwrap();
    for sql in [
        include_str!("../migrations/0002_snapshots.sql"),
        include_str!("../migrations/0003_memory_watches.sql"),
        include_str!("../migrations/0004_memory_changes.sql"),
        include_str!("../migrations/0005_sources.sql"),
        include_str!("../migrations/0006_source_capture_window.sql"),
        include_str!("../migrations/0007_memory_families.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 7).unwrap();
    conn.execute(
        "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
             relative_path, sha256, byte_length, observation_json, captured_at_ms)
         VALUES (?1, 'file-1', 1, 1, 'root-1', 'a.txt', ?2, 1, '{}', 5)",
        rusqlite::params![S_1, "0".repeat(64)],
    )
    .unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
             VALUES ('{W_A}', 'file-1', 'active', 5);
         INSERT INTO memory_families VALUES ('fam-1', 5);
         INSERT INTO memory_chains VALUES ('chain-1', 'fam-1', 'file-1', '{S_1}', 5);
         INSERT INTO memory_chain_watches (watch_id, chain_id, reason, linked_at_ms)
             VALUES ('{W_A}', 'chain-1', 'started', 5);
         INSERT INTO memory_members VALUES ('m-1', 'fam-1', 'file-1', 'confirmed_system',
             'memory_on', NULL, 5, 5);"
    ))
    .unwrap();
}

#[test]
fn a_version_seven_database_with_a_work_is_upgraded_and_keeps_every_row() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_seven_database(&dir);
    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    let view = service.memory_family("fam-1");
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_version(&conn), LATEST_SCHEMA_VERSION);
    let live: String = conn
        .query_row(
            "SELECT live_family_id FROM memory_live_families WHERE family_id = 'fam-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        live, "fam-1",
        "every work is its own live work until merged"
    );
    for table in [
        "memory_families",
        "memory_chains",
        "memory_members",
        "memory_chain_watches",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "{table}");
    }
    for table in [
        "memory_work_decisions",
        "memory_family_merges",
        "memory_work_changes",
        "memory_record_holds",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    // The work's ID is not a UUID in this hand-made database, so the view
    // refuses it; the rows are what matters here.
    assert!(view.is_err());
}

#[test]
fn work_decisions_merges_changes_and_holds_keep_their_rules() {
    let (_temp, conn) = fresh_database();
    insert_root(&conn, "root-1");
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
    insert_file(&conn, "file-2", "root-1", "b.txt").unwrap();
    for (id, file, sequence) in [(S_1, "file-1", 1), (S_2, "file-1", 2), (S_3, "file-2", 1)] {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, ?2, ?3, 1, 'root-1', 'a.txt', ?4, 1, '{}', 0)",
            rusqlite::params![id, file, sequence, "0".repeat(64)],
        )
        .unwrap();
    }
    let ok = |sql: &str| {
        conn.execute(sql, [])
            .unwrap_or_else(|err| panic!("{sql}: {err}"))
    };
    let refused = |sql: &str| assert!(conn.execute(sql, []).is_err(), "{sql}");
    let token = |c: char| c.to_string().repeat(64);
    ok("INSERT INTO memory_families VALUES ('fam-1', 0), ('fam-2', 0), ('fam-3', 0)");
    ok(
        "INSERT INTO memory_members VALUES ('m-1', 'fam-1', 'file-1', 'confirmed_system',
        'memory_on', NULL, 0, 0)",
    );
    ok(
        "INSERT INTO memory_members VALUES ('m-2', 'fam-2', 'file-1', 'rejected',
        'user_decision', NULL, 0, 0)",
    );
    ok(&format!(
        "INSERT INTO memory_edges VALUES ('e-1', 'fam-1', 'continuation', '{S_1}', '{S_2}',
         'confirmed_user', 0, NULL, 0, 0)"
    ));
    // A merge decision names its two works; the merge row must match it.
    refused(&format!(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, page_token,
             decided_at_ms) VALUES ('d-0', 'merge', 'fam-1', '{}', 0)",
        token('a')
    ));
    ok(&format!(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, other_family_id,
             page_token, decided_at_ms) VALUES ('d-1', 'merge', 'fam-1', 'fam-2', '{}', 0)",
        token('a')
    ));
    refused("INSERT INTO memory_family_merges VALUES ('fam-3', 'fam-1', 'd-1', 0)");
    ok("INSERT INTO memory_family_merges VALUES ('fam-2', 'fam-1', 'd-1', 0)");
    refused("UPDATE memory_work_decisions SET decided_at_ms = 1");
    refused("DELETE FROM memory_work_decisions");
    refused("UPDATE memory_family_merges SET merged_at_ms = 1");
    refused("DELETE FROM memory_family_merges");
    // The same page twice.
    refused(&format!(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, other_family_id,
             page_token, decided_at_ms) VALUES ('d-x', 'merge', 'fam-1', 'fam-3', '{}', 0)",
        token('a')
    ));
    // fam-2 now means fam-1: nothing new goes under it, and it absorbs
    // nothing.
    let live: String = conn
        .query_row(
            "SELECT live_family_id FROM memory_live_families WHERE family_id = 'fam-2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, "fam-1");
    refused(
        "INSERT INTO memory_members VALUES ('m-3', 'fam-2', 'file-2', 'candidate',
             'candidate_capture', NULL, 0, 0)",
    );
    refused(&format!(
        "INSERT INTO memory_edges VALUES ('e-2', 'fam-2', 'unplaced', NULL, '{S_3}',
         'unknown', 0, NULL, 0, 0)"
    ));
    refused("INSERT INTO memory_chains VALUES ('chain-x', 'fam-2', 'file-2', NULL, 0)");
    refused(&format!(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, other_family_id,
             page_token, decided_at_ms) VALUES ('d-2', 'merge', 'fam-3', 'fam-2', '{}', 0)",
        token('b')
    ));
    refused(
        "INSERT INTO memory_members VALUES ('m-4', 'fam-1', 'file-2', 'confirmed_user',
             'merge', NULL, 0, 0)",
    );
    // A retirement needs its change row first.
    refused("UPDATE memory_members SET status = 'superseded' WHERE member_id = 'm-2'");
    refused("UPDATE memory_edges SET status = 'disputed' WHERE edge_id = 'e-1'");
    ok(
        "INSERT INTO memory_work_changes VALUES ('c-1', 'd-1', 'm-2', NULL, 'rejected',
        'superseded', 0)",
    );
    ok("UPDATE memory_members SET status = 'superseded' WHERE member_id = 'm-2'");
    refused("UPDATE memory_work_changes SET changed_at_ms = 1");
    refused("DELETE FROM memory_work_changes");
    refused(
        "INSERT INTO memory_work_changes VALUES ('c-2', 'd-1', 'm-1', 'e-1', NULL,
             'superseded', 0)",
    );
    // A membership may still be marked disputed directly (#33/#34 tests).
    ok("UPDATE memory_members SET status = 'disputed' WHERE member_id = 'm-1'");

    // Holds: only for a stopped watch of the record; while held, no watch
    // joins the record.
    ok(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
        VALUES ('w-1', 'file-1', 'active', 0)",
    );
    ok("INSERT INTO memory_chains VALUES ('chain-1', 'fam-1', 'file-1', NULL, 0)");
    ok(
        "INSERT INTO memory_chain_watches (watch_id, chain_id, reason, linked_at_ms)
        VALUES ('w-1', 'chain-1', 'started', 0)",
    );
    ok(&format!(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, edge_id,
             page_token, decided_at_ms) VALUES ('d-3', 'revoke', 'fam-1', 'e-1', '{}', 0)",
        token('c')
    ));
    refused(
        "INSERT INTO memory_record_holds VALUES ('h-1', 'chain-1', 'w-1', 'e-1', 'd-3',
             5, NULL, NULL)",
    );
    ok("UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 5 WHERE watch_id = 'w-1'");
    ok(
        "INSERT INTO memory_record_holds VALUES ('h-1', 'chain-1', 'w-1', 'e-1', 'd-3', 5,
        NULL, NULL)",
    );
    refused(
        "INSERT INTO memory_record_holds VALUES ('h-2', 'chain-1', 'w-1', NULL, 'd-3', 6,
             NULL, NULL)",
    );
    ok(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
        VALUES ('w-2', 'file-1', 'active', 7)",
    );
    refused(
        "INSERT INTO memory_chain_watches (watch_id, chain_id, reason, previous_watch_id,
             linked_at_ms) VALUES ('w-2', 'chain-1', 'restarted', 'w-1', 7)",
    );
    refused("UPDATE memory_record_holds SET held_at_ms = 6");
    ok("UPDATE memory_record_holds SET released_at_ms = 7, release_reason = 'resumed'");
    refused("UPDATE memory_record_holds SET released_at_ms = 8");
    refused("DELETE FROM memory_record_holds");
    ok(
        "INSERT INTO memory_chain_watches (watch_id, chain_id, reason, previous_watch_id,
        linked_at_ms) VALUES ('w-2', 'chain-1', 'restarted', 'w-1', 7)",
    );
}

/// A database as migration 8 left it, with one Memory analysis whose model
/// answer was refused, before reasons were recorded.
fn version_eight_database(dir: &std::path::Path) {
    version_seven_database(dir);
    let conn = Connection::open(database_path(dir)).unwrap();
    conn.execute_batch(include_str!("../migrations/0008_memory_work_decisions.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 8).unwrap();
    conn.execute(
        "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
             relative_path, sha256, byte_length, observation_json, captured_at_ms)
         VALUES (?1, 'file-1', 2, 1, 'root-1', 'a.txt', ?2, 1, '{}', 6)",
        rusqlite::params![S_2, "1".repeat(64)],
    )
    .unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO memory_consents VALUES ('consent-1', 'file-1', 'openai', 'gpt-6-luna', 1,
             5, NULL);
         INSERT INTO memory_pairs (pair_id, before_snapshot_id, after_snapshot_id,
             differ_version, outcome, before_reader, after_reader, before_complete,
             after_complete, hunks, compared_at_ms)
         VALUES ('pair-1', '{S_1}', '{S_2}', 'lines-1', 'changed', 'txt-md-1', 'txt-md-1', 1, 1,
             1, 6);
         INSERT INTO memory_change_analyses (analysis_id, pair_id, consent_id, provider, model,
             settings_generation, prompt_version, schema_version, status, sent_hunks,
             left_out_hunks, request_bytes, started_at_ms, finished_at_ms, retryable)
         VALUES ('analysis-1', 'pair-1', 'consent-1', 'openai', 'gpt-6-luna', 1,
             'memory-changes-1', 'memory-changes-1', 'invalid_output', 1, 0, 10, 7, 8, 1);"
    ))
    .unwrap();
}

#[test]
fn a_version_eight_database_keeps_its_refused_answers_without_a_made_up_reason() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_eight_database(&dir);
    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_version(&conn), LATEST_SCHEMA_VERSION);
    let (status, reason): (String, Option<String>) = conn
        .query_row(
            "SELECT status, invalid_reason FROM memory_change_analyses
             WHERE analysis_id = 'analysis-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "invalid_output");
    assert_eq!(reason, None, "not recorded then, and not made up now");
}

#[test]
fn an_invalid_reason_only_on_a_refused_answer_and_never_too_long() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_eight_database(&dir);
    drop(OrganizeService::start(dir.clone()));
    let conn = open_for_inspection(&dir);
    // A finished outcome never changes (0004's trigger), so each case is
    // a new row of its own.
    let n = std::cell::Cell::new(0);
    let insert = |status: &str, reason: Option<&str>| {
        n.set(n.get() + 1);
        conn.execute(
            "INSERT INTO memory_change_analyses (analysis_id, pair_id, consent_id, provider,
                 model, settings_generation, prompt_version, schema_version, status,
                 sent_hunks, left_out_hunks, request_bytes, started_at_ms, finished_at_ms,
                 retryable, invalid_reason)
             VALUES (?1, 'pair-1', 'consent-1', 'openai', 'gpt-6-luna', 1, 'p', 's', ?2, 1, 0,
                 10, 9, 10, 1, ?3)",
            rusqlite::params![format!("analysis-new-{}", n.get()), status, reason],
        )
    };
    insert(
        "invalid_output",
        Some("change 1's after_quote is not in the added lines of its hunks"),
    )
    .unwrap();
    insert("invalid_output", Some(&"x".repeat(300))).unwrap();
    insert("invalid_output", None).unwrap();
    assert!(
        insert("invalid_output", Some(&"x".repeat(301))).is_err(),
        "longer than 300"
    );
    assert!(insert("invalid_output", Some("")).is_err(), "empty");
    assert!(
        insert("cancelled", Some("a reason")).is_err(),
        "only on a refused answer"
    );
    insert("cancelled", None).unwrap();
}

/// A database as migration 11 left it, with one work whose record watches
/// file-1 (built on the version 8 one).
fn version_eleven_database(dir: &std::path::Path) {
    version_eight_database(dir);
    let conn = Connection::open(database_path(dir)).unwrap();
    for sql in [
        include_str!("../migrations/0009_memory_invalid_reasons.sql"),
        include_str!("../migrations/0010_organize_auto_drop.sql"),
        include_str!("../migrations/0011_memory_auto.sql"),
    ] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 11).unwrap();
}

#[test]
fn a_version_eleven_database_is_upgraded_with_no_automatic_link() {
    let temp = tempfile::tempdir().unwrap();
    let dir = app_data_dir(&temp);
    version_eleven_database(&dir);
    let service = OrganizeService::start(dir.clone());
    assert_eq!(expect_ready(&service).schema_version, LATEST_SCHEMA_VERSION);
    drop(service);
    let conn = open_for_inspection(&dir);
    assert_eq!(user_version(&conn), 12);
    assert_eq!(user_tables(&conn), all_tables());
    for (table, rows) in [
        ("memory_families", 1),
        ("memory_chains", 1),
        ("memory_members", 1),
        ("memory_chain_watches", 1),
        ("snapshots", 2),
        ("memory_auto_links", 0),
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, rows, "{table}");
    }
}

#[test]
fn an_automatic_link_needs_its_rows_and_never_changes() {
    let (_temp, conn) = fresh_database();
    insert_root(&conn, "root-1");
    insert_file(&conn, "file-1", "root-1", "a.txt").unwrap();
    insert_file(&conn, "file-2", "root-1", "b.txt").unwrap();
    for (id, file) in [(S_1, "file-1"), (S_2, "file-2")] {
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
                 relative_path, sha256, byte_length, observation_json, captured_at_ms)
             VALUES (?1, ?2, 1, 1, 'root-1', 'a.txt', ?3, 1, '{}', 0)",
            rusqlite::params![id, file, "0".repeat(64)],
        )
        .unwrap();
    }
    let ok = |sql: &str| {
        conn.execute(sql, [])
            .unwrap_or_else(|err| panic!("{sql}: {err}"))
    };
    let refused = |sql: &str| assert!(conn.execute(sql, []).is_err(), "{sql}");
    let token = "a".repeat(64);
    ok("INSERT INTO memory_families VALUES ('fam-1', 0)");
    ok(&format!(
        "INSERT INTO memory_chains VALUES ('chain-1', 'fam-1', 'file-1', '{S_1}', 0)"
    ));
    ok(&format!(
        "INSERT INTO memory_decisions (decision_id, chain_id, batch_id, choice,
             parent_snapshot_id, page_token, decided_at_ms)
         VALUES ('d-1', 'chain-1', 'b-1', 'continue', '{S_1}', '{token}', 1)"
    ));
    ok(
        "INSERT INTO memory_members VALUES ('m-2', 'fam-1', 'file-2', 'confirmed_system',
        'user_decision', 'd-1', 1, 1)",
    );
    ok(&format!(
        "INSERT INTO memory_edges VALUES ('e-1', 'fam-1', 'continuation', '{S_1}', '{S_2}',
             'confirmed_system', 0, 'd-1', 1, 1)"
    ));
    // Every reference must exist.
    refused("INSERT INTO memory_auto_links VALUES ('e-x', 'm-2', 'chain-1', 'd-1', 1)");
    refused("INSERT INTO memory_auto_links VALUES ('e-1', 'm-2', 'chain-1', 'd-x', 1)");
    ok("INSERT INTO memory_auto_links VALUES ('e-1', 'm-2', 'chain-1', 'd-1', 1)");
    // One row per relation and per decision.
    refused("INSERT INTO memory_auto_links VALUES ('e-1', 'm-2', 'chain-1', 'd-1', 2)");
    refused("UPDATE memory_auto_links SET linked_at_ms = 2");
    refused("DELETE FROM memory_auto_links");
}
