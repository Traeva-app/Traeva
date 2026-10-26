//! The PDF child for the core's own tests (#186): what Traeva's executable
//! does when started with `PDF_TEXT_FLAG` (src-tauri/src/main.rs), without
//! Tauri. The app never runs this file.

#[global_allocator]
static ALLOCATOR: traeva_core::CappedAlloc = traeva_core::CappedAlloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(traeva_core::pdf_child_main(&args));
}
