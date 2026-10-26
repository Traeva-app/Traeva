//! Associating a file with a work by hand (#31 material §2 source 4 and its
//! supplement, §5, §6, §7 "Save As"; shared-contract.md §4.3a).
//!
//! The user chooses a file (in the picker, or one Traeva already knows, such
//! as the file a same-content hint is about) and a work, and says how they
//! are related. The page shows the file, the work, what Traeva knows (the
//! same bytes as a version of the work, the same physical object as one of
//! its files; never a relation by itself) and the choices the file's
//! standing allows:
//!   not_recorded     no saved version, in no work: "same work" (membership
//!                    only: a relation needs a real version) or
//!                    "independent". Memory is not turned on for it; once it
//!                    is, the file can be associated again to place it.
//!   saved_no_work    saved versions, in no work: continue, branch, same
//!                    work with unknown order, or independent.
//!   in_other_work    a member of another work: continue, branch and
//!                    unknown merge that whole work into this one (a file is
//!                    a member of one work at a time); independent changes
//!                    nothing there.
//!   member_without_parent  a member of this work whose first version has no
//!                    known parent (for example after a merge): continue or
//!                    branch place it.
//!
//! "Continue" goes on from the last version of a member whose line did not
//! go on yet. The record whose line ends there stops (material §5: the old
//! watch stops), and no watch starts or moves: the file keeps its own
//! record, if it has one, and Memory is turned on for it separately (§2:
//! associating never turns Memory on). "Branch" starts at any version of a
//! member saved before the file's first version. Both are confirmed by the
//! user, and Memory saw nothing between the two versions: their pair always
//! shows a gap (`skipped`).
//!
//! Only a picked file with no saved version is read: once, stably, for its
//! SHA-256 (supplement §2). The bytes are not kept and no version is saved.
//! A file with versions is never read: its first version is the relation's
//! end. Only hashes are compared, locally.
//!
//! Refused, writing nothing: a waiting candidate or the unread object at a
//! record's place (decide it in that record), the moved file of a record
//! (choose its new place there), a member of this work that is placed
//! already (revoke its relation first) or has no version (turn Memory on
//! for it first).
//!
//! Confirming writes one work decision with every row it made or changed
//! (`memory_work_changes`), in one transaction, after looking at everything
//! again under the execution lock: when anything the page showed changed,
//! nothing is written.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::candidates::page_outdated;
use crate::capture::Read;
use crate::contracts::{AppError, ErrorCode};
use crate::family::{
    EdgeKind, MemoryFamilyView, MemoryRecordView, PlaceView, RelationStatus, candidate_pending,
    goes_on_in, latest_snapshot_in, latest_watch_of_chain_in, line_front_in, stop_chain_in,
    to_text,
};
use crate::family_merge::{
    OverrideView, WorkDecisionOutcome, check_ids, check_token, override_views, overrides_in,
};
use crate::memory_consent::memory_error;
use crate::memory_contracts::SnapshotRef;
use crate::observe::{self, StableRead};
use crate::platform::macos::{self, Kind, Observation};
use crate::snapshot::{query_snapshots, unknown_file};
use crate::store::{FileRow, Store, observe_in, query_files, sqlite_error};
use crate::works::{
    GROUP, WorkDecisionKind, WorkDecisionRow, insert_edge_in, insert_member_in,
    insert_work_decision_in, live_family_in, set_edge_status_in, set_member_status_in,
    unknown_family, work_token_lines,
};
use crate::{
    OrganizeService, evidence_of, file_error, is_canonical_uuid, read_in_root, relative_location,
    seen_file,
};

/// How the chosen file stands; it decides the choices (module note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStanding {
    NotRecorded,
    SavedNoWork,
    InOtherWork,
    MemberWithoutParent,
}

/// One way the file may be related to the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociateChoiceKind {
    Continue,
    Branch,
    Unknown,
    Independent,
}

/// The user's choice. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssociateChoice {
    /// From one of `AssociationPreview.continue_from`.
    Continue {
        parent_snapshot_id: String,
    },
    /// From one of `AssociationPreview.fork_points`.
    Branch {
        parent_snapshot_id: String,
    },
    // Empty braces, not unit variants: serde ignores unknown fields next to
    // the tag of a unit variant.
    Unknown {},
    Independent {},
}

/// Which file the page is about. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssociationSubject {
    /// The file the picker returned, exactly as the preview's
    /// `subject.place` showed it.
    Picked {
        root_id: String,
        relative_path: String,
    },
    /// A file Traeva already knows.
    File { file_id: String },
}

/// The chosen file, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssociationSubjectView {
    /// Null for a picked file Traeva does not know yet: confirming registers
    /// it (no version is saved).
    pub file_id: Option<String>,
    pub place: PlaceView,
    /// The SHA-256 compared: of the picked file's one read, or of its first
    /// saved version; null for a known file with no version.
    pub sha256: Option<String>,
    pub versions: Vec<SnapshotRef>,
}

/// What Traeva knows; never a relation by itself (material §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssociationEvidence {
    /// Versions of the work's members with exactly the file's bytes.
    pub same_bytes_as: Vec<SnapshotRef>,
    /// Member files of the work that are the same physical object.
    pub same_object_as: Vec<String>,
}

/// One version "continue" may go on from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContinueOption {
    /// The last version of a member whose line did not go on yet.
    pub parent: SnapshotRef,
    /// The record whose line ends at it (on or lost): it stops when the
    /// file continues it.
    pub stops_record: Option<MemoryRecordView>,
}

/// What associating the file with the work would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssociationPreview {
    /// The work (live).
    pub family_id: String,
    pub subject: AssociationSubjectView,
    /// Send it back unchanged as `AssociationRequest.subject`: a picked
    /// file is confirmed by its place (and looked at again), a known file
    /// by its ID.
    pub request_subject: AssociationSubject,
    pub standing: FileStanding,
    /// `in_other_work`: the file's own work. Continue, branch and unknown
    /// merge all of it into this one (`merge_scope`).
    pub other_family_id: Option<String>,
    pub merge_scope: Option<MemoryFamilyView>,
    /// The file has no saved version: only its membership can be confirmed.
    /// Turn Memory on for it, then associate it again to place it.
    pub needs_recording: bool,
    pub evidence: AssociationEvidence,
    pub choices: Vec<AssociateChoiceKind>,
    pub continue_from: Vec<ContinueOption>,
    pub fork_points: Vec<SnapshotRef>,
    /// The relation "same work, order unknown" of the file's first version:
    /// continue and branch replace it.
    pub replaces_unknown_placement: Option<String>,
    /// Earlier "unrelated" judgements continue, branch and unknown make stop
    /// counting.
    pub overrides: Vec<OverrideView>,
    /// Bring it back to confirm.
    pub page_token: String,
}

/// Confirming an association page. Unknown fields are refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssociationRequest {
    pub family_id: String,
    pub subject: AssociationSubject,
    pub choice: AssociateChoice,
    pub page_token: String,
}

/// The chosen file as looked at.
struct Seen {
    /// Its file record: a known file, or the present record of the picked
    /// object at its path.
    file: Option<FileRow>,
    place: PlaceView,
    /// A picked file: the object at its path (as read, when it was read).
    observed: Option<Observation>,
    segments: Vec<String>,
    name: String,
    /// The one read of a picked file with no version.
    read: Option<StableRead>,
}

/// Everything the page shows, worked out on one connection.
struct Facts {
    live: String,
    file_id: Option<String>,
    versions: Vec<SnapshotRef>,
    sha256: Option<String>,
    standing: FileStanding,
    other: Option<String>,
    /// (member, file, work) of earlier "unrelated" judgements.
    overrides: Vec<(String, String, String)>,
    unknown_placement: Option<String>,
    /// Parent versions and the record each one stops.
    continue_from: Vec<(SnapshotRef, Option<String>)>,
    fork_points: Vec<SnapshotRef>,
    same_bytes: Vec<SnapshotRef>,
    same_object: Vec<String>,
    choices: Vec<AssociateChoiceKind>,
    token: String,
}

impl OrganizeService {
    /// Looks at the file the user chose in the picker (`picked`, in root
    /// `root_id`) as a file to associate with work `family_id`. Reads the
    /// file only when it has no saved version (for its SHA-256); writes
    /// nothing.
    pub fn association_preview(
        &self,
        family_id: &str,
        root_id: &str,
        picked: &Path,
    ) -> Result<AssociationPreview, AppError> {
        self.association_preview_with(family_id, root_id, picked, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn association_preview_with(
        &self,
        family_id: &str,
        root_id: &str,
        picked: &Path,
        read: Read,
    ) -> Result<AssociationPreview, AppError> {
        let store = self.store()?;
        check_ids(&[family_id, root_id])?;
        let _no_capture = self.lock_execution();
        let seen = self.look_at_picked(store, root_id, picked, read)?;
        let subject = AssociationSubject::Picked {
            root_id: root_id.to_string(),
            relative_path: seen.place.relative_path.clone(),
        };
        self.preview_of(store, family_id, seen, subject)
    }

    /// The same page for a file Traeva already knows (`file_id`). Reads no
    /// user file and writes nothing.
    pub fn association_preview_for_file(
        &self,
        family_id: &str,
        file_id: &str,
    ) -> Result<AssociationPreview, AppError> {
        let store = self.store()?;
        check_ids(&[family_id])?;
        let _no_capture = self.lock_execution();
        let seen = look_at_file(store, file_id)?;
        let subject = AssociationSubject::File {
            file_id: file_id.to_string(),
        };
        self.preview_of(store, family_id, seen, subject)
    }

    /// Associates the file with the work as the page showed it (see the
    /// module note).
    pub fn associate_file(
        &self,
        request: &AssociationRequest,
    ) -> Result<WorkDecisionOutcome, AppError> {
        self.associate_file_with(request, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn associate_file_with(
        &self,
        request: &AssociationRequest,
        read: Read,
    ) -> Result<WorkDecisionOutcome, AppError> {
        let store = self.store()?;
        check_ids(&[&request.family_id])?;
        check_token(&request.page_token)?;
        let (kind, parent) = match &request.choice {
            AssociateChoice::Continue { parent_snapshot_id } => {
                (AssociateChoiceKind::Continue, Some(parent_snapshot_id))
            }
            AssociateChoice::Branch { parent_snapshot_id } => {
                (AssociateChoiceKind::Branch, Some(parent_snapshot_id))
            }
            AssociateChoice::Unknown {} => (AssociateChoiceKind::Unknown, None),
            AssociateChoice::Independent {} => (AssociateChoiceKind::Independent, None),
        };
        if parent.is_some_and(|p| !is_canonical_uuid(p)) {
            return Err(invalid("The saved version ID is not valid."));
        }
        let _no_capture = self.lock_execution();
        let seen = match &request.subject {
            AssociationSubject::Picked {
                root_id,
                relative_path,
            } => {
                check_ids(&[root_id])?;
                // Only a plain relative path, as the preview gave it.
                if relative_path.is_empty()
                    || relative_path.starts_with('/')
                    || relative_path.contains('\0')
                    || relative_path
                        .split('/')
                        .any(|part| part.is_empty() || part == "." || part == "..")
                {
                    return Err(invalid("The file is not the one the page showed."));
                }
                let root = store
                    .get_root(root_id)?
                    .ok_or_else(|| invalid("The file is not the one the page showed."))?;
                let picked = PathBuf::from(&root.display_path).join(relative_path);
                let seen = self.look_at_picked(store, root_id, &picked, read)?;
                if seen.place.relative_path != *relative_path {
                    return Err(invalid("The file is not the one the page showed."));
                }
                seen
            }
            AssociationSubject::File { file_id } => look_at_file(store, file_id)?,
        };
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the association", &err);
        let now = self.now();
        let work_decision_id = uuid::Uuid::new_v4().to_string();
        let (live, stopped) = {
            let mut conn = store.connection()?;
            let tx = conn.transaction().map_err(fail)?;
            let facts = association_facts(&tx, &request.family_id, &seen)?;
            if facts.token != request.page_token {
                return Err(page_outdated(
                    "The file or the work changed since the page was opened. Nothing was \
                     changed; open it again.",
                ));
            }
            if !facts.choices.contains(&kind) {
                return Err(conflict(
                    "The page did not offer this choice for this file.",
                ));
            }
            let offered = match kind {
                AssociateChoiceKind::Continue => facts
                    .continue_from
                    .iter()
                    .any(|(p, _)| Some(&p.snapshot_id) == parent),
                AssociateChoiceKind::Branch => facts
                    .fork_points
                    .iter()
                    .any(|p| Some(&p.snapshot_id) == parent),
                _ => true,
            };
            if !offered {
                return Err(conflict("The page did not offer this version."));
            }
            let file_id = match (&facts.file_id, &seen.read) {
                (Some(file_id), _) => file_id.clone(),
                (None, Some(stable)) => register_in(&tx, &seen, stable, now)?,
                (None, None) => return Err(unknown_file()),
            };
            let positive = kind != AssociateChoiceKind::Independent;
            let merging = positive && facts.standing == FileStanding::InOtherWork;
            let evidence = if !facts.same_bytes.is_empty() {
                "same_bytes"
            } else if !facts.same_object.is_empty() {
                "same_object"
            } else {
                "none"
            };
            insert_work_decision_in(
                &tx,
                &WorkDecisionRow {
                    work_decision_id: &work_decision_id,
                    kind: WorkDecisionKind::Associate,
                    family_id: &facts.live,
                    other_family_id: if merging {
                        facts.other.as_deref()
                    } else {
                        None
                    },
                    edge_id: None,
                    file_id: Some(&file_id),
                    choice: Some(kind),
                    parent_snapshot_id: parent.map(String::as_str),
                    evidence: Some(evidence),
                    page_token: &request.page_token,
                },
                now,
            )?;
            if merging && let Some(other) = &facts.other {
                tx.execute(
                    "INSERT INTO memory_family_merges (absorbed_family_id, into_family_id,
                         work_decision_id, merged_at_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![other, facts.live, work_decision_id, now],
                )
                .map_err(fail)?;
            }
            if positive {
                for (member_id, _, _) in &facts.overrides {
                    set_member_status_in(
                        &tx,
                        &work_decision_id,
                        member_id,
                        RelationStatus::Rejected,
                        RelationStatus::Superseded,
                        now,
                    )?;
                }
            }
            let child = facts.versions.first().map(|s| s.snapshot_id.as_str());
            let places = match kind {
                AssociateChoiceKind::Continue | AssociateChoiceKind::Branch => true,
                AssociateChoiceKind::Unknown => facts.standing == FileStanding::SavedNoWork,
                AssociateChoiceKind::Independent => false,
            };
            // One live placement per version: the unknown one goes first.
            if places && let Some(edge_id) = &facts.unknown_placement {
                set_edge_status_in(
                    &tx,
                    &work_decision_id,
                    edge_id,
                    RelationStatus::Unknown,
                    RelationStatus::Superseded,
                    now,
                )?;
            }
            match facts.standing {
                FileStanding::NotRecorded | FileStanding::SavedNoWork => {
                    let status = if positive {
                        RelationStatus::ConfirmedUser
                    } else {
                        RelationStatus::Rejected
                    };
                    insert_member_in(
                        &tx,
                        &work_decision_id,
                        &facts.live,
                        &file_id,
                        status,
                        crate::family::MemberBasis::UserDecision,
                        now,
                    )?;
                }
                FileStanding::InOtherWork if !positive => {
                    insert_member_in(
                        &tx,
                        &work_decision_id,
                        &facts.live,
                        &file_id,
                        RelationStatus::Rejected,
                        crate::family::MemberBasis::UserDecision,
                        now,
                    )?;
                }
                _ => {}
            }
            let mut stopped = Vec::new();
            if places && let Some(child) = child {
                let (edge_kind, status) = match kind {
                    AssociateChoiceKind::Continue => {
                        (EdgeKind::Continuation, RelationStatus::ConfirmedUser)
                    }
                    AssociateChoiceKind::Branch => {
                        (EdgeKind::Branch, RelationStatus::ConfirmedUser)
                    }
                    _ => (EdgeKind::Unplaced, RelationStatus::Unknown),
                };
                insert_edge_in(
                    &tx,
                    &work_decision_id,
                    &facts.live,
                    edge_kind,
                    parent.map(String::as_str),
                    child,
                    status,
                    now,
                )?;
            }
            if kind == AssociateChoiceKind::Continue
                && let Some((_, Some(chain_id))) = facts
                    .continue_from
                    .iter()
                    .find(|(p, _)| Some(&p.snapshot_id) == parent)
            {
                stop_chain_in(&tx, chain_id, now)?;
                stopped.push(chain_id.clone());
            }
            tx.commit().map_err(fail)?;
            (facts.live, stopped)
        };
        if !stopped.is_empty() {
            // As in `stop_memory_record`: the watcher was running until now.
            self.note_run_seen(true);
        }
        Ok(WorkDecisionOutcome {
            work_decision_id,
            kind: WorkDecisionKind::Associate,
            family: self.memory_family(&live)?,
            other_family_id: None,
            stopped_records: stopped,
            held_records: Vec::new(),
        })
    }

    /// The object at the picked path, its file record if Traeva knows it,
    /// and, when it has no saved version, one stable read for its SHA-256.
    fn look_at_picked(
        &self,
        store: &Store,
        root_id: &str,
        picked: &Path,
        read: Read,
    ) -> Result<Seen, AppError> {
        let (root_row, folder) = self.verified_root(root_id)?;
        let shown = picked.display().to_string();
        let (segments, name) = relative_location(&root_row.display_path, picked)?;
        let names: Vec<&str> = segments.iter().map(String::as_str).collect();
        let observed = macos::open_chain(&folder, &names)
            .and_then(|chain| macos::observe(chain.last().unwrap_or(&folder), &name))
            .map_err(|err| file_error(&shown, err))?;
        if observed.kind != Kind::File {
            return Err(AppError::observation(
                ErrorCode::UnsupportedFile,
                format!("{shown} is not a file."),
            ));
        }
        let mut relative_path = segments.join("/");
        if !relative_path.is_empty() {
            relative_path.push('/');
        }
        relative_path.push_str(&name);
        let (file, has_versions) = {
            let conn = store.connection()?;
            let file = query_files(
                &conn,
                "WHERE root_id = ?1 AND relative_path = ?2 AND presence = 'present'",
                params![root_id, relative_path],
            )?
            .into_iter()
            .find(|row| row.evidence.same_object(&evidence_of(&observed)));
            let has_versions = match &file {
                Some(file) => {
                    !query_snapshots(&conn, "WHERE file_id = ?1 LIMIT 1", [&file.file_id])?
                        .is_empty()
                }
                None => false,
            };
            (file, has_versions)
        };
        // Nothing is read while an action in the folder is not settled: the
        // file may be one Organize is moving.
        if let Some(action) = store.unsettled_action_in_root(root_id)? {
            let mut err = crate::execute::unresolved("", None, &action);
            err.file_id = file.as_ref().map(|f| f.file_id.clone());
            return Err(err);
        }
        let (observed, stable) = if has_versions {
            (observed, None)
        } else {
            let stable = read_in_root(&folder, &segments, &name, &shown, read)?;
            if !evidence_of(&stable.observed).same_object(&evidence_of(&observed)) {
                return Err(AppError::observation(
                    ErrorCode::FileChanging,
                    format!("{shown} was replaced while Traeva read it. Choose it again."),
                ));
            }
            (stable.observed.clone(), Some(stable))
        };
        Ok(Seen {
            file,
            place: PlaceView {
                display_path: format!("{}/{relative_path}", root_row.display_path),
                root_id: root_id.to_string(),
                relative_path,
            },
            observed: Some(observed),
            segments,
            name,
            read: stable,
        })
    }

    fn preview_of(
        &self,
        store: &Store,
        family_id: &str,
        seen: Seen,
        request_subject: AssociationSubject,
    ) -> Result<AssociationPreview, AppError> {
        let facts = {
            let conn = store.connection()?;
            association_facts(&conn, family_id, &seen)?
        };
        let mut continue_from = Vec::new();
        for (parent, chain_id) in &facts.continue_from {
            continue_from.push(ContinueOption {
                parent: parent.clone(),
                stops_record: match chain_id {
                    Some(chain_id) => Some(self.memory_record(chain_id)?),
                    None => None,
                },
            });
        }
        Ok(AssociationPreview {
            request_subject,
            subject: AssociationSubjectView {
                file_id: facts.file_id.clone(),
                place: seen.place,
                sha256: facts.sha256.clone(),
                versions: facts.versions.clone(),
            },
            standing: facts.standing,
            merge_scope: match &facts.other {
                Some(other) => Some(self.memory_family(other)?),
                None => None,
            },
            other_family_id: facts.other.clone(),
            needs_recording: facts.versions.is_empty(),
            evidence: AssociationEvidence {
                same_bytes_as: facts.same_bytes.clone(),
                same_object_as: facts.same_object.clone(),
            },
            choices: facts.choices.clone(),
            continue_from,
            fork_points: facts.fork_points.clone(),
            replaces_unknown_placement: facts.unknown_placement.clone(),
            overrides: override_views(store, &facts.overrides)?,
            page_token: facts.token,
            family_id: facts.live,
        })
    }
}

/// A known file: its record and place. Nothing is read.
fn look_at_file(store: &Store, file_id: &str) -> Result<Seen, AppError> {
    if !is_canonical_uuid(file_id) {
        return Err(invalid("The file ID is not valid."));
    }
    let (file, _) = store.get_file(file_id)?.ok_or_else(unknown_file)?;
    let root = store.get_root(&file.root_id)?;
    let display_path = match &root {
        Some(root) => format!("{}/{}", root.display_path, file.relative_path),
        None => file.relative_path.clone(),
    };
    Ok(Seen {
        place: PlaceView {
            root_id: file.root_id.clone(),
            relative_path: file.relative_path.clone(),
            display_path,
        },
        file: Some(file),
        observed: None,
        segments: Vec::new(),
        name: String::new(),
        read: None,
    })
}

/// What the page shows about `seen` and work `family_id` (module note),
/// and its token. Refusals are returned as errors.
fn association_facts(conn: &Connection, family_id: &str, seen: &Seen) -> Result<Facts, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the Memory records", &err);
    let live = live_family_in(conn, family_id)?.ok_or_else(unknown_family)?;
    let file_id = seen.file.as_ref().map(|f| f.file_id.clone());
    let versions = match &file_id {
        Some(id) => query_snapshots(conn, "WHERE file_id = ?1 ORDER BY sequence", [id])?,
        None => Vec::new(),
    };
    // A candidate waiting for a decision, or an object Memory never read
    // at the place of a record that is on (that record decides about it),
    // is decided there.
    let waits: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM memory_candidates WHERE file_id = ?1
                            AND state = 'pending')
                 OR ?4 AND EXISTS (
                    SELECT 1 FROM memory_chain_watches AS link
                    JOIN memory_watches AS w ON w.watch_id = link.watch_id
                                            AND w.status = 'active'
                    JOIN files AS watched ON watched.file_id = w.file_id
                    WHERE watched.file_id IS NOT ?1
                      AND watched.root_id = ?2 AND watched.relative_path = ?3
                      AND NOT EXISTS (SELECT 1 FROM memory_gaps AS g
                                      WHERE g.chain_id = link.chain_id AND g.kind = 'lost'
                                        AND g.ended_at_ms IS NULL))",
            params![
                file_id,
                seen.place.root_id,
                seen.place.relative_path,
                versions.is_empty()
            ],
            |r| r.get(0),
        )
        .map_err(fail)?;
    if waits {
        return Err(match &file_id {
            Some(id) => candidate_pending(id),
            None => memory_error(
                ErrorCode::CandidatePending,
                "This file is at the place a Memory record looks at. Decide about it in that \
                 record, or stop that record first.",
                false,
                None,
            ),
        });
    }
    // A picked file that is the file a record watches, moved outside
    // Traeva: its record chooses the new place (relocation).
    if let Some(observed) = &seen.observed {
        let watched = query_files(
            conn,
            "WHERE file_id IN (SELECT file_id FROM memory_watches WHERE status = 'active')",
            [],
        )?;
        if watched.iter().any(|row| {
            Some(&row.file_id) != file_id.as_ref()
                && evidence_of(observed).same_object(&row.evidence)
        }) {
            return Err(conflict(
                "This is the file a Memory record watches: it was moved or renamed outside \
                 Traeva. Choose its new place in that record instead.",
            ));
        }
    }
    // The file's memberships: its work (confirmed or disputed), and whether
    // it was judged unrelated to this work.
    let rows: Vec<(String, String)> = match &file_id {
        Some(id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT l.live_family_id, m.status FROM memory_members AS m
                     JOIN memory_live_families AS l ON l.family_id = m.family_id
                     WHERE m.file_id = ?1 AND m.status <> 'superseded'",
                )
                .map_err(fail)?;
            stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(fail)?
                .collect::<Result<_, _>>()
                .map_err(fail)?
        }
        None => Vec::new(),
    };
    let home = rows.iter().find(|(_, status)| {
        ["confirmed_system", "confirmed_user", "disputed"].contains(&status.as_str())
    });
    if home.is_some_and(|(_, status)| status == "disputed") {
        return Err(conflict(
            "This file's membership is disputed. Correct it first; nothing was changed.",
        ));
    }
    let rejected_here = rows
        .iter()
        .any(|(work, status)| work == &live && status == "rejected");
    // The live placement of the file's first version, if any.
    let placement: Option<(String, String)> = match versions.first() {
        Some(first) => conn
            .query_row(
                "SELECT edge_id, status FROM memory_edges WHERE child_snapshot_id = ?1
                 AND status IN ('confirmed_system', 'confirmed_user', 'disputed', 'unknown')",
                [&first.snapshot_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(fail)?,
        None => None,
    };
    let placed = placement.as_ref().is_some_and(|(_, s)| s != "unknown");
    let unknown_placement = placement
        .as_ref()
        .filter(|(_, s)| s == "unknown")
        .map(|(id, _)| id.clone());
    let (standing, other) = match home {
        Some((work, _)) if work == &live => {
            if versions.is_empty() {
                return Err(conflict(
                    "This file is in this work already, with no saved version to place. Turn \
                     Memory on for it first.",
                ));
            }
            if placed {
                return Err(conflict(
                    "This file already has its place in this work. Revoke its relation first; \
                     nothing was changed.",
                ));
            }
            (FileStanding::MemberWithoutParent, None)
        }
        Some((work, _)) => (FileStanding::InOtherWork, Some(work.clone())),
        None if versions.is_empty() => (FileStanding::NotRecorded, None),
        None if placed => {
            return Err(conflict(
                "This file's first version already has a parent. Revoke that relation first; \
                 nothing was changed.",
            ));
        }
        None => (FileStanding::SavedNoWork, None),
    };
    // Where it can go: versions of this work's confirmed members (before a
    // merge: never of its own work) saved before the file's first version.
    let mut continue_from = Vec::new();
    let mut fork_points = Vec::new();
    if let Some(first) = versions.first().filter(|_| !placed) {
        fork_points = query_snapshots(
            conn,
            &format!(
                "WHERE file_id IN (SELECT file_id FROM memory_members WHERE family_id IN {GROUP}
                                   AND status IN ('confirmed_system', 'confirmed_user'))
                   AND file_id <> ?2
                   AND rowid < (SELECT rowid FROM snapshots WHERE snapshot_id = ?3)
                 ORDER BY rowid"
            ),
            params![live, first.file_id, first.snapshot_id],
        )?;
        // Records of the work that are on (or lost), and the file their
        // line ends at.
        let mut fronts: Vec<(String, String)> = Vec::new();
        let chains: Vec<String> = crate::family::family_facts_in(conn, &live)?
            .chains
            .into_iter()
            .map(|c| c.chain_id)
            .collect();
        for chain_id in chains {
            if let Some(watch) = latest_watch_of_chain_in(conn, &chain_id)?
                && watch.active
            {
                fronts.push((line_front_in(conn, &watch.file_id)?, chain_id));
            }
        }
        for version in &fork_points {
            let last = latest_snapshot_in(conn, &version.file_id)?.as_deref()
                == Some(version.snapshot_id.as_str());
            if last && !goes_on_in(conn, &version.snapshot_id)? {
                let stops = fronts
                    .iter()
                    .find(|(front, _)| front == &version.file_id)
                    .map(|(_, chain_id)| chain_id.clone());
                continue_from.push((version.clone(), stops));
            }
        }
    }
    // Earlier "unrelated" judgements a positive choice contradicts: of this
    // file here; and, when its work merges in, every one between the two.
    let overrides = match &other {
        Some(other) => overrides_in(conn, &live, other)?,
        None => match &file_id {
            Some(id) => {
                let mut stmt = conn
                    .prepare(&format!(
                        "SELECT member_id, file_id, family_id FROM memory_members
                         WHERE family_id IN {GROUP} AND file_id = ?2 AND status = 'rejected'
                         ORDER BY member_id"
                    ))
                    .map_err(fail)?;
                stmt.query_map(params![live, id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(fail)?
                    .collect::<Result<_, _>>()
                    .map_err(fail)?
            }
            None => Vec::new(),
        },
    };
    // Evidence: the same bytes (never for empty content: every empty file
    // has the same hash) and the same physical object.
    let read_hash = seen.read.as_ref().map(|r| r.sha256.clone());
    let sha256 = read_hash.or_else(|| versions.first().map(|s| s.sha256.clone()));
    let empty = match (&seen.read, versions.first()) {
        (Some(read), _) => read.bytes.is_empty(),
        (None, Some(first)) => first.byte_length == 0,
        (None, None) => true,
    };
    let same_bytes = match &sha256 {
        Some(sha256) if !empty => query_snapshots(
            conn,
            &format!(
                "WHERE sha256 = ?2 AND byte_length > 0 AND file_id IS NOT ?3
                   AND file_id IN (SELECT file_id FROM memory_members WHERE family_id IN {GROUP}
                                   AND status IN ('confirmed_system', 'confirmed_user'))
                 ORDER BY rowid LIMIT 20"
            ),
            params![live, sha256, file_id],
        )?,
        _ => Vec::new(),
    };
    let evidence = match (&seen.observed, &seen.file) {
        (Some(observed), _) => Some(evidence_of(observed)),
        (None, Some(file)) => Some(file.evidence.clone()),
        (None, None) => None,
    };
    let same_object = match &evidence {
        Some(evidence) => query_files(
            conn,
            &format!(
                "WHERE file_id IN (SELECT file_id FROM memory_members WHERE family_id IN {GROUP}
                                   AND status IN ('confirmed_system', 'confirmed_user'))
                   AND file_id IS NOT ?2
                 ORDER BY file_id"
            ),
            params![live, file_id],
        )?
        .into_iter()
        .filter(|row| row.evidence.same_object(evidence))
        .map(|row| row.file_id)
        .collect(),
        None => Vec::new(),
    };
    let mut choices = Vec::new();
    if !continue_from.is_empty() {
        choices.push(AssociateChoiceKind::Continue);
    }
    if !fork_points.is_empty() {
        choices.push(AssociateChoiceKind::Branch);
    }
    if standing != FileStanding::MemberWithoutParent {
        choices.push(AssociateChoiceKind::Unknown);
        if !rejected_here {
            choices.push(AssociateChoiceKind::Independent);
        }
    }
    if choices.is_empty() {
        return Err(conflict(
            "Nothing in this work was saved before this file's first version, so there is no \
             version to place it after.",
        ));
    }
    // The object: as read when it was read; otherwise only which physical
    // object it is (its saves do not make the page outdated).
    let object = match (&seen.observed, &seen.read) {
        (Some(o), Some(_)) => format!(
            "{}:{}:{}:{}:{}:{}:{}",
            o.id.dev, o.id.ino, o.birthtime_ns, o.size, o.mtime_ns, o.ctime_ns, o.link_count
        ),
        (Some(o), None) => format!("{}:{}:{}", o.id.dev, o.id.ino, o.birthtime_ns),
        (None, _) => "-".to_string(),
    };
    let ids = |list: &[SnapshotRef]| {
        list.iter()
            .map(|s| s.snapshot_id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut lines = vec![
        "memory-associate-1".to_string(),
        format!(
            "subject:{}:{}:{}:{object}:{}",
            file_id.as_deref().unwrap_or("-"),
            seen.place.root_id,
            seen.place.relative_path,
            sha256.as_deref().unwrap_or("-")
        ),
        // Only the first version is a relation's end: saving the file again
        // does not make the page outdated.
        format!(
            "first:{}",
            versions.first().map_or("-", |s| s.snapshot_id.as_str())
        ),
        format!(
            "standing:{}:{}",
            to_text(standing),
            other.as_deref().unwrap_or("-")
        ),
        format!("placement:{}", unknown_placement.as_deref().unwrap_or("-")),
        format!("fork:{}", ids(&fork_points)),
        format!(
            "choices:{}",
            choices
                .iter()
                .map(|c| to_text(*c))
                .collect::<Vec<_>>()
                .join(",")
        ),
        format!("evidence:{}:{}", ids(&same_bytes), same_object.join(",")),
    ];
    lines.extend(continue_from.iter().map(|(p, stops)| {
        format!(
            "continue:{}:{}",
            p.snapshot_id,
            stops.as_deref().unwrap_or("-")
        )
    }));
    lines.extend(overrides.iter().map(|(id, _, _)| format!("override:{id}")));
    lines.extend(
        work_token_lines(conn, &live)?
            .into_iter()
            .map(|l| format!("work:{l}")),
    );
    if let Some(other) = &other {
        lines.extend(
            work_token_lines(conn, other)?
                .into_iter()
                .map(|l| format!("other:{l}")),
        );
    }
    Ok(Facts {
        token: observe::sha256_hex(lines.join("\n").as_bytes()),
        live,
        file_id,
        versions,
        sha256,
        standing,
        other,
        overrides,
        unknown_placement,
        continue_from,
        fork_points,
        same_bytes,
        same_object,
        choices,
    })
}

/// Registers the picked file under Organize's rules (no version is saved),
/// and checks it is still the object read and has nothing recorded yet.
fn register_in(
    tx: &Connection,
    seen: &Seen,
    stable: &StableRead,
    now: i64,
) -> Result<String, AppError> {
    let outdated = || {
        page_outdated(
            "The file changed while Traeva looked at it. Nothing was changed; choose it again.",
        )
    };
    let row = seen_file(&seen.place.root_id, &seen.segments, &seen.name, stable, now);
    let file = observe_in(tx, &row)?;
    if !file.evidence.same_object(&evidence_of(&stable.observed)) {
        return Err(outdated());
    }
    let recorded: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM snapshots WHERE file_id = ?1)
                 OR EXISTS (SELECT 1 FROM memory_members WHERE file_id = ?1
                            AND status <> 'superseded')",
            [&file.file_id],
            |r| r.get(0),
        )
        .map_err(|err| sqlite_error("Cannot save the association", &err))?;
    if recorded {
        return Err(outdated());
    }
    Ok(file.file_id)
}

fn invalid(message: &str) -> AppError {
    memory_error(ErrorCode::InvalidInput, message, false, None)
}

fn conflict(message: &str) -> AppError {
    memory_error(ErrorCode::DecisionConflict, message, false, None)
}

#[cfg(test)]
#[path = "family_associate_tests.rs"]
mod tests;
