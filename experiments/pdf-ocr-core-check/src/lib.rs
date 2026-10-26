//! Compile unregistered core sources without changing the product manifest.
mod memory_contracts {
    pub(crate) use traeva_core::{TextExtraction, TextStatus, TextSummary};
}
mod reader {
    pub(crate) use traeva_core::MAX_MEMORY_TEXT_CHARS;
}
// Private product constant; check.py asserts equality with observe.rs.
mod observe {
    pub(crate) const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
}
pub mod pdf_child {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};
    include!(concat!(env!("OUT_DIR"), "/product_allocator.rs"));
}
#[allow(dead_code)]
#[path = "../../../crates/traeva-core/src/pdf.rs"]
mod pdf;
#[path = "../../../crates/traeva-core/src/pdf_ocr/mod.rs"]
pub mod pdf_ocr;
