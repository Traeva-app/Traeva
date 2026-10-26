//! Starts Traeva and holds the core service. No business rules live here.

mod command_names;
mod commands;
mod drop_commands;
mod family_commands;
mod find_commands;
mod memory_commands;
mod source_commands;
mod source_receiver;
mod window_commands;

use std::sync::Arc;

use tauri::Runtime;
use traeva_core::{MEMORY_CHECK_INTERVAL, OrganizeService};

pub use command_names::COMMANDS;

pub fn run() {
    // Open storage (and check unsettled actions) before the window exists:
    // the normal app-data folder, or the test folder named by
    // TRAEVA_APP_DATA_DIR (#65). If that fails, the service starts blocked
    // and the window shows the reason; it does not crash, retry, or fall
    // back to another folder.
    let service = Arc::new(OrganizeService::start_for_app());
    let executable = std::env::current_exe().ok();
    // Organize reads a PDF's text layer in a child: this same executable
    // with the PDF flag (organize-spec.md §14.3).
    service.set_pdf_helper(executable.clone().map(traeva_core::PdfHelper::traeva));
    // A PDF without a text layer: on-device text recognition, in another
    // child of this executable, prepared once per launch in the background
    // (§14.8). On by default (`OCR_ENABLED`); nothing else turns it on or
    // off. Before macOS 13 there is no recognition: it stays off.
    if traeva_core::OCR_ENABLED
        && traeva_core::ocr_supported_here()
        && let Some(executable) = executable
    {
        service.set_ocr_helper(Some(traeva_core::OcrHelper::traeva(executable)));
        start_ocr_preparation(Arc::clone(&service));
    }
    start_memory_watcher(Arc::clone(&service));
    start_memory_analysis(Arc::clone(&service));

    let app = app_builder(tauri::Builder::default(), Arc::clone(&service))
        .build(tauri::generate_context!())
        .expect("error while building Traeva");
    app.run(move |app, event| {
        // Quitting: a running text recognition child is stopped.
        if matches!(event, tauri::RunEvent::Exit) {
            service.stop_ocr();
        }
        #[cfg(target_os = "macos")]
        match event {
            // A file dropped on the Dock icon, or opened with Traeva (#168):
            // only file URLs go to the core, which decides what happens.
            tauri::RunEvent::Opened { urls } => {
                let paths: Vec<std::path::PathBuf> = urls
                    .into_iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .collect();
                if !paths.is_empty() {
                    drop_commands::receive_opened_files(&service, paths);
                }
            }
            // The Dock icon clicked: the main window comes back, also after
            // its close button hid it (#208).
            tauri::RunEvent::Reopen { .. } => {
                use tauri::Manager;
                if let Some(main) = app.get_webview_window(window_commands::MAIN) {
                    window_commands::show(&main);
                }
            }
            _ => {}
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (&service, app, event);
    });
}

/// Text recognition's preparation (§14.8): one run, up to 90 seconds, on
/// its own thread. The core keeps the state; a preparation that fails is
/// not tried again until the next launch. If the thread cannot start,
/// recognition stays unprepared and scanned PDFs go to the manual pick.
fn start_ocr_preparation(service: Arc<OrganizeService>) {
    let _ = std::thread::Builder::new()
        .name("ocr-preparation".to_string())
        .spawn(move || {
            service.start_ocr_preparation();
        });
}

/// Memory's watcher (#32): about every two seconds, one pass of
/// `check_memory` over the files Memory is on for. It runs for the life of
/// the app. A pass that fails (storage blocked, for example) is tried again
/// at the next one. If the thread cannot start, no new versions are saved
/// and the window shows that no file was looked at.
fn start_memory_watcher(service: Arc<OrganizeService>) {
    let _ = std::thread::Builder::new()
        .name("memory-watcher".to_string())
        .spawn(move || {
            loop {
                let _ = service.check_memory();
                std::thread::sleep(MEMORY_CHECK_INTERVAL);
            }
        });
}

/// Memory's analysis (#33): about every five seconds, one pass of
/// `analyze_memory`, on its own thread so its network wait (up to 60
/// seconds) never delays the watcher. Each pass checks the agreement, the
/// setup and what is due itself; a pass that fails is noted (its code only,
/// never a file name or text) and the next one runs as usual.
fn start_memory_analysis(service: Arc<OrganizeService>) {
    let _ = std::thread::Builder::new()
        .name("memory-analysis".to_string())
        .spawn(move || {
            loop {
                if let Err(error) = service.analyze_memory() {
                    eprintln!("Memory analysis pass did not finish: {:?}", error.code);
                }
                std::thread::sleep(MEMORY_ANALYSIS_INTERVAL);
            }
        });
}

/// The pause between two passes of Memory's analysis.
const MEMORY_ANALYSIS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// The app's plugin, state and command handlers, on any runtime: the app
/// uses the real one, the IPC tests a mock one with the same config.
pub fn app_builder<R: Runtime>(
    builder: tauri::Builder<R>,
    service: Arc<OrganizeService>,
) -> tauri::Builder<R> {
    // Source's local receiver (#27): off unless the user turned Source on;
    // then it listens on 127.0.0.1 only (source_receiver.rs).
    let source_receiver = Arc::new(source_receiver::SourceReceiver::restore(&service));
    builder
        .manage(source_receiver)
        // Native pickers, opened from Rust only: the window has no dialog
        // permission (see capabilities/main-window.json).
        .plugin(tauri_plugin_dialog::init())
        .manage(service)
        // The desktop pet (#208): its drops, the main window's close button,
        // and the page the pet asks the main window for.
        .manage(window_commands::PageRequest::default())
        .manage(window_commands::MovedNote::default())
        .on_window_event(window_commands::on_window_event)
        .setup(|app| {
            window_commands::place_pet(app.handle());
            Ok(())
        })
        // One handler per name in command_names.rs.
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::choose_root,
            commands::revoke_root,
            commands::choose_file,
            commands::get_operation,
            commands::cancel_operation,
            commands::start_analysis,
            commands::answer_clarification,
            commands::choose_manual_location,
            commands::get_model_settings,
            commands::save_model_settings,
            commands::set_analysis_consent,
            commands::set_image_sending,
            commands::list_folders,
            commands::build_plan,
            commands::get_plan,
            commands::cancel_plan,
            commands::apply_plan,
            commands::get_action,
            commands::preview_undo,
            commands::reconcile_action,
            memory_commands::get_memory_consent,
            memory_commands::set_memory_consent,
            memory_commands::get_memory_changes,
            memory_commands::get_change_lines,
            memory_commands::start_memory_search,
            memory_commands::get_memory_search,
            memory_commands::answer_memory_search,
            memory_commands::retry_memory_search,
            memory_commands::cancel_memory_search,
            memory_commands::open_memory_version,
            memory_commands::preview_memory_docx,
            memory_commands::preview_memory_text,
            drop_commands::get_auto_drop_settings,
            drop_commands::set_auto_drop_settings,
            drop_commands::get_drop_queue,
            drop_commands::authorize_drop_source,
            drop_commands::cancel_drop,
            drop_commands::continue_drops,
            drop_commands::start_drop_operation,
            memory_commands::choose_memory_export,
            memory_commands::export_memory_version,
            source_commands::source_state,
            source_commands::source_set_enabled,
            source_commands::source_refresh,
            source_commands::source_choose_file,
            source_commands::source_confirm,
            source_commands::source_reject,
            source_commands::source_unlink,
            source_commands::source_open_link,
            source_commands::source_connection,
            family_commands::list_memory_records,
            family_commands::start_memory_record,
            family_commands::start_memory_for_file,
            family_commands::stop_memory_record,
            family_commands::get_memory_family,
            family_commands::get_candidate_capture,
            family_commands::set_candidate_capture,
            family_commands::get_pending_candidates,
            family_commands::decide_pending_candidates,
            family_commands::preview_memory_relocation,
            family_commands::relocate_memory_record,
            family_commands::list_memory_works,
            family_commands::preview_memory_merge,
            family_commands::merge_memory_works,
            family_commands::preview_memory_association,
            family_commands::preview_memory_association_for_file,
            family_commands::associate_memory_file,
            family_commands::preview_memory_relation,
            family_commands::correct_memory_relation,
            family_commands::preview_memory_move,
            family_commands::move_memory_members,
            find_commands::get_find_setup,
            find_commands::set_find_consent,
            find_commands::start_find,
            find_commands::get_find,
            find_commands::cancel_find,
            find_commands::get_recent_files,
            find_commands::reveal_find_result,
            find_commands::open_find_result,
            family_commands::get_auto_memory,
            family_commands::set_auto_memory,
            window_commands::show_main_window,
            window_commands::take_page_request,
            window_commands::set_pet_visible,
            window_commands::get_pet_state,
            window_commands::get_pet_signal,
            window_commands::show_quick_search,
            window_commands::resize_quick_search,
            window_commands::hide_quick_search,
            window_commands::show_drop_popup,
            window_commands::resize_drop_popup,
            window_commands::hide_drop_popup,
            window_commands::show_moved_note,
            window_commands::take_moved_note,
            commands::get_ocr_state,
        ])
}
