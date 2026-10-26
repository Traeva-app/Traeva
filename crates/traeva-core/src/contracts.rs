//! Data sent from the core to the window, and the one error type.
//!
//! These Rust types are the source of truth. `src/features/organize/types.ts`
//! mirrors them by hand; keep the two files in sync.

use serde::{Deserialize, Serialize};

/// Everything the window needs to draw its first screen (`get_app_state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppState {
    pub app_version: String,
    pub storage: StorageStatus,
    pub model: ModelStatus,
    /// The authorized folders, each checked on disk now. Empty while
    /// storage is blocked.
    pub roots: Vec<RootView>,
    /// The 50 most recent actions, newest first.
    pub recent_actions: Vec<ActionView>,
    /// Every action whose result needs the user's decision, oldest first.
    pub decisions_needed: Vec<ActionView>,
}

/// One chosen file on its way to a plan, as the window sees it while it
/// polls (about every 300 ms while the stage is `observing` or `analyzing`).
/// Kept in memory only: a restart ends it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationView {
    pub operation_id: String,
    /// Goes up with every change. A late answer never replaces a newer state.
    pub generation: u64,
    pub stage: OperationStage,
    pub source_root_id: String,
    pub target_root_id: String,
    /// The registered file, once it was read.
    pub file: Option<FileView>,
    /// Why the operation stopped or failed, or a note (no usable text).
    pub error: Option<AppError>,
    /// The latest question, once one was asked (#16; up to `MAX_QUESTIONS`,
    /// one at a time, organize-spec.md §15).
    pub question: Option<QuestionView>,
    /// The user's locked answer to that question, once given.
    pub answer: Option<String>,
    /// Every question answered so far, with its locked answer, oldest first.
    pub answered: Vec<AnsweredView>,
    /// The checked suggestion, in `recommendation_ready` and in
    /// `needs_manual_location` after an analysis.
    pub suggestion: Option<SuggestionView>,
    /// Requests handed to the model in this operation (first, after the
    /// answer, and explicit retries).
    pub model_requests: u32,
    /// The latest plan built through this operation.
    pub plan_id: Option<String>,
    /// The action that plan became once confirmed (filled in when the
    /// operation is read with `get_operation`).
    pub action_id: Option<String>,
}

/// Where an operation is (organize-spec.md §6; #16 material §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStage {
    /// Reading the file twice with a pause and recording it.
    Observing,
    /// Read, with text the model may read: the user starts an analysis or
    /// picks the folder by hand.
    ReadyForAnalysis,
    /// Waiting for the model. Can be cancelled or left for a manual choice.
    Analyzing,
    /// A question is shown; the user answers it, picks by hand or cancels.
    NeedsClarification,
    /// One candidate is recommended. The user accepts it, picks another
    /// target, or cancels; nothing moves before preview and confirmation.
    RecommendationReady,
    /// The user picks the target folder by hand (any candidates are only
    /// offered, never preselected).
    NeedsManualLocation,
    /// The model request failed; retry (when the error allows it), pick by
    /// hand, or cancel.
    AnalysisFailed,
    /// The answer broke the #15 rules; retry, pick by hand, or cancel.
    InvalidModelOutput,
    /// The file kept changing: wait until it is saved, then choose it again.
    WaitingForFile,
    /// The file changed or moved since it was read: choose it again.
    Stale,
    /// The file cannot be organized (unsupported, protected, too large...),
    /// or a folder is no longer authorized.
    Blocked,
    /// Cancelled by the user; a result that arrives later is dropped.
    Cancelled,
}

/// What a question may be about (#15 material §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertainField {
    /// What the file is about.
    Topic,
    /// What the file is for.
    Purpose,
    /// Where the user likes to keep such files.
    Destination,
}

/// One question, with Traeva's own IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionView {
    pub question_id: String,
    pub uncertain_field: UncertainField,
    pub question: String,
    /// Empty: the user answers in their own words.
    pub options: Vec<ClarificationOption>,
    /// Which question this is for the file, from 1.
    pub number: u32,
    /// At most this many questions per file (`MAX_QUESTIONS`).
    pub max_questions: u32,
}

/// A question the user answered, with the locked answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnsweredView {
    pub question_id: String,
    pub number: u32,
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClarificationOption {
    pub option_id: String,
    pub text: String,
}

/// The user's answer to the current question: one option, or their own words.
/// Unknown fields are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClarificationAnswer {
    pub question_id: String,
    #[serde(default)]
    pub option_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

/// The checked suggestion of one model request, as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuggestionView {
    pub request_id: String,
    pub summary: String,
    /// From 0 to 1, or null. A score, not an accuracy.
    pub confidence: Option<f64>,
    /// Only in `recommendation_ready`: the one candidate Traeva recommends.
    pub recommended_id: Option<String>,
    pub candidates: Vec<CandidateView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateView {
    pub candidate_id: String,
    /// The listed folder, as a path inside the target root ("" is the root).
    pub folder_path: String,
    /// New folders the plan would create inside it, outermost first.
    pub new_segments: Vec<String>,
    pub reason: String,
    pub evidence_ids: Vec<String>,
}

/// Storage is either ready, or blocked with the reason.
/// While blocked, Traeva starts no new operation of any kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StorageStatus {
    Ready(StorageInfo),
    Blocked { error: AppError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageInfo {
    pub app_data_dir: String,
    pub database_path: String,
    /// `PRAGMA user_version`: how many migrations have been applied.
    pub schema_version: i64,
    pub pragmas: PragmaSettings,
    /// The normal app-data folder, or a separate test folder named by
    /// `TRAEVA_APP_DATA_DIR` (#65).
    pub app_data_source: AppDataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppDataSource {
    /// `~/Library/Application Support/Traeva/`.
    Default,
    /// A separate folder for development and tests, with its own Keychain
    /// account for the API key.
    Test,
}

/// The connection settings as SQLite reports them (read back, not assumed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PragmaSettings {
    pub foreign_keys: bool,
    pub journal_mode: String,
    pub synchronous: String,
    pub fullfsync: bool,
}

/// Whether Traeva may send a request to the model now (organize-spec.md §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    /// No provider and model chosen yet, or no API key in the Keychain.
    NotConfigured,
    /// Set up, but the user has not agreed to send, or the provider or
    /// model changed since they agreed.
    ConsentRequired,
    /// Set up and agreed: requests may be sent.
    Configured,
}

/// The single error type every command returns (Tauri turns it into a rejected Promise).
/// `message` is for people and must not contain SQL, file contents or secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub stage: ErrorStage,
    /// True when trying again later can succeed after the user fixes the cause.
    /// It never means Traeva retries a file operation by itself.
    pub retryable: bool,
    pub file_id: Option<String>,
    pub plan_id: Option<String>,
    pub action_id: Option<String>,
    pub allowed_next: Vec<NextStep>,
}

/// Also read back from the database, where an action keeps its problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The app-data folder or its lock file cannot be created or opened.
    AppDataUnavailable,
    /// Another Traeva process holds the app-data lock.
    AnotherInstanceRunning,
    /// The database cannot be opened, is read-only, or stopped responding.
    DatabaseUnavailable,
    /// The database file is not a valid SQLite database or fails its integrity check.
    DatabaseCorrupt,
    /// The database was written by a newer Traeva (or has an unknown version).
    UnsupportedSchemaVersion,
    /// A migration failed; it was rolled back and the database was kept.
    MigrationFailed,
    /// SQLite did not accept one of the required connection settings.
    PragmaMismatch,
    /// The input is malformed (for example a relative path or a badly formed ID).
    InvalidInput,
    /// No authorized root has this ID.
    UnknownRoot,
    /// The root was revoked. Revocation is permanent; authorizing again gives a new root.
    RootRevoked,
    /// The root folder is missing or cannot be opened right now.
    RootUnavailable,
    /// The folder at the root's path is not the folder that was authorized.
    RootChanged,
    /// The folder is not a real folder on a local, writable APFS volume.
    UnsupportedLocation,
    /// The folder is hidden, a package, a project folder or app data.
    ProtectedLocation,
    /// The folder contains Traeva's app data, is inside it, or Traeva cannot tell.
    OverlapsAppData,
    /// The chosen file does not exist (any more).
    FileNotFound,
    /// The chosen file is not inside the authorized folder.
    OutsideRoot,
    /// Not a plain file Traeva can organize: a folder, link, special file,
    /// hard-linked or hidden file, cloud placeholder, or on another volume.
    UnsupportedFile,
    /// Bigger than 20 MiB; such files cannot be moved, not even by hand.
    FileTooLarge,
    /// The file kept changing while Traeva read it (waiting_for_file).
    FileChanging,
    /// The file could not be read.
    FileUnreadable,
    /// No registered file has this ID.
    UnknownFile,
    /// No plan has this ID.
    UnknownPlan,
    /// A new folder name breaks the naming rules (organize-spec.md §4.2) or
    /// there are more than 3 new levels. Names are never changed silently.
    InvalidFolderName,
    /// The folder list this choice came from is out of date: list folders again.
    FolderListOutdated,
    /// The target name, or a folder to create, is already taken.
    TargetConflict,
    /// The file changed, moved or was replaced since it was chosen or planned.
    FileChanged,
    /// This plan can no longer run (cancelled, replaced, invalidated or expired).
    StalePlan,
    /// No action has this ID.
    UnknownAction,
    /// The file has an action that is still running or whose result needs
    /// checking; nothing new is planned or run for it until that is settled.
    UnresolvedAction,
    /// macOS refused to create a folder or move the file (for example, no
    /// permission). The file did not move.
    OperationRefused,
    /// The file moved, or may have moved, but Traeva could not confirm or
    /// record the result. Nothing is moved back automatically.
    ResultUnconfirmed,
    /// The run stopped before it finished (Traeva quit, crashed or could not
    /// record the result); a record check found the file did not move.
    RunInterrupted,
    /// Undo is refused: the file was edited, moved or replaced (even by a
    /// copy with the same bytes), its old folder changed, the old name is
    /// taken, or a folder is no longer authorized. Nothing was moved.
    UndoConflict,
    /// There is nothing to undo: the action did not organize a file (it
    /// failed, is not settled, or is itself an undo).
    NothingToUndo,
    /// No operation has this ID (operations end when Traeva restarts).
    UnknownOperation,
    /// No model is set up: choose the provider and model, and save an API key.
    ModelNotConfigured,
    /// Sending to the model is not enabled, or the provider or model changed
    /// since it was enabled. Nothing was sent.
    ConsentRequired,
    /// The API key could not be read from the Keychain (for example, access
    /// was denied). Nothing was sent.
    KeyUnavailable,
    /// The provider did not accept the API key.
    ModelKeyRejected,
    /// Traeva could not reach the provider (offline, name lookup or TLS failed).
    ModelUnreachable,
    /// No complete answer within 60 seconds.
    ModelTimeout,
    /// The provider answered with an error (busy, out of credit, unknown
    /// model, request refused).
    ModelServiceError,
    /// The request would be larger than 64 KiB. Nothing was sent.
    ModelRequestTooLarge,
    /// The answer was larger than 64 KiB. It was dropped.
    ModelResponseTooLarge,
    /// The answer is not a valid structured result (organize-spec.md §5).
    /// Nothing is chosen or moved because of it.
    InvalidModelOutput,
    /// The file has no text the model may read (not a UTF-8 .txt or .md
    /// file, or blank). Nothing was sent; the user picks the folder by hand.
    NoUsableText,
    /// The request was cancelled, or the model setup changed while it ran.
    /// Its answer, if one came, was dropped.
    AnalysisCancelled,
    /// No model request has this ID.
    UnknownAnalysis,
    /// The operation cannot do this in its current stage (for example,
    /// analyzing again after a manual choice, or changing a locked answer).
    WrongStage,
    /// This candidate is not part of the operation's current suggestion (it
    /// was cancelled, answered or retried since). Choose again.
    SuggestionOutdated,
    /// No saved version (snapshot) has this ID (#65).
    UnknownSnapshot,
    /// The saved copy of this version is missing or cannot be read. It is
    /// never replaced with the file as it is now.
    SnapshotUnavailable,
    /// The saved copy of this version no longer matches its SHA-256. It is
    /// not used and not repaired.
    SnapshotCorrupt,
    /// No Memory record (record task, #31) has this ID.
    UnknownRecord,
    /// No work (family, #31) has this ID.
    UnknownFamily,
    /// What the page showed, or the object at the record's place, changed
    /// since the page was opened. Nothing was changed; open it again.
    PageOutdated,
    /// The choices break a relation rule (not the relation shown, an earlier
    /// object not decided, a parent that is not on the line, a record that
    /// already went on as another file...). Nothing was changed.
    DecisionConflict,
    /// Memory is off for this record, or it is lost: recording is not reopened.
    RecordStopped,
    /// The file waits for a decision in a Memory record (it was saved as a
    /// candidate, or it is the object at a record's place). Decide there first.
    CandidatePending,
    /// The chosen file is not the physical object the record was watching
    /// (a copy, a file on another volume, a file with other links, or another
    /// file): the record does not go on there (#31 M2). Nothing was changed.
    NotSameObject,
    /// No version relation has this ID (#31 M2b).
    UnknownRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStage {
    Storage,
    Authorization,
    Observation,
    Plan,
    Execution,
    /// Setting up or asking the model.
    Analysis,
    /// Memory (#65): reading saved versions now; more Memory errors later.
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextStep {
    /// Quit Traeva, fix the cause shown in the message, then open Traeva again.
    RestartApp,
    /// Choose a folder again: the same one after fixing the cause, or another
    /// one. After a failed model request: pick the target folder by hand.
    ChooseFolder,
    /// Choose a file again.
    ChooseFile,
    /// Wait a moment (for example until an editor finishes saving), then try again.
    TryAgain,
    /// Build a new plan (look at the file and folders again) and preview it.
    PreviewAgain,
}

/// What the user asks for: move this file to a target. Unknown fields are
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub file_id: String,
    /// The file revision the user saw; an older one means the file changed.
    pub file_revision: i64,
    pub target_root_id: String,
    /// The operation the choice was made in, when the file went through an
    /// analysis (#16). Without it the choice is a plain manual one.
    #[serde(default)]
    pub operation_id: Option<String>,
    pub target: PlanTarget,
}

/// Where the file should go.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanTarget {
    /// A candidate of the operation's current suggestion (needs `operation_id`).
    /// Its folder and new folders come from Traeva's own copy of the result.
    Candidate { candidate_id: String },
    /// A folder from `FolderCatalogView` (folder IDs only mean something
    /// within one listing), plus new folders inside it, outermost first.
    Folder {
        catalog_id: String,
        folder_id: String,
        #[serde(default)]
        new_folders: Vec<String>,
    },
}

/// How the user chose an organize plan's target (organize-spec.md §3; #16
/// material confirmation 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserChoice {
    /// The recommended candidate.
    Accept,
    /// Another target while a recommendation was shown.
    Correct,
    /// Picked without a recommendation.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BuildOutcome {
    Planned {
        plan: PlanView,
    },
    /// The file is already there: no plan, no action, nothing to undo.
    AlreadyInPlace {
        file_id: String,
    },
}

/// An immutable plan as the window shows it. Everything except `lifecycle`,
/// `status`, `can_confirm`, `reason` and `action_id` was fixed when the plan
/// was saved and never changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanView {
    pub plan_id: String,
    pub kind: PlanKind,
    pub file_id: String,
    pub file_revision: i64,
    pub source: PlanEnd,
    /// The full target path, including new folders and the unchanged file name.
    pub target: PlanEnd,
    pub basename: String,
    /// Every folder that will be created, outermost first, as paths inside the
    /// target root. Empty when no folder needs to be created.
    pub folders_to_create: Vec<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub lifecycle: PlanLifecycle,
    /// Worked out when read, never stored.
    pub status: PlanStatus,
    pub can_confirm: bool,
    /// Why the plan cannot be confirmed, or null.
    pub reason: Option<String>,
    /// Set once an action has used this plan.
    pub action_id: Option<String>,
    /// For an undo plan: the organize action it puts back.
    pub undo_of_action_id: Option<String>,
    /// For an organize plan: how the target was chosen.
    pub user_choice: Option<UserChoice>,
    /// The model request whose suggestion was shown, if any.
    pub analysis_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanEnd {
    pub root_id: String,
    pub relative_path: String,
    pub display_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Organize,
    Undo,
}

/// The stored part of a plan's life. Only an open plan can change; the
/// other three are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanLifecycle {
    Open,
    Cancelled,
    Superseded,
    Invalidated,
}

/// Whether a plan can be confirmed now, worked out from stored facts each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Confirmable,
    /// An action already used this plan; that action has the result.
    Consumed,
    Cancelled,
    Superseded,
    Invalidated,
    /// Older than 15 minutes (or the clock went backwards).
    Expired,
    /// The source or target root is no longer authorized.
    RootRevoked,
}

/// The one action a confirmed plan became, as the window shows it. Read from
/// the database every time, so it looks the same after a restart. Asking for
/// it never runs anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionView {
    pub action_id: String,
    pub plan_id: String,
    pub kind: PlanKind,
    pub file_id: String,
    pub status: ActionStatus,
    /// Where the file was when the plan was confirmed.
    pub source: PlanEnd,
    /// Where the plan moves it.
    pub target: PlanEnd,
    /// Folders this action created and holds a receipt for, outermost first,
    /// as paths inside the target root.
    pub folders_created: Vec<String>,
    /// Empty folders this action removed again: an organize action its own
    /// ones after a record check, an undo the ones the organize action created.
    pub folders_removed: Vec<String>,
    /// Folders this action was about to create but holds no receipt for.
    /// Traeva does not treat them as its own and never removes them.
    pub folders_without_receipt: Vec<String>,
    /// For a committed undo: folders the organize action created that were
    /// kept (not empty, changed, or not checkable). Not empty = the cleanup
    /// warning. The file itself is back either way.
    pub folders_kept: Vec<String>,
    pub created_at_ms: i64,
    pub committed_at_ms: Option<i64>,
    pub recovered_at_ms: Option<i64>,
    /// Why the action failed or needs a decision; null otherwise.
    pub problem: Option<AppError>,
    /// For an undo: the organize action it puts back.
    pub undo_of_action_id: Option<String>,
    /// For an organize action: whether it can be undone, worked out from the
    /// saved records only. Previewing the undo checks the files again.
    pub undo_status: UndoStatus,
    /// The latest undo of this action that is in progress or committed.
    pub undo_action_id: Option<String>,
}

/// Whether an organize action can be undone, from saved records only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoStatus {
    /// Committed and not undone, with no undo in progress.
    Available,
    /// An undo is prepared, running or waiting for a decision.
    InProgress,
    /// A committed undo put the file back.
    Undone,
    /// Not an organized file: not committed, or itself an undo.
    NotAvailable,
    /// Organized and not undone, but a folder it used (where the file was or
    /// where it went) was removed from Traeva. Removing a folder is final, and
    /// adding it again gives a new root (organize-spec.md §2.1), so this
    /// action cannot be undone any more (#79). The file itself is unchanged.
    RootRevoked,
}

/// What asking to undo an organize action gives (organize-spec.md §9
/// `preview_undo`). Undo conflicts are errors (`undo_conflict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UndoPreview {
    /// A new, immutable undo plan, ready to confirm.
    Planned { plan: PlanView },
    /// Already undone: the undo that did it. Nothing new is created.
    AlreadyUndone { undo: ActionView },
    /// An undo is in progress or waiting for a decision: that undo.
    UndoInProgress { undo: ActionView },
}

/// The stored state of an action (organize-spec.md §6). Only `committed`
/// means the file moved and everything was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    /// Confirmed; nothing has changed on disk yet.
    Prepared,
    /// Creating folders or moving the file.
    Running,
    /// Moved, checked and recorded.
    Committed,
    /// Failed, with enough evidence that the file did not move.
    FailedNoMove,
    /// The result is uncertain. Traeva moves nothing automatically.
    NeedsUserDecision,
    /// A committed Undo put the file back (comes with Undo, #11).
    Undone,
}

/// What the window shows about a registered file. Never contains file text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileView {
    pub file_id: String,
    /// Goes up whenever a new observation differs from the saved one; plans
    /// built for an older revision are stale.
    pub file_revision: i64,
    pub root_id: String,
    pub relative_path: String,
    pub display_path: String,
    pub byte_length: u64,
    /// SHA-256 of the file's exact bytes (including any BOM), in hex.
    pub sha256: String,
    pub extraction: ExtractionSummary,
}

/// The text extraction result, without the text itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtractionSummary {
    pub status: ExtractionStatus,
    /// Unicode scalar values in the whole text (after removing a BOM and
    /// turning CRLF/CR into LF); 0 when nothing was extracted.
    pub total_chars: usize,
    /// True when only the first 6,000 and last 2,000 characters are used.
    pub truncated: bool,
    /// True when the text is empty or only whitespace: the user picks by hand.
    pub blank: bool,
    /// True when some of the text read can be sent as a text piece. Without
    /// one the model is not asked and the user picks the folder by hand.
    pub usable_text: bool,
    /// Why extraction was unsupported or failed.
    pub note: Option<String>,
    /// True when the text was read by on-device text recognition from a
    /// scanned PDF's pages (organize-spec.md §14.8): always `partial`, may
    /// contain mistakes, and never moved by the automatic mode (§13.4).
    pub recognized: bool,
    /// True when Organize read a JPEG or PNG image, or a PDF with no text
    /// layer at all: with sending images on, a smaller copy may go to the
    /// model (§14.10). Never moved by the automatic mode.
    pub image_input: bool,
}

/// Extraction status is separate from whether the observation itself is valid:
/// a stable, hashed PDF is a valid observation with status `unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    /// The whole text is available.
    Ok,
    /// Part of the text: only the head and tail are used (`truncated`), or
    /// the reader left part of the file out, such as a PDF's images
    /// (`note` says what; #196).
    Partial,
    /// Not a supported text file (type, UTF-16 or NUL bytes).
    Unsupported,
    /// Should be text, but is not valid UTF-8.
    Failed,
}

/// The bounded folder list of one root (organize-spec.md §4: at most 500
/// folders, 6 levels). Folder IDs are only valid within this one listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderCatalogView {
    pub root_id: String,
    /// Identifies this listing. A plan request names it together with a
    /// folder ID; a newer listing makes older IDs unusable.
    pub catalog_id: String,
    pub folders: Vec<FolderView>,
    /// True when the 500-folder or 6-level limit left folders out.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderView {
    pub folder_id: String,
    pub parent_id: Option<String>,
    /// Empty for the root itself.
    pub relative_path: String,
    pub depth: usize,
    /// Why this folder can never be a target (a project folder), or null.
    pub protected: Option<String>,
}

/// A folder the user authorized. Only active or revoked; nothing in between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootView {
    pub root_id: String,
    pub display_path: String,
    pub status: RootStatus,
    pub granted_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    /// Worked out each time it is read: why this active root cannot be used
    /// right now, or null when it can.
    pub problem: Option<AppError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootStatus {
    Active,
    Revoked,
}

impl AppError {
    /// A storage error. Retryable ones tell the user to restart after fixing the cause;
    /// the others need a person to look at the database file, which Traeva keeps as is.
    pub fn storage(code: ErrorCode, message: impl Into<String>) -> AppError {
        // OverlapsAppData here: the data folder was moved into an authorized
        // folder while Traeva was closed (#47); moving it out fixes it.
        let retryable = matches!(
            code,
            ErrorCode::AppDataUnavailable
                | ErrorCode::AnotherInstanceRunning
                | ErrorCode::DatabaseUnavailable
                | ErrorCode::OverlapsAppData
        );
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Storage,
            retryable,
            file_id: None,
            plan_id: None,
            action_id: None,
            allowed_next: if retryable {
                vec![NextStep::RestartApp]
            } else {
                Vec::new()
            },
        }
    }

    /// An error while building, checking or cancelling a plan.
    pub fn plan(
        code: ErrorCode,
        message: impl Into<String>,
        file_id: Option<&str>,
        plan_id: Option<&str>,
    ) -> AppError {
        let allowed_next = match code {
            ErrorCode::TargetConflict
            | ErrorCode::FolderListOutdated
            | ErrorCode::SuggestionOutdated => vec![NextStep::ChooseFolder],
            ErrorCode::InvalidFolderName => vec![NextStep::ChooseFolder],
            ErrorCode::FileChanged => vec![NextStep::ChooseFile],
            ErrorCode::StalePlan => vec![NextStep::PreviewAgain],
            ErrorCode::FileChanging => vec![NextStep::TryAgain, NextStep::PreviewAgain],
            _ => Vec::new(),
        };
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Plan,
            retryable: code == ErrorCode::FileChanging,
            file_id: file_id.map(str::to_string),
            plan_id: plan_id.map(str::to_string),
            action_id: None,
            allowed_next,
        }
    }

    /// An error about an action: why it could not start, or why it failed.
    /// Never retryable as is: trying again means a new plan and a new
    /// confirmation.
    pub fn execution(
        code: ErrorCode,
        message: impl Into<String>,
        file_id: Option<&str>,
        plan_id: Option<&str>,
        action_id: Option<&str>,
    ) -> AppError {
        let allowed_next = match code {
            ErrorCode::TargetConflict => vec![NextStep::ChooseFolder],
            ErrorCode::FileChanged | ErrorCode::FileChanging => vec![NextStep::ChooseFile],
            ErrorCode::DatabaseUnavailable | ErrorCode::DatabaseCorrupt => {
                vec![NextStep::RestartApp]
            }
            // Wait for the record check (#10); nothing to retry by hand.
            ErrorCode::ResultUnconfirmed
            | ErrorCode::UnresolvedAction
            | ErrorCode::UnknownAction
            | ErrorCode::InvalidInput
            | ErrorCode::UndoConflict
            | ErrorCode::NothingToUndo => Vec::new(),
            _ => vec![NextStep::PreviewAgain],
        };
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Execution,
            retryable: false,
            file_id: file_id.map(str::to_string),
            plan_id: plan_id.map(str::to_string),
            action_id: action_id.map(str::to_string),
            allowed_next,
        }
    }

    /// An error while choosing and reading a file. Only "still changing" is
    /// worth retrying as is; everything else needs another file.
    pub fn observation(code: ErrorCode, message: impl Into<String>) -> AppError {
        let changing = code == ErrorCode::FileChanging;
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Observation,
            retryable: changing,
            file_id: None,
            plan_id: None,
            action_id: None,
            allowed_next: if changing {
                vec![NextStep::TryAgain, NextStep::ChooseFile]
            } else {
                vec![NextStep::ChooseFile]
            },
        }
    }

    /// An error while setting up or asking the model. Nothing is ever moved
    /// because of it (organize-spec.md §9). After a failed request the user
    /// can try again (when `retryable`) or pick the folder by hand.
    pub fn analysis(
        code: ErrorCode,
        message: impl Into<String>,
        retryable: bool,
        file_id: Option<&str>,
    ) -> AppError {
        let allowed_next = match code {
            // Wrong input to a settings call or an unknown request ID:
            // nothing to retry and no folder to pick.
            ErrorCode::InvalidInput | ErrorCode::UnknownAnalysis | ErrorCode::WrongStage => {
                Vec::new()
            }
            // The file changed or is unknown: choose it again first.
            ErrorCode::FileChanged | ErrorCode::UnknownFile => vec![NextStep::ChooseFile],
            _ if retryable => vec![NextStep::TryAgain, NextStep::ChooseFolder],
            _ => vec![NextStep::ChooseFolder],
        };
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Analysis,
            retryable,
            file_id: file_id.map(str::to_string),
            plan_id: None,
            action_id: None,
            allowed_next,
        }
    }

    /// An error while reading a saved version (#65). Never retryable as is:
    /// a missing or damaged copy does not come back by trying again, and
    /// Traeva never rebuilds it from the file as it is now. Only for snapshot
    /// reads: other Memory errors (#33 and later) get their own constructor
    /// with their own `retryable` and next steps.
    pub fn snapshot(
        code: ErrorCode,
        message: impl Into<String>,
        file_id: Option<&str>,
    ) -> AppError {
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Memory,
            retryable: false,
            file_id: file_id.map(str::to_string),
            plan_id: None,
            action_id: None,
            allowed_next: Vec::new(),
        }
    }

    /// An error while authorizing, revoking or checking a root folder.
    /// Only a missing or unreachable folder can work later without a new choice.
    pub fn authorization(code: ErrorCode, message: impl Into<String>) -> AppError {
        let choose_folder = matches!(
            code,
            ErrorCode::RootRevoked
                | ErrorCode::RootUnavailable
                | ErrorCode::RootChanged
                | ErrorCode::UnsupportedLocation
                | ErrorCode::ProtectedLocation
                | ErrorCode::OverlapsAppData
        );
        AppError {
            code,
            message: message.into(),
            stage: ErrorStage::Authorization,
            retryable: code == ErrorCode::RootUnavailable,
            file_id: None,
            plan_id: None,
            action_id: None,
            allowed_next: if choose_folder {
                vec![NextStep::ChooseFolder]
            } else {
                Vec::new()
            },
        }
    }
}
