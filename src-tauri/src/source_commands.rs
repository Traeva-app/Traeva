use std::sync::Arc;

use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use traeva_core::{
    AppError, ErrorCode, OrganizeService, SourceLinkKind, SourceRecordView, SourceRelation,
    SourceState,
};

use crate::source_receiver::{SourceConnection, SourceReceiver};

type Service<'a> = State<'a, Arc<OrganizeService>>;

#[tauri::command(async)]
pub async fn source_state(service: Service<'_>) -> Result<SourceState, AppError> {
    let service = Arc::clone(&service);
    source_work(move || service.source_state()).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_set_enabled(
    service: Service<'_>,
    receiver: State<'_, Arc<SourceReceiver>>,
    enabled: bool,
) -> Result<SourceState, AppError> {
    let service = Arc::clone(&service);
    let receiver = Arc::clone(&receiver);
    source_work(move || receiver.set_enabled(&service, enabled)).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_refresh(
    service: Service<'_>,
    source_id: String,
) -> Result<SourceRecordView, AppError> {
    let service = Arc::clone(&service);
    source_work(move || service.refresh_source(&source_id)).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_choose_file<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    service: Service<'_>,
    source_id: String,
) -> Result<Option<SourceRecordView>, AppError> {
    let service = Arc::clone(&service);
    source_work(move || {
        let picked = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Locate the file for this source")
            .blocking_pick_file();
        let Some(path) = picked.and_then(|picked| picked.into_path().ok()) else {
            return Ok(None);
        };
        service.suggest_source_file(&source_id, &path).map(Some)
    })
    .await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_confirm(
    service: Service<'_>,
    source_id: String,
    file_id: String,
    relation: SourceRelation,
    note: String,
) -> Result<SourceRecordView, AppError> {
    let service = Arc::clone(&service);
    source_work(move || service.confirm_source(&source_id, &file_id, relation, &note)).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_reject(
    service: Service<'_>,
    source_id: String,
    file_id: String,
) -> Result<SourceRecordView, AppError> {
    let service = Arc::clone(&service);
    source_work(move || service.reject_source(&source_id, &file_id)).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_unlink(
    service: Service<'_>,
    source_id: String,
    note: String,
) -> Result<SourceRecordView, AppError> {
    let service = Arc::clone(&service);
    source_work(move || service.unlink_source(&source_id, &note)).await
}

#[tauri::command(async, rename_all = "snake_case")]
pub async fn source_open_link(
    service: Service<'_>,
    source_id: String,
    kind: SourceLinkKind,
) -> Result<(), AppError> {
    let service = Arc::clone(&service);
    source_work(move || {
        let link = service.source_link(&source_id, kind)?;
        open_http_link(&link)
    })
    .await
}

#[tauri::command(async)]
pub fn source_connection(
    service: Service<'_>,
    receiver: State<'_, Arc<SourceReceiver>>,
) -> Result<SourceConnection, AppError> {
    receiver.connection(service.source_capture_state()?.enabled)
}

async fn source_work<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| {
            AppError::observation(
                ErrorCode::FileUnreadable,
                "The source request could not finish. Refresh the source before retrying.",
            )
        })?
}

fn open_http_link(link: &str) -> Result<(), AppError> {
    // Core validates the URL. This guard also prevents option-shaped arguments.
    if !(link.starts_with("https://") || link.starts_with("http://")) {
        return Err(AppError::observation(
            ErrorCode::InvalidInput,
            "Only HTTP and HTTPS source links can be opened.",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("-u")
            .arg(link)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Err(AppError::observation(
        ErrorCode::FileUnreadable,
        "The source link could not be opened in your browser.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_web_links_before_starting_a_process() {
        for link in [
            "file:///private/tmp/example",
            "javascript:alert(1)",
            "data:text/plain,example",
            "blob:https://example.test/id",
            "custom-app://open",
            "-a Calculator",
        ] {
            assert_eq!(
                open_http_link(link).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }
}
