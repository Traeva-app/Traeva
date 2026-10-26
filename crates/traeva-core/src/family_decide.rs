//! Deciding about candidates (#31 material §3, §5, §6 and supplement;
//! shared-contract.md §4.3).
//!
//! The confirmation page of a record lists its candidates that wait for a
//! decision, in the order they were seen, and the unread object at its place
//! when there is one. For each it shows the only relation "continue" would
//! confirm (`continue_from`): from the newest version before it on the
//! record's line. The page has a token of everything it shows; a decision
//! brings it back, and when anything changed (a new candidate, a stop, a
//! move, another replacement), nothing is decided: the page must be opened
//! again. Only what the page listed is ever decided.
//!
//! Choices, per object:
//! - continue: it goes on from the work, a confirmed relation from
//!   `continue_from` to its first version;
//! - branch: a branch of the work, from a chosen fork point (or an unknown
//!   one);
//! - unknown: part of the work, order unknown (no parent);
//! - independent: unrelated, never proposed again;
//! - defer: not sure yet; nothing changes and its quota stays used.
//!
//! The middle-object rules: judging an earlier object unrelated only rejects
//! that object. A later object is only linked across it after the page shows
//! that new relation (with the number of objects it skips) and the user
//! confirms it; core never works one out from the exclusion. Judging the
//! object now at the place unrelated stops candidate capture and leaves the
//! work lost, waiting for its new place.
//!
//! Continuing the object now at the place moves the record's watch to it:
//! the old watch stops (it is never reopened) and a new one starts, in the
//! same record task, in the same transaction as the decision. When Memory is
//! off, the folder was removed or the record is lost, only the history is
//! confirmed. Nothing is reported as decided unless it was saved. Traeva's
//! own link of a new object at the record's exact place (auto_link.rs, the
//! student's 10/14 decision) writes through the same function,
//! `continue_current_in`.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::candidates::{
    Occupant, OccupantView, Place, PlaceHolds, Target, occupant_token, open_switch_in,
    page_outdated, switch_off_in,
};
use crate::capture::{MemoryWatchView, Read, WatchRow, query_watches};
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    CANDIDATE_LIMIT, CandidateCapture, GapEnd, MemoryRecordView, PendingCounts, PlaceView,
    RecordState, WentOn, chain_tail_in, end_gaps_in, goes_on_in, insert_gap_in, move_watch_in,
    pending_counts_in, unknown_record,
};
use crate::is_canonical_uuid;
use crate::memory_consent::memory_error;
use crate::memory_contracts::{GapKind, SnapshotRef};
use crate::observe;
use crate::platform::macos::Observation;
use crate::snapshot::{insert_next_snapshot, store_bytes};
use crate::store::{SeenFile, Store, observe_in, query_files, sqlite_error};

// ---------------------------------------------------------------------------
// What the window sees and sends
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateReason {
    /// Found at the place by candidate capture.
    Replaced,
    /// At the place when candidate capture was turned on, saved at once.
    PresentWhenEnabled,
    /// Read when the user decided about the object at the place.
    ReadOnDecision,
}

/// One candidate that waits for a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingCandidate {
    pub candidate_id: String,
    pub file_id: String,
    /// Order seen in this record: unique, increasing, not contiguous on the
    /// page (decided ones are not listed).
    pub seq: i64,
    pub reason: CandidateReason,
    pub display_path: String,
    pub present: bool,
    /// It is the object at the record's place right now.
    pub at_place: bool,
    pub found_at_ms: i64,
    pub snapshots: Vec<SnapshotRef>,
    /// The record's newest version when this object was found.
    pub found_after: Option<SnapshotRef>,
    /// The only parent "continue" accepts; null: "continue" is not possible.
    pub continue_from: Option<SnapshotRef>,
    /// Objects between `continue_from` and this one that are not on the line
    /// (judged unrelated, unknown or a branch). More than 0: show the
    /// relation as a new one before it is confirmed.
    pub continue_skips: u32,
    /// A waiting candidate that must be continued in the same submission
    /// first (an earlier object on the way back).
    pub continue_needs: Option<String>,
    /// The user said "not sure yet" about it before.
    pub deferred: bool,
}

/// The unread object at the place, with what "continue" would confirm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadOccupant {
    pub occupant: OccupantView,
    /// When the gap for it began, if one is open.
    pub detected_at_ms: Option<i64>,
    pub continue_from: Option<SnapshotRef>,
    pub continue_skips: u32,
    pub continue_needs: Option<String>,
}

/// The confirmation page of one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingCandidatesView {
    pub chain_id: String,
    pub family_id: String,
    /// Bring it back with the decision.
    pub page_token: String,
    pub state: RecordState,
    pub root_active: bool,
    pub candidate_capture: CandidateCapture,
    pub pending: PendingCounts,
    pub place: PlaceView,
    pub place_holds: PlaceHolds,
    /// The record's newest saved version.
    pub tail: Option<SnapshotRef>,
    /// Versions a "branch" may name as its fork point: on the line, or of a
    /// branch whose start is known. Oldest first.
    pub fork_points: Vec<SnapshotRef>,
    pub items: Vec<PendingCandidate>,
    pub unread_occupant: Option<UnreadOccupant>,
    pub place_problem: Option<AppError>,
}

/// The user's choices on one page. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDecisionRequest {
    pub chain_id: String,
    pub page_token: String,
    /// Only the candidates the user chose something for.
    #[serde(default)]
    pub items: Vec<CandidateDecision>,
    /// A choice about the unread object at the place, if the page showed one.
    #[serde(default)]
    pub current_object: Option<CurrentObjectChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDecision {
    pub candidate_id: String,
    pub choice: CandidateChoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateChoice {
    Continue {
        parent_snapshot_id: String,
    },
    Branch {
        #[serde(default)]
        parent_snapshot_id: Option<String>,
    },
    // Empty braces, not unit variants: serde ignores unknown fields next to
    // the tag of a unit variant, and requests refuse unknown fields.
    Unknown {},
    Independent {},
    Defer {},
}

/// A choice about the object at the place that is not a waiting candidate
/// (#31 material §3): "continue" is the user's permission to read it and go
/// on recording from it; "independent" reads nothing. Saying it is a branch,
/// or part of the work in an unknown order, needs its content saved first:
/// turn candidate capture on, or turn Memory on for it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentObjectChoice {
    Continue { parent_snapshot_id: String },
    Independent {},
}

/// What a decision did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateDecisionOutcome {
    pub batch_id: String,
    pub record: MemoryRecordView,
    /// The page again, with a new token.
    pub page: PendingCandidatesView,
    /// The watch that now records the object at the place, when the record
    /// went on as it.
    pub continued_watch: Option<MemoryWatchView>,
    /// A "continue" was confirmed but the watch did not move (Memory is off,
    /// the folder was removed, the record is lost, or the object is no
    /// longer at the place): only the history was confirmed.
    pub history_only: bool,
}

// ---------------------------------------------------------------------------
// The page's facts and token
// ---------------------------------------------------------------------------

/// What a live look found at the place, for the page.
#[derive(Debug, Clone)]
pub(crate) enum Live {
    NotLooked,
    Recorded,
    Empty,
    Pending(String),
    Decided(String),
    Unread {
        observed: Observation,
        expected: Option<(String, i64)>,
    },
    Other,
    Unavailable(AppError),
}

impl Live {
    fn token_line(&self) -> String {
        match self {
            Live::NotLooked => "-".to_string(),
            Live::Recorded => "recorded".to_string(),
            Live::Empty => "empty".to_string(),
            Live::Pending(id) => format!("pending:{id}"),
            Live::Decided(id) => format!("decided:{id}"),
            Live::Unread { observed, .. } => format!(
                "unread:{}:{}:{}",
                observed.id.dev, observed.id.ino, observed.birthtime_ns
            ),
            Live::Other => "other".to_string(),
            Live::Unavailable(error) => format!("unavailable:{:?}", error.code),
        }
    }
}

/// One waiting candidate, as the page and the decision see it.
#[derive(Debug, Clone)]
struct Item {
    candidate_id: String,
    file_id: String,
    seq: i64,
    reason: String,
    after: Option<String>,
    first: String,
    present: bool,
    place: String,
    revision: i64,
    /// How many times the user said "not sure yet" about it.
    deferrals: i64,
    found_at_ms: i64,
    snapshots: Vec<SnapshotRef>,
    continue_from: Option<String>,
    skips: u32,
    needs: Option<String>,
    at_place: bool,
}

/// Everything a page shows, read with one connection.
struct PageFacts {
    family_id: String,
    watch: Option<WatchRow>,
    root_active: bool,
    switch: Option<String>,
    lost: Option<String>,
    place: String,
    tail: Option<String>,
    counts: PendingCounts,
    items: Vec<Item>,
    fork_points: Vec<String>,
    /// Save order (rowid) of the fork points and of the items' versions.
    order: HashMap<String, i64>,
    unread_continue: Option<(Option<String>, u32, Option<String>)>,
    replaced_since: Option<i64>,
    live: Live,
}

impl PageFacts {
    fn token(&self, chain_id: &str) -> String {
        let mut lines = vec![
            "memory-page-1".to_string(),
            format!("chain:{chain_id}"),
            format!("work:{}", self.family_id),
            format!(
                "watch:{}",
                self.watch
                    .as_ref()
                    .filter(|watch| watch.active)
                    .map_or("-", |watch| watch.watch_id.as_str())
            ),
            format!(
                "root:{}",
                if self.root_active {
                    "active"
                } else {
                    "revoked"
                }
            ),
            format!("switch:{}", self.switch.as_deref().unwrap_or("-")),
            format!("lost:{}", self.lost.as_deref().unwrap_or("-")),
            format!("place:{}", self.place),
            format!("tail:{}", self.tail.as_deref().unwrap_or("-")),
            format!("counts:{}/{}", self.counts.objects, self.counts.snapshots),
        ];
        for item in &self.items {
            let snapshots: Vec<&str> = item
                .snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .collect();
            lines.push(format!(
                "item:{}:{}:{}:{}:{}@{}:{}:{}:{}:{}:{}:{}",
                item.seq,
                item.candidate_id,
                item.file_id,
                u8::from(item.present),
                item.place,
                item.revision,
                item.deferrals,
                item.after.as_deref().unwrap_or("-"),
                item.continue_from.as_deref().unwrap_or("-"),
                item.skips,
                item.needs.as_deref().unwrap_or("-"),
                snapshots.join(",")
            ));
        }
        if let Some((from, skips, needs)) = &self.unread_continue {
            lines.push(format!(
                "unread:{}:{skips}:{}",
                from.as_deref().unwrap_or("-"),
                needs.as_deref().unwrap_or("-")
            ));
        }
        lines.push(format!("live:{}", self.live.token_line()));
        observe::sha256_hex(lines.join("\n").as_bytes())
    }

    fn active(&self) -> bool {
        self.watch.as_ref().is_some_and(|watch| watch.active) && self.lost.is_none()
    }
}

/// A waiting candidate as its columns: ID, file, seq, reason, after, first
/// snapshot, when found.
type RawCandidate = (String, String, i64, String, Option<String>, String, i64);

/// A candidate of the chain, for walking back along the line.
struct LineCandidate {
    candidate_id: String,
    state: String,
    after: Option<String>,
}

fn page_facts(conn: &Connection, chain_id: &str, live: Live) -> Result<PageFacts, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    // The live work: new memberships and relations go there (#31 M2b).
    let family_id =
        crate::works::live_family_of_chain_in(conn, chain_id)?.ok_or_else(unknown_record)?;
    let watch = query_watches(
        conn,
        "WHERE watch_id IN (SELECT watch_id FROM memory_chain_watches WHERE chain_id = ?1)
           AND watch_id NOT IN (SELECT previous_watch_id FROM memory_chain_watches
                                WHERE previous_watch_id IS NOT NULL)
         ORDER BY status = 'active' DESC, rowid DESC LIMIT 1",
        [chain_id],
    )?
    .pop();
    let (place, root_active): (String, bool) = match &watch {
        Some(watch) => conn
            .query_row(
                "SELECT f.root_id || '/' || f.relative_path, r.status = 'active'
                 FROM files AS f JOIN authorized_roots AS r ON r.root_id = f.root_id
                 WHERE f.file_id = ?1",
                [&watch.file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(fail)?,
        None => ("-".to_string(), false),
    };
    let switch = open_switch_in(conn, chain_id)?;
    let lost: Option<String> = conn
        .query_row(
            "SELECT gap_id FROM memory_gaps WHERE chain_id = ?1 AND kind = 'lost'
             AND ended_at_ms IS NULL",
            [chain_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(fail)?;
    let replaced: Option<(Option<String>, i64)> = conn
        .query_row(
            "SELECT after_snapshot_id, started_at_ms FROM memory_gaps WHERE chain_id = ?1
             AND kind = 'replaced_uncaptured' AND ended_at_ms IS NULL",
            [chain_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(fail)?;
    let tail = chain_tail_in(conn, chain_id)?;
    let counts = pending_counts_in(conn, chain_id)?;

    // The line: the chain's watch files, and its candidates by file.
    let watch_files: HashSet<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT w.file_id FROM memory_chain_watches AS link
                 JOIN memory_watches AS w ON w.watch_id = link.watch_id
                 WHERE link.chain_id = ?1",
            )
            .map_err(fail)?;
        stmt.query_map([chain_id], |r| r.get(0))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)?
    };
    let line_candidates: HashMap<String, LineCandidate> = {
        let mut stmt = conn
            .prepare(
                "SELECT file_id, candidate_id, state, after_snapshot_id FROM memory_candidates
                 WHERE chain_id = ?1",
            )
            .map_err(fail)?;
        stmt.query_map([chain_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                LineCandidate {
                    candidate_id: r.get(1)?,
                    state: r.get(2)?,
                    after: r.get(3)?,
                },
            ))
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    let file_of = |snapshot_id: &str| -> Result<Option<String>, AppError> {
        conn.query_row(
            "SELECT file_id FROM snapshots WHERE snapshot_id = ?1",
            [snapshot_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(fail)
    };
    // Back from `start` to the newest version on the line: the only parent
    // "continue" may confirm. An earlier waiting object comes first (it must
    // be continued too), but only when the line can still be reached through
    // it. A version that already goes on as another file (the record went on
    // at the file's new place) is not a parent "continue" may confirm again.
    let walk_back =
        |start: Option<String>| -> Result<(Option<String>, u32, Option<String>), AppError> {
            let mut at = start;
            let mut skips = 0;
            let mut waiting: Option<(String, String)> = None;
            let reached = |line: String, skips: u32, waiting: Option<(String, String)>| {
                // A line version whose file was moved to another work
                // (M2b-3b) is not a parent here: the relation would cross.
                let in_work: bool = conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM snapshots AS s
                                        JOIN memory_members AS m ON m.file_id = s.file_id
                                        JOIN memory_live_families AS l
                                             ON l.family_id = m.family_id
                                        WHERE s.snapshot_id = ?1 AND l.live_family_id = ?2
                                          AND m.status IN ('confirmed_system', 'confirmed_user'))",
                        params![line, family_id],
                        |r| r.get(0),
                    )
                    .map_err(fail)?;
                Ok(if !in_work || goes_on_in(conn, &line)? {
                    (None, skips, None)
                } else {
                    match waiting {
                        Some((snapshot, candidate_id)) => {
                            (Some(snapshot), skips, Some(candidate_id))
                        }
                        None => (Some(line), skips, None),
                    }
                })
            };
            while let Some(snapshot) = at {
                let Some(file) = file_of(&snapshot)? else {
                    return Ok((None, skips, None));
                };
                match line_candidates.get(&file) {
                    Some(c) if c.state == "continued" => return reached(snapshot, skips, waiting),
                    Some(c) if c.state == "pending" => {
                        if waiting.is_none() {
                            waiting = Some((snapshot, c.candidate_id.clone()));
                        }
                        at = c.after.clone();
                    }
                    Some(c) => {
                        if waiting.is_none() {
                            skips += 1;
                        }
                        at = c.after.clone();
                    }
                    None if watch_files.contains(&file) => {
                        return reached(snapshot, skips, waiting);
                    }
                    None => return Ok((None, skips, None)),
                }
            }
            Ok((None, skips, None))
        };

    let raw: Vec<RawCandidate> = {
        let mut stmt = conn
            .prepare(
                "SELECT candidate_id, file_id, seq, reason, after_snapshot_id, first_snapshot_id,
                        found_at_ms
                 FROM memory_candidates WHERE chain_id = ?1 AND state = 'pending' ORDER BY seq",
            )
            .map_err(fail)?;
        stmt.query_map([chain_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map_err(fail)?
        .collect::<Result<_, _>>()
        .map_err(fail)?
    };
    let mut items = Vec::new();
    for (candidate_id, file_id, seq, reason, after, first, found_at_ms) in raw {
        let file = query_files(conn, "WHERE file_id = ?1", [&file_id])?
            .pop()
            .ok_or_else(unknown_record)?;
        let present: bool = conn
            .query_row(
                "SELECT presence = 'present' FROM files WHERE file_id = ?1",
                [&file_id],
                |r| r.get(0),
            )
            .map_err(fail)?;
        let deferrals: i64 = conn
            .query_row(
                "SELECT count(*) FROM memory_decisions WHERE candidate_id = ?1
                 AND choice = 'defer'",
                [&candidate_id],
                |r| r.get(0),
            )
            .map_err(fail)?;
        let snapshots = crate::snapshot::query_snapshots(
            conn,
            "WHERE file_id = ?1 ORDER BY sequence",
            [&file_id],
        )?;
        let (continue_from, skips, needs) = walk_back(after.clone())?;
        items.push(Item {
            at_place: matches!(&live, Live::Pending(id) if *id == candidate_id),
            candidate_id,
            file_id,
            seq,
            reason,
            after,
            first,
            present,
            place: format!("{}/{}", file.root_id, file.relative_path),
            revision: file.record_revision,
            deferrals,
            found_at_ms,
            snapshots,
            continue_from,
            skips,
            needs,
        });
    }
    let fork_points: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT s.snapshot_id FROM snapshots AS s
                 WHERE s.file_id IN (
                     SELECT w.file_id FROM memory_chain_watches AS link
                     JOIN memory_watches AS w ON w.watch_id = link.watch_id
                     WHERE link.chain_id = ?1
                     UNION
                     SELECT c.file_id FROM memory_candidates AS c
                     WHERE c.chain_id = ?1 AND c.state = 'continued'
                     UNION
                     SELECT c.file_id FROM memory_candidates AS c
                     JOIN memory_edges AS e ON e.child_snapshot_id = c.first_snapshot_id
                     WHERE c.chain_id = ?1 AND c.state = 'branch'
                       AND e.parent_snapshot_id IS NOT NULL)
                   -- Only files of the record's work: a part moved to
                   -- another work (M2b-3b) is not a fork point here.
                   AND s.file_id IN (SELECT m.file_id FROM memory_members AS m
                                     JOIN memory_live_families AS l ON l.family_id = m.family_id
                                     WHERE l.live_family_id = ?2
                                       AND m.status IN ('confirmed_system', 'confirmed_user'))
                 ORDER BY s.rowid",
            )
            .map_err(fail)?;
        stmt.query_map(params![chain_id, family_id], |r| r.get(0))
            .map_err(fail)?
            .collect::<Result<_, _>>()
            .map_err(fail)?
    };
    let mut order = HashMap::new();
    let ids = fork_points.iter().cloned().chain(
        items
            .iter()
            .flat_map(|item| item.snapshots.iter().map(|s| s.snapshot_id.clone())),
    );
    for id in ids {
        let rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM snapshots WHERE snapshot_id = ?1",
                [&id],
                |r| r.get(0),
            )
            .map_err(fail)?;
        order.insert(id, rowid);
    }
    let unread_continue = match &live {
        Live::Unread { .. } => Some(walk_back(
            replaced
                .as_ref()
                .map(|(after, _)| after.clone())
                .unwrap_or_else(|| tail.clone()),
        )?),
        _ => None,
    };
    Ok(PageFacts {
        family_id,
        watch,
        root_active,
        switch,
        lost,
        place,
        tail,
        counts,
        items,
        fork_points,
        order,
        unread_continue,
        replaced_since: replaced.map(|(_, at)| at),
        live,
    })
}

// ---------------------------------------------------------------------------
// The rules: a pure function of the page and the choices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Planned {
    Continue { parent: String, skipped: u32 },
    Branch { parent: Option<String> },
    Unknown,
    Independent,
    Defer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchTarget {
    Item(String),
    Current,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    /// (candidate ID, what to do), in the order seen.
    items: Vec<(String, Planned)>,
    current: Option<Planned>,
    /// The object the record goes on as (its watch moves there).
    target: Option<WatchTarget>,
    /// The object at the place was judged unrelated: the work is lost.
    lost: bool,
}

fn conflict(message: impl Into<String>) -> AppError {
    memory_error(ErrorCode::DecisionConflict, message, false, None)
}

/// Checks the choices against the page and works out what to write. Never
/// links an object across one the user excluded unless the page showed that
/// relation.
fn plan_decision(facts: &PageFacts, request: &CandidateDecisionRequest) -> Result<Plan, AppError> {
    let by_id: HashMap<&str, &Item> = facts
        .items
        .iter()
        .map(|item| (item.candidate_id.as_str(), item))
        .collect();
    let mut chosen: Vec<(&Item, &CandidateChoice)> = Vec::new();
    for decision in &request.items {
        let item = by_id.get(decision.candidate_id.as_str()).ok_or_else(|| {
            page_outdated("A chosen object is not on this page any more. Open it again.")
        })?;
        chosen.push((item, &decision.choice));
    }
    chosen.sort_by_key(|(item, _)| item.seq);
    let fork_points: HashSet<&str> = facts.fork_points.iter().map(String::as_str).collect();
    let mut continued: HashSet<&str> = HashSet::new();
    let mut new_forks: HashSet<&str> = HashSet::new();
    let mut plan = Plan {
        items: Vec::new(),
        current: None,
        target: None,
        lost: false,
    };
    let needs_ok = |needs: &Option<String>, continued: &HashSet<&str>| match needs {
        Some(earlier) if !continued.contains(earlier.as_str()) => Err(conflict(
            "Continue the earlier waiting object in the same decision first, or decide it on \
             its own and open the page again.",
        )),
        _ => Ok(()),
    };
    // A branch starts from a version on the work's line (or of a branch whose
    // start is known) saved before the object: never from the object itself
    // or from a later version.
    let branch_ok =
        |parent: &Option<String>, first: &str, new_forks: &HashSet<&str>| -> Result<(), AppError> {
            if let Some(parent) = parent {
                let known =
                    fork_points.contains(parent.as_str()) || new_forks.contains(parent.as_str());
                let older = match (facts.order.get(parent), facts.order.get(first)) {
                    (Some(parent), Some(first)) => parent < first,
                    _ => false,
                };
                if !known || !older {
                    return Err(conflict(
                        "A branch starts from a version on the work's line, or of a branch whose \
                     start is known, saved before the object.",
                    ));
                }
            }
            Ok(())
        };
    for (item, choice) in chosen {
        let planned = match choice {
            CandidateChoice::Continue { parent_snapshot_id } => {
                if item.continue_from.as_deref() != Some(parent_snapshot_id.as_str()) {
                    return Err(conflict(
                        "This is not the relation the page showed. Open the page again and \
                         confirm the relation it shows.",
                    ));
                }
                needs_ok(&item.needs, &continued)?;
                continued.insert(item.candidate_id.as_str());
                new_forks.extend(item.snapshots.iter().map(|s| s.snapshot_id.as_str()));
                Planned::Continue {
                    parent: parent_snapshot_id.clone(),
                    skipped: item.skips,
                }
            }
            CandidateChoice::Branch { parent_snapshot_id } => {
                branch_ok(parent_snapshot_id, &item.first, &new_forks)?;
                if parent_snapshot_id.is_some() {
                    new_forks.extend(item.snapshots.iter().map(|s| s.snapshot_id.as_str()));
                }
                Planned::Branch {
                    parent: parent_snapshot_id.clone(),
                }
            }
            CandidateChoice::Unknown {} => Planned::Unknown,
            CandidateChoice::Independent {} => Planned::Independent,
            CandidateChoice::Defer {} => Planned::Defer,
        };
        if item.at_place && facts.active() {
            match &planned {
                Planned::Continue { .. } if facts.root_active => {
                    plan.target = Some(WatchTarget::Item(item.candidate_id.clone()));
                }
                Planned::Independent => plan.lost = true,
                _ => {}
            }
        }
        plan.items.push((item.candidate_id.clone(), planned));
    }
    if let Some(current) = &request.current_object {
        if plan.items.iter().any(|(id, _)| by_id[id.as_str()].at_place) {
            return Err(conflict("Decide about the object at the place only once."));
        }
        let planned = match current {
            // Unread, decided earlier, or recorded elsewhere: judging it
            // unrelated reads nothing.
            CurrentObjectChoice::Independent {}
                if matches!(
                    facts.live,
                    Live::Unread { .. } | Live::Decided(_) | Live::Other
                ) =>
            {
                Planned::Independent
            }
            CurrentObjectChoice::Independent {} => {
                return Err(page_outdated(
                    "The page shows no object at the place to decide about any more. Open it \
                     again.",
                ));
            }
            CurrentObjectChoice::Continue { parent_snapshot_id } => {
                let Some((from, skips, needs)) = &facts.unread_continue else {
                    return Err(page_outdated(
                        "The page shows no unread object at the place any more. Open it again.",
                    ));
                };
                if from.as_deref() != Some(parent_snapshot_id.as_str()) {
                    return Err(conflict(
                        "This is not the relation the page showed. Open the page again and \
                         confirm the relation it shows.",
                    ));
                }
                needs_ok(needs, &continued)?;
                Planned::Continue {
                    parent: parent_snapshot_id.clone(),
                    skipped: *skips,
                }
            }
        };
        if facts.active() {
            match &planned {
                Planned::Continue { .. } if facts.root_active => {
                    plan.target = Some(WatchTarget::Current);
                }
                Planned::Independent => plan.lost = true,
                _ => {}
            }
        }
        plan.current = Some(planned);
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

impl OrganizeService {
    /// The confirmation page of record `chain_id`: its candidates that wait
    /// for a decision, the unread object at its place, the quota and the
    /// token a decision must bring back. Looks at the place's metadata only.
    pub fn pending_candidates(&self, chain_id: &str) -> Result<PendingCandidatesView, AppError> {
        let store = self.store()?;
        check_record_id(chain_id)?;
        store.chain(chain_id)?.ok_or_else(unknown_record)?;
        let (live, place) = self.live_look(store, chain_id)?;
        let facts = {
            let conn = store.connection()?;
            page_facts(&conn, chain_id, live)?
        };
        self.page_view(store, chain_id, &facts, place)
    }

    /// Applies the user's choices on one page. Only what the page listed is
    /// decided: when anything it showed changed, `page_outdated` and nothing
    /// changes. A rule the choices break gives `decision_conflict` and nothing
    /// changes.
    pub fn decide_pending_candidates(
        &self,
        request: &CandidateDecisionRequest,
    ) -> Result<CandidateDecisionOutcome, AppError> {
        self.decide_pending_candidates_with(request, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn decide_pending_candidates_with(
        &self,
        request: &CandidateDecisionRequest,
        read: Read,
    ) -> Result<CandidateDecisionOutcome, AppError> {
        let store = self.store()?;
        check_record_id(&request.chain_id)?;
        let ids = request
            .items
            .iter()
            .map(|item| item.candidate_id.as_str())
            .chain(request.items.iter().filter_map(|item| match &item.choice {
                CandidateChoice::Continue { parent_snapshot_id } => {
                    Some(parent_snapshot_id.as_str())
                }
                CandidateChoice::Branch {
                    parent_snapshot_id: Some(parent),
                } => Some(parent.as_str()),
                _ => None,
            }));
        let current_parent = match &request.current_object {
            Some(CurrentObjectChoice::Continue { parent_snapshot_id }) => {
                Some(parent_snapshot_id.as_str())
            }
            _ => None,
        };
        if ids.chain(current_parent).any(|id| !is_canonical_uuid(id))
            || request.page_token.len() != 64
            || !request
                .page_token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(memory_error(
                ErrorCode::InvalidInput,
                "An ID or the page token is not valid.",
                false,
                None,
            ));
        }
        if request.items.is_empty() && request.current_object.is_none() {
            return Err(memory_error(
                ErrorCode::InvalidInput,
                "Nothing was chosen.",
                false,
                None,
            ));
        }
        let mut seen_ids = HashSet::new();
        if !request
            .items
            .iter()
            .all(|item| seen_ids.insert(item.candidate_id.as_str()))
        {
            return Err(conflict("An object was chosen twice."));
        }
        let chain_id = request.chain_id.as_str();
        store.chain(chain_id)?.ok_or_else(unknown_record)?;
        let no_capture = self.lock_execution();
        self.ensure_run();
        let (live, place) = self.live_look(store, chain_id)?;
        let facts = {
            let conn = store.connection()?;
            page_facts(&conn, chain_id, live.clone())?
        };
        if facts.token(chain_id) != request.page_token {
            return Err(page_outdated(
                "What the page showed changed since it was opened. Nothing was changed; open it \
                 again.",
            ));
        }
        let plan = plan_decision(&facts, request)?;
        // The unread object is read only when the user chose to keep it.
        let read_current = match (&plan.current, &live, &place) {
            (Some(Planned::Independent) | None, _, _) => None,
            (Some(_), Live::Unread { observed, expected }, Some(place)) => {
                self.no_unsettled_action(store, &place.root_id, &place.relative_path)?;
                let target = Target::New {
                    evidence: crate::evidence_of(observed),
                    expected: expected.clone(),
                };
                let (stable, seen) =
                    self.read_candidate(place, &target, read).map_err(|error| {
                        if error.code == ErrorCode::FileChanging {
                            page_outdated(
                                "The object at the place changed while it was read. Nothing was \
                             changed; open the page again.",
                            )
                        } else {
                            error
                        }
                    })?;
                store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes)?;
                Some(seen)
            }
            _ => return Err(page_outdated("Open the page again.")),
        };
        let batch_id = uuid::Uuid::new_v4().to_string();
        let moved_to = store.write_decision(
            chain_id,
            &request.page_token,
            &batch_id,
            &plan,
            read_current.as_ref(),
            live,
            self.now(),
        )?;
        drop(no_capture);
        let continued_watch = match &moved_to {
            Some(watch) => Some(self.watch_view(store, watch)?),
            None => None,
        };
        let any_continue = plan
            .items
            .iter()
            .map(|(_, planned)| planned)
            .chain(plan.current.as_ref())
            .any(|planned| matches!(planned, Planned::Continue { .. }));
        Ok(CandidateDecisionOutcome {
            batch_id,
            record: self.memory_record(chain_id)?,
            page: self.pending_candidates(chain_id)?,
            continued_watch,
            history_only: any_continue && moved_to.is_none(),
        })
    }

    /// A look at the record's place for its page: nothing when the record is
    /// stopped or lost.
    fn live_look(&self, store: &Store, chain_id: &str) -> Result<(Live, Option<Place>), AppError> {
        let record = self.memory_record(chain_id)?;
        if record.state != RecordState::Active {
            return Ok((Live::NotLooked, None));
        }
        match self.place_occupant(store, chain_id, &record.watch.file_id) {
            Err(error) => Ok((Live::Unavailable(error), None)),
            Ok(seen) => {
                let live = match (seen.holds, seen.unread, seen.occupant) {
                    (PlaceHolds::RecordedFile, ..) => Live::Recorded,
                    (PlaceHolds::Empty, ..) => Live::Empty,
                    (_, Some(observed), Some(Occupant::New { expected })) => {
                        Live::Unread { observed, expected }
                    }
                    (_, _, Some(Occupant::PendingHere { candidate_id, .. })) => {
                        Live::Pending(candidate_id)
                    }
                    (_, _, Some(Occupant::DecidedHere { candidate_id, .. })) => {
                        Live::Decided(candidate_id)
                    }
                    _ => Live::Other,
                };
                Ok((live, Some(seen.place)))
            }
        }
    }

    fn page_view(
        &self,
        store: &Store,
        chain_id: &str,
        facts: &PageFacts,
        place: Option<Place>,
    ) -> Result<PendingCandidatesView, AppError> {
        let record = self.memory_record(chain_id)?;
        let snapshot = |id: &Option<String>| -> Result<Option<SnapshotRef>, AppError> {
            match id {
                Some(id) => store.get_snapshot(id),
                None => Ok(None),
            }
        };
        let mut items = Vec::new();
        for item in &facts.items {
            let (file, _) = store
                .get_file(&item.file_id)?
                .ok_or_else(crate::snapshot::unknown_file)?;
            let root = store.get_root(&file.root_id)?;
            items.push(PendingCandidate {
                candidate_id: item.candidate_id.clone(),
                file_id: item.file_id.clone(),
                seq: item.seq,
                reason: match item.reason.as_str() {
                    "present_when_enabled" => CandidateReason::PresentWhenEnabled,
                    "read_on_decision" => CandidateReason::ReadOnDecision,
                    _ => CandidateReason::Replaced,
                },
                display_path: match root {
                    Some(root) => format!("{}/{}", root.display_path, file.relative_path),
                    None => file.relative_path.clone(),
                },
                present: item.present,
                at_place: item.at_place,
                found_at_ms: item.found_at_ms,
                snapshots: item.snapshots.clone(),
                found_after: snapshot(&item.after)?,
                continue_from: snapshot(&item.continue_from)?,
                continue_skips: item.skips,
                continue_needs: item.needs.clone(),
                deferred: item.deferrals > 0,
            });
        }
        let unread_occupant = match (&facts.live, &facts.unread_continue, &place, &facts.watch) {
            (
                Live::Unread { observed, .. },
                Some((from, skips, needs)),
                Some(place),
                Some(watch),
            ) => {
                let saves_now = facts.counts.objects < CANDIDATE_LIMIT
                    && facts.counts.snapshots < CANDIDATE_LIMIT;
                Some(UnreadOccupant {
                    occupant: OccupantView {
                        occupant_token: occupant_token(
                            chain_id,
                            &watch.watch_id,
                            place,
                            observed,
                            saves_now,
                        ),
                        byte_length: observed.size,
                        modified_at_ms: observed.mtime_ns / 1_000_000,
                    },
                    detected_at_ms: facts.replaced_since,
                    continue_from: snapshot(from)?,
                    continue_skips: *skips,
                    continue_needs: needs.clone(),
                })
            }
            _ => None,
        };
        let place_holds = match &facts.live {
            Live::NotLooked => PlaceHolds::NotLooked,
            Live::Recorded => PlaceHolds::RecordedFile,
            Live::Empty => PlaceHolds::Empty,
            Live::Pending(_) => PlaceHolds::PendingCandidate,
            Live::Decided(_) => PlaceHolds::DecidedCandidate,
            Live::Unread { .. } => PlaceHolds::UnreadObject,
            Live::Other => PlaceHolds::OtherRecord,
            Live::Unavailable(_) => PlaceHolds::Unavailable,
        };
        let fork_points = facts
            .fork_points
            .iter()
            .map(|id| store.get_snapshot(id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(PendingCandidatesView {
            chain_id: chain_id.to_string(),
            family_id: facts.family_id.clone(),
            page_token: facts.token(chain_id),
            state: record.state,
            root_active: record.root_active,
            candidate_capture: record.candidate_capture,
            pending: facts.counts,
            place: record.place,
            place_holds,
            tail: snapshot(&facts.tail)?,
            fork_points,
            items,
            unread_occupant,
            place_problem: match &facts.live {
                Live::Unavailable(error) => Some(error.clone()),
                _ => None,
            },
        })
    }
}

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

// ---------------------------------------------------------------------------
// Writing a decision: one transaction
// ---------------------------------------------------------------------------

impl Store {
    /// Writes `plan` in one transaction, after checking that the page is
    /// still exactly what the user saw (`page_token`, with the same look at
    /// the place). Returns the new watch when the record went on as the
    /// object at the place.
    #[allow(clippy::too_many_arguments)]
    fn write_decision(
        &self,
        chain_id: &str,
        page_token: &str,
        batch_id: &str,
        plan: &Plan,
        current_read: Option<&SeenFile>,
        live: Live,
        now: i64,
    ) -> Result<Option<WatchRow>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the decision", &err);
        let expected = match &live {
            Live::Unread { expected, .. } => expected.clone(),
            _ => None,
        };
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        let facts = page_facts(&tx, chain_id, live)?;
        if facts.token(chain_id) != page_token {
            return Err(page_outdated(
                "What the page showed changed meanwhile. Nothing was changed; open it again.",
            ));
        }
        let family_id = facts.family_id.clone();
        let items: HashMap<&str, &Item> = facts
            .items
            .iter()
            .map(|item| (item.candidate_id.as_str(), item))
            .collect();
        let mut target_decision: Option<(String, String)> = None; // (decision, file)
        for (candidate_id, planned) in &plan.items {
            let item = items
                .get(candidate_id.as_str())
                .ok_or_else(|| page_outdated("Open the page again."))?;
            let decision_id = insert_decision(
                &tx,
                chain_id,
                batch_id,
                candidate_id,
                planned,
                page_token,
                now,
            )?;
            apply_choice(
                &tx,
                &family_id,
                candidate_id,
                &item.file_id,
                &item.first,
                planned,
                &decision_id,
                now,
            )?;
            if plan.target == Some(WatchTarget::Item(candidate_id.clone())) {
                target_decision = Some((decision_id, item.file_id.clone()));
            }
        }
        let mut moved_to = None;
        match (&plan.current, current_read) {
            (Some(Planned::Independent), _) => {
                insert_decision_row(
                    &tx,
                    chain_id,
                    batch_id,
                    None,
                    "independent",
                    None,
                    page_token,
                    now,
                )?;
                end_gaps_in(
                    &tx,
                    chain_id,
                    &[GapKind::ReplacedUncaptured],
                    GapEnd::Decided,
                    now,
                )?;
            }
            (Some(Planned::Continue { parent, skipped }), Some(seen)) => {
                // The watch moves with the decision when the record is on.
                let watch = match plan.target {
                    Some(WatchTarget::Current) => facts.watch.as_ref(),
                    _ => None,
                };
                let continued = continue_current_in(
                    &tx,
                    chain_id,
                    &family_id,
                    seen,
                    &expected,
                    parent,
                    page_token,
                    ContinuedBy::User {
                        batch_id,
                        skipped: *skipped,
                    },
                    watch,
                    now,
                )?;
                moved_to = continued.watch;
            }
            (Some(_), _) => return Err(page_outdated("Open the page again.")),
            (None, _) => {}
        }
        if plan.lost {
            switch_off_in(&tx, chain_id, "occupant_independent", GapEnd::Decided, now)?;
            end_gaps_in(
                &tx,
                chain_id,
                &[GapKind::ReplacedUncaptured],
                GapEnd::Decided,
                now,
            )?;
            let tail = chain_tail_in(&tx, chain_id)?;
            insert_gap_in(
                &tx,
                chain_id,
                GapKind::Lost,
                tail.as_deref(),
                None,
                now,
                None,
            )?;
        }
        if let (Some((decision_id, file_id)), Some(old)) = (target_decision, &facts.watch) {
            moved_to = Some(move_watch_in(
                &tx,
                chain_id,
                old,
                &file_id,
                &decision_id,
                WentOn::Continued,
                now,
            )?);
        }
        let counts = pending_counts_in(&tx, chain_id)?;
        if counts.objects < CANDIDATE_LIMIT && counts.snapshots < CANDIDATE_LIMIT {
            end_gaps_in(&tx, chain_id, &[GapKind::Paused], GapEnd::Released, now)?;
        }
        tx.commit().map_err(fail)?;
        Ok(moved_to)
    }

    /// The automatic link (auto_link.rs) in one transaction: the record's
    /// watch `watch_id` is still on, the folder is still authorized, and the
    /// unread object at the place (`observed`, read as `seen`) goes straight
    /// on from the record's newest version on its line: no earlier object
    /// waits for a decision, and the link steps over no object the user left
    /// off the line (#31: a relation across one is only made after the user
    /// saw it). Then the same rows as the user's "continue" are written
    /// (`continue_current_in`), with status `confirmed_system`, and a
    /// `memory_auto_links` row marks the link as Traeva's. None, with
    /// nothing written, when any of this does not hold.
    pub(crate) fn link_at_place(
        &self,
        chain_id: &str,
        watch_id: &str,
        seen: &SeenFile,
        observed: &Observation,
        expected: &Option<(String, i64)>,
    ) -> Result<Option<Continued>, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the new version", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if !crate::family::watching_in(&tx, chain_id, watch_id)?
            || !crate::candidates::root_active_in(&tx, &seen.root_id)?
        {
            return Ok(None);
        }
        let facts = page_facts(&tx, chain_id, unread(observed, expected))?;
        let Some((parent, old)) = straight_on(&facts, watch_id) else {
            return Ok(None);
        };
        // The decision row needs a page token: this is what the page would
        // have shown at this moment.
        let token = facts.token(chain_id);
        let now = seen.now_ms;
        // A refusal (the place changed, the object is recorded elsewhere)
        // writes nothing: the transaction is dropped.
        let Ok(continued) = continue_current_in(
            &tx,
            chain_id,
            &facts.family_id,
            seen,
            expected,
            parent,
            &token,
            ContinuedBy::Itself,
            Some(old),
            now,
        ) else {
            return Ok(None);
        };
        tx.execute(
            "INSERT INTO memory_auto_links (edge_id, member_id, chain_id, decision_id,
                 linked_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                continued.edge_id,
                continued.member_id,
                chain_id,
                continued.decision_id,
                now
            ],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)?;
        Ok(Some(continued))
    }

    /// Whether the object at chain `chain_id`'s place (`observed`) could be
    /// linked by itself now, by the rule `link_at_place` checks again: asked
    /// before it is read, so an object that cannot be linked (an earlier one
    /// waits for a decision...) is not read again at every pass.
    pub(crate) fn could_link_at_place(
        &self,
        chain_id: &str,
        watch_id: &str,
        observed: &Observation,
        expected: &Option<(String, i64)>,
    ) -> Result<bool, AppError> {
        let conn = self.connection()?;
        let facts = page_facts(&conn, chain_id, unread(observed, expected))?;
        Ok(straight_on(&facts, watch_id).is_some())
    }
}

fn unread(observed: &Observation, expected: &Option<(String, i64)>) -> Live {
    Live::Unread {
        observed: observed.clone(),
        expected: expected.clone(),
    }
}

/// The parent an automatic link would continue from, and the record's
/// watch, when the unread object goes straight on from the record's newest
/// version on its line: no earlier object waits for a decision, and no
/// object the user left off the line is stepped over (#31: a relation
/// across one is only made after the user saw it). The watch must be
/// `watch_id`, the one the pass looked with.
fn straight_on<'a>(facts: &'a PageFacts, watch_id: &str) -> Option<(&'a String, &'a WatchRow)> {
    match (&facts.unread_continue, &facts.watch) {
        (Some((Some(parent), 0, None)), Some(old)) if old.watch_id == watch_id => {
            Some((parent, old))
        }
        _ => None,
    }
}

/// Who says the unread object at the place continues the record.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ContinuedBy<'a> {
    /// The user, on the confirmation page; `skipped`: the objects the
    /// relation steps over, as the page showed them.
    User { batch_id: &'a str, skipped: u32 },
    /// Traeva itself: a new object at the record's exact place, saved by
    /// the user's app (auto_link.rs).
    Itself,
}

/// What continuing the object at the place wrote.
#[derive(Debug, Clone)]
pub(crate) struct Continued {
    /// The object's first version (its new file record's).
    pub(crate) snapshot: SnapshotRef,
    pub(crate) decision_id: String,
    pub(crate) member_id: String,
    pub(crate) edge_id: String,
    /// The record's new watch, when its watch moved.
    pub(crate) watch: Option<WatchRow>,
}

/// The unread object at the place (`seen`, read just now) continues record
/// chain `chain_id` from version `parent`, inside the caller's transaction.
/// The user's "continue" and the automatic link both write through here,
/// so the two cannot drift. Refused, with an error, when the place no
/// longer holds the record `expected` it held when the object was looked
/// at (Organize may have chosen a file there meanwhile: it does not wait
/// for the execution lock), or the object already belongs to a Memory
/// record or work; the caller then drops the transaction. Otherwise:
/// - the object is registered (a new file record) and its first version
///   saved;
/// - the user's choice is kept with a candidate row (`read_on_decision`,
///   continued); Traeva's own link has none (it never waited for anyone);
/// - one `continue` decision, a membership (basis `user_decision`) and a
///   continuation from `parent`: `confirmed_user` for the user, and
///   `confirmed_system` for Traeva;
/// - an open `replaced_uncaptured` gap ends as captured. With none open,
///   the user's choice still records one (what the object replaced was not
///   seen); Traeva's own link records none: it followed the save itself;
/// - given the record's watch `watch`, the watch moves to the new file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn continue_current_in(
    tx: &Connection,
    chain_id: &str,
    family_id: &str,
    seen: &SeenFile,
    expected: &Option<(String, i64)>,
    parent: &str,
    page_token: &str,
    by: ContinuedBy,
    watch: Option<&WatchRow>,
    now: i64,
) -> Result<Continued, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot save the decision", &err);
    let present = query_files(
        tx,
        "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
        params![seen.root_id, seen.relative_path],
    )?
    .into_iter()
    .next()
    .map(|file| (file.file_id, file.record_revision));
    if &present != expected {
        return Err(page_outdated(
            "The file at the place was chosen again meanwhile. Nothing was changed; open the \
             page again.",
        ));
    }
    let tail_before = chain_tail_in(tx, chain_id)?;
    let file = observe_in(tx, seen)?;
    let fresh: bool = tx
        .query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM snapshots WHERE file_id = ?1)
                AND NOT EXISTS (SELECT 1 FROM memory_watches WHERE file_id = ?1
                                AND status = 'active')
                AND NOT EXISTS (SELECT 1 FROM memory_members WHERE file_id = ?1
                                AND status <> 'superseded')",
            [&file.file_id],
            |r| r.get(0),
        )
        .map_err(fail)?;
    if !fresh {
        return Err(conflict(
            "The object at the place belongs to another Memory record or work. Nothing was \
             changed.",
        ));
    }
    let (snapshot, _) = insert_next_snapshot(tx, &file.file_id, file.record_revision, seen)?;
    let (decision_id, status, skipped) = match by {
        ContinuedBy::User { batch_id, skipped } => {
            let candidate_id = uuid::Uuid::new_v4().to_string();
            // Only "continue" reads the object at the place.
            tx.execute(
                "INSERT INTO memory_candidates (candidate_id, chain_id, file_id, seq, reason,
                     after_snapshot_id, first_snapshot_id, state, found_at_ms, decided_at_ms)
                 VALUES (?1, ?2, ?3,
                         (SELECT coalesce(max(seq), 0) + 1 FROM memory_candidates
                          WHERE chain_id = ?2),
                         'read_on_decision', ?4, ?5, 'continued', ?6, ?6)",
                params![
                    candidate_id,
                    chain_id,
                    file.file_id,
                    tail_before,
                    snapshot.snapshot_id,
                    now
                ],
            )
            .map_err(fail)?;
            let decision_id = insert_decision_row(
                tx,
                chain_id,
                batch_id,
                Some(&candidate_id),
                "continue",
                Some(parent),
                page_token,
                now,
            )?;
            (decision_id, "confirmed_user", skipped)
        }
        ContinuedBy::Itself => {
            let batch_id = uuid::Uuid::new_v4().to_string();
            let decision_id = insert_decision_row(
                tx,
                chain_id,
                &batch_id,
                None,
                "continue",
                Some(parent),
                page_token,
                now,
            )?;
            (decision_id, "confirmed_system", 0)
        }
    };
    let member_id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO memory_members (member_id, family_id, file_id, status, basis,
             decision_id, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, 'user_decision', ?5, ?6, ?6)",
        params![member_id, family_id, file.file_id, status, decision_id, now],
    )
    .map_err(fail)?;
    let edge_id = insert_edge_row(
        tx,
        family_id,
        "continuation",
        Some(parent),
        &snapshot.snapshot_id,
        status,
        skipped,
        &decision_id,
        now,
    )?;
    tx.execute(
        "UPDATE memory_chains SET tail_snapshot_id = ?1 WHERE chain_id = ?2",
        params![snapshot.snapshot_id, chain_id],
    )
    .map_err(fail)?;
    let open: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM memory_gaps WHERE chain_id = ?1
                            AND kind = 'replaced_uncaptured' AND ended_at_ms IS NULL)",
            [chain_id],
            |r| r.get(0),
        )
        .map_err(fail)?;
    if open {
        end_gaps_in(
            tx,
            chain_id,
            &[GapKind::ReplacedUncaptured],
            GapEnd::Captured,
            now,
        )?;
    } else if matches!(by, ContinuedBy::User { .. }) {
        // What the object replaced is not known: the pair after it always
        // shows a gap.
        insert_gap_in(
            tx,
            chain_id,
            GapKind::ReplacedUncaptured,
            tail_before.as_deref(),
            None,
            now,
            Some((now, GapEnd::Captured)),
        )?;
    }
    let watch = match watch {
        Some(old) => Some(move_watch_in(
            tx,
            chain_id,
            old,
            &file.file_id,
            &decision_id,
            WentOn::Continued,
            now,
        )?),
        None => None,
    };
    Ok(Continued {
        snapshot,
        decision_id,
        member_id,
        edge_id,
        watch,
    })
}

fn choice_name(planned: &Planned) -> &'static str {
    match planned {
        Planned::Continue { .. } => "continue",
        Planned::Branch { .. } => "branch",
        Planned::Unknown => "unknown",
        Planned::Independent => "independent",
        Planned::Defer => "defer",
    }
}

fn parent_of(planned: &Planned) -> Option<&str> {
    match planned {
        Planned::Continue { parent, .. } => Some(parent),
        Planned::Branch { parent } => parent.as_deref(),
        _ => None,
    }
}

fn insert_decision(
    tx: &Connection,
    chain_id: &str,
    batch_id: &str,
    candidate_id: &str,
    planned: &Planned,
    page_token: &str,
    now: i64,
) -> Result<String, AppError> {
    insert_decision_row(
        tx,
        chain_id,
        batch_id,
        Some(candidate_id),
        choice_name(planned),
        parent_of(planned),
        page_token,
        now,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_decision_row(
    tx: &Connection,
    chain_id: &str,
    batch_id: &str,
    candidate_id: Option<&str>,
    choice: &str,
    parent: Option<&str>,
    page_token: &str,
    now: i64,
) -> Result<String, AppError> {
    let decision_id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO memory_decisions (decision_id, chain_id, batch_id, choice, candidate_id,
             parent_snapshot_id, page_token, decided_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            decision_id,
            chain_id,
            batch_id,
            choice,
            candidate_id,
            parent,
            page_token,
            now
        ],
    )
    .map_err(|err| sqlite_error("Cannot save the decision", &err))?;
    Ok(decision_id)
}

/// The candidate's state, its membership and its relation, for one choice.
#[allow(clippy::too_many_arguments)]
fn apply_choice(
    tx: &Connection,
    family_id: &str,
    candidate_id: &str,
    file_id: &str,
    first_snapshot_id: &str,
    planned: &Planned,
    decision_id: &str,
    now: i64,
) -> Result<(), AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot save the decision", &err);
    let (state, member) = match planned {
        Planned::Defer => return Ok(()),
        Planned::Continue { .. } => ("continued", "confirmed_user"),
        Planned::Branch { .. } => ("branch", "confirmed_user"),
        Planned::Unknown => ("unknown", "confirmed_user"),
        Planned::Independent => ("independent", "rejected"),
    };
    let changed = tx
        .execute(
            "UPDATE memory_candidates SET state = ?2, decided_at_ms = ?3
             WHERE candidate_id = ?1 AND state = 'pending'",
            params![candidate_id, state, now],
        )
        .map_err(fail)?;
    if changed != 1 {
        return Err(page_outdated("Open the page again."));
    }
    // The candidate's membership was written in the record's work, which
    // may have been merged into the live work since.
    let changed = tx
        .execute(
            "UPDATE memory_members SET status = ?3, decision_id = ?4, updated_at_ms = ?5
             WHERE family_id IN (SELECT family_id FROM memory_live_families
                                 WHERE live_family_id = ?1)
               AND file_id = ?2 AND status = 'candidate'",
            params![family_id, file_id, member, decision_id, now],
        )
        .map_err(fail)?;
    if changed != 1 {
        return Err(page_outdated("Open the page again."));
    }
    if !matches!(planned, Planned::Independent) {
        insert_edge(tx, family_id, planned, first_snapshot_id, decision_id, now)?;
    }
    Ok(())
}

fn insert_edge(
    tx: &Connection,
    family_id: &str,
    planned: &Planned,
    child: &str,
    decision_id: &str,
    now: i64,
) -> Result<(), AppError> {
    let (kind, parent, status, skipped) = match planned {
        Planned::Continue { parent, skipped } => (
            "continuation",
            Some(parent.as_str()),
            "confirmed_user",
            *skipped,
        ),
        Planned::Branch {
            parent: Some(parent),
        } => ("branch", Some(parent.as_str()), "confirmed_user", 0),
        Planned::Branch { parent: None } => ("branch", None, "unknown", 0),
        _ => ("unplaced", None, "unknown", 0),
    };
    insert_edge_row(
        tx,
        family_id,
        kind,
        parent,
        child,
        status,
        skipped,
        decision_id,
        now,
    )?;
    Ok(())
}

/// One `memory_edges` row; returns its ID.
#[allow(clippy::too_many_arguments)]
fn insert_edge_row(
    tx: &Connection,
    family_id: &str,
    kind: &str,
    parent: Option<&str>,
    child: &str,
    status: &str,
    skipped: u32,
    decision_id: &str,
    now: i64,
) -> Result<String, AppError> {
    let edge_id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO memory_edges (edge_id, family_id, kind, parent_snapshot_id, child_snapshot_id,
             status, skipped_items, decision_id, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            edge_id,
            family_id,
            kind,
            parent,
            child,
            status,
            skipped,
            decision_id,
            now
        ],
    )
    .map_err(|err| sqlite_error("Cannot save the decision", &err))?;
    Ok(edge_id)
}

#[cfg(test)]
#[path = "family_decide_tests.rs"]
mod tests;
