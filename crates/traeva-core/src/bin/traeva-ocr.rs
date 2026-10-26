//! The text recognition children for the core's own tests (#186, §14.8):
//! what Traeva's executable does when started with `OCR_FLAG` or
//! `OCR_PREPARE_FLAG` (src-tauri/src/main.rs), without Tauri. The app
//! never runs this file.

#[global_allocator]
static ALLOCATOR: traeva_core::CappedAlloc = traeva_core::CappedAlloc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some(traeva_core::OCR_FLAG) => traeva_core::ocr_child_main(&args[2..]),
        Some(traeva_core::OCR_PREPARE_FLAG) => traeva_core::ocr_prepare_main(&args[2..]),
        _ => 2,
    };
    std::process::exit(code);
}
