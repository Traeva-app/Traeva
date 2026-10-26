//! Thin commands: take input, call the core, return its result.
//! No SQL, no file operations, no organizing rules here. Only two things
//! happen here and not in the core: the native pickers are opened (so a path
//! always comes from the picker, never from the window), and the slow work
//! (reading a chosen file, waiting for the model) is started in the
//! background.
//!
//! Every command runs on Tauri's async thread pool (`async`), not the UI
//! thread. Arguments use snake_case names, like the rest of the contract.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath};
use traeva_core::{
    ActionStatus, ActionView, AppError, AppState, BuildOutcome, ClarificationAnswer,
    FolderCatalogView, ModelSettingsInput, ModelSettingsView, OcrState, OperationView,
    OrganizeService, PlanRequest, PlanView, RootView, UndoPreview,
};

type Service<'a> = State<'a, Arc<OrganizeService>>;

#[tauri::command(async)]
pub fn get_app_state(service: Service<'_>) -> Result<AppState, AppError> {
    service.get_app_state()
}

/// Opens the native folder picker and authorizes the chosen folder.
/// Null when the user closes the picker without choosing.
#[tauri::command(async)]
pub fn choose_root<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
) -> Result<Option<RootView>, AppError> {
    let picked = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a folder Traeva may organize")
        .blocking_pick_folder();
    match into_path(picked) {
        Some(path) => service.authorize_root(&path).map(Some),
        None => Ok(None),
    }
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn revoke_root(service: Service<'_>, root_id: String) -> Result<RootView, AppError> {
    service.revoke_root(&root_id)
}

/// Opens the native file picker in the source folder, then reads the
/// chosen file in the background. Returns the new operation right away;
/// the window polls `get_operation`. Null when the picker was closed.
#[tauri::command(async, rename_all = "snake_case")]
pub fn choose_file<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    source_root_id: String,
    target_root_id: String,
) -> Result<Option<OperationView>, AppError> {
    // Only where the picker opens; the core checks the chosen file itself.
    let start_in = service
        .list_roots()?
        .into_iter()
        .find(|root| root.root_id == source_root_id)
        .map(|root| PathBuf::from(root.display_path));
    // A sheet of the window that asked, as the dialog plugin's own open
    // command does; otherwise AppKit picks whichever window is main, which
    // can be the desktop pet.
    let mut picker = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a file to organize");
    if let Some(folder) = start_in {
        picker = picker.set_directory(folder);
    }
    let Some(path) = into_path(picker.blocking_pick_file()) else {
        return Ok(None);
    };
    let operation = service.start_operation(&source_root_id, &target_root_id)?;
    let service = Arc::clone(&service);
    let operation_id = operation.operation_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.run_operation(&operation_id, &path));
    Ok(Some(operation))
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn get_operation(
    service: Service<'_>,
    operation_id: String,
) -> Result<OperationView, AppError> {
    service.get_operation(&operation_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn cancel_operation(
    service: Service<'_>,
    operation_id: String,
) -> Result<OperationView, AppError> {
    service.cancel_operation(&operation_id)
}

/// Starts the analysis of a read file (or retries a failed step) and waits
/// for the model in the background. The window polls `get_operation`. A
/// second call while analyzing sends nothing more (the core runs each
/// request once).
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_analysis(
    service: Service<'_>,
    operation_id: String,
    send_image: Option<bool>,
) -> Result<OperationView, AppError> {
    // `send_image`: the user confirmed sending this file's image (§14.10).
    let view = service.start_analysis_sending(&operation_id, send_image.unwrap_or(false))?;
    run_analysis_in_background(&service, operation_id);
    Ok(view)
}

/// Answers the current question; the core locks the answer, then the request
/// after it runs in the background.
#[tauri::command(async, rename_all = "snake_case")]
pub fn answer_clarification(
    service: Service<'_>,
    operation_id: String,
    answer: ClarificationAnswer,
) -> Result<OperationView, AppError> {
    let view = service.answer_clarification(&operation_id, &answer)?;
    run_analysis_in_background(&service, operation_id);
    Ok(view)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn choose_manual_location(
    service: Service<'_>,
    operation_id: String,
) -> Result<OperationView, AppError> {
    service.choose_manual_location(&operation_id)
}

/// The model setup for the settings card: provider, model, whether a key is
/// saved (never the key), consent, and the notice of what is sent.
#[tauri::command(async)]
pub fn get_model_settings(service: Service<'_>) -> Result<ModelSettingsView, AppError> {
    service.model_settings()
}

/// Saves the provider and model, and a new API key if one is given. The key
/// goes straight to the Keychain through the core; it is not kept here.
#[tauri::command(async, rename_all = "snake_case")]
pub fn save_model_settings(
    service: Service<'_>,
    input: ModelSettingsInput,
) -> Result<ModelSettingsView, AppError> {
    service.save_model_settings(&input)
}

/// Agrees to (or withdraws from) sending to the saved provider and model.
/// This is not permission to move anything.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_analysis_consent(
    service: Service<'_>,
    agree: bool,
) -> Result<ModelSettingsView, AppError> {
    service.set_analysis_consent(agree)
}

/// Turns sending a smaller copy of images to the model on (asking each time
/// or without asking) or off (organize-spec §14.10). Main window only.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_image_sending(
    service: Service<'_>,
    mode: traeva_core::ImageSending,
) -> Result<ModelSettingsView, AppError> {
    service.set_image_sending(mode)
}

/// Where on-device text recognition for scanned PDFs stands (§14.8): off,
/// being prepared, ready, or not available until the next launch.
#[tauri::command(async)]
pub fn get_ocr_state(service: Service<'_>) -> Result<OcrState, AppError> {
    Ok(service.ocr_state())
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn list_folders(service: Service<'_>, root_id: String) -> Result<FolderCatalogView, AppError> {
    service.list_folders(&root_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn build_plan(service: Service<'_>, request: PlanRequest) -> Result<BuildOutcome, AppError> {
    service.build_plan(&request)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn get_plan(service: Service<'_>, plan_id: String) -> Result<PlanView, AppError> {
    service.get_plan(&plan_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn cancel_plan(service: Service<'_>, plan_id: String) -> Result<PlanView, AppError> {
    service.cancel_plan(&plan_id)
}

/// Confirms and runs the plan; resolves when the action is settled. The
/// window polls `get_plan` and `get_action` meanwhile. Closing the window
/// or a timeout does not cancel the run, and calling again for the same
/// plan returns the same action. After a finished move, the switch for
/// keeping versions (#201) runs in the background; the core decides
/// whether it applies, and it never changes the move's result.
#[tauri::command(async, rename_all = "snake_case")]
pub fn apply_plan(service: Service<'_>, plan_id: String) -> Result<ActionView, AppError> {
    let action = service.apply_plan(&plan_id)?;
    if action.status == ActionStatus::Committed {
        let service = Arc::clone(&service);
        let action_id = action.action_id.clone();
        tauri::async_runtime::spawn_blocking(move || service.keep_versions_after(&action_id));
    }
    Ok(action)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn get_action(service: Service<'_>, action_id: String) -> Result<ActionView, AppError> {
    service.get_action(&action_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn preview_undo(service: Service<'_>, action_id: String) -> Result<UndoPreview, AppError> {
    service.preview_undo(&action_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn reconcile_action(service: Service<'_>, action_id: String) -> Result<ActionView, AppError> {
    service.reconcile_action(&action_id)
}

/// Runs the request the operation waits for, off the async pool: it can wait
/// up to 60 seconds for the model and holds no lock while waiting.
fn run_analysis_in_background(service: &Arc<OrganizeService>, operation_id: String) {
    let service = Arc::clone(service);
    tauri::async_runtime::spawn_blocking(move || service.run_analysis(&operation_id));
}

fn into_path(picked: Option<FilePath>) -> Option<PathBuf> {
    picked.and_then(|path| path.into_path().ok())
}
