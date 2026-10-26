//! Helpers shared by the integration tests. Every test uses its own temporary
//! app-data folder; no test touches the real app data or any personal file.

#![allow(dead_code)] // each test file uses a different subset

pub mod disk_image;
pub mod setup;
pub mod snapshot;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use traeva_core::{AppError, DATABASE_FILE_NAME, OrganizeService, StorageInfo, StorageStatus};

pub fn app_data_dir(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("Traeva")
}

pub fn database_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DATABASE_FILE_NAME)
}

pub fn expect_ready(service: &OrganizeService) -> StorageInfo {
    let state = service.get_app_state().expect("get_app_state failed");
    match state.storage {
        StorageStatus::Ready(info) => info,
        StorageStatus::Blocked { error } => panic!("expected ready storage, got {error:?}"),
    }
}

pub fn expect_blocked(service: &OrganizeService) -> AppError {
    let state = service.get_app_state().expect("get_app_state failed");
    match state.storage {
        StorageStatus::Blocked { error } => error,
        StorageStatus::Ready(info) => panic!("expected blocked storage, got {info:?}"),
    }
}

/// A separate connection for inspecting the database, with foreign keys on
/// (SQLite turns them off by default for every new connection).
pub fn open_for_inspection(app_data_dir: &Path) -> Connection {
    let conn = Connection::open(database_path(app_data_dir)).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn
}

pub fn user_tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

pub fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}
