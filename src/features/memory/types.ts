// Hand-written mirror of Memory's Rust types: memory_contracts.rs
// (SnapshotRef), memory_consent.rs, changes.rs (#33), retrieval.rs (#34) and
// opened.rs and export.rs (#35) in crates/traeva-core/src. The Rust types are the source
// of truth; change both together.

import type { GapKind, SnapshotRef } from "../family/types";
import type { ErrorCode } from "../organize/types";

// SnapshotRef (one saved version of one file) is mirrored once, in
// ../family/types.ts.
export type { SnapshotRef };

// --- The agreement to send (memory_consent.rs) ---

export interface MemoryConsentView {
  file_id: string;
  /** Memory may send now, for the provider and model saved now. */
  agreed: boolean;
  /** An agreement that no longer counts (provider, model or notice changed). */
  outdated: boolean;
  agreed_at_ms: number | null;
  provider: string | null;
  model: string | null;
  /** What will be sent; shown before the user agrees. */
  notice: string;
}

// --- Changes between versions (#33, changes.rs) ---

/** Two versions the core lists as consecutive (a usable pair, #31). */
export interface VersionPairView {
  before: SnapshotRef;
  after: SnapshotRef;
  pair_id: string | null;
  state: PairState;
  /** Empty unless the state is "analyzed". */
  changes: ChangeView[];
  basis: PairBasis;
  /** "user" when the pair rests on the user's confirmation. */
  evidence: "system" | "user";
  /** Memory may not have seen every save between the two (gap_kinds says
   * why): the changes are a difference, never one save. */
  gap: boolean;
  gap_kinds: GapKind[];
  /** The file's place in the work is unknown: never used for first/last. */
  unplaced: boolean;
  /** Every one of these files' agreements must count before sending. */
  permission_file_ids: string[];
  /** Both versions' texts were read whole; false when a reader left parts
   * out (hidden slides, tables, fields...) or cut the text. */
  complete: boolean;
  /** The relation the pair rests on (revoking it undoes it); null for two
   * versions of one file. */
  edge_id: string | null;
  /** Traeva made that relation by itself: an app saved the file by
   * replacing it at the same place. False once the user confirmed it
   * again. */
  linked_automatically: boolean;
}

export type PairBasis = "same_file" | "continuation" | "branch";

/** Only "no_text_change" and "analyzed" say what changed. Every other state
 * means Memory cannot tell (yet), never that nothing changed. */
export type StructureChange = "slide_order" | "sheet_order" | "sheet_renamed" | "labels";

export type PairState =
  | { kind: "not_compared" }
  | { kind: "no_text_change" }
  | { kind: "unreadable"; note: string }
  /** The writer's lines are the same; only the structure a reader adds
   * changed (slides or sheets in another order, a sheet renamed, slide
   * numbers). Nothing is sent (contract v23 §4.4). */
  | { kind: "structure_only"; change: StructureChange }
  | { kind: "not_permitted" }
  | { kind: "queued" }
  | { kind: "analyzing" }
  | {
      kind: "analyzed";
      complete: boolean;
      hunks: number;
      left_out_hunks: number;
      minor_hunks: string[];
    }
  | { kind: "failed"; error_code: ErrorCode; will_retry: boolean }
  /** `reason`: the rule the answer broke, in Traeva's words; null for an
   * answer refused before reasons were recorded. */
  | { kind: "invalid_output"; will_retry: boolean; reason: string | null };

export type ChangeAction = "added" | "removed" | "modified";

export interface ChangeView {
  change_id: string;
  position: number;
  action: ChangeAction;
  topic: string;
  description: string;
  evidence: ChangeEvidence;
}

export interface ChangeEvidence {
  hunks: HunkLines[];
  before_quote: QuoteAt | null;
  after_quote: QuoteAt | null;
}

/** Line numbers count from 1 in each version's text. */
export interface HunkLines {
  hunk_id: string;
  before_first_line: number;
  before_lines: number;
  after_first_line: number;
  after_lines: number;
}

export interface QuoteAt {
  hunk_id: string;
  line: number;
  char_start: number;
  chars: number;
}

/** The lines of one change, read again from the saved versions. */
export interface ChangeLinesView {
  change_id: string;
  before: SnapshotRef;
  after: SnapshotRef;
  hunks: HunkText[];
  /** Lines were cut to keep the view small. */
  shortened: boolean;
}

export interface HunkText {
  hunk_id: string;
  before_first_line: number;
  removed: string[];
  after_first_line: number;
  added: string[];
  /** For a slide deck or workbook: which lines the reader wrote (structure
   * lines), not the writer's text. Empty for other files. */
  removed_structure: boolean[];
  added_structure: boolean[];
  /** Where the hunk is (slide and role, or sheet, cell and field); null for
   * other files. */
  place: HunkPlace | null;
}

export interface HunkPlace {
  slide: number | null;
  role: "title" | "text" | "notes" | null;
  sheet: string | null;
  cell: string | null;
  field: "literal" | "formula" | "cache" | null;
}

// --- Finding a version by a change (#34, retrieval.rs) ---

/** A search as the window sees it. It never holds the question text. */
export interface SearchView {
  search_id: string;
  file_id: string;
  state: SearchState;
  coverage: Coverage;
  /** New versions or descriptions arrived since the search started. */
  updates_available: boolean;
}

export interface Coverage {
  pairs: number;
  described: number;
  described_in_part: number;
  waiting: number;
  not_permitted: number;
  failed: number;
  unreadable: number;
  gaps: number;
  /** Memory may have missed saves after the newest version. */
  gap_after_latest: boolean;
  changes_left_out: number;
  complete: boolean;
}

export type SearchState =
  | { kind: "working" }
  | { kind: "found"; candidate: SearchCandidate; reached_by: ReachedBy; openable: boolean }
  | { kind: "candidates"; candidates: SearchCandidate[]; question: SearchQuestion | null }
  | { kind: "not_found" }
  | { kind: "cannot_tell" }
  | { kind: "unsupported"; asked_by: AskedBy }
  | { kind: "unclear" }
  | { kind: "none_of_these" }
  | { kind: "not_permitted" }
  | { kind: "failed"; error_code: ErrorCode; retryable: boolean }
  | { kind: "cancelled" };

export type ReachedBy = "search" | "answer" | "pick";

export type AskedBy = "state" | "time" | "name";

export interface SearchCandidate {
  /** "K1" is versions 1 and 2 of the scope; it never changes. */
  candidate_id: string;
  before: SnapshotRef;
  after: SnapshotRef;
  /** The version on the side asked for; null until the side is known. */
  version: SnapshotRef | null;
  changes: ChangeView[];
  /** Only "maybe" judgements matched it. */
  maybe: boolean;
  /** Some of the pair's changed lines were not described. */
  partial: boolean;
  /** Why the core no longer lists the pair, checked again each time the
   * search is shown (a relation was corrected after it was found): the
   * candidate is no longer certain. Null while the pair counts. */
  no_longer_usable: PairRefusal | null;
}

/** Why the core does not list two versions as a pair (#31,
 * shared-contract.md §4.3). */
export type PairRefusal =
  | "not_a_member"
  | "candidate_version"
  | "no_confirmed_relation"
  | "not_adjacent"
  | "relation_disputed"
  | "relation_superseded";

export interface SearchQuestion {
  question_id: string;
  text: string;
  options: SearchOption[];
}

export interface SearchOption {
  text: string;
  /** The candidates this answer keeps. */
  candidate_ids: string[];
}

export type SearchAnswer =
  | { kind: "option"; question_id: string; index: number }
  | { kind: "pick"; candidate_id: string }
  | { kind: "none_of_these" };

// --- Opening an old version (#35, opened.rs) ---

/** A new read-only copy in Traeva's data folder; not the user's file. */
export interface OpenedCopy {
  snapshot: SnapshotRef;
  path: string;
  sha256: string;
  byte_length: number;
  /** A text file Memory reads (.txt, .md, .markdown): the system may open
   * it. Other copies are only shown by their place. */
  can_open: boolean;
  /** The number in the copy's name, fixed when it was made; null when the
   * name has none. Say this one, not the history's number now (R2-MEM-11). */
  number: number | null;
}

export interface OpenedVersion {
  copy: OpenedCopy;
  /** False when the copy may not be opened, or macOS could not open it; it
   * is still at copy.path. */
  opened: boolean;
}

// --- Exporting an old version to the user's folder (export.rs) ---

/** Where a copy would go; nothing is written for it. */
export interface ExportPreview {
  snapshot: SnapshotRef;
  root_id: string;
  /** The folder inside the root, names joined by "/"; empty for the root. */
  folder: string;
  name: string;
  display_path: string;
  byte_length: number;
  /** Sent back unchanged with the export. */
  page_token: string;
}

/** What the preview showed, sent back; the core checks it all again. */
export interface ExportRequest {
  snapshot_id: string;
  root_id: string;
  folder: string;
  name: string;
  page_token: string;
}

export interface ExportedVersion {
  snapshot: SnapshotRef;
  display_path: string;
  sha256: string;
  byte_length: number;
}
