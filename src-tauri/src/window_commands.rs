//! The desktop pet's window (#208) and showing the main window. No business
//! rules live here: what a dropped file becomes is the core's (#168), the
//! same as for a file dropped on the Dock icon.
//!
//! - Files dropped on the pet reach Rust as a window event and go to the
//!   same queue as the Dock icon's (`receive_opened_files`). The pet's page
//!   may not listen to events (capabilities/pet-window.json), so it never
//!   sees a path. To react at once (10/14, the student's list item 7), the
//!   page reads a tiny signal often (`get_pet_signal`): whether a file is
//!   being dragged over it now, and how many drops it caught. Two numbers
//!   kept here, no path, and no read of the queue.
//! - Clicking the pet shows the main window and asks it for a page ("find"
//!   or "organize"). The main window may not listen to events either: it
//!   takes the request when it gets the focus (`take_page_request`).
//! - The red close button hides the main window while Traeva keeps running
//!   (the pet stays); Cmd+Q quits, and the Dock icon shows it again.
//! - Quick search (#208, the student's idea): when nothing waits for the
//!   user, a click on the pet opens a small opaque search window next to it.
//!   It starts as just the search box and grows down as results come
//!   (`resize_quick_search`, 64 to 480 points high); Esc or losing the focus
//!   hides it. Its own capability allows Find's commands and nothing else.
//! - The pet's popup (10/14, the student's deferred item): when a dropped
//!   file waits for an answer or a folder, a click on the pet opens a small
//!   opaque window next to it instead of the main window. It stays open
//!   until its page hides it (a half-made decision must not vanish when the
//!   focus moves). Its capability allows only Organize's existing commands
//!   for one dropped file; agreeing to send, authorizing a source folder
//!   and pickers stay in the main window.
//! - The moved note (10/17, the student's idea; UI's page): when the
//!   automatic mode moved the one file the pet caught, the pet asks for a
//!   note (`show_moved_note` with the drop's ID, which holds no path). The
//!   popup opens beside the pet as a small note, "Moved to …", with Undo
//!   (a preview of where it goes back first, organize-spec §8). It does not
//!   take the focus: while it is a note it cannot become the key window.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, DragDropEvent, Manager, Runtime, State, WebviewWindow, WindowEvent};
use traeva_core::{AppError, ErrorCode, OrganizeService, PetStateView};

use crate::drop_commands;

pub const MAIN: &str = "main";
pub const PET: &str = "pet";
pub const QUICK: &str = "quick";
/// The quick search window's width and its height range, in points.
const QUICK_WIDTH: f64 = 560.0;
const QUICK_MIN_HEIGHT: f64 = 64.0;
const QUICK_MAX_HEIGHT: f64 = 480.0;
/// Room between the pet and the quick search window, and from the screen's
/// edges, in points.
const QUICK_GAP: f64 = 8.0;
/// The pet's popup window, its width and its height range, in points.
pub const ASK: &str = "ask";
const ASK_WIDTH: f64 = 440.0;
const ASK_MIN_WIDTH: f64 = 280.0;
const ASK_MIN_HEIGHT: f64 = 72.0;
const ASK_MAX_HEIGHT: f64 = 600.0;
const ASK_START_HEIGHT: f64 = 280.0;
/// The popup as the moved note: its first size, in points.
const NOTE_WIDTH: f64 = 320.0;
const NOTE_HEIGHT: f64 = 96.0;
/// The pages the pet's windows may ask the main window for: "automatic" is
/// Settings with the automatic mode's card in view (the popup's "Turn on
/// automatic mode…", 10/15; agreeing to it stays in the main window).
const PAGES: &[&str] = &["find", "organize", "automatic"];

/// The page the main window should show when it next gets the focus.
#[derive(Default)]
pub struct PageRequest(Mutex<Option<String>>);

/// The drop whose moved note the popup should show next (its ID only).
#[derive(Default)]
pub struct MovedNote(Mutex<Option<String>>);

/// Shows and focuses the main window, and asks it for `page`.
#[tauri::command(async, rename_all = "snake_case")]
pub fn show_main_window<R: Runtime>(
    app: AppHandle<R>,
    requests: State<'_, PageRequest>,
    page: String,
) -> Result<(), AppError> {
    if !PAGES.contains(&page.as_str()) {
        return Err(AppError::observation(
            ErrorCode::InvalidInput,
            "The main window can show Find, Organize or the automatic mode's setting.",
        ));
    }
    *requests.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(page);
    if let Some(main) = app.get_webview_window(MAIN) {
        show(&main);
    }
    Ok(())
}

/// What the pet may read of the drop queue, for its face: no paths, names
/// or reasons (R1's P03 review of #208). The pet's window may not call
/// `get_drop_queue`.
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_pet_state(service: State<'_, Arc<OrganizeService>>) -> PetStateView {
    service.pet_state()
}

/// A file is being dragged over the pet now (its drag events set and clear
/// it), and how many drops the pet caught since Traeva started.
static PET_DRAG_OVER: AtomicBool = AtomicBool::new(false);
static PET_DROPS: AtomicU64 = AtomicU64::new(0);

/// What the pet's page reads every few tenths of a second: no path, name or
/// queue state (that is `get_pet_state`, read less often).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PetSignal {
    pub drag_over: bool,
    pub drops: u64,
}

/// The pet's quick signal (see the module docs).
#[tauri::command(async, rename_all = "snake_case")]
pub fn get_pet_signal() -> PetSignal {
    PetSignal {
        drag_over: PET_DRAG_OVER.load(Ordering::Relaxed),
        drops: PET_DROPS.load(Ordering::Relaxed),
    }
}

/// Whether a file is over the pet after this drag event; None: unchanged.
fn drag_over_after(event: &DragDropEvent) -> Option<bool> {
    match event {
        DragDropEvent::Enter { .. } | DragDropEvent::Over { .. } => Some(true),
        DragDropEvent::Leave | DragDropEvent::Drop { .. } => Some(false),
        _ => None,
    }
}

/// The page the pet asked for since the last call, once.
#[tauri::command(async, rename_all = "snake_case")]
pub fn take_page_request(requests: State<'_, PageRequest>) -> Option<String> {
    requests.0.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// Shows or hides the pet. The main window remembers the user's choice and
/// calls this when it starts.
///
/// Showing a window also makes it the key window (tao's show is
/// makeKeyAndOrderFront), so after a launch the keyboard went to the pet:
/// typing right away, into Find's box say, was lost until the main window
/// was clicked. The main window gets the keyboard back when it shows.
#[tauri::command(async, rename_all = "snake_case")]
pub fn set_pet_visible<R: Runtime>(app: AppHandle<R>, visible: bool) -> Result<(), AppError> {
    let Some(pet) = app.get_webview_window(PET) else {
        return Ok(());
    };
    let done = if visible { pet.show() } else { pet.hide() };
    if visible
        && let Some(main) = app.get_webview_window(MAIN)
        && main.is_visible().unwrap_or(false)
    {
        let _ = main.set_focus();
    }
    done.map_err(|_| {
        AppError::observation(
            ErrorCode::InvalidInput,
            "The pet could not be shown or hidden.",
        )
    })
}

/// Opens the quick search window next to the pet (above or below the
/// screen's middle as there is room), at its smallest height, focused.
#[tauri::command(async, rename_all = "snake_case")]
pub fn show_quick_search<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    let failed = || AppError::observation(ErrorCode::InvalidInput, "Quick search could not open.");
    let quick = app.get_webview_window(QUICK).ok_or_else(failed)?;
    if let Some(screen) = pet_screen(&app, &quick) {
        let scale = screen.scale_factor();
        let area = work_area(&screen);
        let pet = app.get_webview_window(PET).and_then(|pet| {
            let at = pet.outer_position().ok()?;
            let size = pet.outer_size().ok()?;
            Some((
                f64::from(at.x),
                f64::from(at.y),
                f64::from(size.width),
                f64::from(size.height),
            ))
        });
        let (x, y) = quick_place(pet, area, scale);
        let _ = quick.set_size(tauri::LogicalSize::new(QUICK_WIDTH, QUICK_MIN_HEIGHT));
        let _ = quick.set_position(tauri::PhysicalPosition::new(x, y));
    }
    quick.show().map_err(|_| failed())?;
    let _ = quick.set_focus();
    Ok(())
}

/// Sets the quick search window's height (points), kept between 64 and
/// 480; the width stays.
#[tauri::command(async, rename_all = "snake_case")]
pub fn resize_quick_search<R: Runtime>(app: AppHandle<R>, height: f64) -> Result<f64, AppError> {
    let height = clamp_quick_height(height);
    if let Some(quick) = app.get_webview_window(QUICK) {
        quick
            .set_size(tauri::LogicalSize::new(QUICK_WIDTH, height))
            .map_err(|_| {
                AppError::observation(
                    ErrorCode::InvalidInput,
                    "Quick search could not change size.",
                )
            })?;
    }
    Ok(height)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn hide_quick_search<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    if let Some(quick) = app.get_webview_window(QUICK) {
        let _ = quick.hide();
    }
    Ok(())
}

/// Opens the pet's popup next to the pet, or shows and focuses it again if
/// it is already open (it keeps its place and height then; a moved note
/// takes the popup's width again and is placed beside the pet for it,
/// 10/17).
#[tauri::command(async, rename_all = "snake_case")]
pub fn show_drop_popup<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    let failed =
        || AppError::observation(ErrorCode::InvalidInput, "The pet's window could not open.");
    let ask = app.get_webview_window(ASK).ok_or_else(failed)?;
    let _ = ask.set_focusable(true);
    if ask.is_visible().unwrap_or(false) {
        if let (Ok(size), Ok(scale)) = (ask.inner_size(), ask.scale_factor()) {
            let width = f64::from(size.width) / scale;
            let height = f64::from(size.height) / scale;
            let _ = ask.set_size(tauri::LogicalSize::new(ASK_WIDTH, height));
            // A note was placed for its own width beside the pet: wider in
            // place it would cover the pet (UI's review of #305), so it is
            // placed again for the popup's width.
            let screen = (width < ASK_WIDTH)
                .then(|| pet_screen(&app, &ask))
                .flatten();
            match screen {
                Some(screen) => {
                    let (x, y) = ask_place(
                        pet_rect(&app),
                        work_area(&screen),
                        screen.scale_factor(),
                        ASK_WIDTH,
                        height,
                    );
                    let _ = ask.set_position(tauri::PhysicalPosition::new(x, y));
                }
                None => keep_on_screen(&ask, ASK_WIDTH, height),
            }
        }
    } else if let Some(screen) = pet_screen(&app, &ask) {
        let scale = screen.scale_factor();
        let (x, y) = ask_place(
            pet_rect(&app),
            work_area(&screen),
            scale,
            ASK_WIDTH,
            ASK_START_HEIGHT,
        );
        let _ = ask.set_size(tauri::LogicalSize::new(ASK_WIDTH, ASK_START_HEIGHT));
        let _ = ask.set_position(tauri::PhysicalPosition::new(x, y));
    }
    ask.show().map_err(|_| failed())?;
    let _ = ask.set_focus();
    Ok(())
}

/// The pet asks for the moved note of drop `drop_id` (see the module docs).
/// The ID is kept for the popup's page (`take_moved_note`). A hidden popup
/// opens beside the pet at the note's size, shown without taking the focus
/// (it cannot become the key window until `show_drop_popup`); an open popup
/// is left as it is, and its page decides.
#[tauri::command(async, rename_all = "snake_case")]
pub fn show_moved_note<R: Runtime>(
    app: AppHandle<R>,
    notes: State<'_, MovedNote>,
    drop_id: String,
) -> Result<(), AppError> {
    if !is_drop_id(&drop_id) {
        return Err(AppError::observation(
            ErrorCode::InvalidInput,
            "This is not a dropped file's ID.",
        ));
    }
    *notes.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(drop_id);
    let failed =
        || AppError::observation(ErrorCode::InvalidInput, "The pet's window could not open.");
    let ask = app.get_webview_window(ASK).ok_or_else(failed)?;
    if ask.is_visible().unwrap_or(false) {
        return Ok(());
    }
    if let Some(screen) = pet_screen(&app, &ask) {
        let scale = screen.scale_factor();
        let (x, y) = ask_place(
            pet_rect(&app),
            work_area(&screen),
            scale,
            NOTE_WIDTH,
            NOTE_HEIGHT,
        );
        let _ = ask.set_size(tauri::LogicalSize::new(NOTE_WIDTH, NOTE_HEIGHT));
        let _ = ask.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = ask.set_focusable(false);
    ask.show().map_err(|_| failed())?;
    Ok(())
}

/// The drop whose moved note to show, once.
#[tauri::command(async, rename_all = "snake_case")]
pub fn take_moved_note(notes: State<'_, MovedNote>) -> Option<String> {
    notes.0.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// A drop's ID as the core makes them: a lowercase, hyphenated UUID.
fn is_drop_id(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(at, c)| match at {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

/// The pet's place and size in physical pixels, if it has one.
fn pet_rect<R: Runtime>(app: &AppHandle<R>) -> Option<(f64, f64, f64, f64)> {
    let pet = app.get_webview_window(PET)?;
    let at = pet.outer_position().ok()?;
    let size = pet.outer_size().ok()?;
    Some((
        f64::from(at.x),
        f64::from(at.y),
        f64::from(size.width),
        f64::from(size.height),
    ))
}

/// Moves the popup, `width` by `height` points, so all of it is on its
/// screen.
fn keep_on_screen<R: Runtime>(ask: &WebviewWindow<R>, width: f64, height: f64) {
    if let (Ok(at), Ok(Some(screen))) = (ask.outer_position(), ask.current_monitor()) {
        let scale = screen.scale_factor();
        let window = (
            f64::from(at.x),
            f64::from(at.y),
            width * scale,
            height * scale,
        );
        let (x, y) = on_screen(window, work_area(&screen), QUICK_GAP * scale);
        if (x, y) != (window.0, window.1) {
            let _ = ask.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

/// Sets the popup's height (points), kept between 72 and 600, and its
/// width: 280 to 440 when given (the moved note, 10/17), otherwise 440. A
/// popup that grew past its screen's edge moves back so all of it stays on
/// the screen (10/15: it grew down out of sight and, with no title bar,
/// could not be moved back). Returns the height it took.
#[tauri::command(async, rename_all = "snake_case")]
pub fn resize_drop_popup<R: Runtime>(
    app: AppHandle<R>,
    height: f64,
    width: Option<f64>,
) -> Result<f64, AppError> {
    let height = clamp_ask_height(height);
    let width = clamp_ask_width(width);
    if let Some(ask) = app.get_webview_window(ASK) {
        ask.set_size(tauri::LogicalSize::new(width, height))
            .map_err(|_| {
                AppError::observation(
                    ErrorCode::InvalidInput,
                    "The pet's window could not change size.",
                )
            })?;
        keep_on_screen(&ask, width, height);
    }
    Ok(height)
}

/// The screen the pet is on (the popup and quick search open beside it),
/// else the main screen.
fn pet_screen<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) -> Option<tauri::Monitor> {
    app.get_webview_window(PET)
        .and_then(|pet| pet.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
}

/// A screen's area without the menu bar and the Dock, in physical pixels:
/// (x, y, width, height).
fn work_area(screen: &tauri::Monitor) -> (f64, f64, f64, f64) {
    let area = screen.work_area();
    (
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    )
}

/// Where a window (x, y, width, height) goes so all of it is on the screen
/// area, `gap` from its edges: moved up, down, left or right as little as
/// needed; a window larger than the area keeps its top left corner in it.
fn on_screen(window: (f64, f64, f64, f64), area: (f64, f64, f64, f64), gap: f64) -> (f64, f64) {
    let (x, y, w, h) = window;
    let (ax, ay, aw, ah) = area;
    let x = x.min(ax + aw - w - gap).max(ax + gap);
    let y = y.min(ay + ah - h - gap).max(ay + gap);
    (x, y)
}

#[tauri::command(async, rename_all = "snake_case")]
pub fn hide_drop_popup<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    if let Some(ask) = app.get_webview_window(ASK) {
        let _ = ask.hide();
    }
    Ok(())
}

/// A width the popup may take; none or not a number is the full width.
fn clamp_ask_width(width: Option<f64>) -> f64 {
    match width {
        Some(width) if width.is_finite() => width.clamp(ASK_MIN_WIDTH, ASK_WIDTH),
        _ => ASK_WIDTH,
    }
}

/// A height the popup may take; not a number counts as its first height.
fn clamp_ask_height(height: f64) -> f64 {
    if height.is_finite() {
        height.clamp(ASK_MIN_HEIGHT, ASK_MAX_HEIGHT)
    } else {
        ASK_START_HEIGHT
    }
}

/// Where the popup goes, in physical pixels: to the left of the pet (or to
/// its right when there is no room), its bottom level with the pet's, kept
/// on the screen. Without the pet: centred near the top of the screen.
/// `pet` and `screen` are (x, y, width, height) in physical pixels; `width`
/// and `height` are the popup's, in points.
fn ask_place(
    pet: Option<(f64, f64, f64, f64)>,
    screen: (f64, f64, f64, f64),
    scale: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let (sx, sy, sw, sh) = screen;
    let width = width * scale;
    let high = height * scale;
    let gap = QUICK_GAP * scale;
    let Some((px, py, pw, ph)) = pet else {
        return ((sx + (sw - width) / 2.0).max(sx), sy + sh * 0.2);
    };
    let left = px - gap - width;
    let x = if left >= sx + gap {
        left
    } else {
        (px + pw + gap).min(sx + sw - width - gap).max(sx + gap)
    };
    let y = (py + ph - high).min(sy + sh - high - gap).max(sy + gap);
    (x, y)
}

/// A height the quick search window may take; not a number counts as the
/// smallest.
fn clamp_quick_height(height: f64) -> f64 {
    if height.is_finite() {
        height.clamp(QUICK_MIN_HEIGHT, QUICK_MAX_HEIGHT)
    } else {
        QUICK_MIN_HEIGHT
    }
}

/// Where the quick search window goes, in physical pixels: to the left of
/// the pet (or to its right when there is no room), its top level with the
/// pet's but moved up so that its tallest form stays on the screen. Without
/// the pet: centred near the top of the screen. `pet` and `screen` are
/// (x, y, width, height) in physical pixels.
fn quick_place(
    pet: Option<(f64, f64, f64, f64)>,
    screen: (f64, f64, f64, f64),
    scale: f64,
) -> (f64, f64) {
    let (sx, sy, sw, sh) = screen;
    let width = QUICK_WIDTH * scale;
    let tallest = QUICK_MAX_HEIGHT * scale;
    let gap = QUICK_GAP * scale;
    let Some((px, py, pw, _)) = pet else {
        return ((sx + (sw - width) / 2.0).max(sx), sy + sh * 0.2);
    };
    let left = px - gap - width;
    let x = if left >= sx + gap {
        left
    } else {
        (px + pw + gap).min(sx + sw - width - gap).max(sx + gap)
    };
    let y = py.min(sy + sh - tallest - gap).max(sy + gap);
    (x, y)
}

/// The window events Traeva handles: a drop on the pet, the main window's
/// close button, and quick search losing the focus (it hides then, like a
/// menu).
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    match (window.label(), event) {
        (QUICK, WindowEvent::Focused(false)) => {
            let _ = window.hide();
        }
        (PET, WindowEvent::DragDrop(drag)) => {
            if let Some(over) = drag_over_after(drag) {
                PET_DRAG_OVER.store(over, Ordering::Relaxed);
            }
            if let DragDropEvent::Drop { paths, .. } = drag
                && !paths.is_empty()
            {
                let service = window.state::<Arc<OrganizeService>>();
                drop_commands::receive_opened_files(&service, paths.clone());
                // After the queue has the file, so the page's next read of
                // the queue finds it.
                PET_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
        (MAIN, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let _ = window.hide();
        }
        _ => {}
    }
}

/// Puts the pet near the bottom right of the main screen (it starts hidden;
/// the main window shows it).
pub fn place_pet<R: Runtime>(app: &AppHandle<R>) {
    let Some(pet) = app.get_webview_window(PET) else {
        return;
    };
    if let Ok(Some(screen)) = pet.primary_monitor() {
        let size = screen.size();
        let scale = screen.scale_factor();
        let x = f64::from(size.width) - 160.0 * scale;
        let y = f64::from(size.height) - 200.0 * scale;
        let _ = pet.set_position(tauri::PhysicalPosition::new(x.max(0.0), y.max(0.0)));
    }
}

/// Shows the main window again (the Dock icon, or the pet).
pub fn show<R: Runtime>(main: &WebviewWindow<R>) {
    let _ = main.unminimize();
    let _ = main.show();
    let _ = main.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (f64, f64, f64, f64) = (0.0, 0.0, 2880.0, 1800.0);

    #[test]
    fn quick_search_opens_left_of_the_pet_and_stays_on_the_screen() {
        // The pet near the bottom right (where Traeva puts it), scale 2.
        let (x, y) = quick_place(Some((2560.0, 1400.0, 192.0, 192.0)), SCREEN, 2.0);
        assert_eq!(x, 2560.0 - 16.0 - 1120.0, "to the left of the pet");
        assert_eq!(
            y,
            1800.0 - 960.0 - 16.0,
            "moved up so its tallest form fits"
        );
        // The pet at the left edge: to its right.
        let (x, _) = quick_place(Some((10.0, 100.0, 192.0, 192.0)), SCREEN, 2.0);
        assert_eq!(x, 10.0 + 192.0 + 16.0);
        // No pet: near the top, centred.
        let (x, y) = quick_place(None, SCREEN, 2.0);
        assert_eq!((x, y), ((2880.0 - 1120.0) / 2.0, 360.0));
    }

    #[test]
    fn a_grown_popup_moves_up_to_stay_on_its_screen() {
        let area = (0.0, 50.0, 2880.0, 1700.0);
        // Grown past the bottom: up just enough.
        assert_eq!(
            on_screen((100.0, 1200.0, 880.0, 1200.0), area, 16.0),
            (100.0, 50.0 + 1700.0 - 1200.0 - 16.0)
        );
        // Already on the screen: unchanged.
        assert_eq!(
            on_screen((100.0, 300.0, 880.0, 560.0), area, 16.0),
            (100.0, 300.0)
        );
        // Off to the right and above the top: back in.
        assert_eq!(
            on_screen((2500.0, 0.0, 880.0, 400.0), area, 16.0),
            (2880.0 - 880.0 - 16.0, 66.0)
        );
        // Taller than the area: its top stays on the screen.
        assert_eq!(on_screen((100.0, 400.0, 880.0, 2000.0), area, 16.0).1, 66.0);
    }

    #[test]
    fn a_file_is_over_the_pet_from_enter_until_leave_or_drop() {
        let at = tauri::PhysicalPosition::new(10.0, 10.0);
        let paths = vec![std::path::PathBuf::from("/tmp/a.txt")];
        let enter = DragDropEvent::Enter {
            paths: paths.clone(),
            position: at,
        };
        assert_eq!(drag_over_after(&enter), Some(true));
        assert_eq!(
            drag_over_after(&DragDropEvent::Over { position: at }),
            Some(true)
        );
        assert_eq!(drag_over_after(&DragDropEvent::Leave), Some(false));
        let drop = DragDropEvent::Drop {
            paths,
            position: at,
        };
        assert_eq!(drag_over_after(&drop), Some(false));
        // The signal carries two numbers and nothing else (no path).
        let signal = serde_json::to_value(PetSignal {
            drag_over: true,
            drops: 3,
        })
        .unwrap();
        assert_eq!(signal, serde_json::json!({ "drag_over": true, "drops": 3 }));
    }

    #[test]
    fn the_popup_opens_beside_the_pet_with_its_bottom_level() {
        // The pet near the bottom right, scale 2: to its left, bottoms level.
        let (x, y) = ask_place(
            Some((2560.0, 1400.0, 192.0, 192.0)),
            SCREEN,
            2.0,
            ASK_WIDTH,
            280.0,
        );
        assert_eq!(x, 2560.0 - 16.0 - 880.0);
        assert_eq!(y, 1400.0 + 192.0 - 560.0);
        // The pet at the left edge and near the top: to its right, on screen.
        let (x, y) = ask_place(
            Some((10.0, 0.0, 192.0, 192.0)),
            SCREEN,
            2.0,
            ASK_WIDTH,
            600.0,
        );
        assert_eq!(x, 10.0 + 192.0 + 16.0);
        assert_eq!(y, 16.0);
        // The moved note: narrower, still beside the pet with its bottom level.
        let (x, y) = ask_place(
            Some((2560.0, 1400.0, 192.0, 192.0)),
            SCREEN,
            2.0,
            NOTE_WIDTH,
            NOTE_HEIGHT,
        );
        assert_eq!(x, 2560.0 - 16.0 - 640.0);
        assert_eq!(y, 1400.0 + 192.0 - 192.0);
        assert_eq!(clamp_ask_height(50.0), 72.0);
        assert_eq!(clamp_ask_height(9000.0), 600.0);
        assert_eq!(clamp_ask_height(f64::NAN), 280.0);
    }

    #[test]
    fn the_popup_s_width_is_kept_in_range_and_a_drop_id_is_a_uuid() {
        assert_eq!(clamp_ask_width(None), ASK_WIDTH);
        assert_eq!(clamp_ask_width(Some(320.0)), 320.0);
        assert_eq!(clamp_ask_width(Some(100.0)), 280.0);
        assert_eq!(clamp_ask_width(Some(900.0)), 440.0);
        assert_eq!(clamp_ask_width(Some(f64::NAN)), ASK_WIDTH);
        assert!(is_drop_id("0b6f3c1e-2d4a-4f5b-9c7d-8e9f0a1b2c3d"));
        for not in [
            "",
            "0B6F3C1E-2D4A-4F5B-9C7D-8E9F0A1B2C3D",
            "0b6f3c1e2d4a4f5b9c7d8e9f0a1b2c3d",
            "0b6f3c1e-2d4a-4f5b-9c7d-8e9f0a1b2c3",
            "../../etc/passwd-0000-0000-000000000000",
            "/Users/someone/Desktop/a-file-name.txt",
        ] {
            assert!(!is_drop_id(not), "{not}");
        }
    }

    #[test]
    fn quick_search_heights_are_kept_in_range() {
        assert_eq!(clamp_quick_height(10.0), 64.0);
        assert_eq!(clamp_quick_height(300.0), 300.0);
        assert_eq!(clamp_quick_height(5000.0), 480.0);
        assert_eq!(clamp_quick_height(f64::NAN), 64.0);
    }
}
