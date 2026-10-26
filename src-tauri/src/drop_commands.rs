//! Files dropped on Traeva's icon (#168, organize-spec.md §13): the
//! automatic mode's settings, the queue, and its background pass. Thin, like
//! commands.rs: the core decides everything; here only the background work
//! is started (the queue reads files and asks the model, which is slow).

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;
use traeva_core::{
    AppError, AutoDropRequest, AutoDropSettingsView, DropQueueView, OperationView, OrganizeService,
};

type Service<'a> = State<'a, Arc<OrganizeService>>;

/// The automatic mode now: on or off, the notice, the target range.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_auto_drop_settings(service: Service<'_>) -> Result<AutoDropSettingsView, AppError> {
    service.auto_drop_settings()
}

/// Turns the automatic mode on (the user agreed to the notice shown, with
/// the target range) or off.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_auto_drop_settings(
    service: Service<'_>,
    request: AutoDropRequest,
) -> Result<AutoDropSettingsView, AppError> {
    service.set_auto_drop(&request)
}

/// The dropped files and where each stands. Asking starts nothing.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_drop_queue(service: Service<'_>) -> Result<DropQueueView, AppError> {
    Ok(service.drop_queue())
}

/// The user agrees to authorize the folder of dropped file `drop_id` (for
/// good, like Add folder); the queue goes on in the background.
#[tauri::command(async, rename_all = "snake_case")]
pub fn authorize_drop_source(
    service: Service<'_>,
    drop_id: String,
) -> Result<DropQueueView, AppError> {
    let queue = service.authorize_drop_source(&drop_id)?;
    run_drops_in_background(Arc::clone(service.inner()));
    Ok(queue)
}

/// Cancels dropped file `drop_id`; the files after it are not processed.
#[tauri::command(async, rename_all = "snake_case")]
pub fn cancel_drop(service: Service<'_>, drop_id: String) -> Result<DropQueueView, AppError> {
    service.cancel_drop(&drop_id)
}

/// Goes on with the queue after the user acted (answered the question, or
/// finished a file in the normal flow).
#[tauri::command(async, rename_all = "snake_case")]
pub fn continue_drops(service: Service<'_>) -> Result<DropQueueView, AppError> {
    run_drops_in_background(Arc::clone(service.inner()));
    Ok(service.drop_queue())
}

/// With the automatic mode off: starts the normal operation for dropped file
/// `drop_id` into root `target_root_id`, and reads it in the background,
/// as for a chosen file.
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_drop_operation(
    service: Service<'_>,
    drop_id: String,
    target_root_id: String,
) -> Result<OperationView, AppError> {
    let (view, path) = service.start_drop_operation(&drop_id, &target_root_id)?;
    let reader = Arc::clone(service.inner());
    let operation_id = view.operation_id.clone();
    tauri::async_runtime::spawn_blocking(move || reader.run_operation(&operation_id, &path));
    Ok(view)
}

/// Files the system handed over (dropped on the Dock icon, or "Open With"):
/// queued in order, then worked through in the background.
pub fn receive_opened_files(service: &Arc<OrganizeService>, paths: Vec<PathBuf>) {
    if service.receive_drops(&paths).is_ok() {
        run_drops_in_background(Arc::clone(service));
    }
}

/// One background pass over the queue; a pass already running makes this
/// one return at once.
fn run_drops_in_background(service: Arc<OrganizeService>) {
    tauri::async_runtime::spawn_blocking(move || service.run_drops());
}
