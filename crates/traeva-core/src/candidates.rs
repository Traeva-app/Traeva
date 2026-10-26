//! Candidate capture after a replacement (#31 material §3 and supplement;
//! shared-contract.md §4.2b).
//!
//! Apps such as TextEdit save by putting a new file in place of the old one.
//! Memory then sees another object at the record's place. Since the
//! student's 10/14 decision it first tries to record it as the next version
//! by itself (auto_link.rs). Only when that is not possible (too large, a
//! hard link, an earlier object waiting for a decision...) does what follows
//! apply. With candidate capture off (the default), it only reports that and
//! reads nothing. With it on, the new object is read the usual stable way,
//! registered as a new file record and saved as a **candidate**: a real
//! snapshot that confirms nothing. The user decides later (family_decide.rs)
//! whether it goes on from the work, is a branch, belongs to it in an
//! unknown order, or is unrelated.
//!
//! - The switch belongs to the record task (chain): only its one exact place
//!   counts, never the folder or an editor's temporary files. It is off until
//!   the user turns it on after a notice (`CANDIDATE_NOTICE_VERSION`), and
//!   turning it off is final for that period.
//! - Turning it on after a replacement already happened saves the object now
//!   at the place at once (D2): the notice must say so, and the page brings
//!   back a token of exactly the object it showed.
//! - Quota: each record task keeps at most 20 unprocessed candidate objects
//!   and 20 unprocessed candidate snapshots. Reaching either pauses capture:
//!   nothing is read, saved or queued, and the pause is a `paused` gap.
//!   Deciding candidates frees their share; nothing resets it otherwise.
//! - Everything one capture writes (the file record, the snapshot, the
//!   candidate, its membership, the counts and gaps) is one transaction, which
//!   first checks again that the watch is on, the switch period is the same,
//!   the folder is still authorized and the place still holds the object
//!   that was read.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::capture::{CheckOutcome, Look, Read, WatchRow, looks_different};
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    CANDIDATE_LIMIT, CandidateCapture, GapEnd, GapSignal, PendingCounts, PlaceView, RecordState,
    chain_tail_in, end_gaps_in, insert_gap_in, pending_counts_in,
};
use crate::memory_consent::memory_error;
use crate::memory_contracts::{GapKind, SnapshotRef};
use crate::observe;
use crate::platform::macos::Observation;
use crate::snapshot::{
    insert_next_snapshot, replaced, revoked_meanwhile, store_bytes, update_same_object_in,
    with_file,
};
use crate::store::{FileEvidence, FileRow, SeenFile, Store, observe_in, query_files, sqlite_error};
use crate::{OrganizeService, evidence_of, is_canonical_uuid, read_in_root, seen_file};

/// The version of the notice shown before candidate capture is turned on.
/// Raise it in the same PR that changes the notice's four points (B's #35
/// text): a page showing an older notice can then no longer turn it on.
pub const CANDIDATE_NOTICE_VERSION: u32 = 1;

/// What is at a record's place now, as the window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceHolds {
    /// The record's own file.
    RecordedFile,
    /// Nothing.
    Empty,
    /// A candidate of this record that waits for a decision.
    PendingCandidate,
    /// A candidate of this record the user already decided about.
    DecidedCandidate,
    /// An object Memory has not read.
    UnreadObject,
    /// A file recorded elsewhere (its own Memory record or work).
    OtherRecord,
    /// The place could not be looked at (`place_problem` says why).
    Unavailable,
    /// Not looked at: the record is stopped or lost.
    NotLooked,
}

/// The object at the place that Memory has not read. `occupant_token` is
/// opaque: the page brings it back exactly, so Traeva saves only the object
/// the user saw. Physical evidence never crosses IPC as numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OccupantView {
    pub occupant_token: String,
    pub byte_length: u64,
    pub modified_at_ms: i64,
}

/// What the notice before turning candidate capture on needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateCaptureInfo {
    pub chain_id: String,
    pub state: RecordState,
    pub candidate_capture: CandidateCapture,
    /// Bring it back when turning the switch on.
    pub notice_version: u32,
    /// Only this exact place: not its folder, not editors' temporary files.
    pub place: PlaceView,
    /// Bring it back when turning the switch on: the permission is for the
    /// place this notice showed. When the file was moved (by Organize or
    /// relocation) after it was shown, turning on is refused as
    /// `page_outdated` and the notice is shown again.
    pub place_token: String,
    pub pending: PendingCounts,
    pub place_holds: PlaceHolds,
    pub unread_occupant: Option<OccupantView>,
    /// Turning the switch on saves `unread_occupant` at once: the notice must
    /// say that this permission includes the object already at the place
    /// (D2). False when there is none, or the quota is full.
    pub saves_occupant_now: bool,
    pub place_problem: Option<AppError>,
    pub root_active: bool,
}

/// Turning candidate capture on or off. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSwitchRequest {
    pub chain_id: String,
    pub on: bool,
    /// Needed to turn it on: the `notice_version` the page showed.
    #[serde(default)]
    pub notice_version: Option<u32>,
    /// Exactly as `CandidateCaptureInfo.unread_occupant` showed it (absent
    /// when the page showed none).
    #[serde(default)]
    pub occupant_token: Option<String>,
    /// Needed to turn it on: the `place_token` the page showed.
    #[serde(default)]
    pub place_token: Option<String>,
}

/// The place a record looks at: where its watched file's record is.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub(crate) root_id: String,
    pub(crate) segments: Vec<String>,
    pub(crate) name: String,
    pub(crate) relative_path: String,
    pub(crate) shown: String,
}

/// Who the object at a place is, for the chain looking at it.
#[derive(Debug, Clone)]
pub(crate) enum Occupant {
    /// A new object: no file record, or one Organize registered that no
    /// record, snapshot or work has. `expected`: that record (file ID and
    /// revision), which must still be the one at the place when saving.
    New { expected: Option<(String, i64)> },
    /// A candidate of this chain that waits for a decision.
    PendingHere { candidate_id: String, file: FileRow },
    /// A candidate of this chain already decided about.
    DecidedHere { candidate_id: String, state: String },
    /// A file recorded elsewhere: never read here.
    OtherRecord,
}

/// The object a candidate capture saves.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    New {
        evidence: FileEvidence,
        expected: Option<(String, i64)>,
    },
    InPlace {
        candidate_id: String,
        file: FileRow,
    },
}

/// Which switch period a capture belongs to.
#[derive(Debug, Clone)]
pub(crate) enum SwitchMode {
    /// The period `switch_id`, which must still be on.
    Open(String),
    /// This capture turns the switch on (D2): the period is written with it.
    TurnOn { notice_version: u32 },
}

/// What one candidate capture transaction did.
pub(crate) enum CandidateRecorded {
    Saved(SnapshotRef, String),
    SameBytes(SnapshotRef),
    Paused,
    WatchStopped,
    SwitchOff,
    RootRevoked,
    PlaceChanged,
    OtherRecord,
}

impl OrganizeService {
    /// What the notice before turning candidate capture on needs: the exact
    /// place, the quota, and the unread object now at the place, if any.
    /// Looks at metadata only; reads nothing.
    pub fn candidate_capture(&self, chain_id: &str) -> Result<CandidateCaptureInfo, AppError> {
        let store = self.store()?;
        check_record_id(chain_id)?;
        let record = self.memory_record(chain_id)?;
        let mut info = CandidateCaptureInfo {
            chain_id: chain_id.to_string(),
            state: record.state,
            candidate_capture: record.candidate_capture,
            notice_version: CANDIDATE_NOTICE_VERSION,
            place_token: place_token(chain_id, &record.watch.watch_id, &record.place),
            place: record.place.clone(),
            pending: record.pending,
            place_holds: PlaceHolds::NotLooked,
            unread_occupant: None,
            saves_occupant_now: false,
            place_problem: None,
            root_active: record.root_active,
        };
        if record.state != RecordState::Active {
            return Ok(info);
        }
        let file_id = record.watch.file_id.as_str();
        match self.place_occupant(store, chain_id, file_id) {
            Err(problem) => {
                info.place_holds = PlaceHolds::Unavailable;
                info.place_problem = Some(problem);
            }
            Ok(seen) => {
                info.place_holds = seen.holds;
                if let Some(observed) = &seen.unread {
                    let saves_now = !full(&record.pending);
                    info.saves_occupant_now = saves_now;
                    info.unread_occupant = Some(OccupantView {
                        occupant_token: occupant_token(
                            chain_id,
                            &record.watch.watch_id,
                            &seen.place,
                            observed,
                            saves_now,
                        ),
                        byte_length: observed.size,
                        modified_at_ms: observed.mtime_ns / 1_000_000,
                    });
                }
            }
        }
        Ok(info)
    }

    /// Turns candidate capture on or off for record `chain_id`.
    ///
    /// Turning it on needs the notice version and the occupant token of the
    /// page: when the object at the place is no longer the one shown,
    /// `page_outdated` and nothing changes. An unread object at the place is
    /// read and saved as the first candidate in the same step (D2); if it
    /// cannot be read, the error comes back and the switch stays off. Saving
    /// it confirms nothing. A stopped or lost record is refused with
    /// `record_stopped`. Turning it off ends the period for good; the
    /// candidates and counts stay.
    pub fn set_candidate_capture(
        &self,
        request: &CandidateSwitchRequest,
    ) -> Result<CandidateCaptureInfo, AppError> {
        self.set_candidate_capture_with(request, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn set_candidate_capture_with(
        &self,
        request: &CandidateSwitchRequest,
        read: Read,
    ) -> Result<CandidateCaptureInfo, AppError> {
        let store = self.store()?;
        check_record_id(&request.chain_id)?;
        let chain_id = request.chain_id.as_str();
        store
            .chain(chain_id)?
            .ok_or_else(crate::family::unknown_record)?;
        if !request.on {
            let _no_capture = self.lock_execution();
            store.turn_switch_off(chain_id, "user", GapEnd::Stopped, self.now())?;
            return self.candidate_capture(chain_id);
        }
        if request.notice_version != Some(CANDIDATE_NOTICE_VERSION) {
            return Err(page_outdated(
                "The notice shown before turning candidate capture on has changed. Open it again.",
            ));
        }
        let no_capture = self.lock_execution();
        self.ensure_run();
        let record = self.memory_record(chain_id)?;
        if record.state != RecordState::Active {
            return Err(memory_error(
                ErrorCode::RecordStopped,
                "Memory is off for this record, or it is lost. Candidate capture is not turned on.",
                false,
                None,
            ));
        }
        let watch_id = record.watch.watch_id.clone();
        // The permission is for the place the notice showed: the file may
        // have been moved since (C's #37 finding, #65 issuecomment-6102864317).
        if request.place_token.as_deref()
            != Some(place_token(chain_id, &watch_id, &record.place).as_str())
        {
            return Err(page_outdated(
                "This file's place changed since the notice was shown, or the notice did not \
                 name it. Nothing was changed; read the notice again.",
            ));
        }
        let seen = self.place_occupant(store, chain_id, &record.watch.file_id)?;
        let saves_now = seen.unread.is_some() && !full(&record.pending);
        let token_now = seen
            .unread
            .as_ref()
            .map(|observed| occupant_token(chain_id, &watch_id, &seen.place, observed, saves_now));
        if request.occupant_token != token_now {
            return Err(page_outdated(
                "The object at this file's place changed since the notice was shown. Nothing was \
                 changed; open it again.",
            ));
        }
        if record.candidate_capture != CandidateCapture::Off {
            drop(no_capture);
            return self.candidate_capture(chain_id);
        }
        let now = self.now();
        match (&seen.unread, &seen.occupant) {
            (Some(observed), Some(Occupant::New { expected })) if saves_now => {
                let target = Target::New {
                    evidence: evidence_of(observed),
                    expected: expected.clone(),
                };
                let mode = SwitchMode::TurnOn {
                    notice_version: CANDIDATE_NOTICE_VERSION,
                };
                self.no_unsettled_action(store, &seen.place.root_id, &record.watch.file_id)?;
                let (stable, seen_file) =
                    self.read_candidate(&seen.place, &target, read)
                        .map_err(|error| match error.code {
                            ErrorCode::FileChanging => page_outdated(
                                "The object at this file's place changed while it was read. \
                             Nothing was changed; open the notice again.",
                            ),
                            _ => error,
                        })?;
                store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes)?;
                match store.record_candidate(
                    chain_id,
                    &watch_id,
                    &mode,
                    &target,
                    &seen_file,
                    "present_when_enabled",
                )? {
                    CandidateRecorded::Saved(..) => {}
                    CandidateRecorded::PlaceChanged | CandidateRecorded::OtherRecord => {
                        return Err(page_outdated(
                            "The object at this file's place changed while it was saved. \
                             Nothing was changed; open the notice again.",
                        ));
                    }
                    CandidateRecorded::RootRevoked => {
                        return Err(revoked_meanwhile(&seen.place.shown));
                    }
                    _ => {
                        return Err(memory_error(
                            ErrorCode::RecordStopped,
                            "Memory was turned off for this record meanwhile. Candidate capture \
                             is not turned on.",
                            false,
                            None,
                        ));
                    }
                }
            }
            _ => store.turn_switch_on(chain_id, &watch_id, CANDIDATE_NOTICE_VERSION, now)?,
        }
        drop(no_capture);
        self.candidate_capture(chain_id)
    }

    /// One look of the watcher found another object at chain `chain_id`'s
    /// place (`observed`): what it is; a new object becomes the next version
    /// when it can (auto_link.rs), otherwise, when candidate capture is on
    /// and the quota has room, the candidate it becomes.
    pub(crate) fn at_replaced_place(
        &self,
        watch: &WatchRow,
        chain_id: &str,
        observed: &Observation,
        read: Read,
    ) -> (CheckOutcome, GapSignal) {
        let file_id = watch.file_id.as_str();
        let problem = |error: AppError| CheckOutcome::Problem {
            error: with_file(error, file_id),
        };
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => return (problem(error), GapSignal::Nothing),
        };
        let place = match self.place_of(file_id) {
            Ok(place) => place,
            Err(error) => return (problem(error), GapSignal::Nothing),
        };
        let facts = (|| -> Result<_, AppError> {
            let conn = store.connection()?;
            let occupant = occupant_at(&conn, chain_id, &place, &evidence_of(observed))?;
            let switch = open_switch_in(&conn, chain_id)?;
            let counts = pending_counts_in(&conn, chain_id)?;
            Ok((occupant, switch, counts))
        })();
        let (occupant, switch, counts) = match facts {
            Ok(facts) => facts,
            Err(error) => return (problem(error), GapSignal::Nothing),
        };
        // A new object at the exact place: an app's save, linked by itself
        // when it can be (auto_link.rs); otherwise as before.
        if let Occupant::New { expected } = &occupant
            && let Some(done) =
                self.link_at_place(watch, chain_id, &place, observed, expected, &mut *read)
        {
            return done;
        }
        let replaced_here = || problem(replaced(&place.shown));
        match (occupant, switch) {
            (Occupant::PendingHere { candidate_id, file }, switch) => {
                let latest = store.latest_sha256(&file.file_id).ok().flatten();
                if !looks_different(&file, observed, latest.as_deref()) {
                    return (CheckOutcome::Unchanged, GapSignal::Covered { saved: false });
                }
                match switch {
                    None => (
                        problem(AppError::observation(
                            ErrorCode::FileChanged,
                            format!(
                                "{} is a candidate that changed while candidate capture was \
                                 off, so the change was not saved.",
                                place.shown
                            ),
                        )),
                        GapSignal::Replaced,
                    ),
                    Some(_) if full(&counts) => self.paused(store, chain_id, &watch.watch_id),
                    Some(switch_id) => self.capture_candidate(
                        watch,
                        chain_id,
                        &place,
                        Target::InPlace { candidate_id, file },
                        switch_id,
                        read,
                    ),
                }
            }
            (Occupant::DecidedHere { state, .. }, _) if state == "independent" => (
                problem(AppError::observation(
                    ErrorCode::FileChanged,
                    format!(
                        "{} holds a file you judged unrelated to this work. Memory does not \
                         record it here.",
                        place.shown
                    ),
                )),
                GapSignal::Replaced,
            ),
            (Occupant::DecidedHere { .. }, _) => (
                problem(AppError::observation(
                    ErrorCode::FileChanged,
                    format!(
                        "{} holds another file of the same work. To record it, stop this record \
                         and turn Memory on for that file.",
                        place.shown
                    ),
                )),
                GapSignal::Nothing,
            ),
            (Occupant::OtherRecord, _) => (
                problem(AppError::observation(
                    ErrorCode::FileChanged,
                    format!(
                        "{} now holds a file that is recorded elsewhere. Memory does not read it \
                         here.",
                        place.shown
                    ),
                )),
                GapSignal::Replaced,
            ),
            (Occupant::New { .. }, None) => (replaced_here(), GapSignal::Replaced),
            (Occupant::New { .. }, Some(_)) if full(&counts) => {
                self.paused(store, chain_id, &watch.watch_id)
            }
            (Occupant::New { expected }, Some(switch_id)) => self.capture_candidate(
                watch,
                chain_id,
                &place,
                Target::New {
                    evidence: evidence_of(observed),
                    expected,
                },
                switch_id,
                read,
            ),
        }
    }

    /// The quota is full: nothing is read, and the pause is a gap.
    fn paused(&self, store: &Store, chain_id: &str, watch_id: &str) -> (CheckOutcome, GapSignal) {
        let _ = store.open_paused_while_watching(chain_id, watch_id, self.now());
        (CheckOutcome::CandidatePaused, GapSignal::Nothing)
    }

    /// Reads and saves `target` as a candidate of chain `chain_id`, under the
    /// execution lock.
    fn capture_candidate(
        &self,
        watch: &WatchRow,
        chain_id: &str,
        place: &Place,
        target: Target,
        switch_id: String,
        read: Read,
    ) -> (CheckOutcome, GapSignal) {
        let file_id = watch.file_id.as_str();
        let problem = |error: AppError| CheckOutcome::Problem {
            error: with_file(error, file_id),
        };
        let _no_run = self.lock_execution();
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => return (problem(error), GapSignal::Nothing),
        };
        // Under the lock, before anything is read: the record may have been
        // stopped, its switch turned off, the work judged lost, the quota
        // filled or the candidate decided while this pass waited for it.
        let ready = (|| -> Result<Option<(CheckOutcome, GapSignal)>, AppError> {
            let conn = store.connection()?;
            if !watching(&conn, chain_id, &watch.watch_id)? {
                return Ok(Some((CheckOutcome::Stopped, GapSignal::Nothing)));
            }
            if open_switch_in(&conn, chain_id)?.as_deref() != Some(switch_id.as_str()) {
                return Ok(Some((problem(replaced(&place.shown)), GapSignal::Replaced)));
            }
            if full(&pending_counts_in(&conn, chain_id)?) {
                return Ok(Some((CheckOutcome::CandidatePaused, GapSignal::Nothing)));
            }
            if let Target::InPlace { candidate_id, .. } = &target
                && !candidate_waits_in(&conn, candidate_id)?
            {
                return Ok(Some((CheckOutcome::Unchanged, GapSignal::Nothing)));
            }
            Ok(None)
        })();
        match ready {
            Ok(None) => {}
            Ok(Some(done)) => return done,
            Err(error) => return (problem(error), GapSignal::Nothing),
        }
        // Nothing is read while an action in this folder is not settled: the
        // object at the place may be one Organize was moving.
        if let Err(error) = self.no_unsettled_action(store, &place.root_id, file_id) {
            let signal = GapSignal::failed(error.code, false);
            return (problem(error), signal);
        }
        let (stable, seen) = match self.read_candidate(place, &target, read) {
            Ok(read) => read,
            Err(error) => {
                let signal = GapSignal::failed(error.code, false);
                return (problem(error), signal);
            }
        };
        if let Err(error) = store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes) {
            let signal = GapSignal::failed(error.code, false);
            return (problem(error), signal);
        }
        let reason = "replaced";
        let recorded = store.record_candidate(
            chain_id,
            &watch.watch_id,
            &SwitchMode::Open(switch_id),
            &target,
            &seen,
            reason,
        );
        match recorded {
            Ok(CandidateRecorded::Saved(snapshot, candidate_id)) => (
                CheckOutcome::CandidateSaved {
                    snapshot,
                    candidate_id,
                },
                GapSignal::Covered { saved: true },
            ),
            Ok(CandidateRecorded::SameBytes(snapshot)) => (
                CheckOutcome::SameBytes { snapshot },
                GapSignal::Covered { saved: false },
            ),
            Ok(CandidateRecorded::Paused) => (CheckOutcome::CandidatePaused, GapSignal::Nothing),
            Ok(CandidateRecorded::WatchStopped) => (CheckOutcome::Stopped, GapSignal::Nothing),
            Ok(CandidateRecorded::SwitchOff) => {
                (problem(replaced(&place.shown)), GapSignal::Replaced)
            }
            Ok(CandidateRecorded::RootRevoked) => (
                problem(revoked_meanwhile(&place.shown)),
                GapSignal::failed(ErrorCode::RootRevoked, false),
            ),
            Ok(CandidateRecorded::PlaceChanged) => (
                problem(AppError::observation(
                    ErrorCode::FileChanging,
                    format!(
                        "{} changed while Memory saved it, so nothing was saved. Memory tries \
                         again.",
                        place.shown
                    ),
                )),
                GapSignal::Nothing,
            ),
            Ok(CandidateRecorded::OtherRecord) => (
                problem(AppError::observation(
                    ErrorCode::FileChanged,
                    format!(
                        "{} now holds a file that is recorded elsewhere. Memory does not save it \
                         here.",
                        place.shown
                    ),
                )),
                GapSignal::Replaced,
            ),
            Err(error) => (problem(error), GapSignal::Nothing),
        }
    }

    /// Reads the object at `place` the stable way, through the root handle,
    /// and checks it is `target`'s object.
    pub(crate) fn read_candidate(
        &self,
        place: &Place,
        target: &Target,
        read: Read,
    ) -> Result<(observe::StableRead, SeenFile), AppError> {
        let (_root_row, root) = self.verified_root(&place.root_id)?;
        let stable = read_in_root(&root, &place.segments, &place.name, &place.shown, read)?;
        let wanted = match target {
            Target::New { evidence, .. } => evidence,
            Target::InPlace { file, .. } => &file.evidence,
        };
        if !evidence_of(&stable.observed).same_object(wanted) {
            return Err(AppError::observation(
                ErrorCode::FileChanging,
                format!(
                    "{} was replaced again while Memory read it, so nothing was saved. Memory \
                     tries again.",
                    place.shown
                ),
            ));
        }
        let seen = seen_file(
            &place.root_id,
            &place.segments,
            &place.name,
            &stable,
            self.now(),
        );
        Ok((stable, seen))
    }

    /// The place file `file_id`'s record says it is at.
    pub(crate) fn place_of(&self, file_id: &str) -> Result<Place, AppError> {
        let store = self.store()?;
        let (file, _) = store
            .get_file(file_id)?
            .ok_or_else(crate::snapshot::unknown_file)?;
        let root = store
            .get_root(&file.root_id)?
            .ok_or_else(crate::snapshot::unknown_file)?;
        let mut segments: Vec<String> = file.relative_path.split('/').map(str::to_string).collect();
        let name = segments.pop().unwrap_or_default();
        Ok(Place {
            shown: format!("{}/{}", root.display_path, file.relative_path),
            root_id: file.root_id,
            relative_path: file.relative_path,
            segments,
            name,
        })
    }

    /// A look at chain `chain_id`'s place (where file `file_id`'s record is):
    /// what it holds, and the unread object if there is one.
    pub(crate) fn place_occupant(
        &self,
        store: &Store,
        chain_id: &str,
        file_id: &str,
    ) -> Result<PlaceSeen, AppError> {
        let place = self.place_of(file_id)?;
        let (_shown, look) = self.look_at_place(file_id)?;
        let observed = match &look {
            Look::Other { observed } => Some(observed.clone()),
            _ => None,
        };
        let (holds, unread, occupant) = match look {
            Look::Watched { .. } => (PlaceHolds::RecordedFile, None, None),
            Look::Empty => (PlaceHolds::Empty, None, None),
            Look::Other { observed } => {
                let occupant = {
                    let conn = store.connection()?;
                    occupant_at(&conn, chain_id, &place, &evidence_of(&observed))?
                };
                match occupant {
                    Occupant::New { .. } => {
                        (PlaceHolds::UnreadObject, Some(observed), Some(occupant))
                    }
                    Occupant::PendingHere { .. } => {
                        (PlaceHolds::PendingCandidate, None, Some(occupant))
                    }
                    Occupant::DecidedHere { .. } => {
                        (PlaceHolds::DecidedCandidate, None, Some(occupant))
                    }
                    Occupant::OtherRecord => (PlaceHolds::OtherRecord, None, Some(occupant)),
                }
            }
        };
        Ok(PlaceSeen {
            place,
            holds,
            unread,
            occupant,
            observed,
        })
    }

    /// Refuses with `unresolved_action` while an action in root `root_id`
    /// is not settled (it may have left an object at the place).
    pub(crate) fn no_unsettled_action(
        &self,
        store: &Store,
        root_id: &str,
        file_id: &str,
    ) -> Result<(), AppError> {
        match store.unsettled_action_in_root(root_id)? {
            Some(action) => Err(crate::execute::unresolved(file_id, None, &action)),
            None => Ok(()),
        }
    }

    pub(crate) fn lock_execution(&self) -> std::sync::MutexGuard<'_, ()> {
        self.execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One look at a record's place.
pub(crate) struct PlaceSeen {
    pub(crate) place: Place,
    pub(crate) holds: PlaceHolds,
    /// The object Memory has not read, when that is what is there.
    pub(crate) unread: Option<Observation>,
    pub(crate) occupant: Option<Occupant>,
    /// Any object that is not the record's file (read or not).
    pub(crate) observed: Option<Observation>,
}

fn full(counts: &PendingCounts) -> bool {
    counts.objects >= CANDIDATE_LIMIT || counts.snapshots >= CANDIDATE_LIMIT
}

/// A token of exactly the unread object the page showed, and whether
/// turning the switch on saves it now.
pub(crate) fn occupant_token(
    chain_id: &str,
    watch_id: &str,
    place: &Place,
    observed: &Observation,
    saves_now: bool,
) -> String {
    let evidence = evidence_of(observed);
    observe::sha256_hex(
        format!(
            "memory-occupant-1\n{chain_id}\n{watch_id}\n{}\n{}\n{}\n{}\n{}\n{}",
            place.root_id,
            place.relative_path,
            evidence.dev,
            evidence.ino,
            evidence.birthtime_ns,
            u8::from(saves_now)
        )
        .as_bytes(),
    )
}

/// The place a candidate capture notice names, for record `chain_id`
/// watching with `watch_id`.
fn place_token(chain_id: &str, watch_id: &str, place: &PlaceView) -> String {
    observe::sha256_hex(
        format!(
            "memory-capture-place-1\n{chain_id}\n{watch_id}\n{}\n{}",
            place.root_id, place.relative_path
        )
        .as_bytes(),
    )
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

pub(crate) fn page_outdated(message: &str) -> AppError {
    memory_error(ErrorCode::PageOutdated, message, true, None)
}

/// Who the object at `place` is for chain `chain_id`, by its evidence.
pub(crate) fn occupant_at(
    conn: &Connection,
    chain_id: &str,
    place: &Place,
    evidence: &FileEvidence,
) -> Result<Occupant, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let present = query_files(
        conn,
        "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
        params![place.root_id, place.relative_path],
    )?
    .into_iter()
    .next();
    // The same object recorded at another place (moved here outside
    // Traeva): never a new object when that record is kept elsewhere. SQL
    // narrows by inode and birth time; `same_object` decides (never `dev`).
    let elsewhere: Option<String> = query_files(
        conn,
        "WHERE presence = 'present'
           AND json_extract(observation_json, '$.ino') = ?1
           AND json_extract(observation_json, '$.birthtime_ns') = ?2
           AND NOT (root_id = ?3 AND relative_path = ?4)",
        params![
            evidence.ino as i64,
            evidence.birthtime_ns,
            place.root_id,
            place.relative_path
        ],
    )?
    .into_iter()
    .find(|file| file.evidence.same_object(evidence))
    .map(|file| file.file_id);
    if let Some(other) = elsewhere
        && recorded_elsewhere(conn, chain_id, &other)?
    {
        return Ok(Occupant::OtherRecord);
    }
    let Some(file) = present.filter(|file| file.evidence.same_object(evidence)) else {
        // No record of this object (a record there is of another object,
        // and becomes missing when this one is registered).
        let expected = query_files(
            conn,
            "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
            params![place.root_id, place.relative_path],
        )?
        .into_iter()
        .next()
        .map(|file| (file.file_id, file.record_revision));
        return Ok(Occupant::New { expected });
    };
    let candidate: Option<(String, String)> = conn
        .query_row(
            "SELECT candidate_id, state FROM memory_candidates WHERE chain_id = ?1 AND file_id = ?2",
            params![chain_id, file.file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(fail)?;
    if let Some((candidate_id, state)) = candidate {
        return Ok(if state == "pending" {
            Occupant::PendingHere { candidate_id, file }
        } else {
            Occupant::DecidedHere {
                candidate_id,
                state,
            }
        });
    }
    if recorded_elsewhere(conn, chain_id, &file.file_id)? {
        return Ok(Occupant::OtherRecord);
    }
    Ok(Occupant::New {
        expected: Some((file.file_id, file.record_revision)),
    })
}

/// File `file_id` has a watch that is on, a snapshot, a place in any work, or
/// was judged unrelated in chain `chain_id`'s work: it is not a new object.
pub(crate) fn recorded_elsewhere(
    conn: &Connection,
    chain_id: &str,
    file_id: &str,
) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM memory_watches WHERE file_id = ?2 AND status = 'active')
             OR EXISTS (SELECT 1 FROM snapshots WHERE file_id = ?2)
             OR EXISTS (SELECT 1 FROM memory_members WHERE file_id = ?2
                        AND status IN ('confirmed_system', 'confirmed_user', 'disputed'))
             OR EXISTS (SELECT 1 FROM memory_members AS m
                        JOIN memory_live_families AS ml ON ml.family_id = m.family_id
                        WHERE ml.live_family_id = ?3 AND m.file_id = ?2
                          AND m.status <> 'superseded')",
        params![
            chain_id,
            file_id,
            crate::works::live_family_of_chain_in(conn, chain_id)?
        ],
        |r| r.get(0),
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// Candidate `candidate_id` still waits for a decision.
fn candidate_waits_in(conn: &Connection, candidate_id: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM memory_candidates WHERE candidate_id = ?1
                        AND state = 'pending')",
        [candidate_id],
        |r| r.get(0),
    )
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

pub(crate) fn open_switch_in(
    conn: &Connection,
    chain_id: &str,
) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT switch_id FROM memory_candidate_switches
         WHERE chain_id = ?1 AND disabled_at_ms IS NULL",
        [chain_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read the Memory records", &err))
}

/// Watch `watch_id` is chain `chain_id`'s watch that is on and the chain is
/// not lost (as family.rs checks before writing a gap).
fn watching(conn: &Connection, chain_id: &str, watch_id: &str) -> Result<bool, AppError> {
    crate::family::watching_in(conn, chain_id, watch_id)
}

pub(crate) fn root_active_in(conn: &Connection, root_id: &str) -> Result<bool, AppError> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM authorized_roots WHERE root_id = ?1",
            [root_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the folders", &err))?;
    Ok(status.as_deref() == Some("active"))
}

impl Store {
    /// An action from or into root `root_id` that is not settled.
    pub(crate) fn unsettled_action_in_root(
        &self,
        root_id: &str,
    ) -> Result<Option<String>, AppError> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT a.action_id FROM actions AS a JOIN plans AS p ON p.plan_id = a.plan_id
             WHERE (p.source_root_id = ?1 OR p.target_root_id = ?1)
               AND a.status IN ('prepared', 'running', 'needs_user_decision')
             ORDER BY a.created_at_ms LIMIT 1",
            [root_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the actions", &err))
    }

    /// Records one candidate capture in one transaction (see the module
    /// note). Nothing is written when the watch is off or lost, the switch
    /// period changed, the folder was removed from Traeva, or the place no
    /// longer holds the record that was there when the object was looked at.
    pub(crate) fn record_candidate(
        &self,
        chain_id: &str,
        watch_id: &str,
        mode: &SwitchMode,
        target: &Target,
        seen: &SeenFile,
        reason: &str,
    ) -> Result<CandidateRecorded, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the candidate", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if !watching(&tx, chain_id, watch_id)? {
            return Ok(CandidateRecorded::WatchStopped);
        }
        let switch_now = open_switch_in(&tx, chain_id)?;
        match (mode, &switch_now) {
            (SwitchMode::Open(id), Some(now)) if id == now => {}
            (SwitchMode::TurnOn { .. }, None) => {}
            _ => return Ok(CandidateRecorded::SwitchOff),
        }
        if !root_active_in(&tx, &seen.root_id)? {
            return Ok(CandidateRecorded::RootRevoked);
        }
        let present = query_files(
            &tx,
            "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
            params![seen.root_id, seen.relative_path],
        )?
        .into_iter()
        .next()
        .map(|file| (file.file_id, file.record_revision));
        let expected = match target {
            Target::New { expected, .. } => expected.clone(),
            Target::InPlace { file, .. } => Some((file.file_id.clone(), file.record_revision)),
        };
        if present != expected {
            return Ok(CandidateRecorded::PlaceChanged);
        }
        let now = seen.now_ms;
        if full(&pending_counts_in(&tx, chain_id)?) {
            if matches!(mode, SwitchMode::TurnOn { .. }) {
                return Ok(CandidateRecorded::SwitchOff);
            }
            open_paused_in(&tx, chain_id, now)?;
            tx.commit().map_err(fail)?;
            return Ok(CandidateRecorded::Paused);
        }
        let tail_before = chain_tail_in(&tx, chain_id)?;
        let (file_id, revision, candidate_id) = match target {
            Target::New { .. } => {
                let file = observe_in(&tx, seen)?;
                let fresh: bool = tx
                    .query_row(
                        "SELECT NOT EXISTS (SELECT 1 FROM memory_watches WHERE file_id = ?1
                                            AND status = 'active')
                            AND NOT EXISTS (SELECT 1 FROM snapshots WHERE file_id = ?1)",
                        [&file.file_id],
                        |r| r.get(0),
                    )
                    .map_err(fail)?;
                if !fresh || recorded_elsewhere(&tx, chain_id, &file.file_id)? {
                    return Ok(CandidateRecorded::OtherRecord);
                }
                (file.file_id, file.record_revision, None)
            }
            Target::InPlace { candidate_id, file } => {
                // Decided meanwhile (a branch or unknown stays where it is,
                // and is not recorded here any more).
                if !candidate_waits_in(&tx, candidate_id)? {
                    return Ok(CandidateRecorded::PlaceChanged);
                }
                let revision = update_same_object_in(
                    &tx,
                    &file.file_id,
                    file.record_revision,
                    &file.evidence,
                    &file.sha256,
                    file.byte_length as i64,
                    seen,
                )?;
                (file.file_id.clone(), revision, Some(candidate_id.clone()))
            }
        };
        let (snapshot, created) = insert_next_snapshot(&tx, &file_id, revision, seen)?;
        if !created {
            tx.commit().map_err(fail)?;
            return Ok(CandidateRecorded::SameBytes(snapshot));
        }
        let candidate_id = match candidate_id {
            Some(id) => id,
            None => {
                let candidate_id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO memory_candidates (candidate_id, chain_id, file_id, seq, reason,
                         after_snapshot_id, first_snapshot_id, state, found_at_ms)
                     VALUES (?1, ?2, ?3,
                             (SELECT coalesce(max(seq), 0) + 1 FROM memory_candidates
                              WHERE chain_id = ?2),
                             ?4, ?5, ?6, 'pending', ?7)",
                    params![
                        candidate_id,
                        chain_id,
                        file_id,
                        reason,
                        tail_before,
                        snapshot.snapshot_id,
                        now
                    ],
                )
                .map_err(fail)?;
                // The record's work now: that of the file it watches.
                let family_id = crate::works::live_family_of_chain_in(&tx, chain_id)?
                    .ok_or_else(crate::family::unknown_record)?;
                tx.execute(
                    "INSERT INTO memory_members (member_id, family_id, file_id, status, basis,
                         created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, 'candidate', 'candidate_capture', ?4, ?4)",
                    params![uuid::Uuid::new_v4().to_string(), family_id, file_id, now],
                )
                .map_err(fail)?;
                candidate_id
            }
        };
        if let SwitchMode::TurnOn { notice_version } = mode {
            tx.execute(
                "INSERT INTO memory_candidate_switches (switch_id, chain_id, notice_version,
                     enabled_at_ms, included_file_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    chain_id,
                    notice_version,
                    now,
                    file_id
                ],
            )
            .map_err(fail)?;
            // The object was at the place before this permission: what it
            // replaced is not known, so the pair after it always shows a gap.
            let open_replaced: bool = tx
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM memory_gaps WHERE chain_id = ?1
                                    AND kind = 'replaced_uncaptured' AND ended_at_ms IS NULL)",
                    [chain_id],
                    |r| r.get(0),
                )
                .map_err(fail)?;
            if !open_replaced {
                insert_gap_in(
                    &tx,
                    chain_id,
                    GapKind::ReplacedUncaptured,
                    tail_before.as_deref(),
                    None,
                    now,
                    Some((now, GapEnd::Captured)),
                )?;
            }
        }
        tx.execute(
            "UPDATE memory_chains SET tail_snapshot_id = ?1 WHERE chain_id = ?2",
            params![snapshot.snapshot_id, chain_id],
        )
        .map_err(fail)?;
        end_gaps_in(
            &tx,
            chain_id,
            &[GapKind::CaptureFailed, GapKind::ReplacedUncaptured],
            GapEnd::Captured,
            now,
        )?;
        if full(&pending_counts_in(&tx, chain_id)?) {
            open_paused_in(&tx, chain_id, now)?;
        }
        tx.commit().map_err(fail)?;
        Ok(CandidateRecorded::Saved(snapshot, candidate_id))
    }

    /// Turns candidate capture on for chain `chain_id` without saving
    /// anything; when the quota is already full, the pause starts with it.
    pub(crate) fn turn_switch_on(
        &self,
        chain_id: &str,
        watch_id: &str,
        notice_version: u32,
        now: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot turn candidate capture on", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if !watching(&tx, chain_id, watch_id)? || open_switch_in(&tx, chain_id)?.is_some() {
            return Ok(());
        }
        tx.execute(
            "INSERT INTO memory_candidate_switches (switch_id, chain_id, notice_version,
                 enabled_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                uuid::Uuid::new_v4().to_string(),
                chain_id,
                notice_version,
                now
            ],
        )
        .map_err(fail)?;
        if full(&pending_counts_in(&tx, chain_id)?) {
            open_paused_in(&tx, chain_id, now)?;
        }
        tx.commit().map_err(fail)
    }

    /// Ends chain `chain_id`'s switch period that is on, with `reason`, and
    /// the pause with `pause_end`.
    pub(crate) fn turn_switch_off(
        &self,
        chain_id: &str,
        reason: &str,
        pause_end: GapEnd,
        now: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot turn candidate capture off", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        switch_off_in(&tx, chain_id, reason, pause_end, now)?;
        tx.commit().map_err(fail)
    }

    /// Starts a `paused` gap if watch `watch_id` is still chain `chain_id`'s
    /// watch that is on, its switch is on and the quota is full.
    pub(crate) fn open_paused_while_watching(
        &self,
        chain_id: &str,
        watch_id: &str,
        now: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save a Memory gap", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if watching(&tx, chain_id, watch_id)?
            && open_switch_in(&tx, chain_id)?.is_some()
            && full(&pending_counts_in(&tx, chain_id)?)
        {
            open_paused_in(&tx, chain_id, now)?;
        }
        tx.commit().map_err(fail)
    }
}

/// Ends the chain's switch period that is on, and its pause.
pub(crate) fn switch_off_in(
    conn: &Connection,
    chain_id: &str,
    reason: &str,
    pause_end: GapEnd,
    now: i64,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE memory_candidate_switches SET disabled_at_ms = ?2, disabled_reason = ?3
         WHERE chain_id = ?1 AND disabled_at_ms IS NULL",
        params![chain_id, now, reason],
    )
    .map_err(|err| sqlite_error("Cannot turn candidate capture off", &err))?;
    end_gaps_in(conn, chain_id, &[GapKind::Paused], pause_end, now)
}

/// A `paused` gap after the chain's newest version, unless one is open.
pub(crate) fn open_paused_in(conn: &Connection, chain_id: &str, now: i64) -> Result<(), AppError> {
    let open: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM memory_gaps WHERE chain_id = ?1 AND kind = 'paused'
                            AND ended_at_ms IS NULL)",
            [chain_id],
            |r| r.get(0),
        )
        .map_err(|err| sqlite_error("Cannot save a Memory gap", &err))?;
    if !open {
        let tail = chain_tail_in(conn, chain_id)?;
        insert_gap_in(
            conn,
            chain_id,
            GapKind::Paused,
            tail.as_deref(),
            None,
            now,
            None,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
pub(crate) mod tests;
