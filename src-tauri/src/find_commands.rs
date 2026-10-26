//! Find's commands (#198): searching the authorized folders for a file.
//! Thin, like commands.rs: take input, call the core, return its result.
//! Two things happen here and not in the core: the search runs in the
//! background (the window polls `get_find`), and the system is asked to
//! show a result in Finder or open it (`/usr/bin/open`). The path comes from
//! the core, which checks the file is still the one the search saw; the
//! window only names the search and the result.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use tauri::State;
use traeva_core::{AppError, ErrorCode, FindSetupView, FindState, FindView, OrganizeService};

type Service<'a> = State<'a, Arc<OrganizeService>>;

/// Whether searches can use the model, and the notice of what is sent.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_find_setup(service: Service<'_>) -> Result<FindSetupView, AppError> {
    service.find_setup()
}

/// Agrees to (or withdraws) Find sending the typed question to the saved
/// provider and model. Separate from Organize's and Memory's agreements.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_find_consent(service: Service<'_>, agree: bool) -> Result<FindSetupView, AppError> {
    service.set_find_consent(agree)
}

/// Starts a search and runs it in the background; any earlier search still
/// working is cancelled.
#[tauri::command(async, rename_all = "snake_case")]
pub fn start_find(service: Service<'_>, question: String) -> Result<FindView, AppError> {
    let view = service.start_find(&question)?;
    if view.state == FindState::Working {
        let service = Arc::clone(&service);
        let find_id = view.find_id.clone();
        tauri::async_runtime::spawn_blocking(move || service.run_find(&find_id));
    }
    Ok(view)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn get_find(service: Service<'_>, find_id: String) -> Result<FindView, AppError> {
    service.find(&find_id)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn cancel_find(service: Service<'_>, find_id: String) -> Result<FindView, AppError> {
    service.cancel_find(&find_id)
}

/// The files opened in the last 30 days (#198), as a finished search: its
/// results open and show in Finder like a search's.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_recent_files(service: Service<'_>) -> Result<FindView, AppError> {
    service.recent_files()
}

/// Shows the result in Finder, selected (`open -R`).
#[tauri::command(async, rename_all = "snake_case")]
pub fn reveal_find_result(
    service: Service<'_>,
    find_id: String,
    result_id: String,
) -> Result<(), AppError> {
    let path = service.find_result_path(&find_id, &result_id)?;
    run_open(&["-R"], &path, "Finder could not show this file.")
}

/// Opens the result in its usual app, as a double-click in Finder would.
#[tauri::command(async, rename_all = "snake_case")]
pub fn open_find_result(
    service: Service<'_>,
    find_id: String,
    result_id: String,
) -> Result<(), AppError> {
    let path = service.find_result_path(&find_id, &result_id)?;
    run_open(&[], &path, "The file could not be opened.")
}

/// `/usr/bin/open` with `flags` and one absolute path (it starts with "/",
/// so it is never read as an option).
fn run_open(flags: &[&str], path: &Path, failed: &str) -> Result<(), AppError> {
    let opened = path.is_absolute()
        && Command::new("/usr/bin/open")
            .args(flags)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    if opened {
        Ok(())
    } else {
        Err(AppError::observation(ErrorCode::FileUnreadable, failed))
    }
}
