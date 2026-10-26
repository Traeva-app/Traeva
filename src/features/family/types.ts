// Mirror of Memory's record types (#31/#32, crates/traeva-core/src/family.rs,
// capture.rs and memory_contracts.rs). Rust is the source of truth: keys are
// snake_case and Rust's Option is `T | null`. Claude-A keeps this file; the
// Memory page (#35, src/features/memory/) imports from here.

import type { AppError, ErrorCode } from "../organize/types";

export interface SnapshotRef {
  snapshot_id: string;
  file_id: string;
  sequence: number;
  file_revision: number;
  root_id: string;
  relative_path: string;
  sha256: string;
  byte_length: number;
  captured_at_ms: number;
}

export type WatchStatus = "active" | "stopped";

export interface MemoryWatchView {
  watch_id: string;
  file_id: string;
  status: WatchStatus;
  root_id: string;
  relative_path: string;
  started_at_ms: number;
  stopped_at_ms: number | null;
  versions: number;
  latest: SnapshotRef | null;
  checked_at_ms: number | null;
  problem: AppError | null;
}

export type RecordState = "active" | "lost" | "stopped";
export type CandidateCapture = "off" | "on" | "paused";

export type GapKind =
  | "memory_off"
  | "paused"
  | "capture_failed"
  | "replaced_uncaptured"
  | "lost"
  | "watcher_stopped"
  | "not_recorded"
  | "skipped";

export type GapEnd =
  | "captured"
  | "recovered"
  | "changed"
  | "resumed"
  | "released"
  | "decided"
  | "continued"
  | "stopped"
  | "relocated"
  | "corrected"
  | "merged";

export interface PlaceView {
  root_id: string;
  relative_path: string;
  display_path: string;
}

export interface PendingCounts {
  objects: number;
  snapshots: number;
  limit: number;
}

export interface RecordedGapView {
  gap_id: string;
  chain_id: string;
  kind: GapKind;
  after: SnapshotRef | null;
  problem: ErrorCode | null;
  started_at_ms: number;
  ended_at_ms: number | null;
  end_reason: GapEnd | null;
}

export interface MemoryRecordView {
  chain_id: string;
  family_id: string;
  origin_file_id: string;
  /** Whose Memory send permission covers the line's versions: the origin,
   * or after a revoked continuation the current part's first file. */
  permission_file_id: string;
  state: RecordState;
  /** Only the current file's versions are in `watch.versions`. */
  watch: MemoryWatchView;
  /** The versions on the record's line (every file it watched or went on as). */
  line_versions: number;
  place: PlaceView;
  root_active: boolean;
  waiting_for_relocation: boolean;
  candidate_capture: CandidateCapture;
  pending: PendingCounts;
  open_gaps: RecordedGapView[];
  /** Memory was turned on for a file of no work whose first version has the
   * bytes of an earlier version in another work: only "may be related",
   * never a relation (#31 M2b). */
  same_content: SameContentHint[];
  /** Paused by a correction: turn Memory on again for the file to go on,
   * or stop the record. */
  hold: RecordHoldView | null;
}

export type HoldReason = "relation_revoked" | "moved_apart";

export interface RecordHoldView {
  work_decision_id: string;
  edge_id: string | null;
  held_at_ms: number;
  why: HoldReason;
}

export interface SameContentHint {
  family_id: string;
  own_version: SnapshotRef;
  same_bytes_as: SnapshotRef;
}

export type RelationStatus =
  | "confirmed_system"
  | "confirmed_user"
  | "candidate"
  | "unknown"
  | "rejected"
  | "disputed"
  | "superseded";

export type MemberBasis =
  | "memory_on"
  | "from_before"
  | "candidate_capture"
  | "user_decision"
  | "relocation"
  | "merge"
  | "correction";

export type EdgeKind = "continuation" | "branch" | "unplaced";

export interface FamilyMemberView {
  file_id: string;
  display_path: string;
  present: boolean;
  status: RelationStatus;
  basis: MemberBasis;
  placed: boolean;
  decided_at_ms: number | null;
  snapshots: SnapshotRef[];
  /** The work this membership was written in, when it was merged into this
   * one (the membership rests on the user's merge). */
  merged_from: string | null;
}

export interface VersionRelationView {
  edge_id: string;
  kind: EdgeKind;
  parent: SnapshotRef | null;
  child: SnapshotRef;
  status: RelationStatus;
  skipped_items: number;
  decided_at_ms: number | null;
  /** Its two files are in two works (a member moved): disputed until it is
   * confirmed again in one work or revoked. */
  crosses_works: boolean;
  /** Traeva made it by itself: the child is a new file an app saved at the
   * record's exact place. The user can revoke it. It says who made it, so
   * it stays true whatever `status` becomes later. */
  linked_automatically: boolean;
}

export interface MemoryFamilyView {
  /** The live work; any work ID merged into it opens it. */
  family_id: string;
  created_at_ms: number;
  merged_family_ids: string[];
  members: FamilyMemberView[];
  records: MemoryRecordView[];
  relations: VersionRelationView[];
  gaps: RecordedGapView[];
  boundaries: string[];
}

export type PlaceHolds =
  | "recorded_file"
  | "empty"
  | "pending_candidate"
  | "decided_candidate"
  | "unread_object"
  | "other_record"
  | "unavailable"
  | "not_looked";

export interface OccupantView {
  occupant_token: string;
  byte_length: number;
  modified_at_ms: number;
}

export interface CandidateCaptureInfo {
  chain_id: string;
  state: RecordState;
  candidate_capture: CandidateCapture;
  notice_version: number;
  place: PlaceView;
  /** Bring it back when turning the switch on: the permission is for the
   * place shown. After the file moved, turning on is refused
   * (`page_outdated`) and the notice is read again. */
  place_token: string;
  pending: PendingCounts;
  place_holds: PlaceHolds;
  unread_occupant: OccupantView | null;
  /** True: the notice must say turning it on saves the object now (D2). */
  saves_occupant_now: boolean;
  place_problem: AppError | null;
  root_active: boolean;
}

export interface CandidateSwitchRequest {
  chain_id: string;
  on: boolean;
  notice_version?: number | null;
  occupant_token?: string | null;
  /** Needed to turn it on: `CandidateCaptureInfo.place_token` as shown. */
  place_token?: string | null;
}

export type CandidateReason = "replaced" | "present_when_enabled" | "read_on_decision";

export interface PendingCandidate {
  candidate_id: string;
  file_id: string;
  seq: number;
  reason: CandidateReason;
  display_path: string;
  present: boolean;
  at_place: boolean;
  found_at_ms: number;
  snapshots: SnapshotRef[];
  found_after: SnapshotRef | null;
  continue_from: SnapshotRef | null;
  continue_skips: number;
  continue_needs: string | null;
  deferred: boolean;
}

export interface UnreadOccupant {
  occupant: OccupantView;
  detected_at_ms: number | null;
  continue_from: SnapshotRef | null;
  continue_skips: number;
  continue_needs: string | null;
}

export interface PendingCandidatesView {
  chain_id: string;
  family_id: string;
  page_token: string;
  state: RecordState;
  root_active: boolean;
  candidate_capture: CandidateCapture;
  pending: PendingCounts;
  place: PlaceView;
  place_holds: PlaceHolds;
  tail: SnapshotRef | null;
  fork_points: SnapshotRef[];
  items: PendingCandidate[];
  unread_occupant: UnreadOccupant | null;
  place_problem: AppError | null;
}

export type CandidateChoice =
  | { kind: "continue"; parent_snapshot_id: string }
  | { kind: "branch"; parent_snapshot_id: string | null }
  | { kind: "unknown" }
  | { kind: "independent" }
  | { kind: "defer" };

/** The object at the place that is not a waiting candidate: "continue"
 * reads it and goes on from it; "independent" reads nothing. */
export type CurrentObjectChoice =
  | { kind: "continue"; parent_snapshot_id: string }
  | { kind: "independent" };

export interface CandidateDecisionRequest {
  chain_id: string;
  page_token: string;
  items: { candidate_id: string; choice: CandidateChoice }[];
  current_object?: CurrentObjectChoice | null;
}

export interface CandidateDecisionOutcome {
  batch_id: string;
  record: MemoryRecordView;
  page: PendingCandidatesView;
  continued_watch: MemoryWatchView | null;
  history_only: boolean;
}

/** What going on at the chosen file would do (#31 M2). There is a preview
 * only when the chosen file is the record's file (same object, one link). */
export interface RelocationPreview {
  chain_id: string;
  old_place: PlaceView;
  /** "recorded_file" only for a lost record whose file is back at its
   * place: then `new_place` is that place and recording resumes there. */
  old_place_holds: PlaceHolds;
  new_place: PlaceView;
  byte_length: number;
  modified_at_ms: number;
  /** The record's candidates that still wait (saved at the old place, or
   * before): they stay, to be decided. */
  waiting_candidates: number;
  /** Goes along (when on) and looks at the new place only. */
  candidate_capture: CandidateCapture;
  page_token: string;
}

/** `root_id` and `relative_path` exactly as `new_place` showed them. */
export interface RelocationRequest {
  chain_id: string;
  root_id: string;
  relative_path: string;
  page_token: string;
}

export interface RelocationOutcome {
  record: MemoryRecordView;
  snapshot: SnapshotRef;
}

/** One live work, for choosing the other side of a merge (#31 M2b). */
export interface WorkSummaryView {
  family_id: string;
  created_at_ms: number;
  merged_family_ids: string[];
  display_paths: string[];
  records: number;
}

/** An earlier "unrelated" judgement the decision makes stop counting. */
export interface OverrideView {
  member_id: string;
  file_id: string;
  display_path: string;
  rejected_in_family_id: string;
}

export interface MergePreview {
  /** The work that stays. */
  family_id: string;
  /** The work that joins it; its ID will open the merged work. */
  other_family_id: string;
  stays: MemoryFamilyView;
  joins: MemoryFamilyView;
  overrides: OverrideView[];
  page_token: string;
}

export interface MergeRequest {
  family_id: string;
  other_family_id: string;
  page_token: string;
}

export type WorkDecisionKind = "merge" | "revoke" | "confirm_again" | "move" | "associate";

export interface WorkDecisionOutcome {
  work_decision_id: string;
  kind: WorkDecisionKind;
  family: MemoryFamilyView;
  /** A move's target work (a new one, or the one chosen). */
  other_family_id: string | null;
  /** Records the decision stopped (an association's "continue"). */
  stopped_records: string[];
  /** Records the decision paused (revoking the continuation they went on by). */
  held_records: string[];
}

export type MoveTarget = { kind: "new_work" } | { kind: "work"; family_id: string };

export interface MoveQuery {
  family_id: string;
  file_ids: string[];
  target: MoveTarget;
}

export interface MovePreview {
  family_id: string;
  /** Null for a new work. */
  target_family_id: string | null;
  moving: FamilyMemberView[];
  records_moving: MemoryRecordView[];
  relations_kept: VersionRelationView[];
  relations_disputed: VersionRelationView[];
  records_held: MemoryRecordView[];
  overrides: OverrideView[];
  page_token: string;
}

export interface MoveRequest extends MoveQuery {
  page_token: string;
}

export type RelationAction = "revoke" | "confirm_again";

export interface RelationPreview {
  family_id: string;
  relation: VersionRelationView;
  can_revoke: boolean;
  can_confirm_again: boolean;
  leaves_unplaced: SnapshotRef;
  records_held: MemoryRecordView[];
  page_token: string;
}

export interface RelationRequest {
  edge_id: string;
  action: RelationAction;
  page_token: string;
}

/** How the chosen file stands; it decides the choices (#31 M2b). */
export type FileStanding =
  | "not_recorded"
  | "saved_no_work"
  | "in_other_work"
  | "member_without_parent";

export type AssociateChoiceKind = "continue" | "branch" | "unknown" | "independent";

export type AssociateChoice =
  | { kind: "continue"; parent_snapshot_id: string }
  | { kind: "branch"; parent_snapshot_id: string }
  | { kind: "unknown" }
  | { kind: "independent" };

export type AssociationSubject =
  | { kind: "picked"; root_id: string; relative_path: string }
  | { kind: "file"; file_id: string };

export interface AssociationSubjectView {
  /** Null for a picked file Traeva does not know yet. */
  file_id: string | null;
  place: PlaceView;
  sha256: string | null;
  versions: SnapshotRef[];
}

/** What Traeva knows; never a relation by itself. */
export interface AssociationEvidence {
  same_bytes_as: SnapshotRef[];
  same_object_as: string[];
}

export interface ContinueOption {
  parent: SnapshotRef;
  /** The record whose line ends at the parent: it stops on confirming. */
  stops_record: MemoryRecordView | null;
}

export interface AssociationPreview {
  family_id: string;
  subject: AssociationSubjectView;
  /** Send it back unchanged as `AssociationRequest.subject`. */
  request_subject: AssociationSubject;
  standing: FileStanding;
  other_family_id: string | null;
  merge_scope: MemoryFamilyView | null;
  needs_recording: boolean;
  evidence: AssociationEvidence;
  choices: AssociateChoiceKind[];
  continue_from: ContinueOption[];
  fork_points: SnapshotRef[];
  replaces_unknown_placement: string | null;
  overrides: OverrideView[];
  page_token: string;
}

export interface AssociationRequest {
  family_id: string;
  subject: AssociationSubject;
  choice: AssociateChoice;
  page_token: string;
}

export interface FamilyApi {
  listRecords(): Promise<MemoryRecordView[]>;
  /** Opens the native picker in the root; null when it was closed. */
  startRecord(rootId: string): Promise<MemoryRecordView | null>;
  startForFile(fileId: string): Promise<MemoryRecordView>;
  stopRecord(chainId: string): Promise<MemoryRecordView>;
  family(familyId: string): Promise<MemoryFamilyView>;
  candidateCapture(chainId: string): Promise<CandidateCaptureInfo>;
  setCandidateCapture(request: CandidateSwitchRequest): Promise<CandidateCaptureInfo>;
  pendingCandidates(chainId: string): Promise<PendingCandidatesView>;
  decide(request: CandidateDecisionRequest): Promise<CandidateDecisionOutcome>;
  /** Opens the native picker in the root; null when it was closed. Reads
   * and writes nothing. */
  previewRelocation(chainId: string, rootId: string): Promise<RelocationPreview | null>;
  relocate(request: RelocationRequest): Promise<RelocationOutcome>;
  listWorks(): Promise<WorkSummaryView[]>;
  previewMerge(familyId: string, otherFamilyId: string): Promise<MergePreview>;
  mergeWorks(request: MergeRequest): Promise<WorkDecisionOutcome>;
  /** Opens the native picker in the root; null when it was closed. Reads
   * the file only when it has no saved version; writes nothing. */
  previewAssociation(familyId: string, rootId: string): Promise<AssociationPreview | null>;
  previewAssociationForFile(familyId: string, fileId: string): Promise<AssociationPreview>;
  associate(request: AssociationRequest): Promise<WorkDecisionOutcome>;
  previewRelation(edgeId: string): Promise<RelationPreview>;
  correctRelation(request: RelationRequest): Promise<WorkDecisionOutcome>;
  previewMove(query: MoveQuery): Promise<MovePreview>;
  moveMembers(request: MoveRequest): Promise<WorkDecisionOutcome>;
  /** The switch for keeping versions of files Traeva organizes (#201). */
  autoMemory(): Promise<AutoMemoryView>;
  setAutoMemory(request: AutoMemoryRequest): Promise<AutoMemoryView>;
}

// --- Keeping versions of files Traeva organizes (#201, auto_memory.rs) ---

/** The switch, as the Memory page shows it. */
export interface AutoMemoryView {
  /** The switch's agreement counts now (saved provider and model, this notice). */
  enabled: boolean;
  /** An agreement is open but for another provider, model or notice. */
  outdated: boolean;
  agreed_at_ms: number | null;
  provider: string | null;
  model: string | null;
  /** The full notice, shown before turning it on. */
  notice: string;
  /** Send this back when turning it on. */
  notice_version: number;
  /** No new record starts while this many Memory records are on (50). */
  limit: number;
  /** Memory records that are on now, all of them. */
  recording: number;
  /** Records the switch started, for a "Started by Traeva" label. */
  started_chain_ids: string[];
}

export interface AutoMemoryRequest {
  enable: boolean;
  notice_version: number;
}
