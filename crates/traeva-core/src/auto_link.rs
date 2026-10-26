//! Following a file an app saved by replacing it (#37, the student's
//! decision of 2026-10-14; shared-contract.md §4.2b).
//!
//! Editors such as TextEdit and Word save by writing a new file and putting
//! it in place of the old one, so each save leaves a new file object at the
//! same path. For a record Memory is on for, a new object at the record's
//! exact place (the same folder and relative path, never anywhere else) is
//! recorded by itself as the next version of the same work: no card, no
//! question. The user sees "Linked automatically" and can undo the link
//! (revoking the relation, family_correct.rs). This changes the student's
//! #31 rule "never link files by path or name" for this one case only; a
//! file renamed or moved elsewhere still needs manual relocation.
//!
//! The watcher tries it at every pass that finds a new object at the place,
//! before candidate capture, under the execution lock:
//! - the record's watch is on, the record is not lost or paused, the folder
//!   is authorized, the object could go straight on from the record's newest
//!   version (checked again below), no Organize action in the folder is
//!   unsettled;
//! - the object is read the usual stable way and must be the one looked at;
//! - in one transaction: the same checks again, the place still holds the
//!   record it held, and the object goes straight on from the record's
//!   newest version (no earlier object waits for a decision, no object the
//!   user left off the line is stepped over). The rows are the user's
//!   "continue" (family_decide.rs `continue_current_in`) with status
//!   `confirmed_system`, plus a `memory_auto_links` row.
//!
//! A file still being written (or replaced again during the read) writes
//! nothing and is tried at the next pass. Anything else that stops it (the
//! file is too large, a hard link, hidden or on another disk, another file
//! at the place waits for a decision...) leaves the place as before: it is
//! reported, or saved as a candidate when candidate capture is on.

use crate::OrganizeService;
use crate::candidates::{Place, Target, root_active_in};
use crate::capture::{CheckOutcome, Read, WatchRow};
use crate::contracts::{AppError, ErrorCode};
use crate::evidence_of;
use crate::family::{GapSignal, watching_in};
use crate::platform::macos::Observation;
use crate::snapshot::{store_bytes, with_file};
use crate::store::{Store, sqlite_error};

impl OrganizeService {
    /// The object at chain `chain_id`'s place (`observed`) is new to Memory:
    /// records it as the next version, by itself (see the module note).
    /// `expected`: the file record at the place when it was looked at. None
    /// when it cannot be linked this way: the caller then handles the place
    /// exactly as before.
    pub(crate) fn link_at_place(
        &self,
        watch: &WatchRow,
        chain_id: &str,
        place: &Place,
        observed: &Observation,
        expected: &Option<(String, i64)>,
        read: Read,
    ) -> Option<(CheckOutcome, GapSignal)> {
        #[cfg(test)]
        if self.no_auto_link.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let _no_run = self.lock_execution();
        let store = self.store().ok()?;
        // Under the lock, before anything is read: the record may have been
        // stopped, paused or judged lost while this pass waited for it, and
        // an object that cannot go straight on from the record's newest
        // version (an earlier one waits for a decision...) is not read at
        // every pass only to be refused.
        let ready = (|| -> Result<bool, AppError> {
            let conn = store.connection()?;
            let on = watching_in(&conn, chain_id, &watch.watch_id)?
                && root_active_in(&conn, &place.root_id)?;
            drop(conn);
            Ok(on && store.could_link_at_place(chain_id, &watch.watch_id, observed, expected)?)
        })();
        if !matches!(ready, Ok(true)) {
            return None;
        }
        // The object at the place may be one Organize was moving.
        self.no_unsettled_action(store, &place.root_id, &watch.file_id)
            .ok()?;
        let target = Target::New {
            evidence: evidence_of(observed),
            expected: expected.clone(),
        };
        let (stable, seen) = match self.read_candidate(place, &target, read) {
            Ok(read) => read,
            // Still being saved: nothing is written, the next pass tries again.
            Err(error) if error.code == ErrorCode::FileChanging => {
                let error = with_file(error, &watch.file_id);
                return Some((CheckOutcome::Problem { error }, GapSignal::Nothing));
            }
            Err(_) => return None,
        };
        store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes).ok()?;
        let linked = store
            .link_at_place(chain_id, &watch.watch_id, &seen, &stable.observed, expected)
            .ok()
            .flatten()?;
        // The pass reports it on the old watch; its gaps were ended in the
        // transaction (the record went on at the new file).
        Some((
            CheckOutcome::LinkedAutomatically {
                snapshot: linked.snapshot,
            },
            GapSignal::Nothing,
        ))
    }
}

impl Store {
    /// Relation `edge_id` was made by Traeva itself and still rests on that
    /// alone (`confirmed_system`). Once the user confirmed it again, the
    /// pair is theirs and no longer says "Linked automatically".
    pub(crate) fn auto_linked(&self, edge_id: &str) -> Result<bool, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM memory_auto_links AS a
                            JOIN memory_edges AS e ON e.edge_id = a.edge_id
                            WHERE a.edge_id = ?1 AND e.status = 'confirmed_system')",
            [edge_id],
            |r| r.get(0),
        )
        .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
    }
}

#[cfg(test)]
#[path = "auto_link_tests.rs"]
mod tests;
