//! Compatibility exports; canonical parser and projection live in core xlsx*.rs.
pub use traeva_xlsx_experiment::{
    ProjectionMode, XLSX_PREVIEW_READER, XLSX_PROJECTION_REVISION, XLSX_READER,
    project_verified_snapshot, read_xlsx_memory, read_xlsx_preview,
};
