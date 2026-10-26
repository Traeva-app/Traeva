pub use traeva_core::{
    AppError, MAX_MEMORY_TEXT_CHARS, OrganizeService, SnapshotRef, TextExtraction, TextStatus,
    TextSummary,
};

pub mod prototype {
    include!(concat!(env!("OUT_DIR"), "/reader.rs"));
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod history_tests;

#[cfg(test)]
#[path = "../../src/docx.rs"]
pub mod unchanged_production_reader;
