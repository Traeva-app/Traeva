//! Choosing a record's new place by hand (#31 material §4 and its
//! supplement; shared-contract.md §4.3a).
//!
//! When the file a record watches is renamed or moved outside Traeva, the
//! record does not follow it: the user chooses the file's new place, and
//! Traeva never searches for it. The chosen file counts only when, looked at
//! again under the execution lock just before anything is written:
//! - it is in a folder Traeva may use, reached without symbolic links;
//! - it is the same physical object as the record's file (device, inode and
//!   creation time, `FileEvidence::same_object`) with exactly one link;
//! - it reads the same twice (a stable read).
//!
//! A copy, a file on another volume, a file with other links or any other
//! file is not the record's file: nothing changes (`not_same_object`) and
//! the record keeps waiting. The physical facts are only evidence for this
//! check: they never merge two file records.
//!
//! The old place is looked at again too. Whatever is there now (nothing, or
//! another object) never joins the work because of the move, and candidates
//! saved there stay as they were, to be decided. A record that is off is not
//! relocated: Memory was not recording it, so there is no record to go on.
//! The file looked for is the newest one on the record's line: the watched
//! file, or the later object a confirmed "continue" went on to while the
//! watch stayed (only the history was confirmed). Choosing the watched file
//! after its line went on is refused: going on there would give the work two
//! ends. A lost record whose own file is back at its place goes on there:
//! the user chooses that place, and recording resumes without a new watch.
//!
//! Confirming writes one transaction: the file at the new place is
//! registered under Organize's rules (a new file record, unless one Organize
//! made there for this object has no record yet), its version is saved, the
//! old file record is marked missing (its object was just found elsewhere
//! with one name, as Organize's own moves do), the old watch stops and one
//! starts at the new place (link `relocated`), and the relation from the old
//! file's last version to the new one is recorded, confirmed by the system
//! (the object is the same) on the user's choice of place. The move itself was never seen: the pair
//! across it always shows a gap. Candidate capture, when on, goes along and
//! looks at the new place only.
//!
//! The preview only looks (nothing is read); its token covers everything it
//! shows, so when anything changed before the confirmation, nothing is done.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::candidates::{
    PlaceHolds, open_switch_in, page_outdated, recorded_elsewhere, root_active_in,
};
use crate::capture::{Read, WatchRow};
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    CandidateCapture, ChainRow, GapEnd, MemoryRecordView, PlaceView, WentOn, chain_tail_in,
    end_gaps_in, goes_on_in, insert_gap_in, latest_watch_of_chain_in, line_front_in, move_watch_in,
    to_text, unknown_record,
};
use crate::memory_consent::memory_error;
use crate::memory_contracts::{GapKind, SnapshotRef};
use crate::observe;
use crate::platform::macos::{self, Kind, Observation};
use crate::snapshot::{insert_next_snapshot, query_snapshots, store_bytes, unknown_file};
use crate::store::{FileRow, SeenFile, Store, observe_in, query_files, sqlite_error};
use crate::{
    OrganizeService, evidence_of, file_error, is_canonical_uuid, read_in_root, relative_location,
    seen_file,
};

/// What relocating a record to the chosen file would do, before the user
/// confirms. There is a preview only when the chosen file is the record's
/// file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelocationPreview {
    pub chain_id: String,
    /// Where the record's file was, and what is there now. `recorded_file`
    /// only for a lost record whose file is back at its place (then
    /// `new_place` is that place).
    pub old_place: PlaceView,
    pub old_place_holds: PlaceHolds,
    /// The chosen file: the same physical object, with one link.
    pub new_place: PlaceView,
    pub byte_length: u64,
    pub modified_at_ms: i64,
    /// The record's candidates that still wait (saved at the old place, or
    /// before): they stay, to be decided, and none of them joins the work
    /// because of the move.
    pub waiting_candidates: i64,
    /// Candidate capture goes along (when on) and looks at the new place
    /// only.
    pub candidate_capture: CandidateCapture,
    /// Bring it back to confirm.
    pub page_token: String,
}

/// Confirming the new place the preview showed. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelocationRequest {
    pub chain_id: String,
    /// Exactly as `RelocationPreview.new_place` showed it.
    pub root_id: String,
    pub relative_path: String,
    pub page_token: String,
}

/// What relocating did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelocationOutcome {
    pub record: MemoryRecordView,
    /// The version saved at the new place: the first of its file record.
    /// For a lost record that resumed at its place, the file's newest
    /// version (a new one only when its content changed).
    pub snapshot: SnapshotRef,
}

/// Everything the preview shows, looked at again before confirming.
struct Facts {
    chain: ChainRow,
    watch: WatchRow,
    /// The record's watched file (at the old place, by its record).
    file: FileRow,
    /// The newest file on the record's line: the watched file, or the later
    /// object a confirmed "continue" went on to. The file looked for.
    front: FileRow,
    /// The newest file's last version: where the relation across the move
    /// starts.
    before: Option<String>,
    /// The record's newest version (candidates included).
    tail: Option<String>,
    old_place: PlaceView,
    old_holds: PlaceHolds,
    old_object: Option<Observation>,
    new_place: PlaceView,
    new_segments: Vec<String>,
    new_name: String,
    observed: Observation,
    switch: Option<String>,
    waiting: i64,
    candidate_capture: CandidateCapture,
}

impl Facts {
    fn token(&self) -> String {
        let object = |o: &Observation| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}",
                o.id.dev, o.id.ino, o.birthtime_ns, o.size, o.mtime_ns, o.ctime_ns, o.link_count
            )
        };
        let lines = [
            "memory-relocation-1".to_string(),
            format!("chain:{}:{}", self.chain.chain_id, self.chain.lost),
            format!(
                "watch:{}:{}:{}",
                self.watch.watch_id, self.file.file_id, self.front.file_id
            ),
            format!("before:{}", self.before.as_deref().unwrap_or("-")),
            format!("tail:{}", self.tail.as_deref().unwrap_or("-")),
            // Which object is at the old place, not its content: the page
            // shows only what it is.
            format!(
                "old:{}:{}:{}:{}",
                self.old_place.root_id,
                self.old_place.relative_path,
                to_text(self.old_holds),
                self.old_object
                    .as_ref()
                    .map_or("-".to_string(), |o| format!(
                        "{}:{}:{}",
                        o.id.dev, o.id.ino, o.birthtime_ns
                    ))
            ),
            format!(
                "new:{}:{}:{}",
                self.new_place.root_id,
                self.new_place.relative_path,
                object(&self.observed)
            ),
            format!("switch:{}", self.switch.as_deref().unwrap_or("-")),
            format!("waiting:{}", self.waiting),
            format!("capture:{}", to_text(self.candidate_capture)),
        ];
        observe::sha256_hex(lines.join("\n").as_bytes())
    }
}

impl OrganizeService {
    /// Looks at the file the user chose in the picker (`picked`, in root
    /// `root_id`) as record `chain_id`'s new place, and at its old place.
    /// Reads and writes nothing.
    pub fn relocation_preview(
        &self,
        chain_id: &str,
        root_id: &str,
        picked: &Path,
    ) -> Result<RelocationPreview, AppError> {
        let store = self.store()?;
        check_ids(chain_id, root_id)?;
        let facts = self.relocation_facts(store, chain_id, root_id, picked)?;
        Ok(RelocationPreview {
            chain_id: chain_id.to_string(),
            page_token: facts.token(),
            old_place: facts.old_place,
            old_place_holds: facts.old_holds,
            new_place: facts.new_place,
            byte_length: facts.observed.size,
            modified_at_ms: facts.observed.mtime_ns.div_euclid(1_000_000),
            waiting_candidates: facts.waiting,
            candidate_capture: facts.candidate_capture,
        })
    }

    /// Relocates the record to the place its preview showed (see the module
    /// note). Nothing changes when anything the preview showed changed, or
    /// the file there is not the record's file.
    pub fn relocate_record(
        &self,
        request: &RelocationRequest,
    ) -> Result<RelocationOutcome, AppError> {
        self.relocate_record_with(request, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn relocate_record_with(
        &self,
        request: &RelocationRequest,
        read: Read,
    ) -> Result<RelocationOutcome, AppError> {
        let store = self.store()?;
        check_ids(&request.chain_id, &request.root_id)?;
        if request.page_token.len() != 64
            || !request
                .page_token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("The page token is not valid."));
        }
        // Only a plain relative path, as the preview gave it, is resolved.
        if request.relative_path.is_empty()
            || request.relative_path.starts_with('/')
            || request.relative_path.contains('\0')
            || request
                .relative_path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(invalid("The new place is not the one the page showed."));
        }
        let chain_id = request.chain_id.as_str();
        store.chain(chain_id)?.ok_or_else(unknown_record)?;
        let root = store
            .get_root(&request.root_id)?
            .ok_or_else(|| invalid("The new place is not the one the page showed."))?;
        let picked = PathBuf::from(&root.display_path).join(&request.relative_path);
        let no_capture = self.lock_execution();
        self.ensure_run();
        let facts = self.relocation_facts(store, chain_id, &request.root_id, &picked)?;
        if facts.new_place.relative_path != request.relative_path {
            return Err(invalid("The new place is not the one the page showed."));
        }
        if facts.token() != request.page_token {
            return Err(page_outdated(
                "The file, its old place or the record changed since the page was opened. \
                 Nothing was changed. Choose the file again.",
            ));
        }
        // Nothing is read while an action in either folder is not settled:
        // the file may be one Organize was moving.
        self.no_unsettled_action(store, &facts.old_place.root_id, &facts.file.file_id)?;
        self.no_unsettled_action(store, &request.root_id, &facts.file.file_id)?;
        let (_root_row, folder) = self.verified_root(&request.root_id)?;
        let shown = facts.new_place.display_path.as_str();
        let stable = read_in_root(&folder, &facts.new_segments, &facts.new_name, shown, read)?;
        if !evidence_of(&stable.observed).shows_object_of(&facts.front.evidence) {
            return Err(page_outdated(&format!(
                "{shown} was replaced while Memory read it. Nothing was changed. Choose the file \
                 again."
            )));
        }
        let seen = seen_file(
            &request.root_id,
            &facts.new_segments,
            &facts.new_name,
            &stable,
            self.now(),
        );
        store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes)?;
        let snapshot = store.record_relocation(&facts, &seen, &request.page_token)?;
        // The last look at the old watch (lost, file not found) is over.
        self.forget_check(&facts.watch.watch_id);
        drop(no_capture);
        self.note_run_seen(true);
        Ok(RelocationOutcome {
            record: self.memory_record(chain_id)?,
            snapshot,
        })
    }

    /// What the preview shows: refused when the record is off, its file is
    /// still at its place, or the chosen file is not its file.
    fn relocation_facts(
        &self,
        store: &Store,
        chain_id: &str,
        root_id: &str,
        picked: &Path,
    ) -> Result<Facts, AppError> {
        let chain = store.chain(chain_id)?.ok_or_else(unknown_record)?;
        let watch = store
            .current_watch_of_chain(chain_id)?
            .ok_or_else(unknown_record)?;
        if !watch.active {
            return Err(memory_error(
                ErrorCode::RecordStopped,
                "Memory is off for this record, so it does not go on at a new place. Turn Memory \
                 on for the file at its new place instead.",
                false,
                Some(&watch.file_id),
            ));
        }
        let (file, _) = store.get_file(&watch.file_id)?.ok_or_else(unknown_file)?;
        let front_id = {
            let conn = store.connection()?;
            line_front_in(&conn, &watch.file_id)?
        };
        let front = if front_id == file.file_id {
            file.clone()
        } else {
            store.get_file(&front_id)?.ok_or_else(unknown_file)?.0
        };
        let went_on_elsewhere = front.file_id != file.file_id;
        let old = self.place_occupant(store, chain_id, &watch.file_id)?;
        let back = old.holds == PlaceHolds::RecordedFile;
        if back && went_on_elsewhere {
            return Err(went_on());
        }
        if back && !chain.lost {
            return Err(conflict(format!(
                "{} still holds the file this record watches: there is nothing to relocate.",
                old.place.shown
            )));
        }
        let (root_row, folder) = self.verified_root(root_id)?;
        let shown = picked.display().to_string();
        let (mut segments, mut name) = relative_location(&root_row.display_path, picked)?;
        let names: Vec<&str> = segments.iter().map(String::as_str).collect();
        let observed = macos::open_chain(&folder, &names)
            .and_then(|chain| macos::observe(chain.last().unwrap_or(&folder), &name))
            .map_err(|err| file_error(&shown, err))?;
        let is_file = observed.kind == Kind::File;
        if !is_file || !evidence_of(&observed).shows_object_of(&front.evidence) {
            if is_file
                && went_on_elsewhere
                && evidence_of(&observed).shows_object_of(&file.evidence)
            {
                return Err(went_on());
            }
            return Err(memory_error(
                ErrorCode::NotSameObject,
                format!(
                    "{shown} is not the file this record watched (it may be a copy, on another \
                     disk, or have other links). Memory does not go on there."
                ),
                false,
                Some(&front.file_id),
            ));
        }
        if back {
            // Back at its place: that is the place its record names (the
            // name the picker gave may differ, for example in letter case).
            if root_id != file.root_id {
                return Err(invalid("The file is back at its place: choose that place."));
            }
            let mut parts: Vec<String> =
                file.relative_path.split('/').map(str::to_string).collect();
            name = parts.pop().unwrap_or_default();
            segments = parts;
        }
        let mut relative_path = segments.join("/");
        if !relative_path.is_empty() {
            relative_path.push('/');
        }
        relative_path.push_str(&name);
        let conn = store.connection()?;
        // The file may already have a record at the new place (registered
        // there, with its own Memory record): then it is not a new place of
        // this record.
        let present = query_files(
            &conn,
            "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
            params![root_id, relative_path],
        )?;
        for row in present {
            if row.file_id != front.file_id
                && row.evidence.same_object(&front.evidence)
                && recorded_elsewhere(&conn, chain_id, &row.file_id)?
            {
                return Err(already_recorded());
            }
        }
        let before = query_snapshots(
            &conn,
            "WHERE file_id = ?1 ORDER BY sequence DESC LIMIT 1",
            [&front.file_id],
        )?
        .pop()
        .map(|s| s.snapshot_id);
        if let Some(before) = &before
            && goes_on_in(&conn, before)?
        {
            return Err(went_on());
        }
        let tail = chain_tail_in(&conn, chain_id)?;
        let switch = open_switch_in(&conn, chain_id)?;
        drop(conn);
        let record = self.memory_record(chain_id)?;
        Ok(Facts {
            old_place: record.place.clone(),
            old_holds: old.holds,
            old_object: old.observed,
            new_place: PlaceView {
                display_path: format!("{}/{relative_path}", root_row.display_path),
                root_id: root_id.to_string(),
                relative_path,
            },
            new_segments: segments,
            new_name: name,
            observed,
            switch,
            waiting: record.pending.objects,
            candidate_capture: record.candidate_capture,
            chain,
            watch,
            file,
            front,
            before,
            tail,
        })
    }
}

impl Store {
    /// Writes one relocation in one transaction (see the module note),
    /// after checking again that the record still watches the same file
    /// and both folders may still be used.
    fn record_relocation(
        &self,
        facts: &Facts,
        seen: &SeenFile,
        page_token: &str,
    ) -> Result<SnapshotRef, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the file's new place", &err);
        let outdated = || {
            page_outdated(
                "The record changed while Memory read the file. Nothing was changed. Choose the \
                 file again.",
            )
        };
        let chain_id = facts.chain.chain_id.as_str();
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        let latest = latest_watch_of_chain_in(&tx, chain_id)?;
        if latest.as_ref().map(|w| (w.watch_id.as_str(), w.active))
            != Some((facts.watch.watch_id.as_str(), true))
            || chain_tail_in(&tx, chain_id)? != facts.tail
        {
            return Err(outdated());
        }
        for (root_id, shown) in [
            (&facts.old_place.root_id, &facts.old_place.display_path),
            (&seen.root_id, &facts.new_place.display_path),
        ] {
            if !root_active_in(&tx, root_id)? {
                return Err(crate::snapshot::revoked_meanwhile(shown));
            }
        }
        let file = observe_in(&tx, seen)?;
        // Back at its own place (a lost record) or at a new file record.
        let back = file.file_id == facts.file.file_id;
        if !file.evidence.shows_object_of(&facts.front.evidence)
            || back != (facts.old_holds == PlaceHolds::RecordedFile)
        {
            return Err(outdated());
        }
        if !back && recorded_elsewhere(&tx, chain_id, &file.file_id)? {
            return Err(already_recorded());
        }
        if let Some(before) = &facts.before
            && goes_on_in(&tx, before)?
        {
            return Err(went_on());
        }
        let now = seen.now_ms;
        let (snapshot, created) =
            insert_next_snapshot(&tx, &file.file_id, file.record_revision, seen)?;
        let decision_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO memory_decisions (decision_id, chain_id, batch_id, choice, page_token,
                 decided_at_ms)
             VALUES (?1, ?2, ?3, 'relocate', ?4, ?5)",
            params![
                decision_id,
                chain_id,
                uuid::Uuid::new_v4().to_string(),
                page_token,
                now
            ],
        )
        .map_err(fail)?;
        // The move itself was never seen. When no gap about the old place
        // starts at the old file's last version yet (the watcher had not
        // looked since, or the record's newest version was a candidate),
        // one is recorded, so the pair across the move shows it. It is
        // written ended: what is at the old place is not this record's any
        // more.
        let marked: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM memory_gaps WHERE chain_id = ?1
                                AND ended_at_ms IS NULL AND after_snapshot_id IS ?2
                                AND kind IN ('capture_failed', 'replaced_uncaptured', 'lost'))",
                params![chain_id, facts.before],
                |r| r.get(0),
            )
            .map_err(fail)?;
        if !marked {
            let (kind, problem) = match facts.old_holds {
                PlaceHolds::UnreadObject | PlaceHolds::OtherRecord => {
                    (GapKind::ReplacedUncaptured, None)
                }
                PlaceHolds::RecordedFile => (GapKind::Lost, None),
                // Nothing there, or a candidate: the record's file was not at
                // its place.
                _ => (GapKind::CaptureFailed, Some(ErrorCode::FileNotFound)),
            };
            insert_gap_in(
                &tx,
                chain_id,
                kind,
                facts.before.as_deref(),
                problem,
                now,
                Some((now, GapEnd::Relocated)),
            )?;
        }
        if back {
            // The watch never stopped: recording resumes where it was.
            end_gaps_in(
                &tx,
                chain_id,
                &[
                    GapKind::CaptureFailed,
                    GapKind::ReplacedUncaptured,
                    GapKind::Lost,
                ],
                GapEnd::Relocated,
                now,
            )?;
            if created {
                set_tail(&tx, chain_id, &snapshot.snapshot_id)?;
            }
            tx.commit().map_err(fail)?;
            return Ok(snapshot);
        }
        // Its object was just found at the new place with one name, so the
        // old file record is not at its path any more (as Organize's own
        // moves record it).
        tx.execute(
            "UPDATE files SET presence = 'missing', updated_at_ms = ?2
             WHERE file_id = ?1 AND presence = 'present'",
            params![facts.front.file_id, now],
        )
        .map_err(fail)?;
        // The record's work now (merged, or moved with its file): read
        // before the watch goes on, while it still watches the old file.
        let family_id =
            crate::works::live_family_of_chain_in(&tx, chain_id)?.ok_or_else(unknown_record)?;
        move_watch_in(
            &tx,
            chain_id,
            &facts.watch,
            &file.file_id,
            &decision_id,
            WentOn::Relocated,
            now,
        )?;
        tx.execute(
            "INSERT INTO memory_members (member_id, family_id, file_id, status, basis,
                 decision_id, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 'confirmed_system', 'relocation', ?4, ?5, ?5)",
            params![
                uuid::Uuid::new_v4().to_string(),
                family_id,
                file.file_id,
                decision_id,
                now
            ],
        )
        .map_err(fail)?;
        if let Some(before) = &facts.before {
            tx.execute(
                "INSERT INTO memory_edges (edge_id, family_id, kind, parent_snapshot_id,
                     child_snapshot_id, status, skipped_items, decision_id, created_at_ms,
                     updated_at_ms)
                 VALUES (?1, ?2, 'continuation', ?3, ?4, 'confirmed_system', 0, ?5, ?6, ?6)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    family_id,
                    before,
                    snapshot.snapshot_id,
                    decision_id,
                    now
                ],
            )
            .map_err(fail)?;
        }
        set_tail(&tx, chain_id, &snapshot.snapshot_id)?;
        tx.commit().map_err(fail)?;
        Ok(snapshot)
    }
}

fn set_tail(tx: &Connection, chain_id: &str, snapshot_id: &str) -> Result<(), AppError> {
    tx.execute(
        "UPDATE memory_chains SET tail_snapshot_id = ?1 WHERE chain_id = ?2",
        params![snapshot_id, chain_id],
    )
    .map_err(|err| sqlite_error("Cannot save the file's new place", &err))?;
    Ok(())
}

fn check_ids(chain_id: &str, root_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(chain_id) && is_canonical_uuid(root_id) {
        Ok(())
    } else {
        Err(invalid("The Memory record or folder ID is not valid."))
    }
}

fn invalid(message: &str) -> AppError {
    memory_error(ErrorCode::InvalidInput, message, false, None)
}

fn conflict(message: String) -> AppError {
    memory_error(ErrorCode::DecisionConflict, message, false, None)
}

fn went_on() -> AppError {
    conflict(
        "This record already went on as another file (a later object was confirmed to continue \
         it), so this file is not its newest one. Nothing was changed."
            .to_string(),
    )
}

fn already_recorded() -> AppError {
    conflict(
        "That file already has its own Memory record or a place in a work, so this record does \
         not go on there. Nothing was changed."
            .to_string(),
    )
}

#[cfg(test)]
#[path = "relocate_tests.rs"]
mod tests;
