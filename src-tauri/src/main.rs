// Counts allocations; only the PDF child (below) sets a cap (#186).
#[global_allocator]
static ALLOCATOR: traeva_core::CappedAlloc = traeva_core::CappedAlloc;

fn main() {
    // Started by the core as a child, with no window: to read one PDF's
    // text layer, to recognize the text of a scanned PDF, or to prepare
    // text recognition (#186, organize-spec.md §14.3 and §14.8).
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some(traeva_core::PDF_TEXT_FLAG) => {
            std::process::exit(traeva_core::pdf_child_main(&args[2..]));
        }
        Some(traeva_core::OCR_FLAG) => {
            std::process::exit(traeva_core::ocr_child_main(&args[2..]));
        }
        Some(traeva_core::OCR_PREPARE_FLAG) => {
            std::process::exit(traeva_core::ocr_prepare_main(&args[2..]));
        }
        _ => {}
    }
    traeva_app::run();
}
