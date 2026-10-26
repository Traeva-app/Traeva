// Hand-written mirror of crates/traeva-core/src/contracts.rs.
// The Rust types are the source of truth; change both files together.

export type ErrorCode =
  | "app_data_unavailable"
  | "another_instance_running"
  | "database_unavailable"
  | "database_corrupt"
  | "unsupported_schema_version"
  | "migration_failed"
  | "pragma_mismatch"
  | "invalid_input"
  | "unknown_root"
  | "root_revoked"
  | "root_unavailable"
  | "root_changed"
  | "unsupported_location"
  | "protected_location"
  | "overlaps_app_data"
  | "file_not_found"
  | "outside_root"
  | "unsupported_file"
  | "file_too_large"
  | "file_changing"
  | "file_unreadable"
  | "unknown_file"
  | "unknown_plan"
  | "invalid_folder_name"
  | "folder_list_outdated"
  | "target_conflict"
  | "file_changed"
  | "stale_plan"
  | "unknown_action"
  | "unresolved_action"
  | "operation_refused"
  | "result_unconfirmed"
  | "run_interrupted"
  | "undo_conflict"
  | "nothing_to_undo"
  | "unknown_operation"
  | "model_not_configured"
  | "consent_required"
  | "key_unavailable"
  | "model_key_rejected"
  | "model_unreachable"
  | "model_timeout"
  | "model_service_error"
  | "model_request_too_large"
  | "model_response_too_large"
  | "invalid_model_output"
  | "no_usable_text"
  | "analysis_cancelled"
  | "unknown_analysis"
  | "wrong_stage"
  | "suggestion_outdated"
  | "unknown_snapshot"
  | "snapshot_unavailable"
  | "snapshot_corrupt"
  | "unknown_record"
  | "unknown_family"
  | "page_outdated"
  | "decision_conflict"
  | "record_stopped"
  | "candidate_pending"
  | "not_same_object"
  | "unknown_relation";

export type ErrorStage =
  | "storage"
  | "authorization"
  | "observation"
  | "plan"
  | "execution"
  | "analysis"
  | "memory";

export type NextStep =
  | "restart_app"
  | "choose_folder"
  | "choose_file"
  | "try_again"
  | "preview_again";

export interface AppError {
  code: ErrorCode;
  message: string;
  stage: ErrorStage;
  retryable: boolean;
  file_id: string | null;
  plan_id: string | null;
  action_id: string | null;
  allowed_next: NextStep[];
}

export interface PragmaSettings {
  foreign_keys: boolean;
  journal_mode: string;
  synchronous: string;
  fullfsync: boolean;
}

export interface StorageInfo {
  app_data_dir: string;
  database_path: string;
  schema_version: number;
  pragmas: PragmaSettings;
  /** The normal folder, or a test folder named by TRAEVA_APP_DATA_DIR (#65). */
  app_data_source: AppDataSource;
}

export type AppDataSource = "default" | "test";

export type StorageStatus =
  | ({ status: "ready" } & StorageInfo)
  | { status: "blocked"; error: AppError };

export type ModelStatus = "not_configured" | "consent_required" | "configured";

/** The model setup for the settings card. Never contains the API key. */
export interface ModelSettingsView {
  status: ModelStatus;
  /** Null until settings were saved. */
  provider: string | null;
  model: string | null;
  /** Whether the Keychain holds a key for the provider. */
  key_saved: boolean;
  /** The Keychain account of the key: "openai", or "openai@test-…" for a test folder. */
  key_account: string;
  /** When the user agreed to send, if that agreement still counts. */
  consent_agreed_at_ms: number | null;
  diagnostics: boolean;
  /** What will be sent, shown before the user agrees. */
  notice: string;
  /** Whether a smaller copy of an image may be sent (organize-spec §14.10). */
  image_sending: ImageSending;
  /** What turning on sending images means, shown next to the choice. */
  image_notice: string;
}

/** Sending images to the model: off unless the user turns it on (§14.10). */
export type ImageSending = "off" | "ask_each_time" | "without_asking";

/** Provider, model, and optionally a new API key (it goes to the Keychain only). */
export interface ModelSettingsInput {
  provider: string;
  model: string;
  api_key?: string;
}

export type RootStatus = "active" | "revoked";

export interface RootView {
  root_id: string;
  display_path: string;
  status: RootStatus;
  granted_at_ms: number;
  revoked_at_ms: number | null;
  /** Worked out when read: why this active root cannot be used now, or null. */
  problem: AppError | null;
}

export type ExtractionStatus = "ok" | "partial" | "unsupported" | "failed";

export interface ExtractionSummary {
  status: ExtractionStatus;
  total_chars: number;
  truncated: boolean;
  blank: boolean;
  /** Some of the text read can be sent; without it the model is not asked. */
  usable_text: boolean;
  note: string | null;
  /**
   * Read by on-device text recognition from a scanned PDF's pages: always
   * "partial" and may contain mistakes; since v0.14 (10/15) the automatic
   * mode moves it like any text once every condition holds.
   */
  recognized: boolean;
  /**
   * A JPEG/PNG image or a PDF with no text layer, read for Organize: with
   * sending images on, a smaller copy may go to the model (§14.10). Never
   * moved automatically.
   */
  image_input: boolean;
}

/** Where text recognition for scanned PDFs stands in this launch. */
export type OcrState =
  | "off"
  | "not_prepared"
  | "preparing"
  | "ready"
  | "failed"
  | "timed_out"
  | "cancelled";

/** A registered file. Never contains file text. */
export interface FileView {
  file_id: string;
  file_revision: number;
  root_id: string;
  relative_path: string;
  display_path: string;
  byte_length: number;
  sha256: string;
  extraction: ExtractionSummary;
}

export interface FolderView {
  folder_id: string;
  parent_id: string | null;
  relative_path: string;
  depth: number;
  protected: string | null;
}

export interface FolderCatalogView {
  root_id: string;
  /** Names this listing; folder IDs only work together with it. */
  catalog_id: string;
  folders: FolderView[];
  truncated: boolean;
}

export interface AppState {
  app_version: string;
  storage: StorageStatus;
  model: ModelStatus;
  /** Authorized folders, checked on disk now; empty while storage is blocked. */
  roots: RootView[];
  /** The 50 most recent actions, newest first. */
  recent_actions: ActionView[];
  /** Every action whose result needs the user's decision. */
  decisions_needed: ActionView[];
}

/** Where an operation is (organize-spec.md §6; #16). */
export type OperationStage =
  | "observing"
  | "ready_for_analysis"
  | "analyzing"
  | "needs_clarification"
  | "recommendation_ready"
  | "needs_manual_location"
  | "analysis_failed"
  | "invalid_model_output"
  | "waiting_for_file"
  | "stale"
  | "blocked"
  | "cancelled";

/** A chosen file on its way to a plan, polled about every 300 ms while it is
 * read or analyzed. In memory only. */
export interface OperationView {
  operation_id: string;
  /** Goes up with every change; never show an older one over a newer one. */
  generation: number;
  stage: OperationStage;
  source_root_id: string;
  target_root_id: string;
  file: FileView | null;
  error: AppError | null;
  /** The latest question, once one was asked (up to 3, one at a time). */
  question: QuestionView | null;
  /** The user's locked answer to that question, once given. */
  answer: string | null;
  /** Every question answered so far, with its locked answer, oldest first. */
  answered: AnsweredView[];
  /** In recommendation_ready, and in needs_manual_location after an analysis. */
  suggestion: SuggestionView | null;
  model_requests: number;
  plan_id: string | null;
  /** The action that plan became once confirmed (filled in by get_operation). */
  action_id: string | null;
}

export type UncertainField = "topic" | "purpose" | "destination";

export interface ClarificationOption {
  option_id: string;
  text: string;
}

export interface QuestionView {
  question_id: string;
  uncertain_field: UncertainField;
  question: string;
  /** Empty: the user answers in their own words. */
  options: ClarificationOption[];
  /** Which question this is for the file, from 1. */
  number: number;
  /** At most this many questions per file. */
  max_questions: number;
}

/** A question the user answered, with the locked answer. */
export interface AnsweredView {
  question_id: string;
  number: number;
  question: string;
  answer: string;
}

/** One option, or the user's own words (not both). */
export interface ClarificationAnswer {
  question_id: string;
  option_id?: string;
  text?: string;
}

export interface SuggestionView {
  request_id: string;
  summary: string;
  /** A score, not an accuracy. */
  confidence: number | null;
  /** Only in recommendation_ready. */
  recommended_id: string | null;
  candidates: CandidateView[];
}

export interface CandidateView {
  candidate_id: string;
  /** Path inside the target root; "" is the root itself. */
  folder_path: string;
  new_segments: string[];
  reason: string;
  evidence_ids: string[];
}

/** True when a rejected command value is Traeva's AppError (not, say, a JS bug). */
export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as AppError).code === "string" &&
    typeof (value as AppError).message === "string"
  );
}

/** Move this file to a target: a candidate of the operation's current
 * suggestion, or a listed folder (inside up to 3 new folders). */
export interface PlanRequest {
  file_id: string;
  file_revision: number;
  target_root_id: string;
  /** The operation the choice was made in, if the file was analyzed. */
  operation_id?: string;
  target: PlanTarget;
}

export type PlanTarget =
  | { kind: "candidate"; candidate_id: string }
  | { kind: "folder"; catalog_id: string; folder_id: string; new_folders: string[] };

export type UserChoice = "accept" | "correct" | "manual";

export type BuildOutcome =
  | { outcome: "planned"; plan: PlanView }
  | { outcome: "already_in_place"; file_id: string };

export type PlanKind = "organize" | "undo";

export type PlanLifecycle = "open" | "cancelled" | "superseded" | "invalidated";

export type PlanStatus =
  | "confirmable"
  | "consumed"
  | "cancelled"
  | "superseded"
  | "invalidated"
  | "expired"
  | "root_revoked";

export interface PlanEnd {
  root_id: string;
  relative_path: string;
  display_path: string;
}

/** An immutable plan. status, can_confirm and reason are worked out when read. */
export interface PlanView {
  plan_id: string;
  kind: PlanKind;
  file_id: string;
  file_revision: number;
  source: PlanEnd;
  target: PlanEnd;
  basename: string;
  folders_to_create: string[];
  created_at_ms: number;
  expires_at_ms: number;
  lifecycle: PlanLifecycle;
  status: PlanStatus;
  can_confirm: boolean;
  reason: string | null;
  action_id: string | null;
  /** For an undo plan: the organize action it puts back. */
  undo_of_action_id: string | null;
  /** For an organize plan: how the target was chosen. */
  user_choice: UserChoice | null;
  /** The model request whose suggestion was shown, if any. */
  analysis_request_id: string | null;
}

/** Only "committed" means the file moved and everything was recorded. */
export type ActionStatus =
  | "prepared"
  | "running"
  | "committed"
  | "failed_no_move"
  | "needs_user_decision"
  | "undone";

/** The one action a confirmed plan became. Reading it never runs anything. */
export interface ActionView {
  action_id: string;
  plan_id: string;
  kind: PlanKind;
  file_id: string;
  status: ActionStatus;
  source: PlanEnd;
  target: PlanEnd;
  /** Folders this action created and holds a receipt for, outermost first. */
  folders_created: string[];
  /** Of those, the empty ones a record check removed again. */
  folders_removed: string[];
  /** Folders it was about to create but holds no receipt for; never removed. */
  folders_without_receipt: string[];
  /** For a committed undo: the organize action's folders that stayed (the cleanup warning). */
  folders_kept: string[];
  created_at_ms: number;
  committed_at_ms: number | null;
  recovered_at_ms: number | null;
  /** Why the action failed or needs a decision, or null. */
  problem: AppError | null;
  /** For an undo: the organize action it puts back. */
  undo_of_action_id: string | null;
  /** For an organize action, from saved records only. */
  undo_status: UndoStatus;
  /** The latest undo of this action that is in progress or committed. */
  undo_action_id: string | null;
}

/** root_revoked: a folder the action used was removed from Traeva, so it
 * can no longer be undone (#79). */
export type UndoStatus = "available" | "in_progress" | "undone" | "not_available" | "root_revoked";

/** What asking to undo gives; conflicts are AppError "undo_conflict". */
export type UndoPreview =
  | { outcome: "planned"; plan: PlanView }
  | { outcome: "already_undone"; undo: ActionView }
  | { outcome: "undo_in_progress"; undo: ActionView };
