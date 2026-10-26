//! Memory's record tasks and works (#31; the accepted student material and
//! shared-contract.md §4.2b and §4.3 have the rules).
//!
//! - A **record chain** is one record task: the watches (capture.rs) that go
//!   on from one another. Turning Memory on for a file with no chain starts
//!   one; turning it on again after it was off goes on with the same chain,
//!   with a `memory_off` gap in between. Its candidate switch, its quota and
//!   its gaps belong to the chain, so none of them starts over when the chain
//!   goes on.
//! - A **work** (family) is the file records that belong together. A file
//!   Memory is turned on for starts its own work. No file record is ever
//!   merged into another, and same content or a same name never links two.
//! - A **gap** is recorded evidence that Memory may not have seen every save
//!   (D3): Memory was off, the place could not be captured, the place held an
//!   object Memory did not read, candidate capture was paused, or the work is
//!   lost. A gap's start and end are written once each, on the pass where the
//!   situation changes, never at every pass. Each gap lies after the chain's
//!   newest version when it began; every version pair starting there is
//!   marked (lineage.rs).
//! - A **run** is one period in which the watcher ran (`memory_runs`). The
//!   time from one run's last sign of life to the next run's start is a gap
//!   too (Traeva was quit or crashed; #34 student decision, 2026-10-09), even
//!   when the file is the same afterwards. Only closing the window, with the
//!   watcher still running, is not.
//!
//! What this never does: read a file to decide whether it belongs to a work,
//! link files by path, name, hash or time, or claim a gap was recorded when it
//! could not be saved. When storage itself is broken, a gap may not be saved;
//! nothing then says it was.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::capture::{MemoryWatchView, Read, WatchRow, query_watches};
use crate::contracts::{AppError, ErrorCode, RootStatus};
use crate::memory_consent::memory_error;
use crate::memory_contracts::{GapKind, SnapshotRef};
use crate::observe;
use crate::snapshot::{check_file_id, unknown_file};
use crate::store::{Store, sqlite_error};
use crate::works::{GROUP, merged_family_ids_in};
use crate::{OrganizeService, is_canonical_uuid};

/// Each record task keeps at most this many unprocessed candidate objects,
/// and this many unprocessed candidate snapshots; reaching either pauses its
/// candidate capture (#31 material, supplement).
pub const CANDIDATE_LIMIT: i64 = 20;

/// While the watcher runs, its run's last sign of life is written at least
/// this often (and after every pass that saved a version). A crash can make
/// the gap that follows look this much longer, never shorter.
const RUN_SEEN_EVERY_MS: i64 = 30_000;

// ---------------------------------------------------------------------------
// What the window sees
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordState {
    /// Memory records the chain's place.
    Active,
    /// The object at the place was judged unrelated: nothing is looked at
    /// until the work's new place is chosen (M2), or the record is stopped.
    Lost,
    /// Memory was turned off for this record.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateCapture {
    Off,
    On,
    /// On, but the record task holds 20 unprocessed candidate objects or
    /// snapshots: nothing new is saved until some are decided.
    Paused,
}

/// The one place a record chain looks at: where its watched file's record is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaceView {
    pub root_id: String,
    pub relative_path: String,
    pub display_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PendingCounts {
    pub objects: i64,
    pub snapshots: i64,
    pub limit: i64,
}

/// How a gap ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapEnd {
    /// A version (or candidate) was saved again.
    Captured,
    /// The place looked as recorded again, with nothing new to save.
    Recovered,
    /// The place's situation turned into another gap: a failure with
    /// another problem, or an unread object where nothing was (or the other
    /// way round). The new situation is the next gap.
    Changed,
    /// Memory was turned on again.
    Resumed,
    /// Candidates were decided, so the quota has room again.
    Released,
    /// The user decided about the object at the place.
    Decided,
    /// The record went on as the object at the place.
    Continued,
    /// Memory was turned off for the record.
    Stopped,
    /// (M2) the work's new place was chosen.
    Relocated,
    /// (M2) a decision was corrected.
    Corrected,
    /// (M2) two works were merged.
    Merged,
}

/// One recorded gap of a record chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordedGapView {
    pub gap_id: String,
    pub chain_id: String,
    pub kind: GapKind,
    /// The chain's newest version when the gap began; null before the first.
    pub after: Option<SnapshotRef>,
    /// For `capture_failed`: why the place could not be captured.
    pub problem: Option<ErrorCode>,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<GapEnd>,
}

/// One record task, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryRecordView {
    pub chain_id: String,
    pub family_id: String,
    /// The file Memory was turned on for.
    pub origin_file_id: String,
    /// Whose Memory send permission covers the versions on this record's
    /// line (D4): the file Memory was turned on for, or, after a
    /// continuation on the line was revoked, the file the record's current
    /// part starts at (#31 M2b).
    pub permission_file_id: String,
    pub state: RecordState,
    /// The watch that is on, or the latest one (#32's shape). Its `versions`
    /// counts only that file's versions.
    pub watch: MemoryWatchView,
    /// The versions on the record's line: every file it watched or was
    /// confirmed to go on as (the versions of `usable_pairs_of_line`); after
    /// a revoked continuation, only the current part's.
    pub line_versions: i64,
    pub place: PlaceView,
    /// False once the place's folder was removed from Traeva: then nothing
    /// is recorded again for this record.
    pub root_active: bool,
    /// The work is lost, or nothing is at its place: its new place has to be
    /// chosen (M2).
    pub waiting_for_relocation: bool,
    pub candidate_capture: CandidateCapture,
    pub pending: PendingCounts,
    pub open_gaps: Vec<RecordedGapView>,
    /// Memory was turned on for a file of no work whose first version has
    /// exactly the bytes of an earlier version in another work: only a hint
    /// that they may be related, never a relation (material §2 source 3).
    pub same_content: Vec<crate::family_merge::SameContentHint>,
    /// Recording was paused by a correction that took away what it rested
    /// on (material §7): turn Memory on again for the record's file to go
    /// on (the same record), or stop it.
    pub hold: Option<RecordHoldView>,
}

/// Why and since when a record is paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordHoldView {
    pub work_decision_id: String,
    /// The continuation the recording rested on.
    pub edge_id: Option<String>,
    pub held_at_ms: i64,
    pub why: HoldReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    /// The user revoked the continuation the record went on by.
    RelationRevoked,
    /// The user moved members so that a relation the record went on by
    /// crosses two works (it is disputed).
    MovedApart,
}

/// One file record of a work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyMemberView {
    pub file_id: String,
    pub display_path: String,
    pub present: bool,
    pub status: RelationStatus,
    pub basis: MemberBasis,
    /// Its place in the work's history is known (not only "same work,
    /// order unknown").
    pub placed: bool,
    pub decided_at_ms: Option<i64>,
    pub snapshots: Vec<SnapshotRef>,
    /// The work this membership was written in, when that work was merged
    /// into this one (the membership then rests on the user's merge).
    pub merged_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationStatus {
    ConfirmedSystem,
    ConfirmedUser,
    Candidate,
    Unknown,
    Rejected,
    Disputed,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberBasis {
    MemoryOn,
    FromBefore,
    CandidateCapture,
    UserDecision,
    Relocation,
    Merge,
    Correction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Continuation,
    Branch,
    Unplaced,
}

/// A relation between two real versions (never between names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionRelationView {
    pub edge_id: String,
    pub kind: EdgeKind,
    /// Null: the parent is unknown, and the relation is never used to
    /// compare versions.
    pub parent: Option<SnapshotRef>,
    pub child: SnapshotRef,
    pub status: RelationStatus,
    pub skipped_items: i64,
    pub decided_at_ms: Option<i64>,
    /// Its two files are members of two works (a member was moved, M2b-3b):
    /// the relation is disputed until it is confirmed again in one work or
    /// revoked.
    pub crosses_works: bool,
    /// Traeva made it by itself: the child is a new file an app saved at the
    /// record's exact place (auto_link.rs). The user can revoke it. It says
    /// who made it, so it stays true whatever `status` becomes later.
    pub linked_automatically: bool,
}

/// One work: its file records, record tasks, relations and gaps. Reads no
/// user file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryFamilyView {
    /// The live work (any work ID merged into it opens it).
    pub family_id: String,
    pub created_at_ms: i64,
    /// Works merged into this one, in the order they were merged (#31 M2b).
    pub merged_family_ids: Vec<String>,
    pub members: Vec<FamilyMemberView>,
    pub records: Vec<MemoryRecordView>,
    pub relations: Vec<VersionRelationView>,
    pub gaps: Vec<RecordedGapView>,
    /// Versions whose parent is unknown (a branch whose start was not chosen,
    /// or a file of the work whose order is unknown).
    pub boundaries: Vec<String>,
}

/// Every watch of one file and the gaps around them (#34).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileWatchesView {
    pub file_id: String,
    /// When this run of the watcher began; null when it has not run since
    /// Traeva started. Coverage from then on has no watcher gap.
    pub current_run_started_at_ms: Option<i64>,
    /// Every watch of the file, on or off, oldest first. No limit.
    pub watches: Vec<MemoryWatchView>,
    /// Every recorded gap of the file's record chain, and every stretch the
    /// watcher was not running, that touches one of these watches: oldest
    /// first. A stretch still going on has no end.
    pub gaps: Vec<CoverageGapView>,
}

/// A stretch in which Memory knows it may not have seen every save of a
/// file. It never says the file changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageGapView {
    pub kind: GapKind,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub problem: Option<ErrorCode>,
    /// The recorded gap and its chain; null for watcher stretches, which
    /// are worked out from the runs.
    pub gap_id: Option<String>,
    pub chain_id: Option<String>,
}

/// What one look at a chain's place means for its gaps (capture.rs tells
/// `note_gaps` after every look).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapSignal {
    /// The place is as recorded: the watched file, saved or unchanged.
    Covered { saved: bool },
    /// The place holds an object Memory did not read.
    Replaced,
    /// The place could not be captured. `unlocked`: seen without the
    /// execution lock (it is looked at again under the lock before a gap is
    /// written, so a move Organize is making is not taken for a failure).
    Failed { code: ErrorCode, unlocked: bool },
    /// Nothing to record (a file still being saved, a watch turned off).
    Nothing,
}

impl GapSignal {
    /// A failure with code `code`; a file still being saved is not one.
    pub(crate) fn failed(code: ErrorCode, unlocked: bool) -> GapSignal {
        if code == ErrorCode::FileChanging {
            GapSignal::Nothing
        } else {
            GapSignal::Failed { code, unlocked }
        }
    }
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// This run of the watcher, kept in memory: the row it writes to.
pub(crate) struct RunState {
    run_id: String,
    written_at_ms: i64,
}

impl OrganizeService {
    /// Registers the file at `picked` (from the native picker) in root
    /// `root_id`, the way Organize does, then turns Memory on for it
    /// (`start_memory_for_file`). Nothing is written inside the root.
    pub fn start_memory_at(
        &self,
        root_id: &str,
        picked: &Path,
    ) -> Result<MemoryRecordView, AppError> {
        self.start_memory_at_with(root_id, picked, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn start_memory_at_with(
        &self,
        root_id: &str,
        picked: &Path,
        read: Read,
    ) -> Result<MemoryRecordView, AppError> {
        let observed = self.register_file_with(root_id, picked, read)?;
        self.start_memory_for_file_with(&observed.file.file_id, read)
    }

    /// Turns Memory on for registered file `file_id` and saves its first
    /// version; its record as it is afterwards. A file with no record chain
    /// starts one, in its own work. Turning it on again for a stopped record
    /// goes on with the same chain: the quota is kept, candidate capture
    /// stays off, and the time it was off is a `memory_off` gap. Refused with
    /// `candidate_pending` while the file waits for a decision in a record,
    /// and with `decision_conflict` when its record went on as another file.
    pub fn start_memory_for_file(&self, file_id: &str) -> Result<MemoryRecordView, AppError> {
        self.start_memory_for_file_with(file_id, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn start_memory_for_file_with(
        &self,
        file_id: &str,
        read: Read,
    ) -> Result<MemoryRecordView, AppError> {
        let watch = self.start_memory_with(file_id, read)?;
        let store = self.store()?;
        let chain = store
            .chain_of_watch(&watch.watch_id)?
            .ok_or_else(unknown_record)?;
        self.record_view(store, &chain)
    }

    /// Stops record `chain_id`: its watch is turned off, candidate capture
    /// ends and every open gap ends. Saved versions, candidates and
    /// relations stay. Stopping again changes nothing.
    pub fn stop_memory_record(&self, chain_id: &str) -> Result<MemoryRecordView, AppError> {
        let store = self.store()?;
        check_record_id(chain_id)?;
        let chain = store.chain(chain_id)?.ok_or_else(unknown_record)?;
        // Waits for a capture under way, like `stop_memory`.
        let _no_capture = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        store.stop_chain(chain_id, self.now())?;
        // As in `stop_memory`: the watcher was running until now.
        self.note_run_seen(true);
        self.record_view(store, &chain)
    }

    /// Every record: the ones on first, then the lost ones, then the stopped
    /// ones, latest first. No limit.
    pub fn memory_records(&self) -> Result<Vec<MemoryRecordView>, AppError> {
        let store = self.store()?;
        let mut records = store
            .chains("ORDER BY created_at_ms DESC, rowid DESC", [])?
            .iter()
            .map(|chain| self.record_view(store, chain))
            .collect::<Result<Vec<_>, _>>()?;
        let rank = |state: RecordState| match state {
            RecordState::Active => 0,
            RecordState::Lost => 1,
            RecordState::Stopped => 2,
        };
        // Stable: within a state, the order above (latest first) stays.
        records.sort_by_key(|record| rank(record.state));
        Ok(records)
    }

    /// One record.
    pub fn memory_record(&self, chain_id: &str) -> Result<MemoryRecordView, AppError> {
        let store = self.store()?;
        check_record_id(chain_id)?;
        let chain = store.chain(chain_id)?.ok_or_else(unknown_record)?;
        self.record_view(store, &chain)
    }

    /// One work: its members (candidates and rejected files too), relations,
    /// records, gaps and unknown-parent boundaries. Reads no user file.
    pub fn memory_family(&self, family_id: &str) -> Result<MemoryFamilyView, AppError> {
        let store = self.store()?;
        if !is_canonical_uuid(family_id) {
            return Err(memory_error(
                ErrorCode::InvalidInput,
                "The work ID is not valid.",
                false,
                None,
            ));
        }
        let family_id = store
            .live_family(family_id)?
            .ok_or_else(crate::works::unknown_family)?;
        let family_id = family_id.as_str();
        let created_at_ms = store
            .family_created_at(family_id)?
            .ok_or_else(crate::works::unknown_family)?;
        let merged_family_ids = {
            let conn = store.connection()?;
            merged_family_ids_in(&conn, family_id)?
        };
        let facts = store.family_facts(family_id)?;
        let placed = crate::lineage::placed_files(&facts);
        let mut members = Vec::new();
        for member in &facts.members {
            let (file, present) = store.get_file(&member.file_id)?.ok_or_else(unknown_file)?;
            let root = store.get_root(&file.root_id)?;
            members.push(FamilyMemberView {
                file_id: member.file_id.clone(),
                display_path: shown_path(
                    root.as_ref().map(|r| r.display_path.as_str()),
                    &file.relative_path,
                ),
                present,
                status: member.status,
                basis: member.basis,
                placed: placed.contains(&member.file_id),
                decided_at_ms: member.decided_at_ms,
                snapshots: facts.snapshots_of(&member.file_id).cloned().collect(),
                merged_from: (member.family_id != family_id).then(|| member.family_id.clone()),
            });
        }
        // A record is listed in the work of the file it watches now; a
        // line part elsewhere still counts here for pairs and parts.
        let mut records = Vec::new();
        for chain in &facts.chains {
            let work = {
                let conn = store.connection()?;
                crate::works::live_family_of_chain_in(&conn, &chain.chain_id)?
            };
            if work.as_deref() == Some(family_id) {
                records.push(self.record_view(store, chain)?);
            }
        }
        // A relation whose other end is in another work (it was disputed
        // when a member moved) is shown too, with that version.
        let snapshot_of = |id: &str| -> Result<Option<SnapshotRef>, AppError> {
            match facts.snapshot(id) {
                Some(snapshot) => Ok(Some(snapshot.clone())),
                None => store.get_snapshot(id),
            }
        };
        let mut relations = Vec::new();
        let mut boundaries = Vec::new();
        for edge in &facts.edges {
            let Some(child) = snapshot_of(&edge.child_snapshot_id)? else {
                continue;
            };
            if edge.parent_snapshot_id.is_none() {
                boundaries.push(child.snapshot_id.clone());
            }
            let parent = match edge.parent_snapshot_id.as_deref() {
                Some(id) => snapshot_of(id)?,
                None => None,
            };
            let crosses_works = match &parent {
                Some(parent) => {
                    store.family_of_file(&parent.file_id)?
                        != store.family_of_file(&child.file_id)?
                }
                None => false,
            };
            relations.push(VersionRelationView {
                edge_id: edge.edge_id.clone(),
                kind: edge.kind,
                parent,
                child,
                status: edge.status,
                skipped_items: edge.skipped_items,
                decided_at_ms: edge.decided_at_ms,
                crosses_works,
                linked_automatically: edge.auto_linked,
            });
        }
        let gaps = facts
            .gaps
            .iter()
            .map(|gap| gap_view(store, gap))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MemoryFamilyView {
            family_id: family_id.to_string(),
            created_at_ms,
            merged_family_ids,
            members,
            records,
            relations,
            gaps,
            boundaries,
        })
    }

    /// Every watch of file `file_id`, on or off, oldest first, with no limit
    /// (`memory_watches` stops at 200 of all files), and the gaps around
    /// them: the recorded gaps of its record chain and the stretches the
    /// watcher was not running, including any after its latest version.
    pub fn memory_watches_of(&self, file_id: &str) -> Result<FileWatchesView, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        let rows = {
            let conn = store.connection()?;
            query_watches(
                &conn,
                "WHERE file_id = ?1 ORDER BY started_at_ms, rowid",
                [file_id],
            )?
        };
        let periods: Vec<(i64, Option<i64>)> = rows
            .iter()
            .map(|watch| (watch.started_at_ms, watch.stopped_at_ms))
            .collect();
        // A recorded gap touching a watch at an end counts (Memory off lies
        // between two watches); a watcher stretch must overlap one: when the
        // last sign of life is the moment Memory was turned off, the
        // stretch after it is not this watch's.
        let touches = |start: i64, end: Option<i64>| {
            periods.iter().any(|(from, to)| {
                start <= to.unwrap_or(i64::MAX) && *from <= end.unwrap_or(i64::MAX)
            })
        };
        let overlaps = |start: i64, end: Option<i64>| {
            periods
                .iter()
                .any(|(from, to)| start < to.unwrap_or(i64::MAX) && *from < end.unwrap_or(i64::MAX))
        };
        let mut gaps = Vec::new();
        if let Some(chain_id) = store.chain_of_file(file_id)? {
            for gap in store.gaps_of_chain(&chain_id)? {
                if touches(gap.started_at_ms, gap.ended_at_ms) {
                    gaps.push(CoverageGapView {
                        kind: gap.kind,
                        started_at_ms: gap.started_at_ms,
                        ended_at_ms: gap.ended_at_ms,
                        problem: gap.problem,
                        gap_id: Some(gap.gap_id),
                        chain_id: Some(gap.chain_id),
                    });
                }
            }
        }
        let runs = self.run_history(store)?;
        if let Some(first_watch) = periods.first() {
            let not_recorded_until = runs.first_started();
            if not_recorded_until.is_none_or(|until| first_watch.0 < until) {
                gaps.push(CoverageGapView {
                    kind: GapKind::NotRecorded,
                    started_at_ms: first_watch.0,
                    ended_at_ms: not_recorded_until,
                    problem: None,
                    gap_id: None,
                    chain_id: None,
                });
            }
        }
        for (start, end) in runs.downtimes() {
            if overlaps(start, end) {
                gaps.push(CoverageGapView {
                    kind: GapKind::WatcherStopped,
                    started_at_ms: start,
                    ended_at_ms: end,
                    problem: None,
                    gap_id: None,
                    chain_id: None,
                });
            }
        }
        gaps.sort_by_key(|gap| (gap.started_at_ms, gap.ended_at_ms.unwrap_or(i64::MAX)));
        let watches = rows
            .iter()
            .map(|watch| self.watch_view(store, watch))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FileWatchesView {
            file_id: file_id.to_string(),
            current_run_started_at_ms: runs.current_started(),
            watches,
            gaps,
        })
    }

    /// Records that the watcher runs in this start of Traeva, once: called
    /// before anything Memory captures, so every version saved in this run
    /// is saved after the run began. If it cannot be written now, the next
    /// pass tries again (until then, this run counts as not running).
    pub(crate) fn ensure_run(&self) {
        let mut run = self
            .memory_run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if run.is_some() {
            return;
        }
        let Ok(store) = self.store() else { return };
        let now = self.now();
        let run_id = uuid::Uuid::new_v4().to_string();
        if store.insert_run(&run_id, now).is_ok() {
            *run = Some(RunState {
                run_id,
                written_at_ms: now,
            });
        }
    }

    /// After a pass: keeps this run's last sign of life recorded, at least
    /// every `RUN_SEEN_EVERY_MS` and after every pass that saved a version.
    pub(crate) fn note_run_seen(&self, saved: bool) {
        let mut run = self
            .memory_run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = run.as_mut() else { return };
        let now = self.now();
        if !saved && now - state.written_at_ms < RUN_SEEN_EVERY_MS {
            return;
        }
        let Ok(store) = self.store() else { return };
        if store.update_run_seen(&state.run_id, now).is_ok() {
            state.written_at_ms = now;
        }
    }

    /// Every run of the watcher, and which one is this start's.
    pub(crate) fn run_history(&self, store: &Store) -> Result<RunHistory, AppError> {
        let current = self
            .memory_run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|run| run.run_id.clone());
        Ok(RunHistory {
            runs: store.runs()?,
            current,
        })
    }

    /// Records what one look at chain `chain_id`'s place found (`signal`),
    /// for its watch `watch_id` on file `file_id`. Writes only when the
    /// situation changed: a gap starts or ends. Each write checks again, in
    /// its transaction, that the watch is still the chain's watch that is on
    /// and the chain is not lost; anything else writes nothing. A write that
    /// fails is not reported: the next pass reads the open gaps from the
    /// database again and tries once more (a failure that ended before
    /// storage worked again is not recorded; contract §5).
    pub(crate) fn note_gaps(
        &self,
        watch_id: &str,
        chain_id: &str,
        file_id: &str,
        signal: GapSignal,
    ) {
        let Ok(store) = self.store() else { return };
        let Ok(open) = store.open_gaps(chain_id) else {
            return;
        };
        let is_open = |kind: GapKind| open.iter().any(|gap| gap.kind == kind);
        let now = self.now();
        match signal {
            GapSignal::Covered { saved } => {
                if is_open(GapKind::CaptureFailed) || is_open(GapKind::ReplacedUncaptured) {
                    let end = if saved {
                        GapEnd::Captured
                    } else {
                        GapEnd::Recovered
                    };
                    let _ = store.end_gaps_while_watching(
                        chain_id,
                        watch_id,
                        &[GapKind::CaptureFailed, GapKind::ReplacedUncaptured],
                        end,
                        now,
                    );
                }
            }
            GapSignal::Replaced => {
                if !is_open(GapKind::ReplacedUncaptured) {
                    let _ = store.open_gap_while_watching(
                        chain_id,
                        watch_id,
                        GapKind::ReplacedUncaptured,
                        None,
                        now,
                    );
                }
            }
            GapSignal::Failed { code, unlocked } => {
                let same = open
                    .iter()
                    .any(|gap| gap.kind == GapKind::CaptureFailed && gap.problem == Some(code));
                if same {
                    return;
                }
                if unlocked {
                    // Organize holds this lock for a whole move: under it, a
                    // file that was only on its way looks as recorded.
                    let _no_run = self
                        .execution
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if self.place_failure(file_id) != Some(code) {
                        return;
                    }
                }
                let _ = store.open_gap_while_watching(
                    chain_id,
                    watch_id,
                    GapKind::CaptureFailed,
                    Some(code),
                    now,
                );
            }
            GapSignal::Nothing => {}
        }
    }

    /// Why file `file_id`'s place cannot be captured now, as an error code;
    /// None when it can (or is only still being saved).
    fn place_failure(&self, file_id: &str) -> Option<ErrorCode> {
        match self.look_at_place(file_id) {
            Err(error) if error.code == ErrorCode::FileChanging => None,
            Err(error) => Some(error.code),
            Ok((_, crate::capture::Look::Empty)) => Some(ErrorCode::FileNotFound),
            Ok(_) => None,
        }
    }

    /// What a lost record reports at every pass: nothing is looked at.
    pub(crate) fn lost(&self, file_id: &str) -> AppError {
        let mut error = AppError::observation(
            ErrorCode::FileNotFound,
            "The object at this file's place was judged unrelated, so Memory records nothing \
             there. Choose the file's new place, or stop this record.",
        );
        error.file_id = Some(file_id.to_string());
        error
    }

    fn record_view(&self, store: &Store, chain: &ChainRow) -> Result<MemoryRecordView, AppError> {
        let watch = store
            .current_watch_of_chain(&chain.chain_id)?
            .ok_or_else(unknown_record)?;
        let gaps = store.open_gaps(&chain.chain_id)?;
        let lost = gaps.iter().any(|gap| gap.kind == GapKind::Lost);
        let state = match (watch.active, lost) {
            (true, false) => RecordState::Active,
            (true, true) => RecordState::Lost,
            (false, _) => RecordState::Stopped,
        };
        let (file, _) = store.get_file(&watch.file_id)?.ok_or_else(unknown_file)?;
        let root = store.get_root(&file.root_id)?;
        let root_active = root
            .as_ref()
            .is_some_and(|root| root.status == RootStatus::Active);
        let place = PlaceView {
            display_path: shown_path(
                root.as_ref().map(|r| r.display_path.as_str()),
                &file.relative_path,
            ),
            root_id: file.root_id.clone(),
            relative_path: file.relative_path.clone(),
        };
        let pending = store.pending_counts(&chain.chain_id)?;
        let (family_id, same_content, permission_file_id, line_versions, hold) = {
            let conn = store.connection()?;
            let family_id = crate::works::live_family_of_chain_in(&conn, &chain.chain_id)?
                .ok_or_else(bad_row)?;
            let hints = crate::family_merge::same_content_in(&conn, &chain.chain_id, &family_id)?;
            let (first, versions) = current_part_in(&conn, &chain.chain_id, &watch.file_id)?;
            let hold = open_hold_in(&conn, &chain.chain_id)?;
            (family_id, hints, first, versions, hold)
        };
        let full = pending.objects >= CANDIDATE_LIMIT || pending.snapshots >= CANDIDATE_LIMIT;
        let candidate_capture = match store.open_switch(&chain.chain_id)? {
            Some(_) if state == RecordState::Active && root_active && full => {
                CandidateCapture::Paused
            }
            Some(_) if state == RecordState::Active && root_active => CandidateCapture::On,
            _ => CandidateCapture::Off,
        };
        let waiting_for_relocation = lost
            || gaps.iter().any(|gap| {
                gap.kind == GapKind::CaptureFailed && gap.problem == Some(ErrorCode::FileNotFound)
            });
        let open_gaps = gaps
            .iter()
            .map(|gap| gap_view(store, gap))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MemoryRecordView {
            chain_id: chain.chain_id.clone(),
            family_id,
            origin_file_id: chain.origin_file_id.clone(),
            permission_file_id,
            state,
            watch: self.watch_view(store, &watch)?,
            line_versions,
            place,
            root_active,
            waiting_for_relocation,
            candidate_capture,
            pending,
            open_gaps,
            same_content,
            hold,
        })
    }
}

/// The part of record chain `chain_id`'s line that holds file `file_id`
/// (its latest watch's): its first file and how many versions it has.
pub(crate) fn current_part_in(
    conn: &Connection,
    chain_id: &str,
    file_id: &str,
) -> Result<(String, i64), AppError> {
    let line = crate::lineage::chain_line_in(conn, chain_id)?;
    let parts = crate::lineage::line_parts(&line, &std::collections::HashSet::new());
    let part = crate::lineage::part_of(&parts, file_id).ok_or_else(bad_row)?;
    let mut versions = 0;
    for file in &part.file_ids {
        versions += conn
            .query_row(
                "SELECT count(*) FROM snapshots WHERE file_id = ?1",
                [file],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|err| sqlite_error("Cannot read the Memory records", &err))?;
    }
    Ok((part.first_file_id.clone(), versions))
}

/// Record chain `chain_id`'s open hold, if it is paused.
pub(crate) fn open_hold_in(
    conn: &Connection,
    chain_id: &str,
) -> Result<Option<RecordHoldView>, AppError> {
    conn.query_row(
        "SELECT h.work_decision_id, h.edge_id, h.held_at_ms, d.kind = 'move'
         FROM memory_record_holds AS h
         JOIN memory_work_decisions AS d ON d.work_decision_id = h.work_decision_id
         WHERE h.chain_id = ?1 AND h.released_at_ms IS NULL",
        [chain_id],
        |r| {
            Ok(RecordHoldView {
                work_decision_id: r.get(0)?,
                edge_id: r.get(1)?,
                held_at_ms: r.get(2)?,
                why: if r.get(3)? {
                    HoldReason::MovedApart
                } else {
                    HoldReason::RelationRevoked
                },
            })
        },
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// Releases record chain `chain_id`'s open hold, if any: Memory was turned
/// on again for it (`resumed`) or it was stopped (`stopped`).
pub(crate) fn release_hold_in(
    conn: &Connection,
    chain_id: &str,
    reason: &str,
    now: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE memory_record_holds SET released_at_ms = max(held_at_ms, ?2),
             release_reason = ?3
         WHERE chain_id = ?1 AND released_at_ms IS NULL",
        params![chain_id, now, reason],
    )
    .map_err(|err| sqlite_error("Cannot save the Memory record", &err))?;
    Ok(())
}

/// `<folder>/<relative path>`, as the user sees it.
fn shown_path(root_display_path: Option<&str>, relative_path: &str) -> String {
    match root_display_path {
        Some(root) => format!("{root}/{relative_path}"),
        None => relative_path.to_string(),
    }
}

fn gap_view(store: &Store, gap: &GapRow) -> Result<RecordedGapView, AppError> {
    let after = match &gap.after_snapshot_id {
        Some(id) => store.get_snapshot(id)?,
        None => None,
    };
    Ok(RecordedGapView {
        gap_id: gap.gap_id.clone(),
        chain_id: gap.chain_id.clone(),
        kind: gap.kind,
        after,
        problem: gap.problem,
        started_at_ms: gap.started_at_ms,
        ended_at_ms: gap.ended_at_ms,
        end_reason: gap.end_reason,
    })
}

/// The runs of the watcher, oldest first.
pub(crate) struct RunHistory {
    /// (run ID, started, last seen).
    runs: Vec<(String, i64, i64)>,
    /// This start's run, when the watcher has run since Traeva started.
    current: Option<String>,
}

impl RunHistory {
    /// Every stretch the watcher was not running, as (start, end): from one
    /// run's last sign of life to the next run's start, and, when the
    /// watcher has not run since this start of Traeva, from the last run on
    /// (no end). Runs are in the order they were recorded; when the clock
    /// was set back between two runs, the stretch between the two times is
    /// still kept (too long rather than lost).
    pub(crate) fn downtimes(&self) -> Vec<(i64, Option<i64>)> {
        let mut stretches: Vec<(i64, Option<i64>)> = self
            .runs
            .windows(2)
            .filter(|pair| pair[1].1 != pair[0].2)
            .map(|pair| {
                let (seen, next) = (pair[0].2, pair[1].1);
                (seen.min(next), Some(seen.max(next)))
            })
            .collect();
        if let Some(last) = self.runs.last()
            && self.current.as_deref() != Some(last.0.as_str())
        {
            stretches.push((last.2, None));
        }
        stretches
    }

    /// When the first run began: before it, when the watcher ran was not
    /// recorded. None when it never ran.
    pub(crate) fn first_started(&self) -> Option<i64> {
        self.runs.iter().map(|run| run.1).min()
    }

    fn current_started(&self) -> Option<i64> {
        let current = self.current.as_deref()?;
        self.runs
            .iter()
            .find(|run| run.0 == current)
            .map(|run| run.1)
    }

    /// The watcher gaps between two versions saved at `from` and `to`.
    pub(crate) fn kinds_between(&self, from: i64, to: i64) -> Vec<GapKind> {
        let mut kinds = Vec::new();
        let stopped = self
            .downtimes()
            .iter()
            .any(|(start, end)| *start < to && end.is_none_or(|end| end > from));
        if stopped {
            kinds.push(GapKind::WatcherStopped);
        }
        if self.first_started().is_none_or(|first| from < first) {
            kinds.push(GapKind::NotRecorded);
        }
        kinds
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn check_record_id(chain_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(chain_id) {
        Ok(())
    } else {
        Err(memory_error(
            ErrorCode::InvalidInput,
            "The Memory record ID is not valid.",
            false,
            None,
        ))
    }
}

pub(crate) fn unknown_record() -> AppError {
    memory_error(
        ErrorCode::UnknownRecord,
        "There is no Memory record with this ID.",
        false,
        None,
    )
}

pub(crate) fn candidate_pending(file_id: &str) -> AppError {
    memory_error(
        ErrorCode::CandidatePending,
        "This file waits for a decision in a Memory record (it was saved there as a \
         candidate), or it is at the place another record looks at. Decide about it in that \
         record, or stop that record first.",
        false,
        Some(file_id),
    )
}

fn went_on_elsewhere(file_id: &str) -> AppError {
    memory_error(
        ErrorCode::DecisionConflict,
        "This file's Memory record already went on as another file. Turn Memory on there, \
         or stop that record first.",
        false,
        Some(file_id),
    )
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

/// One row of `memory_chains`, and whether the chain is lost now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChainRow {
    pub(crate) chain_id: String,
    pub(crate) family_id: String,
    pub(crate) origin_file_id: String,
    pub(crate) lost: bool,
}

/// One row of `memory_gaps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GapRow {
    pub(crate) gap_id: String,
    pub(crate) chain_id: String,
    pub(crate) kind: GapKind,
    pub(crate) after_snapshot_id: Option<String>,
    pub(crate) problem: Option<ErrorCode>,
    pub(crate) started_at_ms: i64,
    pub(crate) ended_at_ms: Option<i64>,
    pub(crate) end_reason: Option<GapEnd>,
}

/// A value of an enum as the snake_case text the database keeps.
pub(crate) fn to_text<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The enum value of a snake_case text the database keeps.
pub(crate) fn from_text<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(text.to_string())).ok()
}

fn bad_row() -> AppError {
    AppError::storage(
        ErrorCode::DatabaseCorrupt,
        "A saved Memory record could not be read. Traeva kept it unchanged.",
    )
}

impl Store {
    pub(crate) fn chain(&self, chain_id: &str) -> Result<Option<ChainRow>, AppError> {
        Ok(self.chains("WHERE chain_id = ?1", [chain_id])?.pop())
    }

    /// The chain watch `watch_id` belongs to (none for a watch written
    /// directly by a test).
    pub(crate) fn chain_of_watch(&self, watch_id: &str) -> Result<Option<ChainRow>, AppError> {
        Ok(self
            .chains(
                "WHERE chain_id = (SELECT chain_id FROM memory_chain_watches WHERE watch_id = ?1)",
                [watch_id],
            )?
            .pop())
    }

    /// The chain whose watches file `file_id` has.
    pub(crate) fn chain_of_file(&self, file_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        chain_of_file_in(&conn, file_id)
    }

    /// `filter` is always a constant written in this file; values go in `params`.
    pub(crate) fn chains<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> Result<Vec<ChainRow>, AppError> {
        let conn = self.connection()?;
        query_chains(&conn, filter, params)
    }

    pub(crate) fn family_created_at(&self, family_id: &str) -> Result<Option<i64>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT created_at_ms FROM memory_families WHERE family_id = ?1",
            [family_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
    }

    /// The chain's latest watch: the one no later watch goes on from (the
    /// one that is on, if any). Found by the links, not by times, so a clock
    /// set back never makes an older watch look latest.
    pub(crate) fn current_watch_of_chain(
        &self,
        chain_id: &str,
    ) -> Result<Option<WatchRow>, AppError> {
        let conn = self.connection()?;
        latest_watch_of_chain_in(&conn, chain_id)
    }

    /// How many versions the chain's line holds: the snapshots of the files
    /// it watched and of the candidates confirmed to continue it.
    pub(crate) fn open_gaps(&self, chain_id: &str) -> Result<Vec<GapRow>, AppError> {
        let conn = self.connection()?;
        query_gaps(
            &conn,
            "WHERE chain_id = ?1 AND ended_at_ms IS NULL ORDER BY started_at_ms, rowid",
            [chain_id],
        )
    }

    pub(crate) fn gaps_of_chain(&self, chain_id: &str) -> Result<Vec<GapRow>, AppError> {
        let conn = self.connection()?;
        query_gaps(
            &conn,
            "WHERE chain_id = ?1 ORDER BY started_at_ms, rowid",
            [chain_id],
        )
    }

    /// Unprocessed candidates of the chain: objects, and their snapshots.
    pub(crate) fn pending_counts(&self, chain_id: &str) -> Result<PendingCounts, AppError> {
        let conn = self.connection()?;
        pending_counts_in(&conn, chain_id)
    }

    /// The chain's candidate switch period that is on.
    pub(crate) fn open_switch(&self, chain_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT switch_id FROM memory_candidate_switches
             WHERE chain_id = ?1 AND disabled_at_ms IS NULL",
            [chain_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
    }

    /// The file's active watch, or a new one joined to its record chain, in
    /// one transaction (so a file never gets two, and a chain never has two
    /// on):
    /// - a file that waits for a decision in a record is refused;
    /// - a file whose chain was stopped goes on with that chain (restarted,
    ///   with the time it was off as a `memory_off` gap);
    /// - a file whose chain went on as another file is refused;
    /// - a file with no chain starts one, in the work it is a confirmed
    ///   member of, or in a new work of its own.
    pub(crate) fn start_watch_in_chain(
        &self,
        file_id: &str,
        now: i64,
    ) -> Result<WatchRow, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot turn Memory on", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if let Some(active) =
            query_watches(&tx, "WHERE file_id = ?1 AND status = 'active'", [file_id])?.pop()
        {
            return Ok(active);
        }
        let waits: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM memory_candidates
                                WHERE file_id = ?1 AND state = 'pending')
                     OR EXISTS (
                        SELECT 1 FROM memory_chain_watches AS link
                        JOIN memory_watches AS w ON w.watch_id = link.watch_id
                                                AND w.status = 'active'
                        JOIN files AS watched ON watched.file_id = w.file_id
                        JOIN files AS this ON this.file_id = ?1
                        WHERE watched.file_id <> this.file_id
                          AND watched.root_id = this.root_id
                          AND watched.relative_path = this.relative_path
                          AND NOT EXISTS (SELECT 1 FROM memory_gaps AS g
                                          WHERE g.chain_id = link.chain_id AND g.kind = 'lost'
                                            AND g.ended_at_ms IS NULL))",
                [file_id],
                |r| r.get(0),
            )
            .map_err(fail)?;
        if waits {
            return Err(candidate_pending(file_id));
        }
        let watch = WatchRow {
            watch_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            active: true,
            started_at_ms: now,
            stopped_at_ms: None,
        };
        let insert_watch = |tx: &Connection| {
            tx.execute(
                "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
                 VALUES (?1, ?2, 'active', ?3)",
                params![watch.watch_id, watch.file_id, watch.started_at_ms],
            )
            .map_err(fail)
        };
        let chain = match chain_of_file_in(&tx, file_id)? {
            Some(chain_id) => Some((chain_id, None)),
            None => match continued_front_in(&tx, file_id)? {
                Some((chain, decision)) => Some((chain, Some(decision))),
                // On a record's line, but not its newest object: that record
                // went on as a later file.
                None if continued_anywhere_in(&tx, file_id)? => {
                    return Err(went_on_elsewhere(file_id));
                }
                None => None,
            },
        };
        match chain {
            Some((chain_id, continued_by)) => {
                let latest = latest_watch_of_chain_in(&tx, &chain_id)?.ok_or_else(bad_row)?;
                // On elsewhere, or its line went on as another file. A file
                // the record was confirmed to go on as (only the history, as
                // Memory was off then) is the front of the line: the record
                // goes on there.
                // The watched file itself is not the front once its last
                // version went on as another file (only the history was
                // confirmed while its watch stayed).
                let went_on = match latest_snapshot_in(&tx, file_id)? {
                    Some(last) => goes_on_in(&tx, &last)?,
                    None => false,
                };
                let front = match &continued_by {
                    None => latest.file_id == file_id && !went_on,
                    Some((_, decided_at)) => latest.started_at_ms <= *decided_at,
                };
                if latest.active || !front {
                    return Err(went_on_elsewhere(file_id));
                }
                // Turning Memory on again is the user's answer to a pause
                // after a correction: the hold ends first (0008 refuses a
                // new watch on a held record).
                release_hold_in(&tx, &chain_id, "resumed", now)?;
                insert_watch(&tx)?;
                let (reason, decision) = match &continued_by {
                    None => ("restarted", None),
                    Some((decision_id, _)) => ("continued", Some(decision_id.clone())),
                };
                tx.execute(
                    "INSERT INTO memory_chain_watches (watch_id, chain_id, reason,
                         previous_watch_id, decision_id, linked_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        watch.watch_id,
                        chain_id,
                        reason,
                        latest.watch_id,
                        decision,
                        now
                    ],
                )
                .map_err(fail)?;
                let tail = chain_tail_in(&tx, &chain_id)?;
                let off_since = latest.stopped_at_ms.unwrap_or(now).min(now);
                insert_gap_in(
                    &tx,
                    &chain_id,
                    GapKind::MemoryOff,
                    tail.as_deref(),
                    None,
                    off_since,
                    Some((now, GapEnd::Resumed)),
                )?;
            }
            None => {
                let family: Option<String> = tx
                    .query_row(
                        "SELECT l.live_family_id FROM memory_members AS m
                         JOIN memory_live_families AS l ON l.family_id = m.family_id
                         WHERE m.file_id = ?1
                           AND m.status IN ('confirmed_system', 'confirmed_user', 'disputed')",
                        [file_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(fail)?;
                let family_id = match family {
                    Some(family_id) => family_id,
                    None => {
                        let family_id = uuid::Uuid::new_v4().to_string();
                        tx.execute(
                            "INSERT INTO memory_families (family_id, created_at_ms)
                             VALUES (?1, ?2)",
                            params![family_id, now],
                        )
                        .map_err(fail)?;
                        tx.execute(
                            "INSERT INTO memory_members (member_id, family_id, file_id, status,
                                 basis, created_at_ms, updated_at_ms)
                             VALUES (?1, ?2, ?3, 'confirmed_system', 'memory_on', ?4, ?4)",
                            params![uuid::Uuid::new_v4().to_string(), family_id, file_id, now],
                        )
                        .map_err(fail)?;
                        family_id
                    }
                };
                let chain_id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO memory_chains (chain_id, family_id, origin_file_id,
                         tail_snapshot_id, created_at_ms)
                     VALUES (?1, ?2, ?3,
                             (SELECT snapshot_id FROM snapshots WHERE file_id = ?3
                              ORDER BY rowid DESC LIMIT 1),
                             ?4)",
                    params![chain_id, family_id, file_id, now],
                )
                .map_err(fail)?;
                insert_watch(&tx)?;
                tx.execute(
                    "INSERT INTO memory_chain_watches (watch_id, chain_id, reason, linked_at_ms)
                     VALUES (?1, ?2, 'started', ?3)",
                    params![watch.watch_id, chain_id, now],
                )
                .map_err(fail)?;
            }
        }
        tx.commit().map_err(fail)?;
        Ok(watch)
    }

    /// Turns the file's active watch off, and with it its record chain; or,
    /// when none is on, returns the latest watch unchanged (none when there
    /// never was one). A chain is only stopped when a watch of this file was
    /// really on: after the chain went on as another file, the old file's
    /// watch is already off and that chain keeps recording.
    pub(crate) fn stop_file_watch(
        &self,
        file_id: &str,
        now: i64,
    ) -> Result<Option<WatchRow>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot turn Memory off", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if let Some(active) =
            query_watches(&tx, "WHERE file_id = ?1 AND status = 'active'", [file_id])?.pop()
        {
            tx.execute(
                "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = ?2
                 WHERE watch_id = ?1 AND status = 'active'",
                params![active.watch_id, now],
            )
            .map_err(fail)?;
            let chain: Option<String> = tx
                .query_row(
                    "SELECT chain_id FROM memory_chain_watches WHERE watch_id = ?1",
                    [&active.watch_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(fail)?;
            if let Some(chain_id) = chain {
                stop_effects_in(&tx, &chain_id, now)?;
            }
        }
        let latest = query_watches(
            &tx,
            "WHERE file_id = ?1 ORDER BY started_at_ms DESC, rowid DESC LIMIT 1",
            [file_id],
        )?
        .pop();
        tx.commit().map_err(fail)?;
        Ok(latest)
    }

    /// Stops chain `chain_id`: its watch that is on, its candidate switch and
    /// every open gap. Stopping a stopped chain changes nothing.
    pub(crate) fn stop_chain(&self, chain_id: &str, now: i64) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot turn Memory off", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        stop_chain_in(&tx, chain_id, now)?;
        tx.commit().map_err(fail)
    }

    /// Starts a gap of kind `kind` on chain `chain_id`, after its newest
    /// version, if watch `watch_id` is still its watch that is on, the chain
    /// is not lost, and no such gap is open. `capture_failed` and
    /// `replaced_uncaptured` say what is at the place now, so at most one of
    /// them is open: the other one, or a failure with another problem, ends
    /// first (`changed`).
    pub(crate) fn open_gap_while_watching(
        &self,
        chain_id: &str,
        watch_id: &str,
        kind: GapKind,
        problem: Option<ErrorCode>,
        now: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save a Memory gap", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if !watching_in(&tx, chain_id, watch_id)? {
            return Ok(());
        }
        let open = query_gaps(
            &tx,
            "WHERE chain_id = ?1 AND kind = ?2 AND ended_at_ms IS NULL",
            params![chain_id, to_text(kind)],
        )?;
        if open.first().is_some_and(|gap| gap.problem == problem) {
            return Ok(());
        }
        let place_kinds = [GapKind::CaptureFailed, GapKind::ReplacedUncaptured];
        if place_kinds.contains(&kind) {
            end_gaps_in(&tx, chain_id, &place_kinds, GapEnd::Changed, now)?;
        }
        let tail = chain_tail_in(&tx, chain_id)?;
        insert_gap_in(&tx, chain_id, kind, tail.as_deref(), problem, now, None)?;
        tx.commit().map_err(fail)
    }

    /// Ends the open gaps of these kinds on chain `chain_id`, if watch
    /// `watch_id` is still its watch that is on and the chain is not lost.
    pub(crate) fn end_gaps_while_watching(
        &self,
        chain_id: &str,
        watch_id: &str,
        kinds: &[GapKind],
        end: GapEnd,
        now: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save a Memory gap", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if !watching_in(&tx, chain_id, watch_id)? {
            return Ok(());
        }
        end_gaps_in(&tx, chain_id, kinds, end, now)?;
        tx.commit().map_err(fail)
    }

    fn insert_run(&self, run_id: &str, now: i64) -> Result<(), AppError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO memory_runs (run_id, started_at_ms, last_seen_at_ms) VALUES (?1, ?2, ?2)",
            params![run_id, now],
        )
        .map(|_| ())
        .map_err(|err| sqlite_error("Cannot save the watcher's run", &err))
    }

    fn update_run_seen(&self, run_id: &str, now: i64) -> Result<(), AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE memory_runs SET last_seen_at_ms = max(last_seen_at_ms, ?2) WHERE run_id = ?1",
            params![run_id, now],
        )
        .map(|_| ())
        .map_err(|err| sqlite_error("Cannot save the watcher's run", &err))
    }

    fn runs(&self) -> Result<Vec<(String, i64, i64)>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the watcher's runs", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT run_id, started_at_ms, last_seen_at_ms FROM memory_runs ORDER BY rowid",
            )
            .map_err(fail)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(fail)?;
        rows.collect::<Result<_, _>>().map_err(fail)
    }
}

/// See `Store::current_watch_of_chain`.
pub(crate) fn latest_watch_of_chain_in(
    conn: &Connection,
    chain_id: &str,
) -> Result<Option<WatchRow>, AppError> {
    Ok(query_watches(
        conn,
        "WHERE watch_id IN (SELECT watch_id FROM memory_chain_watches WHERE chain_id = ?1)
               AND watch_id NOT IN (SELECT previous_watch_id FROM memory_chain_watches
                                    WHERE previous_watch_id IS NOT NULL)
         ORDER BY status = 'active' DESC, rowid DESC LIMIT 1",
        [chain_id],
    )?
    .pop())
}

/// Watch `watch_id` is chain `chain_id`'s watch that is on, and the chain is
/// not lost.
pub(crate) fn watching_in(
    conn: &Connection,
    chain_id: &str,
    watch_id: &str,
) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM memory_chain_watches AS link
             JOIN memory_watches AS w ON w.watch_id = link.watch_id
             WHERE link.watch_id = ?2 AND link.chain_id = ?1 AND w.status = 'active')
         AND NOT EXISTS (SELECT 1 FROM memory_gaps
                         WHERE chain_id = ?1 AND kind = 'lost' AND ended_at_ms IS NULL)",
        [chain_id, watch_id],
        |r| r.get(0),
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// Stops chain `chain_id` on any connection (see `Store::stop_chain`).
pub(crate) fn stop_chain_in(conn: &Connection, chain_id: &str, now: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = ?2
         WHERE status = 'active'
           AND watch_id IN (SELECT watch_id FROM memory_chain_watches WHERE chain_id = ?1)",
        params![chain_id, now],
    )
    .map_err(|err| sqlite_error("Cannot turn Memory off", &err))?;
    release_hold_in(conn, chain_id, "stopped", now)?;
    stop_effects_in(conn, chain_id, now)
}

/// What stopping a chain does besides turning its watch off: candidate
/// capture ends, and every open gap ends.
fn stop_effects_in(conn: &Connection, chain_id: &str, now: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE memory_candidate_switches SET disabled_at_ms = ?2, disabled_reason = 'stopped'
         WHERE chain_id = ?1 AND disabled_at_ms IS NULL",
        params![chain_id, now],
    )
    .map_err(|err| sqlite_error("Cannot turn Memory off", &err))?;
    conn.execute(
        "UPDATE memory_gaps SET ended_at_ms = max(started_at_ms, ?2), end_reason = 'stopped'
         WHERE chain_id = ?1 AND ended_at_ms IS NULL",
        params![chain_id, now],
    )
    .map_err(|err| sqlite_error("Cannot turn Memory off", &err))?;
    Ok(())
}

pub(crate) fn chain_of_file_in(
    conn: &Connection,
    file_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT link.chain_id FROM memory_chain_watches AS link
         JOIN memory_watches AS w ON w.watch_id = link.watch_id
         WHERE w.file_id = ?1 LIMIT 1",
        [file_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// (chain, (its continue decision, when it was decided)).
type ContinuedFront = (String, (String, i64));

/// File `file_id` was confirmed to go on a record's line.
fn continued_anywhere_in(conn: &Connection, file_id: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM memory_candidates AS c WHERE c.file_id = ?1
                        AND c.state = 'continued' AND {STANDS})"
            .replace("{STANDS}", CONTINUATION_STANDS)
            .as_str(),
        [file_id],
        |r| r.get(0),
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// A continued candidate `c` counts while the continuation into its first
/// version stands (it was not revoked).
const CONTINUATION_STANDS: &str = "EXISTS (SELECT 1 FROM memory_edges AS e
    WHERE e.child_snapshot_id = c.first_snapshot_id AND e.kind = 'continuation'
      AND e.status IN ('confirmed_system', 'confirmed_user', 'disputed'))";

/// The chain whose line file `file_id` was confirmed to go on (a continued
/// candidate with no watch), when it is the newest such object of that
/// chain: (chain, (its continue decision, when it was decided)).
fn continued_front_in(
    conn: &Connection,
    file_id: &str,
) -> Result<Option<ContinuedFront>, AppError> {
    conn.query_row(
        "SELECT c.chain_id, d.decision_id, c.decided_at_ms
         FROM memory_candidates AS c
         JOIN memory_decisions AS d ON d.candidate_id = c.candidate_id AND d.choice = 'continue'
         WHERE c.file_id = ?1 AND c.state = 'continued' AND {STANDS}
           AND c.seq = (SELECT max(seq) FROM memory_candidates AS k
                        WHERE k.chain_id = c.chain_id AND k.state = 'continued'
                          AND {STANDS_K})
         LIMIT 1"
            .replace("{STANDS_K}", &CONTINUATION_STANDS.replace("c.", "k."))
            .replace("{STANDS}", CONTINUATION_STANDS)
            .as_str(),
        [file_id],
        |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))),
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

pub(crate) fn chain_tail_in(conn: &Connection, chain_id: &str) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT tail_snapshot_id FROM memory_chains WHERE chain_id = ?1",
        [chain_id],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

pub(crate) fn pending_counts_in(
    conn: &Connection,
    chain_id: &str,
) -> Result<PendingCounts, AppError> {
    conn.query_row(
        "SELECT (SELECT count(*) FROM memory_candidates WHERE chain_id = ?1 AND state = 'pending'),
                (SELECT count(*) FROM snapshots AS s
                 JOIN memory_candidates AS c ON c.file_id = s.file_id
                 WHERE c.chain_id = ?1 AND c.state = 'pending')",
        [chain_id],
        |r| {
            Ok(PendingCounts {
                objects: r.get(0)?,
                snapshots: r.get(1)?,
                limit: CANDIDATE_LIMIT,
            })
        },
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// A gap row; `ended` is given for a gap that is over when it is written.
pub(crate) fn insert_gap_in(
    conn: &Connection,
    chain_id: &str,
    kind: GapKind,
    after_snapshot_id: Option<&str>,
    problem: Option<ErrorCode>,
    started_at_ms: i64,
    ended: Option<(i64, GapEnd)>,
) -> Result<String, AppError> {
    let gap_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO memory_gaps (gap_id, chain_id, kind, after_snapshot_id, problem,
             started_at_ms, ended_at_ms, end_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            gap_id,
            chain_id,
            to_text(kind),
            after_snapshot_id,
            problem.map(to_text),
            started_at_ms,
            ended.map(|(at, _)| at),
            ended.map(|(_, end)| to_text(end)),
        ],
    )
    .map_err(|err| sqlite_error("Cannot save a Memory gap", &err))?;
    Ok(gap_id)
}

/// The newest version of file `file_id`.
pub(crate) fn latest_snapshot_in(
    conn: &Connection,
    file_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT snapshot_id FROM snapshots WHERE file_id = ?1 ORDER BY sequence DESC LIMIT 1",
        [file_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// Version `snapshot_id` already goes on as another file: a confirmed (or
/// disputed) continuation starts at it.
pub(crate) fn goes_on_in(conn: &Connection, snapshot_id: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM memory_edges WHERE parent_snapshot_id = ?1
                        AND kind = 'continuation'
                        AND status IN ('confirmed_system', 'confirmed_user', 'disputed'))",
        [snapshot_id],
        |r| r.get(0),
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// The newest file on the line that file `file_id` is on: from its last
/// version, the confirmed continuations are followed to the file each one
/// goes on to (the file itself when its line did not go on). A relation
/// always points from an older version to a newer one, so this ends.
pub(crate) fn line_front_in(conn: &Connection, file_id: &str) -> Result<String, AppError> {
    let mut front = file_id.to_string();
    loop {
        let next: Option<String> = conn
            .query_row(
                "SELECT child.file_id FROM memory_edges AS e
                 JOIN snapshots AS child ON child.snapshot_id = e.child_snapshot_id
                 WHERE e.kind = 'continuation'
                   AND e.status IN ('confirmed_system', 'confirmed_user', 'disputed')
                   AND e.parent_snapshot_id = (SELECT snapshot_id FROM snapshots
                                               WHERE file_id = ?1
                                               ORDER BY sequence DESC LIMIT 1)",
                [&front],
                |r| r.get(0),
            )
            .optional()
            .map_err(|err| sqlite_error("Cannot read the Memory records", &err))?;
        match next {
            Some(next) if next != front => front = next,
            _ => return Ok(front),
        }
    }
}

/// Why a record went on as another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WentOn {
    /// The user confirmed the object at the place goes on from the work.
    Continued,
    /// The user chose the file's new place after it was moved (M2).
    Relocated,
}

/// The record goes on as file `file_id`: its watch `old` stops (it is never
/// reopened) and a new watch starts there, in the same record task. The
/// switch belongs to the record, so it goes along; the quota too. The gaps
/// about the old place end: it is not the record's place any more.
pub(crate) fn move_watch_in(
    tx: &Connection,
    chain_id: &str,
    old: &WatchRow,
    file_id: &str,
    decision_id: &str,
    why: WentOn,
    now: i64,
) -> Result<WatchRow, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot save the decision", &err);
    let stopped = tx
        .execute(
            "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = ?2
             WHERE watch_id = ?1 AND status = 'active'",
            params![old.watch_id, now],
        )
        .map_err(fail)?;
    if stopped != 1 {
        return Err(crate::candidates::page_outdated(
            "Memory was turned off meanwhile. Open the page again.",
        ));
    }
    let watch = WatchRow {
        watch_id: uuid::Uuid::new_v4().to_string(),
        file_id: file_id.to_string(),
        active: true,
        started_at_ms: now,
        stopped_at_ms: None,
    };
    tx.execute(
        "INSERT INTO memory_watches (watch_id, file_id, status, started_at_ms)
         VALUES (?1, ?2, 'active', ?3)",
        params![watch.watch_id, watch.file_id, now],
    )
    .map_err(|err| match err.sqlite_error_code() {
        Some(rusqlite::ErrorCode::ConstraintViolation) => memory_error(
            ErrorCode::DecisionConflict,
            "That file is already recorded by Memory. Nothing was changed.",
            false,
            None,
        ),
        _ => fail(err),
    })?;
    let (reason, ended, end) = match why {
        WentOn::Continued => (
            "continued",
            &[GapKind::CaptureFailed, GapKind::ReplacedUncaptured][..],
            GapEnd::Continued,
        ),
        WentOn::Relocated => (
            "relocated",
            &[
                GapKind::CaptureFailed,
                GapKind::ReplacedUncaptured,
                GapKind::Lost,
            ][..],
            GapEnd::Relocated,
        ),
    };
    tx.execute(
        "INSERT INTO memory_chain_watches (watch_id, chain_id, reason, previous_watch_id,
             decision_id, linked_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            watch.watch_id,
            chain_id,
            reason,
            old.watch_id,
            decision_id,
            now
        ],
    )
    .map_err(fail)?;
    end_gaps_in(tx, chain_id, ended, end, now)?;
    Ok(watch)
}

pub(crate) fn end_gaps_in(
    conn: &Connection,
    chain_id: &str,
    kinds: &[GapKind],
    end: GapEnd,
    now: i64,
) -> Result<(), AppError> {
    for kind in kinds {
        conn.execute(
            "UPDATE memory_gaps SET ended_at_ms = max(started_at_ms, ?3), end_reason = ?4
             WHERE chain_id = ?1 AND kind = ?2 AND ended_at_ms IS NULL",
            params![chain_id, to_text(*kind), now, to_text(end)],
        )
        .map_err(|err| sqlite_error("Cannot save a Memory gap", &err))?;
    }
    Ok(())
}

/// The files with a membership that counts (confirmed or disputed) in the
/// works live work `?1` stands for, as a subquery.
const CONFIRMED: &str = "(SELECT file_id FROM memory_members WHERE family_id IN
    (SELECT family_id FROM memory_live_families WHERE live_family_id = ?1)
    AND status IN ('confirmed_system', 'confirmed_user', 'disputed'))";

/// The files with any membership that was not replaced in those works:
/// a relation's child is in the work where its file has its row.
const ROWS: &str = "(SELECT file_id FROM memory_members WHERE family_id IN
    (SELECT family_id FROM memory_live_families WHERE live_family_id = ?1)
    AND status <> 'superseded')";

/// `family_facts` without the snapshots, on any connection (a transaction
/// too). A file with rows in two works of the group (one merged in) counts
/// once, by its most definite row: confirmed, then candidate, unknown,
/// rejected.
pub(crate) fn family_facts_in(conn: &Connection, family_id: &str) -> Result<FamilyFacts, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let raw_members: Vec<RawMember> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT m.member_id, m.family_id, m.file_id, m.status, m.basis,
                        coalesce(d.decided_at_ms,
                                 (SELECT max(wc.changed_at_ms) FROM memory_work_changes AS wc
                                  WHERE wc.member_id = m.member_id))
                 FROM memory_members AS m
                 LEFT JOIN memory_decisions AS d ON d.decision_id = m.decision_id
                 WHERE m.family_id IN {GROUP} AND m.status <> 'superseded'
                 ORDER BY m.created_at_ms, m.rowid"
            ))
            .map_err(fail)?;
        stmt.query_map([family_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    let rank = |status: RelationStatus| match status {
        RelationStatus::ConfirmedSystem
        | RelationStatus::ConfirmedUser
        | RelationStatus::Disputed => 0,
        RelationStatus::Candidate => 1,
        RelationStatus::Unknown => 2,
        _ => 3,
    };
    let mut members: Vec<MemberRow> = Vec::new();
    for (member_id, row_family, file_id, status, basis, decided_at_ms) in raw_members {
        let row = MemberRow {
            member_id,
            family_id: row_family,
            file_id,
            status: from_text(&status).ok_or_else(bad_row)?,
            basis: from_text(&basis).ok_or_else(bad_row)?,
            decided_at_ms,
        };
        match members.iter_mut().find(|m| m.file_id == row.file_id) {
            Some(kept) if rank(row.status) < rank(kept.status) => *kept = row,
            Some(_) => {}
            None => members.push(row),
        }
    }
    // A work's record lines are the line files that are its members, and
    // its records the chains with such a file: moving a file (M2b-3b) takes
    // its part of a line along, while chain rows keep the work they were
    // made in. Before any move this is exactly the chains of the work.
    let line_files = format!(
        "SELECT link.chain_id, w.file_id FROM memory_chain_watches AS link
         JOIN memory_watches AS w ON w.watch_id = link.watch_id
         WHERE w.file_id IN {CONFIRMED}
         UNION
         SELECT cand.chain_id, cand.file_id FROM memory_candidates AS cand
         WHERE cand.state = 'continued' AND cand.file_id IN {CONFIRMED}"
    );
    let work_chains = format!("(SELECT chain_id FROM ({line_files}))");
    let chains = query_chains(
        conn,
        &format!("WHERE chain_id IN {work_chains} ORDER BY created_at_ms, rowid"),
        [family_id],
    )?;
    let lines: Vec<(String, String)> = {
        let mut stmt = conn.prepare(&line_files).map_err(fail)?;
        stmt.query_map([family_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)?
    };
    let raw_edges: Vec<RawEdge> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT e.edge_id, e.family_id, e.kind, e.parent_snapshot_id, e.child_snapshot_id,
                        e.status, e.skipped_items,
                        coalesce(d.decided_at_ms,
                                 (SELECT max(wc.changed_at_ms) FROM memory_work_changes AS wc
                                  WHERE wc.edge_id = e.edge_id)),
                        EXISTS (SELECT 1 FROM memory_work_changes AS wc
                                JOIN memory_work_decisions AS wd
                                     ON wd.work_decision_id = wc.work_decision_id
                                WHERE wc.edge_id = e.edge_id AND wc.from_status IS NULL
                                  AND wd.kind = 'associate'),
                        EXISTS (SELECT 1 FROM memory_auto_links AS a
                                WHERE a.edge_id = e.edge_id)
                 FROM memory_edges AS e
                 LEFT JOIN memory_decisions AS d ON d.decision_id = e.decision_id
                 WHERE (SELECT file_id FROM snapshots WHERE snapshot_id = e.child_snapshot_id)
                       IN {ROWS}
                    OR (SELECT file_id FROM snapshots WHERE snapshot_id = e.parent_snapshot_id)
                       IN {CONFIRMED}
                 ORDER BY e.created_at_ms, e.rowid"
            ))
            .map_err(fail)?;
        stmt.query_map([family_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    let edges = raw_edges
        .into_iter()
        .map(
            |(
                edge_id,
                row_family,
                kind,
                parent,
                child,
                status,
                skipped_items,
                decided_at_ms,
                associated,
                auto_linked,
            )| {
                Ok(EdgeRow {
                    edge_id,
                    family_id: row_family,
                    kind: from_text(&kind).ok_or_else(bad_row)?,
                    parent_snapshot_id: parent,
                    child_snapshot_id: child,
                    status: from_text(&status).ok_or_else(bad_row)?,
                    skipped_items,
                    decided_at_ms,
                    associated,
                    auto_linked,
                })
            },
        )
        .collect::<Result<Vec<_>, AppError>>()?;
    let gaps = query_gaps(
        conn,
        &format!("WHERE chain_id IN {work_chains} ORDER BY started_at_ms, rowid"),
        [family_id],
    )?;
    let candidates: Vec<CandidateRow> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT c.chain_id, c.file_id, c.state, c.after_snapshot_id, c.first_snapshot_id
                 FROM memory_candidates AS c
                 WHERE c.chain_id IN {work_chains} ORDER BY c.found_at_ms, c.seq"
            ))
            .map_err(fail)?;
        stmt.query_map([family_id], |r| {
            Ok(CandidateRow {
                chain_id: r.get(0)?,
                file_id: r.get(1)?,
                state: r.get(2)?,
                after_snapshot_id: r.get(3)?,
                first_snapshot_id: r.get(4)?,
            })
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    Ok(FamilyFacts {
        family_id: family_id.to_string(),
        members,
        chains,
        lines,
        edges,
        gaps,
        candidates,
        snapshots: HashMap::new(),
    })
}

/// `filter` is always a constant written in this file; values go in `params`.
fn query_chains<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<ChainRow>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let sql = format!(
        "SELECT chain_id, family_id, origin_file_id,
                EXISTS (SELECT 1 FROM memory_gaps AS g
                        WHERE g.chain_id = memory_chains.chain_id AND g.kind = 'lost'
                          AND g.ended_at_ms IS NULL)
         FROM memory_chains {filter}"
    );
    let mut stmt = conn.prepare(&sql).map_err(fail)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(ChainRow {
                chain_id: row.get(0)?,
                family_id: row.get(1)?,
                origin_file_id: row.get(2)?,
                lost: row.get(3)?,
            })
        })
        .map_err(fail)?;
    rows.collect::<Result<_, _>>().map_err(fail)
}

/// `filter` is always a constant written in this file; values go in `params`.
pub(crate) fn query_gaps<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<GapRow>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory gaps", &err);
    let sql = format!(
        "SELECT gap_id, chain_id, kind, after_snapshot_id, problem, started_at_ms, ended_at_ms,
                end_reason
         FROM memory_gaps {filter}"
    );
    let mut stmt = conn.prepare(&sql).map_err(fail)?;
    let raw = stmt
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(fail)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    raw.into_iter()
        .map(
            |(gap_id, chain_id, kind, after, problem, started, ended, end)| {
                Ok(GapRow {
                    gap_id,
                    chain_id,
                    kind: from_text(&kind).ok_or_else(bad_row)?,
                    after_snapshot_id: after,
                    problem: match problem {
                        Some(text) => Some(from_text(&text).ok_or_else(bad_row)?),
                        None => None,
                    },
                    started_at_ms: started,
                    ended_at_ms: ended,
                    end_reason: match end {
                        Some(text) => Some(from_text(&text).ok_or_else(bad_row)?),
                        None => None,
                    },
                })
            },
        )
        .collect()
}

/// Everything about one work, read with one connection: what lineage.rs
/// works out pairs from, and what the work's view shows.
pub(crate) struct FamilyFacts {
    pub(crate) family_id: String,
    pub(crate) members: Vec<MemberRow>,
    pub(crate) chains: Vec<ChainRow>,
    /// Each chain's line: its watch files and continued candidates, as
    /// (chain ID, file ID).
    pub(crate) lines: Vec<(String, String)>,
    pub(crate) edges: Vec<EdgeRow>,
    pub(crate) gaps: Vec<GapRow>,
    /// The candidates of the work's chains: (chain, file, state, after,
    /// first snapshot).
    pub(crate) candidates: Vec<CandidateRow>,
    /// Every snapshot of every member file, by file, in sequence order.
    pub(crate) snapshots: HashMap<String, Vec<SnapshotRef>>,
}

impl FamilyFacts {
    pub(crate) fn snapshots_of<'a>(
        &'a self,
        file_id: &str,
    ) -> impl Iterator<Item = &'a SnapshotRef> {
        self.snapshots.get(file_id).into_iter().flatten()
    }

    pub(crate) fn snapshot(&self, snapshot_id: &str) -> Option<&SnapshotRef> {
        self.snapshots
            .values()
            .flatten()
            .find(|snapshot| snapshot.snapshot_id == snapshot_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateRow {
    pub(crate) chain_id: String,
    pub(crate) file_id: String,
    pub(crate) state: String,
    pub(crate) after_snapshot_id: Option<String>,
    pub(crate) first_snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberRow {
    pub(crate) member_id: String,
    /// The work the row was written in: the live work, or one merged into
    /// it (then the membership rests on the user's merge).
    pub(crate) family_id: String,
    pub(crate) file_id: String,
    pub(crate) status: RelationStatus,
    pub(crate) basis: MemberBasis,
    pub(crate) decided_at_ms: Option<i64>,
}

/// An edge as its columns: ID, work, kind, parent, child, status, skipped
/// objects, decision time, made by an association, made by Traeva itself.
type RawEdge = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    Option<i64>,
    bool,
    bool,
);

/// A membership as its columns: ID, work, file, status, basis, decision
/// time.
type RawMember = (String, String, String, String, String, Option<i64>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EdgeRow {
    pub(crate) edge_id: String,
    /// The work the relation was written in (see `MemberRow::family_id`).
    pub(crate) family_id: String,
    pub(crate) kind: EdgeKind,
    pub(crate) parent_snapshot_id: Option<String>,
    pub(crate) child_snapshot_id: String,
    pub(crate) status: RelationStatus,
    pub(crate) skipped_items: i64,
    pub(crate) decided_at_ms: Option<i64>,
    /// Made by associating a file with the work: Memory never saw what
    /// happened between its two versions.
    pub(crate) associated: bool,
    /// Made by Traeva itself (`memory_auto_links`, auto_link.rs).
    pub(crate) auto_linked: bool,
}

impl Store {
    /// Everything about live work `family_id` and the works merged into it
    /// (`memory_live_families`). The caller passes a live work ID.
    pub(crate) fn family_facts(&self, family_id: &str) -> Result<FamilyFacts, AppError> {
        let conn = self.connection()?;
        let facts = family_facts_in(&conn, family_id)?;
        drop(conn);
        let mut facts = facts;
        for member in &facts.members {
            facts
                .snapshots
                .insert(member.file_id.clone(), self.snapshots_of(&member.file_id)?);
        }
        Ok(facts)
    }

    /// The work file `file_id` is a confirmed member of.
    pub(crate) fn family_of_file(&self, file_id: &str) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT l.live_family_id FROM memory_members AS m
             JOIN memory_live_families AS l ON l.family_id = m.family_id
             WHERE m.file_id = ?1 AND m.status IN ('confirmed_system', 'confirmed_user')",
            [file_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
    }

    /// How file `file_id` stands in any work, when it is not a confirmed
    /// member: a candidate (or rejected) somewhere, or not in a work at all.
    pub(crate) fn is_candidate_file(&self, file_id: &str) -> Result<bool, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM memory_candidates WHERE file_id = ?1)",
            [file_id],
            |r| r.get(0),
        )
        .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
    }

    pub(crate) fn all_family_ids(&self) -> Result<Vec<String>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT family_id FROM memory_families
                 WHERE family_id NOT IN (SELECT absorbed_family_id FROM memory_family_merges)
                 ORDER BY created_at_ms, rowid",
            )
            .map_err(fail)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(fail)?;
        rows.collect::<Result<_, _>>().map_err(fail)
    }
}

#[cfg(test)]
#[path = "family_tests.rs"]
mod tests;
