//! Memory's record commands (#31/#32, Claude-A): turning Memory on and off
//! for a file, the list of records, one work, candidate capture, the
//! decisions about candidates, and the decisions about works (merge,
//! associate a file, revoke or confirm again a relation, move members). Thin, like commands.rs:
//! take input, call the core, return its result. The only thing that happens
//! here and not in the core: the native file picker is opened, so a path
//! always comes from the picker. The exceptions are confirming a file's new
//! place and confirming an association with a picked file: the window names
//! again the place the picker returned to the preview, as a plain relative
//! path, and the core refuses any other (the page token covers it).
//!
//! The commands that read a file (turning Memory on, turning candidate
//! capture on with an object already at the place, deciding to keep the
//! unread object at the place, confirming a file's new place, and the
//! association page and its confirmation for a picked file) run
//! synchronously on
//! Tauri's async pool, like `apply_plan`: a stable read and the execution
//! lock take at most a few seconds. Version pairs and `memory_watches_of`
//! are for the core's own features (#33/#34) and have no command.
//!
//! The switch for keeping versions of files Traeva organizes (#201) is
//! here too; the window turns it on or off, and `apply_plan` runs it after
//! a move.
//!
//! B's #33/#34/#35 commands are in memory_commands.rs.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath};
use traeva_core::{
    AppError, AssociationPreview, AssociationRequest, AutoMemoryRequest, AutoMemoryView,
    CandidateCaptureInfo, CandidateDecisionOutcome, CandidateDecisionRequest,
    CandidateSwitchRequest, MemoryFamilyView, MemoryRecordView, MergePreview, MergeRequest,
    MovePreview, MoveQuery, MoveRequest, OrganizeService, PendingCandidatesView, RelationPreview,
    RelationRequest, RelocationOutcome, RelocationPreview, RelocationRequest, WorkDecisionOutcome,
    WorkSummaryView,
};

type Service<'a> = State<'a, Arc<OrganizeService>>;

/// Every Memory record: the ones on first, then lost, then stopped.
#[tauri::command(async)]
pub fn list_memory_records(service: Service<'_>) -> Result<Vec<MemoryRecordView>, AppError> {
    service.memory_records()
}

/// Opens the native file picker in root `root_id` and turns Memory on for
/// the chosen file. Null when the picker was closed.
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_memory_record<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    root_id: String,
) -> Result<Option<MemoryRecordView>, AppError> {
    // Only where the picker opens; the core checks the chosen file itself.
    let start_in = service
        .list_roots()?
        .into_iter()
        .find(|root| root.root_id == root_id)
        .map(|root| PathBuf::from(root.display_path));
    let mut picker = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a file for Memory to record");
    if let Some(folder) = start_in {
        picker = picker.set_directory(folder);
    }
    match into_path(picker.blocking_pick_file()) {
        Some(path) => service.start_memory_at(&root_id, &path).map(Some),
        None => Ok(None),
    }
}

/// Turns Memory on for a file Traeva already registered.
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_memory_for_file(
    service: Service<'_>,
    file_id: String,
) -> Result<MemoryRecordView, AppError> {
    service.start_memory_for_file(&file_id)
}

/// Turns Memory off for a record; its saved versions stay.
#[tauri::command(async, rename_all = "snake_case")]
pub fn stop_memory_record(
    service: Service<'_>,
    chain_id: String,
) -> Result<MemoryRecordView, AppError> {
    service.stop_memory_record(&chain_id)
}

/// One work: its files, records, relations and gaps.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_memory_family(
    service: Service<'_>,
    family_id: String,
) -> Result<MemoryFamilyView, AppError> {
    service.memory_family(&family_id)
}

/// What the notice before turning candidate capture on needs.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_candidate_capture(
    service: Service<'_>,
    chain_id: String,
) -> Result<CandidateCaptureInfo, AppError> {
    service.candidate_capture(&chain_id)
}

/// Turns candidate capture on (with the notice version and occupant token
/// the page showed) or off.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_candidate_capture(
    service: Service<'_>,
    request: CandidateSwitchRequest,
) -> Result<CandidateCaptureInfo, AppError> {
    service.set_candidate_capture(&request)
}

/// A record's confirmation page: its waiting candidates and the token.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_pending_candidates(
    service: Service<'_>,
    chain_id: String,
) -> Result<PendingCandidatesView, AppError> {
    service.pending_candidates(&chain_id)
}

/// Applies the user's choices on one page.
#[tauri::command(async, rename_all = "snake_case")]
pub fn decide_pending_candidates(
    service: Service<'_>,
    request: CandidateDecisionRequest,
) -> Result<CandidateDecisionOutcome, AppError> {
    service.decide_pending_candidates(&request)
}

/// Opens the native file picker in root `root_id` to choose where record
/// `chain_id`'s file is now, and shows what going on there would do. Null
/// when the picker was closed. Nothing is read or written.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_relocation<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    chain_id: String,
    root_id: String,
) -> Result<Option<RelocationPreview>, AppError> {
    // Only where the picker opens; the core checks the chosen file itself.
    let start_in = service
        .list_roots()?
        .into_iter()
        .find(|root| root.root_id == root_id)
        .map(|root| PathBuf::from(root.display_path));
    let mut picker = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose where this file is now");
    if let Some(folder) = start_in {
        picker = picker.set_directory(folder);
    }
    match into_path(picker.blocking_pick_file()) {
        Some(path) => service
            .relocation_preview(&chain_id, &root_id, &path)
            .map(Some),
        None => Ok(None),
    }
}

/// Confirms the new place a preview showed. The place must be the one the
/// picker returned: the page token covers it, so another one is refused.
#[tauri::command(async, rename_all = "snake_case")]
pub fn relocate_memory_record(
    service: Service<'_>,
    request: RelocationRequest,
) -> Result<RelocationOutcome, AppError> {
    service.relocate_record(&request)
}

/// Every live work, for choosing the other side of a merge (#31 M2b).
#[tauri::command(async)]
pub fn list_memory_works(service: Service<'_>) -> Result<Vec<WorkSummaryView>, AppError> {
    service.memory_works()
}

/// What merging work `other_family_id` into work `family_id` would do:
/// both works, and the earlier "unrelated" judgements it would override.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_merge(
    service: Service<'_>,
    family_id: String,
    other_family_id: String,
) -> Result<MergePreview, AppError> {
    service.merge_preview(&family_id, &other_family_id)
}

/// Confirms the merge page.
#[tauri::command(async, rename_all = "snake_case")]
pub fn merge_memory_works(
    service: Service<'_>,
    request: MergeRequest,
) -> Result<WorkDecisionOutcome, AppError> {
    service.merge_works(&request)
}

/// Opens the native file picker in root `root_id` to choose a file to
/// associate with work `family_id`, and shows the association page. Null
/// when the picker was closed. The chosen file is read only when it has no
/// saved version (for its SHA-256); nothing is written.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_association<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    family_id: String,
    root_id: String,
) -> Result<Option<AssociationPreview>, AppError> {
    // Only where the picker opens; the core checks the chosen file itself.
    let start_in = service
        .list_roots()?
        .into_iter()
        .find(|root| root.root_id == root_id)
        .map(|root| PathBuf::from(root.display_path));
    let mut picker = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a file to add to this work");
    if let Some(folder) = start_in {
        picker = picker.set_directory(folder);
    }
    match into_path(picker.blocking_pick_file()) {
        Some(path) => service
            .association_preview(&family_id, &root_id, &path)
            .map(Some),
        None => Ok(None),
    }
}

/// The association page for a file Traeva already knows (for example the
/// file a same-content hint is about). Reads no user file.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_association_for_file(
    service: Service<'_>,
    family_id: String,
    file_id: String,
) -> Result<AssociationPreview, AppError> {
    service.association_preview_for_file(&family_id, &file_id)
}

/// Confirms an association page. A picked file is named again by the place
/// the preview showed (`request_subject`); the page token covers it.
#[tauri::command(async, rename_all = "snake_case")]
pub fn associate_memory_file(
    service: Service<'_>,
    request: AssociationRequest,
) -> Result<WorkDecisionOutcome, AppError> {
    service.associate_file(&request)
}

/// What revoking or confirming again one relation would do, and which
/// records it would pause. Reads no user file.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_relation(
    service: Service<'_>,
    edge_id: String,
) -> Result<RelationPreview, AppError> {
    service.relation_preview(&edge_id)
}

/// Confirms a relation page.
#[tauri::command(async, rename_all = "snake_case")]
pub fn correct_memory_relation(
    service: Service<'_>,
    request: RelationRequest,
) -> Result<WorkDecisionOutcome, AppError> {
    service.correct_relation(&request)
}

/// What moving members out of a work would do: the relations that go
/// along or become disputed, and the records that go along or pause. Reads
/// no user file.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_move(
    service: Service<'_>,
    query: MoveQuery,
) -> Result<MovePreview, AppError> {
    service.move_preview(&query)
}

/// Confirms a move page.
#[tauri::command(async, rename_all = "snake_case")]
pub fn move_memory_members(
    service: Service<'_>,
    request: MoveRequest,
) -> Result<WorkDecisionOutcome, AppError> {
    service.move_members(&request)
}

fn into_path(picked: Option<FilePath>) -> Option<PathBuf> {
    picked.and_then(|path| path.into_path().ok())
}

/// The switch for keeping versions of files Traeva organizes (#201).
#[tauri::command(async)]
pub fn get_auto_memory(service: Service<'_>) -> Result<AutoMemoryView, AppError> {
    service.auto_memory()
}

/// Turns the switch on (with the notice version the page showed) or off.
#[tauri::command(async)]
pub fn set_auto_memory(
    service: Service<'_>,
    request: AutoMemoryRequest,
) -> Result<AutoMemoryView, AppError> {
    service.set_auto_memory(&request)
}
