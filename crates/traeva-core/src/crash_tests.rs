//! O18-C (#21): the run really dies at one of the four checkpoints of
//! organize-plan.md §7. A child process (this test binary, started again)
//! runs a plan the way the app does and waits at the checkpoint; the test
//! then kills it with SIGKILL. Nothing after that point runs: no failure
//! handling, no unwinding, no destructors, and only the kernel lets go of
//! the process lock. A new process then starts on the same app data, which
//! runs the record check of organize-spec.md §7, and one more start shows
//! that nothing is recorded or moved twice. The test itself only looks at
//! the files and reads the database.
//!
//! These results are kept apart from O18-R (recover_tests.rs and
//! undo_tests.rs), where the service is dropped and started again inside one
//! process. Neither is a power cut or a disk failure: everything written
//! before the kill had already reached macOS.
//!
//! The checkpoint is the closure `apply_plan_with` takes, which exists for
//! tests only; the app's `apply_plan` passes one that does nothing, so the
//! normal flow has no fault switch. Synthetic temporary files only.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::OrganizeService;
use crate::contracts::{BuildOutcome, PlanRequest, PlanTarget, UndoPreview};
use crate::execute::Checkpoint;
use crate::observe::{self, sha256_hex};
use crate::test_fixture::steps;
use crate::test_snapshot as snapshot;

/// The environment variable that tells the child process its job.
const CHILD_JOB: &str = "TRAEVA_O18C_CHILD_JOB";
/// Lines the child prints for the test start with this.
const SAYS: &str = "O18C ";
/// How long the test waits for the child to report.
const PATIENCE: Duration = Duration::from_secs(60);
/// If the test is gone and nobody kills the waiting child, it ends itself
/// after this long with `NOT_KILLED`.
const LONGEST_WAIT: Duration = Duration::from_secs(120);
const NOT_KILLED: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Task {
    /// Organize todo.txt and stop at the point.
    Organize,
    /// Organize todo.txt into Notes/Archive, then undo it and stop at the
    /// point of the undo run.
    Undo,
    /// Start the service (this runs the record check) and report.
    Start,
}

/// The four checkpoints of organize-plan.md §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Point {
    /// The new folder exists; its receipt is not saved yet.
    AfterMkdir,
    /// The move intent is saved; the move call is next.
    BeforeMove,
    /// The move call succeeded; the checks and the final record are next.
    AfterMove,
}

impl Point {
    fn checkpoint(self) -> Checkpoint {
        match self {
            Point::AfterMkdir => Checkpoint::AfterMkdir(0),
            Point::BeforeMove => Checkpoint::BeforeMove,
            Point::AfterMove => Checkpoint::AfterMove,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Job {
    task: Task,
    /// The app-data folder (what the app passes to `OrganizeService::start`).
    app_data: PathBuf,
    root: PathBuf,
    point: Option<Point>,
    /// For `Start`: the actions to report.
    actions: Vec<String>,
}

// ---------------------------------------------------------------------------
// The child process
// ---------------------------------------------------------------------------

/// The child process. Only the tests below start it, with its job in
/// `CHILD_JOB`; run any other way it does nothing.
#[test]
#[ignore = "the child process of the O18-C tests, started by them"]
fn o18c_child_process() {
    let Ok(job) = std::env::var(CHILD_JOB) else {
        return;
    };
    let job: Job = serde_json::from_str(&job).unwrap();
    // Like the app: storage opens and the record check runs at the start.
    let service = OrganizeService::start(job.app_data.clone());
    match job.task {
        Task::Start => {
            let state = service.get_app_state().unwrap();
            let actions: Vec<Value> = job
                .actions
                .iter()
                .map(|id| serde_json::to_value(service.get_action(id).unwrap()).unwrap())
                .collect();
            say(
                "started",
                &json!({ "storage": state.storage, "actions": actions }),
            );
        }
        Task::Organize | Task::Undo => {
            let point = job.point.unwrap().checkpoint();
            let root_id = service.authorize_root(&job.root).unwrap().root_id;
            let file = service
                .register_file(&root_id, &job.root.join("todo.txt"))
                .unwrap()
                .file;
            let plan_id = if job.task == Task::Organize {
                let (folder, new_folders) = match point {
                    Checkpoint::AfterMkdir(_) => ("Notes", vec!["Archive"]),
                    _ => ("Taxes", vec![]),
                };
                let plan_id = plan(&service, &root_id, &file, folder, &new_folders);
                say(
                    "ids",
                    &json!({ "file_id": file.file_id, "plan_id": plan_id }),
                );
                plan_id
            } else {
                let plan_id = plan(&service, &root_id, &file, "Notes", &["Archive"]);
                let organized = service.apply_plan(&plan_id).unwrap();
                assert_eq!(organized.status, crate::ActionStatus::Committed);
                let UndoPreview::Planned { plan } =
                    service.preview_undo(&organized.action_id).unwrap()
                else {
                    panic!("expected an undo plan");
                };
                say(
                    "ids",
                    &json!({
                        "file_id": file.file_id,
                        "organize_action_id": organized.action_id,
                        "plan_id": plan.plan_id,
                    }),
                );
                plan.plan_id
            };
            // The app's own reads; only the checkpoint is the test's.
            let result = service.apply_plan_with(
                &plan_id,
                &mut |parent, name, root_dev| observe::read_stable(parent, name, root_dev),
                &mut |now| {
                    if now == point {
                        say(
                            "waiting",
                            &json!({ "pid": std::process::id(), "at": format!("{now:?}") }),
                        );
                        // Wait to be killed.
                        thread::sleep(LONGEST_WAIT);
                        std::process::exit(NOT_KILLED);
                    }
                },
            );
            panic!("the run never reached {point:?}: {result:?}");
        }
    }
}

/// A plan for `file` into `folder` plus `new_folders`, built like the page
/// does: from the folder list.
fn plan(
    service: &OrganizeService,
    root_id: &str,
    file: &crate::FileView,
    folder: &str,
    new_folders: &[&str],
) -> String {
    let catalog = service.list_folders(root_id).unwrap();
    let folder_id = catalog
        .folders
        .iter()
        .find(|f| f.relative_path == folder)
        .unwrap()
        .folder_id
        .clone();
    let outcome = service
        .build_plan(&PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: root_id.to_string(),
            operation_id: None,
            target: PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id,
                new_folders: new_folders.iter().map(|name| name.to_string()).collect(),
            },
        })
        .unwrap();
    match outcome {
        BuildOutcome::Planned { plan } => plan.plan_id,
        other => panic!("expected a plan, got {other:?}"),
    }
}

/// One line for the test, written out at once.
fn say(what: &str, value: &Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{SAYS}{what} {value}").unwrap();
    out.flush().unwrap();
}

// ---------------------------------------------------------------------------
// The test side
// ---------------------------------------------------------------------------

/// A child process and the lines it says. Killed if the test ends early.
struct ChildRun {
    child: Child,
    lines: Receiver<String>,
}

impl ChildRun {
    fn spawn(job: &Job) -> ChildRun {
        // This test binary again, running only the child test.
        let (_crate, module) = module_path!().split_once("::").unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                &format!("{module}::o18c_child_process"),
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_JOB, serde_json::to_string(job).unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, lines) = channel();
        thread::spawn(move || {
            // The test harness starts the first line with the test's name.
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(at) = line.find(SAYS) {
                    let _ = send.send(line[at + SAYS.len()..].to_string());
                }
            }
        });
        ChildRun { child, lines }
    }

    /// The next thing the child says, which must be `what`.
    fn next(&self, what: &str) -> Value {
        let line = self
            .lines
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| panic!("the child never said “{what}”"));
        let (said, value) = line.split_once(' ').unwrap();
        assert_eq!(said, what, "{line}");
        serde_json::from_str(value).unwrap()
    }

    fn wait(&mut self) -> ExitStatus {
        self.child.wait().unwrap()
    }
}

impl Drop for ChildRun {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the killed process left behind.
struct Killed {
    ids: Value,
    pid: u32,
    status: ExitStatus,
}

impl Killed {
    fn id(&self, name: &str) -> String {
        self.ids[name].as_str().unwrap().to_string()
    }
}

/// One app-data folder and one root with the sample tree plus an empty
/// "Taxes" folder, both new and temporary.
struct Place {
    app: tempfile::TempDir,
    root: tempfile::TempDir,
}

impl Place {
    fn new() -> Place {
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
        Place { app, root }
    }

    fn job(&self, task: Task, point: Option<Point>, actions: &[&str]) -> Job {
        Job {
            task,
            app_data: self.app.path().join("Traeva"),
            root: self.root.path().to_path_buf(),
            point,
            actions: actions.iter().map(|id| id.to_string()).collect(),
        }
    }

    /// Runs `task` in a child process and kills it with SIGKILL while it
    /// waits at `point`.
    fn kill_at(&self, task: Task, point: Point) -> Killed {
        let mut run = ChildRun::spawn(&self.job(task, Some(point), &[]));
        let ids = run.next("ids");
        let waiting = run.next("waiting");
        assert_eq!(waiting["at"], format!("{:?}", point.checkpoint()));
        run.child.kill().unwrap();
        let status = run.wait();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{status:?}");
        assert_eq!(status.code(), None, "killed, not exited");
        Killed {
            ids,
            pid: waiting["pid"].as_u64().unwrap() as u32,
            status,
        }
    }

    /// Starts Traeva in a new process, which runs the record check, and
    /// returns what it reported: storage and the given actions.
    fn start(&self, actions: &[&str]) -> Value {
        let mut run = ChildRun::spawn(&self.job(Task::Start, None, actions));
        let started = run.next("started");
        let status = run.wait();
        assert!(status.success(), "the new process ended with {status:?}");
        started
    }

    /// Starts once more and checks that nothing changes: the same views, the
    /// same records, the same files.
    fn start_again_changes_nothing(&self, actions: &[&str], earlier: &Value) {
        let records = self.counts();
        let tree = self.tree();
        let again = self.start(actions);
        assert_eq!(again["actions"], earlier["actions"]);
        assert_eq!(self.counts(), records, "nothing recorded twice");
        assert_eq!(self.tree(), tree, "nothing moved");
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn tree(&self) -> Vec<snapshot::Entry> {
        snapshot::snapshot(self.root.path())
    }

    /// Bytes hash and inode of the file at `relative`.
    fn file(&self, relative: &str) -> (String, u64) {
        let path = self.path(relative);
        (
            sha256_hex(&fs::read(&path).unwrap()),
            fs::metadata(&path).unwrap().ino(),
        )
    }

    fn database(&self) -> PathBuf {
        self.app
            .path()
            .join("Traeva")
            .join(crate::DATABASE_FILE_NAME)
    }

    /// A read-only connection; each look opens and closes its own.
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open_with_flags(
            self.database(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }

    fn action_of(&self, plan_id: &str) -> String {
        self.db()
            .query_row(
                "SELECT action_id FROM actions WHERE plan_id = ?1",
                [plan_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn status(&self, action_id: &str) -> String {
        self.db()
            .query_row(
                "SELECT status FROM actions WHERE action_id = ?1",
                [action_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn journal(&self, action_id: &str) -> Vec<(String, String)> {
        self.rows(
            "SELECT step, phase FROM action_events WHERE action_id = ?1 ORDER BY sequence",
            action_id,
        )
    }

    /// Where the database says the file is.
    fn recorded_path(&self, file_id: &str) -> String {
        self.db()
            .query_row(
                "SELECT relative_path FROM files WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// The file's path history as (reason, to), oldest first.
    fn history(&self, file_id: &str) -> Vec<(String, String)> {
        self.rows(
            "SELECT reason, to_relative_path FROM path_history WHERE file_id = ?1
             ORDER BY recorded_at_ms, rowid",
            file_id,
        )
    }

    fn rows(&self, sql: &str, param: &str) -> Vec<(String, String)> {
        let db = self.db();
        let mut stmt = db.prepare(sql).unwrap();
        stmt.query_map([param], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// Rows in actions, action_events and path_history.
    fn counts(&self) -> (i64, i64, i64) {
        let db = self.db();
        let count = |table: &str| -> i64 {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        (
            count("actions"),
            count("action_events"),
            count("path_history"),
        )
    }

    /// A trigger that refuses every new journal entry, as when the journal
    /// cannot be written. The only write the test makes to the database.
    fn refuse_journal_entries(&self, refuse: bool) {
        let db = rusqlite::Connection::open(self.database()).unwrap();
        db.execute_batch(if refuse {
            "CREATE TRIGGER test_refuse BEFORE INSERT ON action_events
             BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;"
        } else {
            "DROP TRIGGER test_refuse;"
        })
        .unwrap();
    }
}

/// Prints what happened, for the evidence log (`--nocapture`).
fn note(test: &str, killed: &Killed, facts: &[(&str, String)]) {
    println!(
        "O18C-RESULT {test}: child pid {} ended by signal {:?} (exit code {:?})",
        killed.pid,
        killed.status.signal(),
        killed.status.code()
    );
    for (what, value) in facts {
        println!("O18C-RESULT {test}:   {what}: {value}");
    }
}

fn start_status(started: &Value) -> &str {
    started["storage"]["status"].as_str().unwrap()
}

// ---------------------------------------------------------------------------
// The four checkpoints
// ---------------------------------------------------------------------------

#[test]
fn o18c_killed_after_mkdir_before_its_receipt_keeps_the_folder_and_the_file() {
    let place = Place::new();
    let original = place.file("todo.txt");
    let killed = place.kill_at(Task::Organize, Point::AfterMkdir);
    let action_id = place.action_of(&killed.id("plan_id"));

    // What the killed process left: a running action, the new folder, and
    // no receipt for it.
    assert_eq!(place.status(&action_id), "running");
    assert_eq!(
        place.journal(&action_id),
        steps(&[("confirm", "result"), ("mkdir", "intent")])
    );
    assert!(place.path("Notes/Archive").is_dir());
    let left = place.tree();

    let started = place.start(&[&action_id]);

    assert_eq!(start_status(&started), "ready");
    let view = &started["actions"][0];
    assert_eq!(view["status"], "failed_no_move");
    assert_eq!(view["problem"]["code"], "run_interrupted");
    // Not claimed and not removed: the folder is only reported.
    assert_eq!(view["folders_without_receipt"], json!(["Notes/Archive"]));
    assert_eq!(view["folders_created"], json!([]));
    assert_eq!(view["folders_removed"], json!([]));
    assert_eq!(place.tree(), left, "the check changed nothing on disk");
    assert!(place.path("Notes/Archive").is_dir());
    assert_eq!(place.file("todo.txt"), original);
    assert_eq!(place.recorded_path(&killed.id("file_id")), "todo.txt");
    assert_eq!(
        place.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("mkdir", "intent"),
            ("reconcile", "result"),
        ])
    );
    place.start_again_changes_nothing(&[&action_id], &started);
    note(
        "after mkdir, before its receipt",
        &killed,
        &[
            ("status before the new start", "running".into()),
            ("status after", view["status"].to_string()),
            (
                "folder without receipt",
                view["folders_without_receipt"].to_string(),
            ),
            (
                "todo.txt sha256/inode before = after",
                format!("{original:?}"),
            ),
        ],
    );
}

#[test]
fn o18c_killed_after_the_move_intent_does_not_move_the_file() {
    let place = Place::new();
    let original = place.file("todo.txt");
    let killed = place.kill_at(Task::Organize, Point::BeforeMove);
    let action_id = place.action_of(&killed.id("plan_id"));

    assert_eq!(place.status(&action_id), "running");
    assert_eq!(
        place.journal(&action_id),
        steps(&[("confirm", "result"), ("move", "intent")])
    );
    let left = place.tree();

    let started = place.start(&[&action_id]);

    assert_eq!(start_status(&started), "ready");
    let view = &started["actions"][0];
    assert_eq!(view["status"], "failed_no_move");
    assert_eq!(view["problem"]["code"], "run_interrupted");
    // The remaining move is not done by the check.
    assert_eq!(place.tree(), left);
    assert_eq!(place.file("todo.txt"), original);
    assert!(!place.path("Taxes/todo.txt").exists());
    assert_eq!(place.recorded_path(&killed.id("file_id")), "todo.txt");
    assert_eq!(
        place.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("reconcile", "result"),
        ])
    );
    place.start_again_changes_nothing(&[&action_id], &started);
    note(
        "after the move intent, before the move",
        &killed,
        &[
            ("status before the new start", "running".into()),
            ("status after", view["status"].to_string()),
            (
                "todo.txt sha256/inode before = after",
                format!("{original:?}"),
            ),
        ],
    );
}

#[test]
fn o18c_killed_after_the_move_is_committed_once_by_the_next_process() {
    let place = Place::new();
    let original = place.file("todo.txt");
    let killed = place.kill_at(Task::Organize, Point::AfterMove);
    let action_id = place.action_of(&killed.id("plan_id"));
    let file_id = killed.id("file_id");

    // The file moved, but nothing says so yet.
    assert_eq!(place.status(&action_id), "running");
    assert_eq!(
        place.file("Taxes/todo.txt"),
        original,
        "the same file object"
    );
    assert!(!place.path("todo.txt").exists());
    assert_eq!(place.recorded_path(&file_id), "todo.txt");
    assert_eq!(
        place.history(&file_id),
        [("intake".into(), "todo.txt".into())]
    );
    let left = place.tree();

    let started = place.start(&[&action_id]);

    assert_eq!(start_status(&started), "ready");
    let view = &started["actions"][0];
    assert_eq!(view["status"], "committed");
    assert_eq!(view["problem"], Value::Null);
    assert!(view["committed_at_ms"].is_i64() && view["recovered_at_ms"].is_i64());
    assert_eq!(place.tree(), left, "no second move");
    assert_eq!(place.file("Taxes/todo.txt"), original);
    assert_eq!(place.recorded_path(&file_id), "Taxes/todo.txt");
    assert_eq!(
        place.history(&file_id),
        [
            ("intake".into(), "todo.txt".into()),
            ("organize".into(), "Taxes/todo.txt".into()),
        ]
    );
    assert_eq!(
        place.journal(&action_id),
        steps(&[
            ("confirm", "result"),
            ("move", "intent"),
            ("move", "result"),
            ("reconcile", "result"),
        ])
    );
    place.start_again_changes_nothing(&[&action_id], &started);
    note(
        "after the move, before the final record",
        &killed,
        &[
            ("status before the new start", "running".into()),
            ("status after", view["status"].to_string()),
            ("recovered_at_ms", view["recovered_at_ms"].to_string()),
            (
                "Taxes/todo.txt sha256/inode = original",
                format!("{original:?}"),
            ),
        ],
    );
}

#[test]
fn o18c_killed_after_the_undo_move_is_committed_once_by_the_next_process() {
    let place = Place::new();
    let original = place.file("todo.txt");
    let killed = place.kill_at(Task::Undo, Point::AfterMove);
    let undo_id = place.action_of(&killed.id("plan_id"));
    let organize_id = killed.id("organize_action_id");
    let file_id = killed.id("file_id");

    // The file is back on disk; the records still say Notes/Archive.
    assert_eq!(place.status(&undo_id), "running");
    assert_eq!(place.status(&organize_id), "committed");
    assert_eq!(place.file("todo.txt"), original);
    assert_eq!(place.recorded_path(&file_id), "Notes/Archive/todo.txt");
    assert!(place.path("Notes/Archive").is_dir());

    let started = place.start(&[&undo_id, &organize_id]);

    assert_eq!(start_status(&started), "ready");
    let undo = &started["actions"][0];
    let organize = &started["actions"][1];
    assert_eq!(undo["status"], "committed");
    assert!(undo["recovered_at_ms"].is_i64());
    assert_eq!(organize["status"], "undone");
    assert_eq!(place.file("todo.txt"), original, "not moved again");
    assert_eq!(place.recorded_path(&file_id), "todo.txt");
    assert_eq!(
        place.history(&file_id),
        [
            ("intake".into(), "todo.txt".into()),
            ("organize".into(), "Notes/Archive/todo.txt".into()),
            ("undo".into(), "todo.txt".into()),
        ]
    );
    // The folder the organize action created (with a receipt) is empty
    // again, so the recovered undo removes it, after its cleanup intent.
    assert_eq!(undo["folders_removed"], json!(["Notes/Archive"]));
    assert!(!place.path("Notes/Archive").exists());
    assert!(place.path("Notes/2026/plan.md").exists());
    place.start_again_changes_nothing(&[&undo_id, &organize_id], &started);
    note(
        "undo: after its move, before the final record",
        &killed,
        &[
            ("undo status before the new start", "running".into()),
            ("undo status after", undo["status"].to_string()),
            ("organize status after", organize["status"].to_string()),
            ("todo.txt sha256/inode = original", format!("{original:?}")),
        ],
    );
}

// ---------------------------------------------------------------------------
// The new process finds something else, or cannot write its check
// ---------------------------------------------------------------------------

#[test]
fn o18c_killed_after_the_move_then_edited_needs_a_decision_and_moves_nothing() {
    let place = Place::new();
    let killed = place.kill_at(Task::Organize, Point::AfterMove);
    let action_id = place.action_of(&killed.id("plan_id"));
    let file_id = killed.id("file_id");
    // Before Traeva starts again, the moved file is edited.
    fs::write(place.path("Taxes/todo.txt"), "buy oat milk\n").unwrap();
    let left = place.tree();

    let started = place.start(&[&action_id]);

    let view = &started["actions"][0];
    assert_eq!(view["status"], "needs_user_decision");
    assert_eq!(view["problem"]["code"], "result_unconfirmed");
    assert_eq!(place.tree(), left, "nothing moved back or anywhere");
    assert_eq!(place.recorded_path(&file_id), "todo.txt");
    assert_eq!(place.history(&file_id).len(), 1, "no organize entry");
    place.start_again_changes_nothing(&[&action_id], &started);
    note(
        "after the move, file edited before the new start",
        &killed,
        &[("status after", view["status"].to_string())],
    );
}

#[test]
fn o18c_a_check_that_cannot_be_saved_blocks_storage_and_runs_again_at_the_next_start() {
    let place = Place::new();
    let original = place.file("todo.txt");
    let killed = place.kill_at(Task::Organize, Point::AfterMove);
    let action_id = place.action_of(&killed.id("plan_id"));
    let file_id = killed.id("file_id");
    let left = place.tree();
    let records = place.counts();

    // The journal cannot be written: the new process stops before anything
    // else and changes nothing.
    place.refuse_journal_entries(true);
    let blocked = place.start(&[]);
    assert_eq!(start_status(&blocked), "blocked");
    assert_eq!(place.status(&action_id), "running");
    assert_eq!(place.counts(), records);
    assert_eq!(place.tree(), left);
    assert_eq!(place.recorded_path(&file_id), "todo.txt");

    // Once it can be written, the next start records the move once.
    place.refuse_journal_entries(false);
    let started = place.start(&[&action_id]);
    assert_eq!(start_status(&started), "ready");
    assert_eq!(started["actions"][0]["status"], "committed");
    assert_eq!(place.file("Taxes/todo.txt"), original);
    assert_eq!(place.recorded_path(&file_id), "Taxes/todo.txt");
    assert_eq!(place.tree(), left, "no second move");
    place.start_again_changes_nothing(&[&action_id], &started);
    note(
        "after the move; the first new start cannot save its check",
        &killed,
        &[
            ("first new start", start_status(&blocked).to_string()),
            (
                "second new start",
                started["actions"][0]["status"].to_string(),
            ),
        ],
    );
}
