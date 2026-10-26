//! SQLite storage: app-data folder, process lock, connection settings, migrations.
//!
//! Startup order (each step stops at the first problem and keeps every file as is):
//!   0. before writing anything, read the saved folders without writing
//!      (`saved_roots`) so the caller can check that the app-data folder is
//!      not inside one of them (#47)
//!   1. create the app-data folder
//!   2. take the process lock, so a second Traeva cannot write the same database
//!   3. open the one database connection
//!   4. apply the PRAGMA settings and read them back
//!   5. run SQLite's quick integrity check
//!   6. apply missing migrations, each in its own transaction
//!
//! Traeva never deletes or recreates the database to get past an error.
//!
//! Organize's queries live in this file. A Memory or Source feature keeps
//! its own queries in its own file, in its own `impl Store` block, through
//! `connection`, `sqlite_error` and `to_json` (#65). Its tables still come
//! from a numbered migration listed in `MIGRATIONS` here.

use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, MAIN_DB, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionStatus, AppDataSource, AppError, ErrorCode, PlanKind, PlanLifecycle, PragmaSettings,
    RootStatus, StorageInfo, UserChoice,
};
use crate::platform::macos;

pub const DATABASE_FILE_NAME: &str = "traeva.sqlite3";
pub const LOCK_FILE_NAME: &str = "traeva.lock";

/// Migration N is at index N - 1, and `PRAGMA user_version` = number of applied migrations.
/// Never edit a migration after it is committed: add a new file at the end instead.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_snapshots.sql"),
    include_str!("../migrations/0003_memory_watches.sql"),
    include_str!("../migrations/0004_memory_changes.sql"),
    include_str!("../migrations/0005_sources.sql"),
    include_str!("../migrations/0006_source_capture_window.sql"),
    include_str!("../migrations/0007_memory_families.sql"),
    include_str!("../migrations/0008_memory_work_decisions.sql"),
    include_str!("../migrations/0009_memory_invalid_reasons.sql"),
    include_str!("../migrations/0010_organize_auto_drop.sql"),
    include_str!("../migrations/0011_memory_auto.sql"),
    include_str!("../migrations/0012_memory_auto_links.sql"),
];

/// The schema version this build of Traeva understands.
pub const LATEST_SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub struct Store {
    app_data_dir: PathBuf,
    database_path: PathBuf,
    /// Where the app-data folder came from (#65); shown with the storage.
    source: AppDataSource,
    /// The single write connection. Hold the mutex only for short database work,
    /// never across a file operation or a network wait.
    conn: Mutex<Connection>,
    /// Kept open for the life of the process. The operating system drops the lock
    /// when the process exits, even after a crash, so no stale lock is left behind.
    _process_lock: File,
}

/// The folders saved as authorized, read before Traeva writes anything
/// (#47). SQLite opens the file read-only and `immutable`: it takes no lock
/// and leaves a leftover rollback journal alone, so nothing is written.
/// Normally only the active roots count. While a rollback journal is left
/// over from a crash, every root counts, because that journal may still undo
/// a revocation. Empty when there is no database yet.
pub fn saved_roots(app_data_dir: &Path) -> Result<Vec<RootEvidence>, AppError> {
    let database = app_data_dir.join(DATABASE_FILE_NAME);
    match fs::metadata(&database) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(AppError::storage(
                ErrorCode::AppDataUnavailable,
                format!(
                    "Cannot look at the database in {} ({}).",
                    app_data_dir.display(),
                    err.kind()
                ),
            ));
        }
    }
    let fail = |err: rusqlite::Error| {
        sqlite_error("Cannot read the authorized folders before starting", &err)
    };
    let uri = format!("file:{}?mode=ro&immutable=1", uri_path(&database));
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(&uri, flags).map_err(fail)?;
    let has_roots: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master
             WHERE type = 'table' AND name = 'authorized_roots'",
            [],
            |r| r.get(0),
        )
        .map_err(fail)?;
    if !has_roots {
        return Ok(Vec::new());
    }
    let journal = app_data_dir.join(format!("{DATABASE_FILE_NAME}-journal"));
    let leftover_journal = fs::metadata(&journal).is_ok_and(|meta| meta.len() > 0);
    let sql = if leftover_journal {
        "SELECT evidence_json FROM authorized_roots"
    } else {
        "SELECT evidence_json FROM authorized_roots WHERE status = 'active'"
    };
    let mut stmt = conn.prepare(sql).map_err(fail)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(fail)?;
    let mut roots = Vec::new();
    for row in rows {
        let evidence = serde_json::from_str(&row.map_err(fail)?).map_err(|_| {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved folder could not be read. Traeva kept it unchanged.",
            )
        })?;
        roots.push(evidence);
    }
    Ok(roots)
}

/// A path for an SQLite `file:` URI: every byte except letters, digits,
/// `-._~` and `/` is written as `%XX`.
fn uri_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl Store {
    pub fn open(app_data_dir: &Path) -> Result<Store, AppError> {
        fs::create_dir_all(app_data_dir).map_err(|err| {
            AppError::storage(
                ErrorCode::AppDataUnavailable,
                format!(
                    "Cannot create the app data folder {} ({}).",
                    app_data_dir.display(),
                    err.kind()
                ),
            )
        })?;

        let process_lock = take_process_lock(app_data_dir)?;

        let database_path = app_data_dir.join(DATABASE_FILE_NAME);
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
        let mut conn = Connection::open_with_flags(&database_path, flags)
            .map_err(|err| sqlite_error("Cannot open the database", &err))?;

        // SQLite silently falls back to read-only when the file is write-protected.
        let read_only = conn
            .is_readonly(MAIN_DB)
            .map_err(|err| sqlite_error("Cannot open the database", &err))?;
        if read_only {
            return Err(AppError::storage(
                ErrorCode::DatabaseUnavailable,
                format!(
                    "The database {} is read-only. Traeva needs to write to it.",
                    database_path.display()
                ),
            ));
        }

        apply_pragmas(&conn)?;
        check_integrity(&conn)?;
        migrate(&mut conn)?;

        Ok(Store {
            app_data_dir: app_data_dir.to_path_buf(),
            database_path,
            source: AppDataSource::Default,
            conn: Mutex::new(conn),
            _process_lock: process_lock,
        })
    }

    /// Reads the live database state (not a copy saved at startup).
    pub fn status(&self) -> Result<StorageInfo, AppError> {
        let conn = self.connection()?;
        Ok(StorageInfo {
            app_data_dir: self.app_data_dir.display().to_string(),
            database_path: self.database_path.display().to_string(),
            schema_version: read_user_version(&conn)?,
            pragmas: read_pragmas(&conn)?,
            app_data_source: self.source,
        })
    }

    /// Marks this as a test folder named by `TRAEVA_APP_DATA_DIR` (#65).
    pub(crate) fn mark_test_folder(&mut self) {
        self.source = AppDataSource::Test;
    }

    /// The one write connection. Hold the guard only for short database
    /// work, never across a file operation or a network wait.
    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.conn.lock().map_err(|_| {
            AppError::storage(
                ErrorCode::DatabaseUnavailable,
                "An earlier database operation stopped unexpectedly. Restart Traeva.",
            )
        })
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    /// Saves `new_root`, unless an active root already points at the same
    /// physical folder; then that root is returned and nothing is written.
    /// The check and the insert run in one transaction.
    pub fn insert_root_unless_active(&self, new_root: &RootRow) -> Result<RootRow, AppError> {
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|err| sqlite_error("Cannot save the folder", &err))?;
        let active = query_roots(&tx, "WHERE status = 'active'", [])?;
        if let Some(existing) = active
            .into_iter()
            .find(|row| row.evidence.same_object(&new_root.evidence))
        {
            return Ok(existing);
        }
        let evidence = serde_json::to_string(&new_root.evidence).map_err(|_| {
            AppError::storage(ErrorCode::DatabaseUnavailable, "Cannot save the folder.")
        })?;
        tx.execute(
            "INSERT INTO authorized_roots
                 (root_id, display_path, evidence_json, status, granted_at_ms, revoked_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new_root.root_id,
                new_root.display_path,
                evidence,
                status_text(new_root.status),
                new_root.granted_at_ms,
                new_root.revoked_at_ms
            ],
        )
        .map_err(|err| sqlite_error("Cannot save the folder", &err))?;
        tx.commit()
            .map_err(|err| sqlite_error("Cannot save the folder", &err))?;
        Ok(new_root.clone())
    }

    pub fn get_root(&self, root_id: &str) -> Result<Option<RootRow>, AppError> {
        let conn = self.connection()?;
        Ok(query_roots(&conn, "WHERE root_id = ?1", [root_id])?
            .into_iter()
            .next())
    }

    pub fn active_roots(&self) -> Result<Vec<RootRow>, AppError> {
        let conn = self.connection()?;
        query_roots(&conn, "WHERE status = 'active' ORDER BY granted_at_ms", [])
    }

    /// Marks an active root as revoked. Revocation is permanent: no code sets a
    /// root back to active. Returns the row as it is now, or None if unknown.
    pub fn revoke_root(&self, root_id: &str, now_ms: i64) -> Result<Option<RootRow>, AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE authorized_roots SET status = 'revoked', revoked_at_ms = ?2
             WHERE root_id = ?1 AND status = 'active'",
            params![root_id, now_ms],
        )
        .map_err(|err| sqlite_error("Cannot revoke the folder", &err))?;
        Ok(query_roots(&conn, "WHERE root_id = ?1", [root_id])?
            .into_iter()
            .next())
    }
}

impl Store {
    /// Records one direct, stable observation of the file at `relative_path`,
    /// in one transaction (organize-spec.md §2.3, O01):
    ///
    /// - The present record at that path is the same physical object → keep its
    ///   file_id; raise `record_revision` if anything about the file changed.
    /// - Otherwise this is a new file → new file_id and an `intake` entry in
    ///   path_history. A present record of a *different* object at that path
    ///   (the file was replaced) becomes `missing`; it is never merged.
    ///
    /// Same content alone never links two records: copies get their own file_id.
    pub fn record_observation(&self, seen: &SeenFile) -> Result<FileRow, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the file", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        let row = observe_in(&tx, seen)?;
        tx.commit().map_err(fail)?;
        Ok(row)
    }
}

/// `record_observation` inside a transaction the caller holds and commits:
/// Memory registers an object saved as a candidate in the same transaction
/// as its snapshot (#31), by exactly these rules.
pub(crate) fn observe_in(tx: &Connection, seen: &SeenFile) -> Result<FileRow, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot save the file", &err);
    let present = query_files(
        tx,
        "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
        params![seen.root_id, seen.relative_path],
    )?
    .into_iter()
    .next();

    let row = match present {
        Some(old) if old.evidence.same_object(&seen.evidence) => {
            let changed = old.sha256 != seen.sha256
                || old.byte_length != seen.byte_length
                || !old.evidence.same_state(&seen.evidence);
            let revision = old.record_revision + i64::from(changed);
            tx.execute(
                "UPDATE files SET sha256 = ?2, byte_length = ?3, observation_json = ?4,
                     observed_at_ms = ?5, record_revision = ?6, updated_at_ms = ?5
                 WHERE file_id = ?1",
                params![
                    old.file_id,
                    seen.sha256,
                    seen.byte_length as i64,
                    to_json(&seen.evidence)?,
                    seen.now_ms,
                    revision
                ],
            )
            .map_err(fail)?;
            FileRow {
                sha256: seen.sha256.clone(),
                byte_length: seen.byte_length,
                evidence: seen.evidence.clone(),
                observed_at_ms: seen.now_ms,
                record_revision: revision,
                updated_at_ms: seen.now_ms,
                ..old
            }
        }
        replaced => {
            if let Some(old) = replaced {
                tx.execute(
                    "UPDATE files SET presence = 'missing', updated_at_ms = ?2 WHERE file_id = ?1",
                    params![old.file_id, seen.now_ms],
                )
                .map_err(fail)?;
            }
            let row = FileRow {
                file_id: uuid::Uuid::new_v4().to_string(),
                root_id: seen.root_id.clone(),
                relative_path: seen.relative_path.clone(),
                basename: seen.basename.clone(),
                sha256: seen.sha256.clone(),
                byte_length: seen.byte_length,
                evidence: seen.evidence.clone(),
                observed_at_ms: seen.now_ms,
                record_revision: 1,
                created_at_ms: seen.now_ms,
                updated_at_ms: seen.now_ms,
            };
            tx.execute(
                "INSERT INTO files (file_id, root_id, relative_path, basename, presence, sha256,
                     byte_length, observation_json, observed_at_ms, record_revision,
                     created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, 'present', ?5, ?6, ?7, ?8, 1, ?8, ?8)",
                params![
                    row.file_id,
                    row.root_id,
                    row.relative_path,
                    row.basename,
                    row.sha256,
                    row.byte_length as i64,
                    to_json(&row.evidence)?,
                    row.observed_at_ms
                ],
            )
            .map_err(fail)?;
            tx.execute(
                "INSERT INTO path_history (path_event_id, file_id, to_root_id, to_relative_path,
                     reason, evidence_json, recorded_at_ms)
                 VALUES (?1, ?2, ?3, ?4, 'intake', ?5, ?6)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    row.file_id,
                    row.root_id,
                    row.relative_path,
                    to_json(&row.evidence)?,
                    row.observed_at_ms
                ],
            )
            .map_err(fail)?;
            row
        }
    };
    Ok(row)
}

/// Physical evidence and times of a file's latest observation, stored as JSON
/// in `files.observation_json`. Evidence only, never the file's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEvidence {
    /// The device number when it was saved; kept to help diagnose problems,
    /// never compared with a later observation (see `same_volume`).
    pub dev: i64,
    /// The volume's UUID (`macos::volume_uuid`). None in records saved
    /// before it existed, or when it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    pub ino: u64,
    pub birthtime_ns: i64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub link_count: u64,
}

/// Two saved or seen objects are on the same volume, as far as Traeva can
/// tell. `dev` is never compared over time: macOS can number a volume
/// differently after a restart, so only the volume UUIDs count, and a record
/// without one (saved before it existed, or unreadable) is not compared.
pub(crate) fn same_volume(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

impl FileEvidence {
    /// The same physical object (its content may still have changed): the
    /// same inode number and birth time, on the same volume.
    pub fn same_object(&self, other: &FileEvidence) -> bool {
        self.ino == other.ino
            && self.birthtime_ns == other.birthtime_ns
            && same_volume(&self.volume, &other.volume)
    }

    /// The same object (`same_object`, so the volumes count when both records
    /// have one), and nothing about it changed: the same times and link
    /// count. Another `dev`, or a `volume` added to an older record, is not a
    /// change.
    pub(crate) fn same_state(&self, other: &FileEvidence) -> bool {
        self.same_object(other)
            && self.mtime_ns == other.mtime_ns
            && self.ctime_ns == other.ctime_ns
            && self.link_count == other.link_count
    }

    /// This fresh observation shows the physical object of an older record
    /// (`older`), under exactly one name: the live check that lets Source's
    /// links (#27) and a Memory record (#31 relocation) go on to the newer
    /// record. It never merges the two records.
    pub(crate) fn shows_object_of(&self, older: &FileEvidence) -> bool {
        self.link_count == 1 && self.same_object(older)
    }
}

/// One stable observation, ready to be recorded.
#[derive(Debug, Clone)]
pub struct SeenFile {
    pub root_id: String,
    pub relative_path: String,
    pub basename: String,
    pub sha256: String,
    pub byte_length: u64,
    pub evidence: FileEvidence,
    pub now_ms: i64,
}

/// One row of `files`. Presence is reported separately by `get_file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub file_id: String,
    pub root_id: String,
    pub relative_path: String,
    pub basename: String,
    pub sha256: String,
    pub byte_length: u64,
    pub evidence: FileEvidence,
    pub observed_at_ms: i64,
    pub record_revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// `filter` is always a constant written in this file; values go in `params`.
pub(crate) fn query_files<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<FileRow>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the files", &err);
    let sql = format!(
        "SELECT file_id, root_id, relative_path, basename, sha256, byte_length, observation_json,
                observed_at_ms, record_revision, created_at_ms, updated_at_ms
         FROM files {filter}"
    );
    let mut stmt = conn.prepare(&sql).map_err(fail)?;
    let raw_rows = stmt
        .query_map(params, |row| {
            Ok((
                (
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ),
                (
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ),
            ))
        })
        .map_err(fail)?;
    let mut files = Vec::new();
    for raw in raw_rows {
        let (
            (file_id, root_id, relative_path, basename, sha256, byte_length),
            (evidence_json, observed_at_ms, record_revision, created_at_ms, updated_at_ms),
        ) = raw.map_err(fail)?;
        let evidence = serde_json::from_str(&evidence_json).map_err(|_| {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved file record could not be read. Traeva kept it unchanged.",
            )
        })?;
        files.push(FileRow {
            file_id,
            root_id,
            relative_path,
            basename,
            sha256,
            byte_length: u64::try_from(byte_length).unwrap_or(0),
            evidence,
            observed_at_ms,
            record_revision,
            created_at_ms,
            updated_at_ms,
        });
    }
    Ok(files)
}

impl Store {
    /// A file record and whether it is still "present" at its saved location.
    pub fn get_file(&self, file_id: &str) -> Result<Option<(FileRow, bool)>, AppError> {
        let conn = self.connection()?;
        let Some(row) = query_files(&conn, "WHERE file_id = ?1", [file_id])?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let present: bool = conn
            .query_row(
                "SELECT presence = 'present' FROM files WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .map_err(|err| sqlite_error("Cannot read the files", &err))?;
        Ok(Some((row, present)))
    }

    /// Saves a new plan. In the same transaction every other open, unused plan
    /// of the same file becomes `superseded`, so a file never has two plans
    /// that could both run. Plans already used by an action are not touched.
    pub fn insert_plan(&self, plan: &PlanRow) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the plan", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        tx.execute(
            "UPDATE plans SET lifecycle = 'superseded', lifecycle_changed_at_ms = ?2,
                 lifecycle_reason = ?3
             WHERE file_id = ?1 AND lifecycle = 'open'
               AND plan_id NOT IN (SELECT plan_id FROM actions)",
            params![
                plan.file_id,
                plan.created_at_ms,
                format!("replaced by plan {}", plan.plan_id)
            ],
        )
        .map_err(fail)?;
        // An organize plan records how its target was chosen; an undo plan
        // instead names the organize action it puts back.
        let kind = match plan.kind {
            PlanKind::Organize => "organize",
            PlanKind::Undo => "undo",
        };
        let user_choice = plan.user_choice.map(choice_text);
        tx.execute(
            "INSERT INTO plans (plan_id, kind, file_id, source_root_id, source_relative_path,
                 expected_source_json, target_root_id, target_segments_json, basename,
                 folder_chain_json, folders_to_create_json, analysis_request_id, user_choice,
                 undo_of_action_id, created_at_ms, lifecycle)
             VALUES (?1, ?12, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?15, ?13,
                 ?14, ?11, 'open')",
            params![
                plan.plan_id,
                plan.file_id,
                plan.source_root_id,
                plan.source_relative_path,
                to_json(&plan.expected_source)?,
                plan.target_root_id,
                to_json(&plan.target_segments)?,
                plan.basename,
                to_json(&plan.folder_chain)?,
                to_json(&plan.folders_to_create)?,
                plan.created_at_ms,
                kind,
                user_choice,
                plan.undo_of_action_id,
                plan.analysis_request_id
            ],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)
    }

    pub fn get_plan(&self, plan_id: &str) -> Result<Option<PlanRow>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the plan", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT p.plan_id, p.kind, p.file_id, p.source_root_id, p.source_relative_path,
                        p.expected_source_json, p.target_root_id, p.target_segments_json,
                        p.basename, p.folder_chain_json, p.folders_to_create_json,
                        p.created_at_ms, p.lifecycle, p.lifecycle_changed_at_ms,
                        p.lifecycle_reason, a.action_id, p.undo_of_action_id,
                        p.user_choice, p.analysis_request_id
                 FROM plans AS p LEFT JOIN actions AS a ON a.plan_id = p.plan_id
                 WHERE p.plan_id = ?1",
            )
            .map_err(fail)?;
        let mut rows = stmt.query([plan_id]).map_err(fail)?;
        let Some(row) = rows.next().map_err(fail)? else {
            return Ok(None);
        };
        let text = |index: usize| row.get::<_, String>(index).map_err(fail);
        let unreadable = || {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved plan could not be read. Traeva kept it unchanged.",
            )
        };
        Ok(Some(PlanRow {
            plan_id: text(0)?,
            kind: match text(1)?.as_str() {
                "organize" => PlanKind::Organize,
                "undo" => PlanKind::Undo,
                _ => return Err(unreadable()),
            },
            file_id: text(2)?,
            source_root_id: text(3)?,
            source_relative_path: text(4)?,
            expected_source: serde_json::from_str(&text(5)?).map_err(|_| unreadable())?,
            target_root_id: text(6)?,
            target_segments: serde_json::from_str(&text(7)?).map_err(|_| unreadable())?,
            basename: text(8)?,
            folder_chain: serde_json::from_str(&text(9)?).map_err(|_| unreadable())?,
            folders_to_create: serde_json::from_str(&text(10)?).map_err(|_| unreadable())?,
            created_at_ms: row.get(11).map_err(fail)?,
            lifecycle: parse_lifecycle(&text(12)?).ok_or_else(unreadable)?,
            lifecycle_changed_at_ms: row.get(13).map_err(fail)?,
            lifecycle_reason: row.get(14).map_err(fail)?,
            action_id: row.get(15).map_err(fail)?,
            undo_of_action_id: row.get(16).map_err(fail)?,
            user_choice: match row.get::<_, Option<String>>(17).map_err(fail)? {
                None => None,
                Some(text) => Some(parse_choice(&text).ok_or_else(unreadable)?),
            },
            analysis_request_id: row.get(18).map_err(fail)?,
        }))
    }

    /// Ends an open plan that no action has used yet (cancelled, superseded or
    /// invalidated). Only the lifecycle columns change; the payload never does.
    /// Returns false when the plan was not open or was already used.
    pub fn close_plan(
        &self,
        plan_id: &str,
        lifecycle: PlanLifecycle,
        now_ms: i64,
        reason: &str,
    ) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "UPDATE plans SET lifecycle = ?2, lifecycle_changed_at_ms = ?3, lifecycle_reason = ?4
                 WHERE plan_id = ?1 AND lifecycle = 'open'
                   AND plan_id NOT IN (SELECT plan_id FROM actions)",
                params![plan_id, lifecycle_text(lifecycle), now_ms, reason],
            )
            .map_err(|err| sqlite_error("Cannot update the plan", &err))?;
        Ok(changed == 1)
    }

    /// Ends every open plan of a file that no action has used: the user's
    /// latest choice is the place the file is in now, so no earlier choice
    /// may still run (#71). Plans already used by an action are not touched.
    pub fn supersede_open_plans(
        &self,
        file_id: &str,
        now_ms: i64,
        reason: &str,
    ) -> Result<usize, AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE plans SET lifecycle = 'superseded', lifecycle_changed_at_ms = ?2,
                 lifecycle_reason = ?3
             WHERE file_id = ?1 AND lifecycle = 'open'
               AND plan_id NOT IN (SELECT plan_id FROM actions)",
            params![file_id, now_ms, reason],
        )
        .map_err(|err| sqlite_error("Cannot update the plans", &err))
    }

    /// At startup: every open plan that no action has used becomes invalidated
    /// (organize-spec.md §3: a restart invalidates unconsumed plans).
    pub fn invalidate_open_plans(&self, now_ms: i64, reason: &str) -> Result<usize, AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE plans SET lifecycle = 'invalidated', lifecycle_changed_at_ms = ?1,
                 lifecycle_reason = ?2
             WHERE lifecycle = 'open' AND plan_id NOT IN (SELECT plan_id FROM actions)",
            params![now_ms, reason],
        )
        .map_err(|err| sqlite_error("Cannot update the plans", &err))
    }
}

/// The expected source of a plan, stored as `plans.expected_source_json`:
/// the full observation the user previewed, plus the folders above the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedSource {
    pub file_revision: i64,
    pub sha256: String,
    pub byte_length: u64,
    pub evidence: FileEvidence,
    pub folder_chain: Vec<ChainLink>,
}

/// One folder of a checked chain: its name and physical evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainLink {
    pub name: String,
    /// For diagnosis only, like `FileEvidence::dev`.
    pub dev: i64,
    /// The volume's UUID; None in plans saved before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    pub ino: u64,
}

impl ChainLink {
    /// The same folder under the same name (the `same_volume` rule).
    pub(crate) fn same_folder(&self, other: &ChainLink) -> bool {
        self.name == other.name && self.ino == other.ino && same_volume(&self.volume, &other.volume)
    }
}

/// One row of `plans`: a manual organize plan or an undo plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRow {
    pub plan_id: String,
    pub kind: PlanKind,
    pub file_id: String,
    pub source_root_id: String,
    pub source_relative_path: String,
    pub expected_source: ExpectedSource,
    pub target_root_id: String,
    /// Existing folders, then the folders to create.
    pub target_segments: Vec<String>,
    pub basename: String,
    /// Evidence for the existing target folders (not the root itself).
    pub folder_chain: Vec<ChainLink>,
    pub folders_to_create: Vec<String>,
    pub created_at_ms: i64,
    pub lifecycle: PlanLifecycle,
    pub lifecycle_changed_at_ms: Option<i64>,
    pub lifecycle_reason: Option<String>,
    /// The action that used this plan, if any (read from `actions`).
    pub action_id: Option<String>,
    /// For an undo plan: the organize action it puts back.
    pub undo_of_action_id: Option<String>,
    /// For an organize plan: how the target was chosen (#16).
    pub user_choice: Option<UserChoice>,
    /// The model request whose suggestion was shown, if any.
    pub analysis_request_id: Option<String>,
}

impl PlanRow {
    /// The full target path inside the target root, e.g. "Notes/2026/todo.txt".
    pub fn target_path(&self) -> String {
        join_path(&self.target_segments, &self.basename)
    }
}

/// ["a", "b"] + "c.txt" → "a/b/c.txt".
fn join_path(folders: &[String], name: &str) -> String {
    folders
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join("/")
}

fn lifecycle_text(lifecycle: PlanLifecycle) -> &'static str {
    match lifecycle {
        PlanLifecycle::Open => "open",
        PlanLifecycle::Cancelled => "cancelled",
        PlanLifecycle::Superseded => "superseded",
        PlanLifecycle::Invalidated => "invalidated",
    }
}

fn parse_lifecycle(text: &str) -> Option<PlanLifecycle> {
    match text {
        "open" => Some(PlanLifecycle::Open),
        "cancelled" => Some(PlanLifecycle::Cancelled),
        "superseded" => Some(PlanLifecycle::Superseded),
        "invalidated" => Some(PlanLifecycle::Invalidated),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Actions and their journal (organize-spec.md §3, §6)
// ---------------------------------------------------------------------------

/// The outcome of the confirmation transaction. Only `Created` wrote anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmation {
    /// The action now exists as `prepared`, with its first journal entry.
    Created,
    /// An action already used this plan.
    AlreadyUsed(String),
    /// The plan is no longer open.
    PlanClosed,
    /// A root of the plan was revoked.
    RootRevoked,
    /// An action that is not settled yet involves the same file or one of
    /// the plan's two locations.
    FileBusy(String),
    /// An undo plan whose organize action is no longer committed.
    NotUndoable,
}

impl Store {
    /// The confirmation: in one short transaction, check that the plan can
    /// still be used, then create its one action as `prepared` together with
    /// the first journal entry. Either all of it commits or nothing does.
    pub fn confirm_plan(
        &self,
        plan: &PlanRow,
        action_id: &str,
        preconditions: &Preconditions,
        now_ms: i64,
    ) -> Result<Confirmation, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot confirm the plan", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        // Returning early drops `tx`, which rolls it back: nothing is written.
        let used: Option<String> = tx
            .query_row(
                "SELECT action_id FROM actions WHERE plan_id = ?1",
                [&plan.plan_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(fail)?;
        if let Some(existing) = used {
            return Ok(Confirmation::AlreadyUsed(existing));
        }
        let lifecycle: String = tx
            .query_row(
                "SELECT lifecycle FROM plans WHERE plan_id = ?1",
                [&plan.plan_id],
                |row| row.get(0),
            )
            .map_err(fail)?;
        if lifecycle != "open" {
            return Ok(Confirmation::PlanClosed);
        }
        let active: i64 = tx
            .query_row(
                "SELECT count(*) FROM authorized_roots
                 WHERE root_id IN (?1, ?2) AND status = 'active'",
                params![plan.source_root_id, plan.target_root_id],
                |row| row.get(0),
            )
            .map_err(fail)?;
        let roots = if plan.source_root_id == plan.target_root_id {
            1
        } else {
            2
        };
        if active != roots {
            return Ok(Confirmation::RootRevoked);
        }
        if let Some(busy) = conflicting_action(&tx, plan)? {
            return Ok(Confirmation::FileBusy(busy));
        }
        // An undo needs its organize action still committed (not undone).
        if let Some(original) = &plan.undo_of_action_id {
            let status: Option<String> = tx
                .query_row(
                    "SELECT status FROM actions WHERE action_id = ?1",
                    [original],
                    |row| row.get(0),
                )
                .optional()
                .map_err(fail)?;
            if status.as_deref() != Some("committed") {
                return Ok(Confirmation::NotUndoable);
            }
        }
        let kind = match plan.kind {
            PlanKind::Organize => "organize",
            PlanKind::Undo => "undo",
        };
        tx.execute(
            "INSERT INTO actions (action_id, plan_id, kind, undo_of_action_id, status,
                 preconditions_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, 'prepared', ?5, ?6)",
            params![
                action_id,
                plan.plan_id,
                kind,
                plan.undo_of_action_id,
                to_json(preconditions)?,
                now_ms
            ],
        )
        .map_err(fail)?;
        insert_event(
            &tx,
            action_id,
            "confirm",
            Phase::Result,
            &Confirmed {
                plan_id: plan.plan_id.clone(),
            },
            now_ms,
        )?;
        tx.commit().map_err(fail)?;
        Ok(Confirmation::Created)
    }

    /// prepared → running, committed on its own before any file change.
    pub fn mark_running(&self, action_id: &str) -> Result<(), AppError> {
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "UPDATE actions SET status = 'running' WHERE action_id = ?1 AND status = 'prepared'",
                [action_id],
            )
            .map_err(|err| sqlite_error("Cannot update the action", &err))?;
        if changed != 1 {
            return Err(unexpected_records("Cannot update the action"));
        }
        Ok(())
    }

    /// Appends one journal entry and commits it on its own: an intent before
    /// the step it describes, or a result after the step was checked.
    pub fn append_event<T: Serialize>(
        &self,
        action_id: &str,
        step: &str,
        phase: Phase,
        detail: &T,
        now_ms: i64,
    ) -> Result<(), AppError> {
        let conn = self.connection()?;
        insert_event(&conn, action_id, step, phase, detail, now_ms)
    }

    /// The final transaction after a checked move: the move result, the
    /// file's new location, one path-history entry and `committed`.
    /// All of it commits, or none of it.
    pub fn commit_move(&self, record: &MoveRecord) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot record the move", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        insert_event(
            &tx,
            record.action_id,
            "move",
            Phase::Result,
            record.result,
            record.now_ms,
        )?;
        record_move(&tx, record)?;
        let committed = tx
            .execute(
                "UPDATE actions SET status = 'committed', committed_at_ms = ?2
                 WHERE action_id = ?1 AND status = 'running'",
                params![record.action_id, record.now_ms],
            )
            .map_err(fail)?;
        if committed != 1 {
            return Err(unexpected_records("Cannot record the move"));
        }
        mark_undone(&tx, record)?;
        tx.commit().map_err(fail)
    }

    /// The record check found the move done exactly as planned: the same
    /// final transaction as `commit_move`, late, plus the check's journal
    /// entry and `recovered_at`. Only an unsettled action can be committed
    /// this way, so checking again never records anything twice.
    pub fn commit_recovered_move(
        &self,
        record: &MoveRecord,
        check: &Reconciliation,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot record the move", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        insert_event(
            &tx,
            record.action_id,
            "move",
            Phase::Result,
            record.result,
            record.now_ms,
        )?;
        insert_event(
            &tx,
            record.action_id,
            "reconcile",
            Phase::Result,
            check,
            record.now_ms,
        )?;
        record_move(&tx, record)?;
        let committed = tx
            .execute(
                "UPDATE actions SET status = 'committed', committed_at_ms = ?2,
                     recovered_at_ms = ?2, result_json = NULL
                 WHERE action_id = ?1
                   AND status IN ('prepared', 'running', 'needs_user_decision')",
                params![record.action_id, record.now_ms],
            )
            .map_err(fail)?;
        if committed != 1 {
            return Err(unexpected_records("Cannot record the move"));
        }
        mark_undone(&tx, record)?;
        tx.commit().map_err(fail)
    }

    /// The record check's decision when the file did not move or the result
    /// is uncertain: the new status with its reason, and a journal entry with
    /// what was found. All or nothing, for an unsettled action only.
    pub fn record_check(
        &self,
        action_id: &str,
        problem: &ActionProblem,
        check: &Reconciliation,
        now_ms: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot record the check", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        insert_event(&tx, action_id, "reconcile", Phase::Result, check, now_ms)?;
        let changed = tx
            .execute(
                "UPDATE actions SET status = ?2, result_json = ?3
                 WHERE action_id = ?1
                   AND status IN ('prepared', 'running', 'needs_user_decision')",
                params![
                    action_id,
                    action_status_text(check.status),
                    to_json(problem)?
                ],
            )
            .map_err(fail)?;
        if changed != 1 {
            return Err(unexpected_records("Cannot record the check"));
        }
        tx.commit().map_err(fail)
    }

    /// The failure handling's record: the final status with its reason, and
    /// a journal entry with what was found at both names. All or nothing.
    /// Only an action that is still prepared or running can be settled.
    pub fn settle_action(
        &self,
        action_id: &str,
        problem: &ActionProblem,
        settlement: &Settlement,
        now_ms: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot record the failure", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        insert_event(&tx, action_id, "settle", Phase::Result, settlement, now_ms)?;
        let changed = tx
            .execute(
                "UPDATE actions SET status = ?2, result_json = ?3
                 WHERE action_id = ?1 AND status IN ('prepared', 'running')",
                params![
                    action_id,
                    action_status_text(settlement.status),
                    to_json(problem)?
                ],
            )
            .map_err(fail)?;
        if changed != 1 {
            return Err(unexpected_records("Cannot record the failure"));
        }
        tx.commit().map_err(fail)
    }

    pub fn get_action(&self, action_id: &str) -> Result<Option<ActionRow>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the action", &err);
        let conn = self.connection()?;
        let raw = conn
            .query_row(
                "SELECT action_id, plan_id, kind, status, created_at_ms, committed_at_ms,
                        recovered_at_ms, result_json
                 FROM actions WHERE action_id = ?1",
                [action_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(fail)?;
        let Some((action_id, plan_id, kind, status, created, committed, recovered, result)) = raw
        else {
            return Ok(None);
        };
        let unreadable = || {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved action could not be read. Traeva kept it unchanged.",
            )
        };
        Ok(Some(ActionRow {
            action_id,
            plan_id,
            kind: match kind.as_str() {
                "organize" => PlanKind::Organize,
                "undo" => PlanKind::Undo,
                _ => return Err(unreadable()),
            },
            status: parse_action_status(&status).ok_or_else(unreadable)?,
            created_at_ms: created,
            committed_at_ms: committed,
            recovered_at_ms: recovered,
            problem: match result {
                Some(json) => Some(serde_json::from_str(&json).map_err(|_| unreadable())?),
                None => None,
            },
        }))
    }

    /// The action's journal in order. An entry whose detail does not have
    /// the shape its step and phase call for is kept as `None`, so a record
    /// check can treat it as a journal that does not match.
    pub fn journal(&self, action_id: &str) -> Result<Vec<JournalRow>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the journal", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT sequence, step, phase, detail_json FROM action_events
                 WHERE action_id = ?1 ORDER BY sequence",
            )
            .map_err(fail)?;
        let rows = stmt
            .query_map([action_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(fail)?;
        let mut journal = Vec::new();
        for row in rows {
            let (sequence, step, phase, detail) = row.map_err(fail)?;
            journal.push(JournalRow {
                sequence,
                entry: parse_entry(&step, &phase, &detail),
            });
        }
        Ok(journal)
    }

    /// Actions that are not settled (prepared, running, needs_user_decision),
    /// oldest first: the ones a record check looks at.
    pub fn unresolved_actions(&self) -> Result<Vec<String>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT action_id FROM actions
                 WHERE status IN ('prepared', 'running', 'needs_user_decision')
                 ORDER BY created_at_ms, action_id",
            )
            .map_err(fail)?;
        let ids = stmt.query_map([], |row| row.get(0)).map_err(fail)?;
        ids.collect::<Result<_, _>>().map_err(fail)
    }

    /// The file's action that is not settled yet (prepared, running or
    /// needs_user_decision), if any. Nothing new is planned or run for the
    /// file until it is settled.
    pub fn unresolved_action_of_file(&self, file_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT a.action_id FROM actions AS a JOIN plans AS p ON p.plan_id = a.plan_id
             WHERE p.file_id = ?1 AND a.status IN ('prepared', 'running', 'needs_user_decision')
             ORDER BY a.created_at_ms LIMIT 1",
            [file_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the actions", &err))
    }

    /// The newest actions, newest first, at most `limit`.
    pub fn recent_actions(&self, limit: u32) -> Result<Vec<String>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT action_id FROM actions ORDER BY created_at_ms DESC, action_id DESC LIMIT ?1",
            )
            .map_err(fail)?;
        let ids = stmt.query_map([limit], |row| row.get(0)).map_err(fail)?;
        ids.collect::<Result<_, _>>().map_err(fail)
    }

    /// Every action whose result needs the user's decision, oldest first.
    pub fn actions_needing_a_decision(&self) -> Result<Vec<String>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT action_id FROM actions WHERE status = 'needs_user_decision'
                 ORDER BY created_at_ms, action_id",
            )
            .map_err(fail)?;
        let ids = stmt.query_map([], |row| row.get(0)).map_err(fail)?;
        ids.collect::<Result<_, _>>().map_err(fail)
    }

    /// The undo actions of an organize action, oldest first.
    pub fn undo_actions_of(
        &self,
        action_id: &str,
    ) -> Result<Vec<(String, ActionStatus)>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT action_id, status FROM actions WHERE undo_of_action_id = ?1
                 ORDER BY created_at_ms, action_id",
            )
            .map_err(fail)?;
        let rows = stmt
            .query_map([action_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(fail)?;
        let mut undos = Vec::new();
        for row in rows {
            let (id, status) = row.map_err(fail)?;
            let status = parse_action_status(&status).ok_or_else(|| {
                AppError::storage(
                    ErrorCode::DatabaseCorrupt,
                    "A saved action could not be read. Traeva kept it unchanged.",
                )
            })?;
            undos.push((id, status));
        }
        Ok(undos)
    }

    /// The unsettled action a new plan would conflict with (student material
    /// for #10): one of the same file, or one whose source or target location
    /// is the plan's source or target location.
    pub fn conflicting_action(&self, plan: &PlanRow) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conflicting_action(&conn, plan)
    }

    /// The plan of every unsettled action, oldest first.
    pub fn unresolved_plans(&self) -> Result<Vec<(String, PlanRow)>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
        let pairs: Vec<(String, String)> = {
            let conn = self.connection()?;
            let mut stmt = conn
                .prepare(
                    "SELECT action_id, plan_id FROM actions
                     WHERE status IN ('prepared', 'running', 'needs_user_decision')
                     ORDER BY created_at_ms, action_id",
                )
                .map_err(fail)?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(fail)?;
            rows.collect::<Result<_, _>>().map_err(fail)?
        };
        let mut plans = Vec::new();
        for (action_id, plan_id) in pairs {
            if let Some(plan) = self.get_plan(&plan_id)? {
                plans.push((action_id, plan));
            }
        }
        Ok(plans)
    }
}

// ---------------------------------------------------------------------------
// Model requests (#14): one `analyses` row per request, metadata only
// ---------------------------------------------------------------------------

/// Where one model request is. Only `analyzing` changes to an outcome;
/// `answered` changes once more, when the caller saves the checked result
/// or rejects the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisStatus {
    /// Sent (or being sent); no outcome yet.
    Analyzing,
    /// The provider answered; the caller has not checked the answer yet.
    Answered,
    /// The caller checked the answer and saved the limited result.
    Validated,
    /// No usable answer; `error_code` says why.
    Failed,
    /// Cancelled, or the model setup changed while it ran. Any answer was dropped.
    Cancelled,
    /// Traeva quit while the request was waiting (set at the next start).
    Interrupted,
}

impl AnalysisStatus {
    fn text(self) -> &'static str {
        match self {
            AnalysisStatus::Analyzing => "analyzing",
            AnalysisStatus::Answered => "answered",
            AnalysisStatus::Validated => "validated",
            AnalysisStatus::Failed => "failed",
            AnalysisStatus::Cancelled => "cancelled",
            AnalysisStatus::Interrupted => "interrupted",
        }
    }

    fn parse(text: &str) -> Option<AnalysisStatus> {
        [
            AnalysisStatus::Analyzing,
            AnalysisStatus::Answered,
            AnalysisStatus::Validated,
            AnalysisStatus::Failed,
            AnalysisStatus::Cancelled,
            AnalysisStatus::Interrupted,
        ]
        .into_iter()
        .find(|status| status.text() == text)
    }
}

/// A new `analyses` row. Only facts about the request, never its text.
pub struct AnalysisRow<'a> {
    pub request_id: &'a str,
    pub file_id: &'a str,
    pub file_sha256: &'a str,
    pub file_revision: i64,
    pub provider: &'a str,
    pub model: &'a str,
    pub requested_at_ms: i64,
    pub prompt_version: &'a str,
    pub output_schema_version: &'a str,
    pub extractor_version: &'a str,
    pub app_version: &'a str,
    /// Parameters and the caller's non-content context.
    pub request_meta: &'a serde_json::Value,
    pub status: AnalysisStatus,
    pub error_code: Option<ErrorCode>,
}

/// How a request that was `analyzing` ended.
pub enum AnalysisEnd {
    Answered {
        latency_ms: i64,
        usage: Option<serde_json::Value>,
        provider_request_id: Option<String>,
    },
    Failed {
        code: ErrorCode,
        latency_ms: Option<i64>,
        provider_request_id: Option<String>,
    },
    Cancelled,
}

impl Store {
    /// Saves a new request. A request ID can be used only once.
    pub fn insert_analysis(&self, row: &AnalysisRow) -> Result<(), AppError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO analyses
                (request_id, file_id, file_sha256, file_revision, provider, model,
                 requested_at_ms, prompt_version, output_schema_version, extractor_version,
                 app_version, request_meta_json, status, error_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                row.request_id,
                row.file_id,
                row.file_sha256,
                row.file_revision,
                row.provider,
                row.model,
                row.requested_at_ms,
                row.prompt_version,
                row.output_schema_version,
                row.extractor_version,
                row.app_version,
                to_json(row.request_meta)?,
                row.status.text(),
                row.error_code.map(code_text),
            ],
        )
        .map_err(|err| match err.sqlite_error_code() {
            Some(rusqlite::ErrorCode::ConstraintViolation) => AppError::analysis(
                ErrorCode::InvalidInput,
                "This request ID was used before.",
                false,
                Some(row.file_id),
            ),
            _ => sqlite_error("Cannot save the model request", &err),
        })?;
        Ok(())
    }

    /// Saves how a request ended, but only while it is still `analyzing`.
    /// False means it was not (it was cancelled meanwhile): the answer is
    /// dropped and the row stays as it is.
    pub fn finish_analysis(&self, request_id: &str, end: &AnalysisEnd) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let (status, error_code, latency_ms, usage, provider_request_id) = match end {
            AnalysisEnd::Answered {
                latency_ms,
                usage,
                provider_request_id,
            } => (
                AnalysisStatus::Answered,
                None,
                Some(*latency_ms),
                usage.as_ref().map(to_json).transpose()?,
                provider_request_id.clone(),
            ),
            AnalysisEnd::Failed {
                code,
                latency_ms,
                provider_request_id,
            } => (
                AnalysisStatus::Failed,
                Some(code_text(*code)),
                *latency_ms,
                None,
                provider_request_id.clone(),
            ),
            AnalysisEnd::Cancelled => (AnalysisStatus::Cancelled, None, None, None, None),
        };
        let changed = conn
            .execute(
                "UPDATE analyses SET status = ?2, error_code = ?3, latency_ms = ?4,
                     usage_json = ?5, provider_request_id = ?6
                 WHERE request_id = ?1 AND status = 'analyzing'",
                params![
                    request_id,
                    status.text(),
                    error_code,
                    latency_ms,
                    usage,
                    provider_request_id
                ],
            )
            .map_err(|err| sqlite_error("Cannot save the model answer", &err))?;
        Ok(changed == 1)
    }

    /// Saves the checked, limited result of an `answered` request.
    pub fn validate_analysis(&self, request_id: &str, result_json: &str) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "UPDATE analyses SET status = 'validated', result_json = ?2
                 WHERE request_id = ?1 AND status = 'answered'",
                params![request_id, result_json],
            )
            .map_err(|err| sqlite_error("Cannot save the model result", &err))?;
        Ok(changed == 1)
    }

    /// An `answered` request whose answer did not pass the caller's checks.
    pub fn reject_analysis(&self, request_id: &str) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "UPDATE analyses SET status = 'failed', error_code = ?2
                 WHERE request_id = ?1 AND status = 'answered'",
                params![request_id, code_text(ErrorCode::InvalidModelOutput)],
            )
            .map_err(|err| sqlite_error("Cannot save the model result", &err))?;
        Ok(changed == 1)
    }

    pub fn analysis_status(&self, request_id: &str) -> Result<Option<AnalysisStatus>, AppError> {
        let conn = self.connection()?;
        let text: Option<String> = conn
            .query_row(
                "SELECT status FROM analyses WHERE request_id = ?1",
                [request_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|err| sqlite_error("Cannot read the model request", &err))?;
        Ok(text.as_deref().and_then(AnalysisStatus::parse))
    }

    /// At startup: requests that were still waiting when Traeva quit. Their
    /// answers can never arrive any more.
    pub fn interrupt_analyses(&self) -> Result<usize, AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE analyses SET status = 'interrupted' WHERE status = 'analyzing'",
            [],
        )
        .map_err(|err| sqlite_error("Cannot update the model requests", &err))
    }
}

/// The error code as saved, e.g. "model_timeout".
fn code_text(code: ErrorCode) -> String {
    match serde_json::to_value(code) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

/// How `plans.user_choice` spells a choice (accept, correct, manual).
fn choice_text(choice: UserChoice) -> &'static str {
    match choice {
        UserChoice::Accept => "accept",
        UserChoice::Correct => "correct",
        UserChoice::Manual => "manual",
    }
}

fn parse_choice(text: &str) -> Option<UserChoice> {
    match text {
        "accept" => Some(UserChoice::Accept),
        "correct" => Some(UserChoice::Correct),
        "manual" => Some(UserChoice::Manual),
        _ => None,
    }
}

/// A place as one full path, root path and path inside the root joined, so
/// that the same place reached through two roots (a nested root, or a folder
/// authorized again under a new root ID) gives the same full path (#48),
/// spelled the one way APFS compares names (#55). Folding a few more names
/// together than the volume does only makes Traeva wait for the unsettled
/// action.
fn full_path(root_path: &str, relative_path: &str) -> String {
    macos::fold_name(&format!("{root_path}/{relative_path}"))
}

/// The first unsettled action that `plan` has to wait for (organize-spec.md
/// §7): one about the same file record, the same file object (device, inode
/// and birth time: through a nested or re-authorized root the same file
/// gets another record, #48), or the same place at either end.
fn conflicting_action(conn: &Connection, plan: &PlanRow) -> Result<Option<String>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the actions", &err);
    let unreadable = || {
        AppError::storage(
            ErrorCode::DatabaseCorrupt,
            "A saved plan could not be read. Traeva kept it unchanged.",
        )
    };
    let root_path = |root_id: &str| -> Result<String, AppError> {
        conn.query_row(
            "SELECT display_path FROM authorized_roots WHERE root_id = ?1",
            [root_id],
            |row| row.get(0),
        )
        .map_err(fail)
    };
    let mine = [
        full_path(
            &root_path(&plan.source_root_id)?,
            &plan.source_relative_path,
        ),
        full_path(&root_path(&plan.target_root_id)?, &plan.target_path()),
    ];
    let mut stmt = conn
        .prepare(
            "SELECT a.action_id, p.file_id, p.expected_source_json,
                    source_root.display_path, p.source_relative_path,
                    target_root.display_path, p.target_segments_json, p.basename
             FROM actions AS a
             JOIN plans AS p ON p.plan_id = a.plan_id
             JOIN authorized_roots AS source_root ON source_root.root_id = p.source_root_id
             JOIN authorized_roots AS target_root ON target_root.root_id = p.target_root_id
             WHERE a.status IN ('prepared', 'running', 'needs_user_decision')
             ORDER BY a.created_at_ms",
        )
        .map_err(fail)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(fail)?;
    for row in rows {
        let (action_id, file_id, expected, source_root, source_path, target_root, segments, name) =
            row.map_err(fail)?;
        if file_id == plan.file_id {
            return Ok(Some(action_id));
        }
        let expected: ExpectedSource = serde_json::from_str(&expected).map_err(|_| unreadable())?;
        if expected
            .evidence
            .same_object(&plan.expected_source.evidence)
        {
            return Ok(Some(action_id));
        }
        let segments: Vec<String> = serde_json::from_str(&segments).map_err(|_| unreadable())?;
        let theirs = [
            full_path(&source_root, &source_path),
            full_path(&target_root, &join_path(&segments, &name)),
        ];
        if mine.iter().any(|place| theirs.contains(place)) {
            return Ok(Some(action_id));
        }
    }
    Ok(None)
}

/// The next journal entry of the action: sequence numbers count up from 1.
fn insert_event<T: Serialize>(
    conn: &Connection,
    action_id: &str,
    step: &str,
    phase: Phase,
    detail: &T,
    now_ms: i64,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO action_events (action_id, sequence, step, phase, detail_json, recorded_at_ms)
         SELECT ?1, COALESCE(MAX(sequence), 0) + 1, ?2, ?3, ?4, ?5
         FROM action_events WHERE action_id = ?1",
        params![action_id, step, phase.text(), to_json(detail)?, now_ms],
    )
    .map_err(|err| sqlite_error("Cannot save the journal", &err))?;
    Ok(())
}

/// The file's new location and its one path-history entry, inside the
/// caller's transaction.
fn record_move(tx: &Connection, record: &MoveRecord) -> Result<(), AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot record the move", &err);
    // The check just found the moved file at the target name, so any other
    // file still recorded as present there is not there any more.
    tx.execute(
        "UPDATE files SET presence = 'missing', updated_at_ms = ?4
         WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'
           AND file_id <> ?3",
        params![
            record.to_root_id,
            record.to_path,
            record.file_id,
            record.now_ms
        ],
    )
    .map_err(fail)?;
    let evidence = &record.result.evidence;
    let moved = tx
        .execute(
            "UPDATE files SET root_id = ?2, relative_path = ?3, basename = ?4,
                 presence = 'present', sha256 = ?5, byte_length = ?6, observation_json = ?7,
                 observed_at_ms = ?8, record_revision = record_revision + 1, updated_at_ms = ?8
             WHERE file_id = ?1",
            params![
                record.file_id,
                record.to_root_id,
                record.to_path,
                record.basename,
                record.result.sha256,
                record.byte_length as i64,
                to_json(evidence)?,
                record.now_ms
            ],
        )
        .map_err(fail)?;
    if moved != 1 {
        return Err(unexpected_records("Cannot record the move"));
    }
    let reason = if record.undo_of.is_some() {
        "undo"
    } else {
        "organize"
    };
    tx.execute(
        "INSERT INTO path_history (path_event_id, file_id, from_root_id, from_relative_path,
             to_root_id, to_relative_path, reason, action_id, evidence_json, recorded_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?10, ?7, ?8, ?9)",
        params![
            uuid::Uuid::new_v4().to_string(),
            record.file_id,
            record.from_root_id,
            record.from_path,
            record.to_root_id,
            record.to_path,
            record.action_id,
            to_json(evidence)?,
            record.now_ms,
            reason
        ],
    )
    .map_err(fail)?;
    Ok(())
}

/// For an undo: its organize action becomes `undone`, in the same
/// transaction as the undo's own commit. Only a committed action can be
/// undone, and the database allows one committed undo per action.
fn mark_undone(tx: &Connection, record: &MoveRecord) -> Result<(), AppError> {
    let Some(original) = record.undo_of else {
        return Ok(());
    };
    let changed = tx
        .execute(
            "UPDATE actions SET status = 'undone' WHERE action_id = ?1 AND status = 'committed'",
            [original],
        )
        .map_err(|err| sqlite_error("Cannot record the undo", &err))?;
    if changed != 1 {
        return Err(unexpected_records("Cannot record the undo"));
    }
    Ok(())
}

/// A write that should have changed exactly one row did not. The transaction
/// is rolled back and nothing is reported as saved.
fn unexpected_records(context: &str) -> AppError {
    AppError::storage(
        ErrorCode::DatabaseUnavailable,
        format!("{context} (the saved records are not as expected)."),
    )
}

fn action_status_text(status: ActionStatus) -> &'static str {
    match status {
        ActionStatus::Prepared => "prepared",
        ActionStatus::Running => "running",
        ActionStatus::Committed => "committed",
        ActionStatus::FailedNoMove => "failed_no_move",
        ActionStatus::NeedsUserDecision => "needs_user_decision",
        ActionStatus::Undone => "undone",
    }
}

fn parse_action_status(text: &str) -> Option<ActionStatus> {
    match text {
        "prepared" => Some(ActionStatus::Prepared),
        "running" => Some(ActionStatus::Running),
        "committed" => Some(ActionStatus::Committed),
        "failed_no_move" => Some(ActionStatus::FailedNoMove),
        "needs_user_decision" => Some(ActionStatus::NeedsUserDecision),
        "undone" => Some(ActionStatus::Undone),
        _ => None,
    }
}

/// A journal entry is the intent for a step (saved before the change) or
/// its result (saved after the change was checked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Intent,
    Result,
}

impl Phase {
    fn text(self) -> &'static str {
        match self {
            Phase::Intent => "intent",
            Phase::Result => "result",
        }
    }
}

/// One row of `actions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRow {
    pub action_id: String,
    pub plan_id: String,
    pub kind: PlanKind,
    pub status: ActionStatus,
    pub created_at_ms: i64,
    pub committed_at_ms: Option<i64>,
    pub recovered_at_ms: Option<i64>,
    pub problem: Option<ActionProblem>,
}

/// `actions.preconditions_json`: what the action expects, saved with the
/// action so checking it later never depends on memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preconditions {
    pub source_root_id: String,
    pub source_path: String,
    pub target_root_id: String,
    pub target_path: String,
    /// Paths inside the target root, outermost first.
    pub folders_to_create: Vec<String>,
    /// The source as the check right before the confirmation saw it.
    pub sha256: String,
    pub byte_length: u64,
    pub evidence: FileEvidence,
}

/// `actions.result_json` of a failed or uncertain action: why, for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionProblem {
    pub code: ErrorCode,
    pub message: String,
}

// Journal entries (`action_events.detail_json`). The step and phase columns
// say which one an entry holds:
//   confirm/result    Confirmed        written with the action itself
//   mkdir/intent      FolderIntent     before creating a folder
//   mkdir/result      FolderReceipt    after the new folder was checked
//   move/intent       MoveIntent       before the move
//   move/result       MoveResult       in the final transaction
//   settle/result     Settlement       written by the failure handling
//   reconcile/result  Reconciliation   written by a record check (#10)
//   cleanup/intent    FolderToRemove   before removing an empty folder
//   cleanup/result    FolderRemoval    after trying to remove it

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmed {
    pub plan_id: String,
}

/// A folder about to be created, as a path inside the target root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderIntent {
    pub path: String,
}

/// Proof that this action created this folder and checked it. Only a folder
/// with a receipt is ever treated as Traeva's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderReceipt {
    pub path: String,
    /// For diagnosis only, like `FileEvidence::dev`.
    pub dev: i64,
    /// The volume's UUID; None in receipts saved before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    pub ino: u64,
    pub birthtime_ns: i64,
}

/// From where to where, and exactly which file is expected to move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveIntent {
    pub from_root_id: String,
    pub from_path: String,
    pub to_root_id: String,
    pub to_path: String,
    pub sha256: String,
    pub byte_length: u64,
    pub evidence: FileEvidence,
}

/// What the check after the move found at the target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveResult {
    pub sha256: String,
    pub evidence: FileEvidence,
}

/// The decision of the failure handling and what it found at both names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub status: ActionStatus,
    pub code: ErrorCode,
    pub source_now: Seen,
    pub target_now: Seen,
}

/// What was at a name when the failure handling looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seen {
    /// Nothing has the name.
    Nothing,
    /// The expected file: same device, inode and birth time.
    ExpectedFile,
    /// Something else has the name.
    SomethingElse,
    /// Not looked at, or the look failed (for example, the folder is gone).
    Unknown,
}

/// What a record check found and decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub status: ActionStatus,
    pub basis: Basis,
    /// False when the journal could not be read in full or does not match
    /// the plan; it then never counts as "no move intent".
    pub journal_matches: bool,
    pub source_now: Found,
    pub target_now: Found,
}

/// Which rule of the student material decided a record check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// The journal is intact and has no move intent: the move was never called.
    NoMoveIntent,
    /// The file is still at its source, unchanged, and nothing is at the target.
    StillAtSource,
    /// The source is empty and the target holds the file exactly as planned.
    MovedAsPlanned,
    /// Anything else: Traeva cannot tell, so the user decides.
    Uncertain,
}

/// What a record check found at a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Found {
    /// Nothing has the name.
    Nothing,
    /// The expected file (same device, inode and birth time) with the
    /// expected bytes.
    ExpectedFile,
    /// The expected file object, but its bytes are not the expected ones or
    /// could not be read.
    ChangedFile,
    /// Something else has the name.
    SomethingElse,
    /// Not looked at: the root, a folder on the way, or the journal did not
    /// allow a reliable look.
    NotChecked,
}

/// An empty folder this action created, about to be removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderToRemove {
    pub path: String,
    pub dev: i64,
    pub ino: u64,
}

/// Whether the folder was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderRemoval {
    pub path: String,
    pub removed: bool,
}

/// One journal entry, by step and phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Confirmed(Confirmed),
    FolderIntent(FolderIntent),
    FolderReceipt(FolderReceipt),
    MoveIntent(MoveIntent),
    MoveResult(MoveResult),
    Settlement(Settlement),
    Reconciliation(Reconciliation),
    FolderToRemove(FolderToRemove),
    FolderRemoval(FolderRemoval),
}

/// One row of `action_events`. `entry` is `None` when the detail does not
/// have the shape its step and phase call for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRow {
    pub sequence: i64,
    pub entry: Option<Entry>,
}

fn parse_entry(step: &str, phase: &str, json: &str) -> Option<Entry> {
    fn read<T: serde::de::DeserializeOwned>(json: &str) -> Option<T> {
        serde_json::from_str(json).ok()
    }
    match (step, phase) {
        ("confirm", "result") => read(json).map(Entry::Confirmed),
        ("mkdir", "intent") => read(json).map(Entry::FolderIntent),
        ("mkdir", "result") => read(json).map(Entry::FolderReceipt),
        ("move", "intent") => read(json).map(Entry::MoveIntent),
        ("move", "result") => read(json).map(Entry::MoveResult),
        ("settle", "result") => read(json).map(Entry::Settlement),
        ("reconcile", "result") => read(json).map(Entry::Reconciliation),
        ("cleanup", "intent") => read(json).map(Entry::FolderToRemove),
        ("cleanup", "result") => read(json).map(Entry::FolderRemoval),
        _ => None,
    }
}

/// Everything the final transaction records after a checked move.
pub struct MoveRecord<'a> {
    pub action_id: &'a str,
    pub file_id: &'a str,
    pub from_root_id: &'a str,
    pub from_path: &'a str,
    pub to_root_id: &'a str,
    pub to_path: &'a str,
    pub basename: &'a str,
    pub byte_length: u64,
    pub result: &'a MoveResult,
    pub now_ms: i64,
    /// For an undo: the organize action that becomes `undone`.
    pub undo_of: Option<&'a str>,
}

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|_| AppError::storage(ErrorCode::DatabaseUnavailable, "Cannot save a record."))
}

/// Physical evidence for an authorized root, stored as JSON in
/// `authorized_roots.evidence_json`. It shows "this is still the folder that
/// was authorized"; it is not the root's identity (that is `root_id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootEvidence {
    /// For diagnosis only, like `FileEvidence::dev`.
    pub dev: i64,
    /// The volume's UUID; None in roots saved before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    pub ino: u64,
    pub birthtime_ns: i64,
    pub fs_type: String,
}

impl RootEvidence {
    /// The same folder: the rule of `FileEvidence::same_object`.
    pub fn same_object(&self, other: &RootEvidence) -> bool {
        self.ino == other.ino
            && self.birthtime_ns == other.birthtime_ns
            && same_volume(&self.volume, &other.volume)
    }
}

/// One row of `authorized_roots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRow {
    pub root_id: String,
    pub display_path: String,
    pub evidence: RootEvidence,
    pub status: RootStatus,
    pub granted_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

/// `filter` is always a constant written in this file; values go in `params`.
fn query_roots<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<RootRow>, AppError> {
    let sql = format!(
        "SELECT root_id, display_path, evidence_json, status, granted_at_ms, revoked_at_ms
         FROM authorized_roots {filter}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| sqlite_error("Cannot read the folders", &err))?;
    let raw_rows = stmt
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(|err| sqlite_error("Cannot read the folders", &err))?;

    let mut roots = Vec::new();
    for raw in raw_rows {
        let (root_id, display_path, evidence_json, status, granted_at_ms, revoked_at_ms) =
            raw.map_err(|err| sqlite_error("Cannot read the folders", &err))?;
        let unreadable = || {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved folder record could not be read. Traeva kept it unchanged.",
            )
        };
        roots.push(RootRow {
            root_id,
            display_path,
            evidence: serde_json::from_str(&evidence_json).map_err(|_| unreadable())?,
            status: parse_status(&status).ok_or_else(unreadable)?,
            granted_at_ms,
            revoked_at_ms,
        });
    }
    Ok(roots)
}

fn status_text(status: RootStatus) -> &'static str {
    match status {
        RootStatus::Active => "active",
        RootStatus::Revoked => "revoked",
    }
}

fn parse_status(text: &str) -> Option<RootStatus> {
    match text {
        "active" => Some(RootStatus::Active),
        "revoked" => Some(RootStatus::Revoked),
        _ => None,
    }
}

fn take_process_lock(app_data_dir: &Path) -> Result<File, AppError> {
    let lock_path = app_data_dir.join(LOCK_FILE_NAME);
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| {
            AppError::storage(
                ErrorCode::AppDataUnavailable,
                format!(
                    "Cannot open the lock file {} ({}).",
                    lock_path.display(),
                    err.kind()
                ),
            )
        })?;

    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(AppError::storage(
            ErrorCode::AnotherInstanceRunning,
            "Another Traeva window is already running and using the same data. \
             Close it, then open Traeva again.",
        )),
        Err(TryLockError::Error(err)) => Err(AppError::storage(
            ErrorCode::AppDataUnavailable,
            format!("Cannot lock {} ({}).", lock_path.display(), err.kind()),
        )),
    }
}

/// architecture.md §5: foreign keys on, rollback journal, EXTRA sync, and F_FULLFSYNC on macOS.
fn apply_pragmas(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = EXTRA;
         PRAGMA fullfsync = ON;",
    )
    .map_err(|err| sqlite_error("Cannot apply the database settings", &err))?;

    let actual = read_pragmas(conn)?;
    let expected = PragmaSettings {
        foreign_keys: true,
        journal_mode: "delete".to_string(),
        synchronous: "extra".to_string(),
        fullfsync: true,
    };
    if actual != expected {
        return Err(AppError::storage(
            ErrorCode::PragmaMismatch,
            format!("SQLite reports settings {actual:?}, but Traeva requires {expected:?}."),
        ));
    }
    Ok(())
}

fn read_pragmas(conn: &Connection) -> Result<PragmaSettings, AppError> {
    let read_int = |name: &str| -> Result<i64, AppError> {
        conn.pragma_query_value(None, name, |row| row.get(0))
            .map_err(|err| sqlite_error("Cannot read the database settings", &err))
    };
    let journal_mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|err| sqlite_error("Cannot read the database settings", &err))?;
    let synchronous = match read_int("synchronous")? {
        0 => "off",
        1 => "normal",
        2 => "full",
        3 => "extra",
        _ => "unknown",
    };
    Ok(PragmaSettings {
        foreign_keys: read_int("foreign_keys")? == 1,
        journal_mode: journal_mode.to_lowercase(),
        synchronous: synchronous.to_string(),
        fullfsync: read_int("fullfsync")? == 1,
    })
}

fn check_integrity(conn: &Connection) -> Result<(), AppError> {
    let first_line: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|err| sqlite_error("Cannot check the database", &err))?;
    if first_line != "ok" {
        return Err(AppError::storage(
            ErrorCode::DatabaseCorrupt,
            "The database failed SQLite's integrity check. Traeva kept the file unchanged.",
        ));
    }
    Ok(())
}

fn read_user_version(conn: &Connection) -> Result<i64, AppError> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|err| sqlite_error("Cannot read the database version", &err))
}

fn migrate(conn: &mut Connection) -> Result<(), AppError> {
    let current = read_user_version(conn)?;
    if !(0..=LATEST_SCHEMA_VERSION).contains(&current) {
        return Err(AppError::storage(
            ErrorCode::UnsupportedSchemaVersion,
            format!(
                "The database has schema version {current}, but this Traeva understands \
                 versions 0 to {LATEST_SCHEMA_VERSION}. It may come from a newer Traeva. \
                 The file was not changed."
            ),
        ));
    }

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        if let Err(err) = apply_migration(conn, sql, version) {
            return Err(AppError::storage(
                ErrorCode::MigrationFailed,
                format!(
                    "Database migration {version} failed ({}). It was rolled back and the \
                     database was kept.",
                    sqlite_reason(&err)
                ),
            ));
        }
    }
    Ok(())
}

/// The migration and its version number commit together, or not at all.
/// If anything fails, dropping `tx` rolls the whole migration back.
fn apply_migration(conn: &mut Connection, sql: &str, version: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(sql)?;
    tx.pragma_update(None, "user_version", version)?;
    tx.commit()
}

/// Turns a SQLite error into an AppError without exposing SQL text.
pub(crate) fn sqlite_error(context: &str, err: &rusqlite::Error) -> AppError {
    let code = match err.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt) => {
            ErrorCode::DatabaseCorrupt
        }
        _ => ErrorCode::DatabaseUnavailable,
    };
    AppError::storage(code, format!("{context} ({}).", sqlite_reason(err)))
}

/// SQLite's generic description of the error code, e.g. "database disk image is malformed".
fn sqlite_reason(err: &rusqlite::Error) -> String {
    match err.sqlite_error() {
        Some(sqlite) => sqlite.to_string(),
        None => "unexpected database error".to_string(),
    }
}
