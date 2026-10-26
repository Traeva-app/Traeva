//! In-memory historical preview, bound to verified Snapshot bytes.
use super::{DOCX_CACHED_READER, TextExtraction, read_docx_cached, summarize, unsupported};
use crate::{AppError, OrganizeService, SnapshotRef};

/// Local plain text only: no viewer launch, file copy, source fallback or model call.
/// The UI adapter must render `text` as escaped plain text and keep the summary visible.
#[allow(clippy::result_large_err)] // Keep the shared AppError return type.
pub fn preview_docx_snapshot(
    service: &OrganizeService,
    snapshot_id: &str,
) -> std::result::Result<(SnapshotRef, TextExtraction), AppError> {
    let (snapshot, bytes) = service.read_snapshot(snapshot_id)?;
    let docx = std::path::Path::new(&snapshot.relative_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"));
    let extraction = if docx {
        read_docx_cached(&bytes)
    } else {
        summarize(
            Err(unsupported(
                "Local DOCX preview requires a .docx saved version.",
            )),
            DOCX_CACHED_READER,
        )
    };
    Ok((snapshot, extraction))
}
