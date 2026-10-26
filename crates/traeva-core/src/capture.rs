//! Memory's automatic capture (#32): the user turns Memory on for one chosen
//! file, and Traeva saves each new version of it (snapshot.rs) until the user
//! turns it off. No other file is ever captured.
//!
//! How changes are found: about every `MEMORY_CHECK_INTERVAL` the app calls
//! `check_memory`. It looks at each file Memory is on for, through the root
//! handle, without reading or writing anything. Only when the file looks
//! different from its record (its saved physical evidence, with the
//! modification and change times, or its size) is it captured: read stably
//! and saved if its bytes are new. Every write moves the change time, so an
//! edit that keeps the size and puts the modification time back is still
//! found. A file that moved is reported without being read. Several quick saves between two
//! checks are seen once, as the state they ended in. The first check after a
//! start finds what changed while Traeva was closed.
//!
//! What is not followed: a file that moved or was renamed outside Traeva is
//! reported and not captured. Whether another file is the same work is for
//! #31's rules (family.rs). The one exception (the student's 10/14
//! decision): a new object at the record's exact place, which is how apps
//! that save by replacing the file leave it, is recorded as the next version
//! by itself (auto_link.rs); when that is not possible it is reported. A move
//! that Traeva itself made (organize or Undo) is part of the file's record,
//! so Memory keeps following the file there.
//!
//! Every watch belongs to a record chain (#31, family.rs). Each pass tells
//! the chain what it saw (`GapSignal`), so a stretch in which the file could
//! not be captured, or its place held an object Memory did not read, is
//! recorded as a gap. Each pass also keeps this run of the watcher recorded
//! (`memory_runs`), so the time Traeva was not running is known too.
//!
//! Turning Memory off is final for that watch, and a capture that was under
//! way saves nothing once the watch is off: the same transaction that would
//! save the snapshot checks it.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::contracts::{AppError, ErrorCode};
use crate::family::GapSignal;
use crate::memory_contracts::SnapshotRef;
use crate::observe::{self, ReadError, ReadLimits, StableRead};
use crate::platform::macos::{self, Folder, FsError, Observation};
use crate::snapshot::{Capture, check_file_id, not_there, replaced, unknown_file, with_file};
use crate::store::{FileRow, Store, sqlite_error};
use crate::{OrganizeService, evidence_of, file_error};

/// How often the app looks at the files Memory is on for.
pub const MEMORY_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// The watcher reads a changed file once per pass (two reads with the pause
/// between them): a file still being saved is tried again at the next pass,
/// instead of holding up a plan's run for several attempts.
const WATCH_READ: ReadLimits = ReadLimits {
    max_attempts: 1,
    ..ReadLimits::SPEC
};

/// One file Memory is on for, or was on for, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryWatchView {
    pub watch_id: String,
    pub file_id: String,
    pub status: WatchStatus,
    /// Where the file's record says it is now.
    pub root_id: String,
    pub relative_path: String,
    pub started_at_ms: i64,
    pub stopped_at_ms: Option<i64>,
    /// How many versions of this file are saved (from any period).
    pub versions: i64,
    pub latest: Option<SnapshotRef>,
    /// When this run of Traeva last looked at the file; null before the
    /// first look after a start.
    pub checked_at_ms: Option<i64>,
    /// Why the last look could not save a version (the file moved, was
    /// replaced, the folder is no longer authorized...), or null.
    pub problem: Option<AppError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchStatus {
    /// New versions are saved automatically.
    Active,
    /// Turned off; nothing new is saved.
    Stopped,
}

/// What one look at one file found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckOutcome {
    /// The file looks exactly as recorded; nothing was read.
    Unchanged,
    /// A new version was saved.
    Saved { snapshot: SnapshotRef },
    /// The file was saved again with the same bytes: nothing new.
    SameBytes { snapshot: SnapshotRef },
    /// Nothing could be saved; the error says why.
    Problem { error: AppError },
    /// Memory was turned off while the file was read: nothing was saved.
    Stopped,
    /// The record's place held another object and candidate capture is on:
    /// it was saved as a candidate (#31). That confirms nothing.
    CandidateSaved {
        snapshot: SnapshotRef,
        candidate_id: String,
    },
    /// Candidate capture is paused at its quota: nothing was read or saved.
    CandidatePaused,
    /// The record's place held a new object (an app saved by replacing the
    /// file): it was saved as the next version and the record went on
    /// there, with a new watch (auto_link.rs). `snapshot` is its version.
    LinkedAutomatically { snapshot: SnapshotRef },
}

/// One line of a `check_memory` pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatchCheck {
    pub watch_id: String,
    pub file_id: String,
    pub outcome: CheckOutcome,
}

/// The last look at a watch, kept in memory for the window.
#[derive(Debug, Clone)]
pub(crate) struct LastCheck {
    at_ms: i64,
    problem: Option<AppError>,
}

pub(crate) type Read<'a> =
    &'a mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>;

/// What is at a watched file's place (where its record says it is), seen
/// without reading anything.
pub(crate) enum Look {
    /// The watched file itself; `changed` when it looks different from its
    /// record, or its latest bytes are not saved yet.
    Watched { changed: bool },
    /// Nothing is there: the file moved, was renamed or deleted outside Traeva.
    Empty,
    /// Another object is there: the file was replaced (or the record is no
    /// longer present and something else took the place).
    Other { observed: Observation },
}

impl OrganizeService {
    /// Turns Memory on for registered file `file_id` and saves its first
    /// version right away. Turning it on again while it is on keeps the same
    /// watch. The file must be where its record says, in an authorized folder.
    pub fn start_memory(&self, file_id: &str) -> Result<MemoryWatchView, AppError> {
        self.start_memory_with(file_id, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn start_memory_with(
        &self,
        file_id: &str,
        read: Read,
    ) -> Result<MemoryWatchView, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        let (file, present) = store.get_file(file_id)?.ok_or_else(unknown_file)?;
        let (root_row, _) = self.verified_root(&file.root_id)?;
        if !present {
            let shown = format!("{}/{}", root_row.display_path, file.relative_path);
            return Err(with_file(not_there(&shown), file_id));
        }
        // Held from joining the chain to the first capture, so no other
        // capture, stop or decision comes in between.
        let no_capture = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.ensure_run();
        let watch = store.start_watch_in_chain(file_id, self.now())?;
        let outcome = outcome_of(self.capture_locked(file_id, Some(&watch.watch_id), read));
        drop(no_capture);
        self.remember(&watch.watch_id, &outcome);
        // As it is now: it may have been turned off meanwhile.
        let watch = store.watch(&watch.watch_id)?.unwrap_or(watch);
        self.watch_view(store, &watch)
    }

    /// Turns Memory off for file `file_id`: from now on nothing new is saved
    /// for it. The saved versions stay. Returns the watch that was turned
    /// off, or the latest one when Memory was already off; null when Memory
    /// was never on for this file. Turning a watch off also stops its record
    /// chain (family.rs); after a chain went on as another file, stop it with
    /// `stop_memory_record`.
    pub fn stop_memory(&self, file_id: &str) -> Result<Option<MemoryWatchView>, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        // Waits for a capture that is under way (it holds this lock), so once
        // Memory is off nothing more of the file is read or saved.
        let _no_capture = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stopped = store.stop_file_watch(file_id, self.now())?;
        // The watcher was running when Memory was turned off: a quit soon
        // after is not counted as a gap of this watch.
        self.note_run_seen(true);
        match stopped {
            Some(watch) => Ok(Some(self.watch_view(store, &watch)?)),
            None => Ok(None),
        }
    }

    /// Every watch: the ones that are on first, then the ones turned off,
    /// newest first (at most 200).
    pub fn memory_watches(&self) -> Result<Vec<MemoryWatchView>, AppError> {
        let store = self.store()?;
        store
            .watches(200)?
            .iter()
            .map(|watch| self.watch_view(store, watch))
            .collect()
    }

    /// One pass of the watcher: looks at every file Memory is on for and
    /// saves the new versions. The app runs it about every
    /// `MEMORY_CHECK_INTERVAL`, on a background thread.
    pub fn check_memory(&self) -> Result<Vec<WatchCheck>, AppError> {
        self.check_memory_with(&mut |root, parent, name| {
            observe::read_stable_with(parent, name, root.id().dev, WATCH_READ, &mut |_| {
                std::thread::sleep(observe::PAUSE_BETWEEN_READS)
            })
        })
    }

    pub(crate) fn check_memory_with(&self, read: Read) -> Result<Vec<WatchCheck>, AppError> {
        let store = self.store()?;
        self.ensure_run();
        let mut checks = Vec::new();
        let mut saved = false;
        for watch in store.active_watches()? {
            // A watch with no chain only exists in tests that write rows
            // directly; it is checked the same way, without gaps. A chain
            // that cannot be read is this watch's problem only.
            let (chain, outcome, signal) = match store.chain_of_watch(&watch.watch_id) {
                Err(error) => (None, CheckOutcome::Problem { error }, GapSignal::Nothing),
                Ok(Some(chain)) if chain.lost => {
                    let error = self.lost(&watch.file_id);
                    (
                        Some(chain),
                        CheckOutcome::Problem { error },
                        GapSignal::Nothing,
                    )
                }
                Ok(chain) => {
                    let chain_id = chain.as_ref().map(|chain| chain.chain_id.as_str());
                    let (outcome, signal) = self.check_place(&watch, chain_id, read);
                    (chain, outcome, signal)
                }
            };
            if let Some(chain) = &chain {
                self.note_gaps(&watch.watch_id, &chain.chain_id, &watch.file_id, signal);
            }
            saved |= matches!(
                outcome,
                CheckOutcome::Saved { .. }
                    | CheckOutcome::CandidateSaved { .. }
                    | CheckOutcome::LinkedAutomatically { .. }
            );
            if matches!(outcome, CheckOutcome::LinkedAutomatically { .. }) {
                // The old watch is over: a problem it had no longer holds.
                self.forget_check(&watch.watch_id);
            } else {
                self.remember(&watch.watch_id, &outcome);
            }
            checks.push(WatchCheck {
                watch_id: watch.watch_id,
                file_id: watch.file_id,
                outcome,
            });
        }
        self.note_run_seen(saved);
        Ok(checks)
    }

    /// Looks at the watched file's place and captures the file when it
    /// changed; what the chain should record about it comes back too.
    fn check_place(
        &self,
        watch: &WatchRow,
        chain_id: Option<&str>,
        read: Read,
    ) -> (CheckOutcome, GapSignal) {
        let file_id = watch.file_id.as_str();
        let (shown, look) = match self.look_at_place(file_id) {
            Ok(seen) => seen,
            Err(error) => {
                let signal = GapSignal::failed(error.code, true);
                return (CheckOutcome::Problem { error }, signal);
            }
        };
        match look {
            Look::Watched { changed: false } => {
                (CheckOutcome::Unchanged, GapSignal::Covered { saved: false })
            }
            Look::Watched { changed: true } => {
                let outcome = outcome_of(self.capture_for(file_id, Some(&watch.watch_id), read));
                let signal = match &outcome {
                    CheckOutcome::Saved { .. } => GapSignal::Covered { saved: true },
                    CheckOutcome::SameBytes { .. } => GapSignal::Covered { saved: false },
                    // Replaced between the look and the capture: not read.
                    CheckOutcome::Problem { error } if error.code == ErrorCode::FileChanged => {
                        GapSignal::Replaced
                    }
                    CheckOutcome::Problem { error } => GapSignal::failed(error.code, false),
                    _ => GapSignal::Nothing,
                };
                (outcome, signal)
            }
            Look::Empty => (
                CheckOutcome::Problem {
                    error: with_file(not_there(&shown), file_id),
                },
                GapSignal::failed(ErrorCode::FileNotFound, true),
            ),
            Look::Other { observed } => match chain_id {
                Some(chain_id) => self.at_replaced_place(watch, chain_id, &observed, read),
                None => (
                    CheckOutcome::Problem {
                        error: with_file(replaced(&shown), file_id),
                    },
                    GapSignal::Replaced,
                ),
            },
        }
    }

    /// Looks at the place where file `file_id`'s record says it is, without
    /// reading or writing anything: what is there, and the place as shown to
    /// the user. An error when the folder cannot be used.
    pub(crate) fn look_at_place(&self, file_id: &str) -> Result<(String, Look), AppError> {
        let store = self.store()?;
        let (file, present) = store.get_file(file_id)?.ok_or_else(unknown_file)?;
        let (root_row, root) = self.verified_root(&file.root_id)?;
        let shown = format!("{}/{}", root_row.display_path, file.relative_path);
        let mut names: Vec<&str> = file.relative_path.split('/').collect();
        let name = names.pop().unwrap_or_default();
        let looked = macos::open_chain(&root, &names)
            .and_then(|chain| macos::observe(chain.last().unwrap_or(&root), name));
        let observed = match looked {
            Ok(observed) => observed,
            Err(FsError::NotFound) => return Ok((shown, Look::Empty)),
            Err(err) => return Err(with_file(file_error(&shown, err), file_id)),
        };
        if !present || !evidence_of(&observed).same_object(&file.evidence) {
            return Ok((shown, Look::Other { observed }));
        }
        let changed = looks_different(&file, &observed, store.latest_sha256(file_id)?.as_deref());
        Ok((shown, Look::Watched { changed }))
    }

    /// Forgets the last look at watch `watch_id` (its problem no longer
    /// holds, for example once the record went on at the file's new place).
    pub(crate) fn forget_check(&self, watch_id: &str) {
        self.memory_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(watch_id);
    }

    fn remember(&self, watch_id: &str, outcome: &CheckOutcome) {
        let problem = match outcome {
            CheckOutcome::Problem { error } => Some(error.clone()),
            _ => None,
        };
        self.memory_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                watch_id.to_string(),
                LastCheck {
                    at_ms: self.now(),
                    problem,
                },
            );
    }

    pub(crate) fn watch_view(
        &self,
        store: &Store,
        watch: &WatchRow,
    ) -> Result<MemoryWatchView, AppError> {
        let (file, _) = store.get_file(&watch.file_id)?.ok_or_else(unknown_file)?;
        let (versions, latest) = store.versions_of(&watch.file_id)?;
        let last = self
            .memory_checks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&watch.watch_id)
            .cloned();
        Ok(MemoryWatchView {
            watch_id: watch.watch_id.clone(),
            file_id: watch.file_id.clone(),
            status: if watch.active {
                WatchStatus::Active
            } else {
                WatchStatus::Stopped
            },
            root_id: file.root_id,
            relative_path: file.relative_path,
            started_at_ms: watch.started_at_ms,
            stopped_at_ms: watch.stopped_at_ms,
            versions,
            latest,
            checked_at_ms: last.as_ref().map(|last| last.at_ms),
            problem: last.and_then(|last| last.problem),
        })
    }
}

/// The last looks, by watch ID. In memory only: a restart starts empty.
pub(crate) type LastChecks = HashMap<String, LastCheck>;

// ---------------------------------------------------------------------------
// The `memory_watches` table
// ---------------------------------------------------------------------------

/// One row of `memory_watches`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchRow {
    pub(crate) watch_id: String,
    pub(crate) file_id: String,
    pub(crate) active: bool,
    pub(crate) started_at_ms: i64,
    pub(crate) stopped_at_ms: Option<i64>,
}

impl Store {
    pub(crate) fn watch(&self, watch_id: &str) -> Result<Option<WatchRow>, AppError> {
        let conn = self.connection()?;
        Ok(query_watches(&conn, "WHERE watch_id = ?1", [watch_id])?.pop())
    }

    /// Whether watch `watch_id` of file `file_id` is on.
    pub(crate) fn watch_is_on(&self, watch_id: &str, file_id: &str) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let on: Option<bool> = conn
            .query_row(
                "SELECT status = 'active' FROM memory_watches WHERE watch_id = ?1 AND file_id = ?2",
                [watch_id, file_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|err| sqlite_error("Cannot read the Memory settings", &err))?;
        Ok(on == Some(true))
    }

    /// The SHA-256 of the file's latest snapshot, if it has one.
    pub(crate) fn latest_sha256(&self, file_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT sha256 FROM snapshots WHERE file_id = ?1 ORDER BY sequence DESC LIMIT 1",
            [file_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the saved versions", &err))
    }

    fn active_watches(&self) -> Result<Vec<WatchRow>, AppError> {
        let conn = self.connection()?;
        query_watches(
            &conn,
            "WHERE status = 'active' ORDER BY started_at_ms, rowid",
            [],
        )
    }

    fn watches(&self, limit: u32) -> Result<Vec<WatchRow>, AppError> {
        let conn = self.connection()?;
        query_watches(
            &conn,
            "ORDER BY status = 'active' DESC, coalesce(stopped_at_ms, started_at_ms) DESC,
                      rowid DESC
             LIMIT ?1",
            [limit],
        )
    }

    /// How many snapshots the file has, and its latest one.
    pub(crate) fn versions_of(
        &self,
        file_id: &str,
    ) -> Result<(i64, Option<SnapshotRef>), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the saved versions", &err);
        let conn = self.connection()?;
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM snapshots WHERE file_id = ?1",
                [file_id],
                |r| r.get(0),
            )
            .map_err(fail)?;
        let latest: Option<String> = conn
            .query_row(
                "SELECT snapshot_id FROM snapshots WHERE file_id = ?1
                 ORDER BY sequence DESC LIMIT 1",
                [file_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(fail)?;
        drop(conn);
        let latest = match latest {
            Some(id) => self.get_snapshot(&id)?,
            None => None,
        };
        Ok((count, latest))
    }
}

/// `filter` is always a constant written in this file; values go in `params`.
pub(crate) fn query_watches<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<WatchRow>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory settings", &err);
    let sql = format!(
        "SELECT watch_id, file_id, status = 'active', started_at_ms, stopped_at_ms
         FROM memory_watches {filter}"
    );
    let mut stmt = conn.prepare(&sql).map_err(fail)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(WatchRow {
                watch_id: row.get(0)?,
                file_id: row.get(1)?,
                active: row.get(2)?,
                started_at_ms: row.get(3)?,
                stopped_at_ms: row.get(4)?,
            })
        })
        .map_err(fail)?;
    rows.collect::<Result<_, _>>().map_err(fail)
}

/// What a capture for a watch did, as one line of a pass.
fn outcome_of(capture: Result<Option<Capture>, AppError>) -> CheckOutcome {
    match capture {
        Ok(Some(capture)) if capture.created => CheckOutcome::Saved {
            snapshot: capture.snapshot,
        },
        Ok(Some(capture)) => CheckOutcome::SameBytes {
            snapshot: capture.snapshot,
        },
        Ok(None) => CheckOutcome::Stopped,
        Err(error) => CheckOutcome::Problem { error },
    }
}

/// The same file looks different from its record: its physical evidence
/// (with the modification and change times; any write moves the change time,
/// so this also covers edits that keep the size and put the modification
/// time back) or its size. The record can also change without a version
/// being saved (the file was chosen again in Organize, or the first capture
/// failed): then the recorded bytes are not the latest version yet.
pub(crate) fn looks_different(
    file: &FileRow,
    observed: &Observation,
    latest_sha256: Option<&str>,
) -> bool {
    !evidence_of(observed).same_state(&file.evidence)
        || observed.size != file.byte_length
        || latest_sha256 != Some(file.sha256.as_str())
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
