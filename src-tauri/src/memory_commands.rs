//! Memory's commands for #33 (changes between versions), #34 (finding a
//! version by a change), #35 (opening and exporting an old version) and
//! #66 (a saved .docx version's text, previewed in the window; A's wiring).
//! Thin, like commands.rs: take input, call the core, return its result.
//! Three things happen here and not in the core: waiting for the model is
//! started in the background (the window polls `get_memory_search`), the
//! system is asked to open a copy (`/usr/bin/open`), and the native save
//! panel is opened for an export (so its place comes from the panel).
//!
//! Turning Memory on or off for a file and the list of watched files are
//! Claude-A's commands (#31/#32, family_commands.rs).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use traeva_core::{
    AppError, ChangeLinesView, DocxPreview, ExportPreview, ExportRequest, ExportedVersion,
    MemoryConsentView, OpenedVersion, OrganizeService, SearchAnswer, SearchState, SearchView,
    VersionPairView, VersionPreview,
};

type Service<'a> = State<'a, Arc<OrganizeService>>;

/// File `file_id`'s Memory agreement: whether its changed lines may be sent
/// to the model, and the notice of what is sent. Separate from Organize's.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_memory_consent(
    service: Service<'_>,
    file_id: String,
) -> Result<MemoryConsentView, AppError> {
    service.memory_consent(&file_id)
}

/// Agrees to (or withdraws) sending file `file_id`'s changed lines to the
/// saved provider and model, now and in later automatic analysis.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_memory_consent(
    service: Service<'_>,
    file_id: String,
    agree: bool,
) -> Result<MemoryConsentView, AppError> {
    service.set_memory_consent(&file_id, agree)
}

/// Every pair of consecutive saved versions of the file, oldest first, with
/// what Memory knows about each.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_memory_changes(
    service: Service<'_>,
    file_id: String,
) -> Result<Vec<VersionPairView>, AppError> {
    service.memory_changes(&file_id)
}

/// The lines one change is about, read again from its two saved versions.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_change_lines(
    service: Service<'_>,
    change_id: String,
) -> Result<ChangeLinesView, AppError> {
    service.change_lines(&change_id)
}

/// Starts a search in the file's history; the model is asked in the
/// background. The question stays in memory only.
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_memory_search(
    service: Service<'_>,
    file_id: String,
    question: String,
) -> Result<SearchView, AppError> {
    let view = service.start_memory_search(&file_id, &question)?;
    run_search_in_background(&service, &view);
    Ok(view)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn get_memory_search(service: Service<'_>, search_id: String) -> Result<SearchView, AppError> {
    service.memory_search(&search_id)
}

/// Answers the question shown, picks a candidate, or says none of them is
/// it. When that needs the model again (the next question), it is asked in
/// the background.
#[tauri::command(async, rename_all = "snake_case")]
pub fn answer_memory_search(
    service: Service<'_>,
    search_id: String,
    answer: SearchAnswer,
) -> Result<SearchView, AppError> {
    let view = service.answer_memory_search(&search_id, &answer)?;
    run_search_in_background(&service, &view);
    Ok(view)
}

/// Tries a failed search again, in the background.
#[tauri::command(async, rename_all = "snake_case")]
pub fn retry_memory_search(
    service: Service<'_>,
    search_id: String,
) -> Result<SearchView, AppError> {
    let view = service.retry_memory_search(&search_id)?;
    run_search_in_background(&service, &view);
    Ok(view)
}

/// Cancels a search; a late answer from the model is dropped.
#[tauri::command(async, rename_all = "snake_case")]
pub fn cancel_memory_search(
    service: Service<'_>,
    search_id: String,
) -> Result<SearchView, AppError> {
    service.cancel_memory_search(&search_id)
}

/// Makes a new read-only copy of saved version `snapshot_id` in Traeva's
/// data folder and, for a text file Memory reads, asks the system to open
/// it with the usual app. The user's file and the saved version are never
/// opened or changed.
#[tauri::command(async, rename_all = "snake_case")]
pub fn open_memory_version(
    service: Service<'_>,
    snapshot_id: String,
) -> Result<OpenedVersion, AppError> {
    let copy = service.copy_for_opening(&snapshot_id)?;
    // Only a text file Memory reads goes to the system (opened.rs).
    let opened = copy.can_open && open_with_the_usual_app(Path::new(&copy.path));
    Ok(OpenedVersion { copy, opened })
}

/// The text of saved .docx version `snapshot_id`, for a plain-text preview
/// in the window (#66): read from the saved version, never from the user's
/// file. Nothing is opened, copied, stored or sent; old Word versions are
/// still not opened in Word.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_docx(
    service: Service<'_>,
    snapshot_id: String,
) -> Result<DocxPreview, AppError> {
    service.preview_docx_version(&snapshot_id)
}

/// The text of saved version `snapshot_id`, whatever its type, for a
/// plain-text preview in the window (contract §4.7, v23): read from the
/// saved version, never from the user's file. Hidden slides and sheets are
/// shown marked as hidden. Nothing is opened, copied, stored or sent.
#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_memory_text(
    service: Service<'_>,
    snapshot_id: String,
) -> Result<VersionPreview, AppError> {
    service.preview_version_text(&snapshot_id)
}

/// Opens the native save panel in root `root_id`, with `name` filled in,
/// and shows where a copy of version `snapshot_id` would go. Nothing is
/// written. Null when the panel was closed.
#[tauri::command(async, rename_all = "snake_case")]
pub fn choose_memory_export<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    snapshot_id: String,
    root_id: String,
    name: String,
) -> Result<Option<ExportPreview>, AppError> {
    // Only where the panel opens; the core checks the chosen place itself.
    let start_in = service
        .list_roots()?
        .into_iter()
        .find(|root| root.root_id == root_id)
        .map(|root| PathBuf::from(root.display_path));
    let mut panel = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Export a copy of this version")
        .set_file_name(name);
    if let Some(folder) = start_in {
        panel = panel.set_directory(folder);
    }
    match panel
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
    {
        Some(path) => service
            .preview_export(&snapshot_id, &root_id, &path)
            .map(Some),
        None => Ok(None),
    }
}

/// Writes the copy where the preview showed, never over a file.
#[tauri::command(async, rename_all = "snake_case")]
pub fn export_memory_version(
    service: Service<'_>,
    request: ExportRequest,
) -> Result<ExportedVersion, AppError> {
    service.export_version(&request)
}

/// `/usr/bin/open` with one argument, the copy's absolute path. True when
/// it says the file was opened.
fn open_with_the_usual_app(path: &Path) -> bool {
    path.is_absolute()
        && Command::new("/usr/bin/open")
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
}

/// Runs the model part of a search that is `working` to its end, off the
/// async pool: understanding the question, then the question to ask when
/// several versions match (R2-MEM-03: one step alone left a search that
/// needs a question working for ever). Each step can wait up to 60 seconds
/// and holds no lock while it waits. The result is read with
/// `get_memory_search`.
fn run_search_in_background(service: &Arc<OrganizeService>, view: &SearchView) {
    if view.state != SearchState::Working {
        return;
    }
    let service = Arc::clone(service);
    let search_id = view.search_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.run_memory_search_to_end(&search_id));
}
