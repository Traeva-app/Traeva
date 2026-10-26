//! The fixture shared by the crate's own plan and execution tests: a
//! temporary app-data folder, one authorized root with a small synthetic
//! tree, and a clock the test controls. Reads skip the 350 ms pause.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::OrganizeService;
use crate::contracts::{
    ActionView, AppError, BuildOutcome, ErrorCode, FileView, PlanRequest, PlanTarget, PlanView,
};
use crate::execute::Checkpoint;
use crate::observe::{ReadError, ReadLimits, StableRead, read_stable_with};
use crate::plan::Revalidation;
use crate::platform::macos::Folder;
use crate::store::PlanRow;
use crate::test_snapshot as snapshot;

pub const START_MS: i64 = 1_800_000_000_000;

/// A stable read without the pause between the two reads.
pub fn quick_read(parent: &Folder, name: &str, root_dev: i64) -> Result<StableRead, ReadError> {
    read_stable_with(parent, name, root_dev, ReadLimits::SPEC, &mut |_| {})
}

/// Edits the file at `path` in place with `text` of the same length, then
/// puts its modification time back: the same file object, the same size and
/// the same modification time. Only its change time shows the edit.
pub fn edit_keeping_size_and_mtime(path: &Path, text: &str) {
    let before = fs::metadata(path).unwrap();
    assert_eq!(text.len() as u64, before.len(), "the edit keeps the size");
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(before.modified().unwrap())
        .unwrap();
    let after = fs::metadata(path).unwrap();
    assert_eq!((after.ino(), after.len()), (before.ino(), before.len()));
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_ne!(
        (after.ctime(), after.ctime_nsec()),
        (before.ctime(), before.ctime_nsec())
    );
}

pub struct Fixture {
    pub app: tempfile::TempDir,
    pub root: tempfile::TempDir,
    service: Option<OrganizeService>,
    clock: Arc<AtomicI64>,
    pub root_id: String,
    /// Replacements at a record's place are not linked by themselves
    /// (`without_auto_link`); kept across restarts.
    no_auto_link: bool,
}

impl Fixture {
    /// A root holding the sample tree plus an empty "Taxes" folder.
    pub fn new() -> Fixture {
        let app = tempfile::Builder::new()
            .prefix("traeva-appdata-")
            .tempdir()
            .unwrap();
        let root = tempfile::Builder::new()
            .prefix("traeva-root-")
            .tempdir()
            .unwrap();
        snapshot::fill_sample_tree(root.path());
        fs::create_dir(root.path().join("Taxes")).unwrap();
        let clock = Arc::new(AtomicI64::new(START_MS));
        let mut fixture = Fixture {
            app,
            root,
            service: None,
            clock,
            root_id: String::new(),
            no_auto_link: false,
        };
        fixture.restart();
        fixture.root_id = fixture
            .service()
            .authorize_root(fixture.root.path())
            .unwrap()
            .root_id;
        fixture
    }

    pub fn service(&self) -> &OrganizeService {
        self.service.as_ref().unwrap()
    }

    /// For the model tests, which point the service at their own server.
    pub fn service_mut(&mut self) -> &mut OrganizeService {
        self.service.as_mut().unwrap()
    }

    /// Stops the service and starts a new one on the same app data.
    ///
    /// The new start can still find the process lock taken for a moment: a
    /// child process that another test is starting (`mkfifo`, for example)
    /// holds a copy of every open file of this process, the lock included,
    /// until it has started. A start that fails on the lock writes nothing,
    /// so this tries again, for at most two seconds. A real restart is a new
    /// process and never meets this.
    pub fn restart(&mut self) {
        self.service = None;
        let stopped = Instant::now();
        loop {
            let clock = Arc::clone(&self.clock);
            let service = OrganizeService::start_with_clock(
                self.app.path().join("Traeva"),
                Box::new(move || clock.load(Ordering::SeqCst)),
            );
            let lock_taken = matches!(
                service.store(),
                Err(error) if error.code == ErrorCode::AnotherInstanceRunning
            );
            if !lock_taken || stopped.elapsed() > Duration::from_secs(2) {
                service
                    .no_auto_link
                    .store(self.no_auto_link, Ordering::SeqCst);
                self.service = Some(service);
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// For the tests of what happens when Traeva cannot link a new object
    /// at a record's place by itself (auto_link.rs): the report, candidate
    /// capture and the user's decisions. Here every replacement takes those
    /// paths, as a file Traeva cannot follow does.
    pub fn without_auto_link(mut self) -> Fixture {
        self.no_auto_link = true;
        self.service().no_auto_link.store(true, Ordering::SeqCst);
        self
    }

    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn register(&self, relative: &str) -> FileView {
        self.service()
            .register_file_with(
                &self.root_id,
                &self.path(relative),
                &mut |_root, parent, name| quick_read(parent, name, parent.id().dev),
            )
            .unwrap()
            .file
    }

    /// Captures a snapshot of the file, with reads that do not pause (#32).
    pub fn capture(&self, file_id: &str) -> Result<crate::snapshot::Capture, AppError> {
        self.service()
            .capture_snapshot_with(file_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            })
    }

    /// Turns Memory on for the file, with reads that do not pause (#32).
    pub fn start_memory(&self, file_id: &str) -> Result<crate::MemoryWatchView, AppError> {
        self.service()
            .start_memory_with(file_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            })
    }

    /// One pass of Memory's watcher, with reads that do not pause (#32).
    pub fn check_memory(&self) -> Vec<crate::WatchCheck> {
        self.service()
            .check_memory_with(&mut |root, parent, name| quick_read(parent, name, root.id().dev))
            .unwrap()
    }

    pub fn try_plan(
        &self,
        file: &FileView,
        folder: &str,
        new_folders: &[&str],
    ) -> Result<BuildOutcome, AppError> {
        let catalog = self.service().list_folders(&self.root_id).unwrap();
        let folder_id = catalog
            .folders
            .iter()
            .find(|f| f.relative_path == folder)
            .unwrap()
            .folder_id
            .clone();
        self.service().build_plan(&PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: self.root_id.clone(),
            operation_id: None,
            target: PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id,
                new_folders: new_folders.iter().map(|name| name.to_string()).collect(),
            },
        })
    }

    pub fn plan(&self, file: &FileView, folder: &str, new_folders: &[&str]) -> PlanView {
        match self.try_plan(file, folder, new_folders).unwrap() {
            BuildOutcome::Planned { plan } => plan,
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    /// The pre-execution check, with reads that do not pause.
    pub fn revalidate(&self, plan_id: &str) -> Result<Revalidation, AppError> {
        self.service()
            .revalidate_plan_with(plan_id, &mut quick_read)
    }

    /// Confirms and runs a plan, with reads that do not pause.
    pub fn apply(&self, plan_id: &str) -> Result<ActionView, AppError> {
        self.apply_with(plan_id, &mut |_| {})
    }

    /// Like `apply`, calling `checkpoint` at each point of the run.
    pub fn apply_with(
        &self,
        plan_id: &str,
        checkpoint: &mut dyn FnMut(Checkpoint),
    ) -> Result<ActionView, AppError> {
        self.service()
            .apply_plan_with(plan_id, &mut quick_read, checkpoint)
    }

    pub fn stored(&self, plan_id: &str) -> PlanRow {
        self.service()
            .store()
            .unwrap()
            .get_plan(plan_id)
            .unwrap()
            .unwrap()
    }

    /// A separate connection for looking at (and, in tests, changing) the
    /// database directly.
    pub fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(
            self.app
                .path()
                .join("Traeva")
                .join(crate::DATABASE_FILE_NAME),
        )
        .unwrap();
        db.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        db
    }

    pub fn count(&self, table: &str) -> i64 {
        self.db()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// An action row written directly, with the given status: for tests of
    /// plans and of files whose action is not settled.
    pub fn insert_action(&self, plan_id: &str, status: &str) -> String {
        let action_id = uuid::Uuid::new_v4().to_string();
        self.db()
            .execute(
                "INSERT INTO actions (action_id, plan_id, kind, status, preconditions_json, created_at_ms)
                 VALUES (?1, ?2, 'organize', ?3, '{}', 0)",
                (&action_id, plan_id, status),
            )
            .unwrap();
        action_id
    }

    /// Simulates a restart after which macOS gave the volume another device
    /// number (seen on 10/15: 16777232 became 16777234): every `dev` saved in
    /// the database goes up by 2. With `legacy`, the saved volume UUIDs are
    /// removed too, as in records saved before they existed. Immutable and
    /// append-only tables are edited with their triggers dropped, which are
    /// then created again. Call `restart` afterwards.
    pub fn renumber_saved_volume(&self, legacy: bool) {
        let db = self.db();
        let column_of = |sql: &str| -> Vec<String> {
            let mut stmt = db.prepare(sql).unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let triggers = column_of("SELECT sql FROM sqlite_schema WHERE type = 'trigger'");
        for name in column_of("SELECT name FROM sqlite_schema WHERE type = 'trigger'") {
            db.execute(&format!("DROP TRIGGER \"{name}\""), []).unwrap();
        }
        let tables = column_of(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        );
        let mut rewritten = 0;
        for table in tables {
            let columns = column_of(&format!(
                "SELECT name FROM pragma_table_info('{table}') WHERE name LIKE '%json'"
            ));
            for column in columns {
                let rows: Vec<(i64, String)> = {
                    let sql =
                        format!("SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL");
                    let mut stmt = db.prepare(&sql).unwrap();
                    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                        .unwrap()
                        .collect::<Result<_, _>>()
                        .unwrap()
                };
                for (rowid, text) in rows {
                    let renumbered = renumber_devs(&db, &text, legacy);
                    if renumbered != text {
                        db.execute(
                            &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
                            rusqlite::params![renumbered, rowid],
                        )
                        .unwrap();
                        rewritten += 1;
                    }
                }
            }
        }
        for sql in triggers {
            db.execute_batch(&sql).unwrap();
        }
        assert!(rewritten > 0, "no saved dev was found to renumber");
    }

    /// The action's journal as (step, phase), in order.
    pub fn journal(&self, action_id: &str) -> Vec<(String, String)> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT step, phase FROM action_events WHERE action_id = ?1 ORDER BY sequence")
            .unwrap();
        stmt.query_map([action_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// `text` (JSON) with every integer `dev` 2 higher and, with `legacy`, the
/// `volume` beside it removed. SQLite's JSON functions keep the key order,
/// so the text stays as Traeva wrote it apart from these values.
fn renumber_devs(db: &rusqlite::Connection, text: &str, legacy: bool) -> String {
    let is_json: bool = db
        .query_row("SELECT json_valid(?1)", [text], |r| r.get(0))
        .unwrap();
    if !is_json {
        return text.to_string();
    }
    let found: Vec<(String, String, i64)> = {
        let mut stmt = db
            .prepare(
                "SELECT fullkey, path, atom FROM json_tree(?1)
                 WHERE key = 'dev' AND type = 'integer'",
            )
            .unwrap();
        stmt.query_map([text], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let mut renumbered = text.to_string();
    for (dev_path, parent, dev) in found {
        renumbered = db
            .query_row(
                "SELECT json_set(?1, ?2, ?3)",
                rusqlite::params![renumbered, dev_path, dev + 2],
                |r| r.get(0),
            )
            .unwrap();
        if legacy {
            renumbered = db
                .query_row(
                    "SELECT json_remove(?1, ?2)",
                    rusqlite::params![renumbered, format!("{parent}.volume")],
                    |r| r.get(0),
                )
                .unwrap();
        }
    }
    renumbered
}

/// `[("confirm", "result"), ...]` as owned strings, to compare with `journal`.
pub fn steps(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(step, phase)| (step.to_string(), phase.to_string()))
        .collect()
}
