//! Real-editor check for Memory's automatic capture (#32). Turns Memory on
//! for one synthetic file, then runs the same watcher pass the app runs
//! (`check_memory`, every 2 s) while a person, or a command, edits the file
//! with a real editor. Every version saved and every problem is printed.
//!
//!   cargo run -p traeva-core --example memory_watch_check -- \
//!       <folder> <file name> <app-data folder> [seconds, default 120]
//!
//! - <folder> is authorized as a root and must hold only synthetic files.
//! - <app-data folder> is where this run keeps its database and saved
//!   versions. It must be outside <folder>, and never the real app data.
//! - Run it again with the same app-data folder to check what a restart
//!   finds: a watch that is already on is not set up again, so the first
//!   pass is the start-up check that the app makes.
//!
//! Nothing is sent anywhere. The file's text is never printed: only
//! hashes, sizes and inode numbers.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use traeva_core::{AppError, CheckOutcome, MEMORY_CHECK_INTERVAL, OrganizeService, WatchStatus};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: memory_watch_check <folder> <file name> <app-data folder> [seconds]");
        return ExitCode::from(2);
    }
    let seconds = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);
    match check(
        Path::new(&args[0]),
        &args[1],
        Path::new(&args[2]),
        Duration::from_secs(seconds),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("FAIL: {message}");
            ExitCode::FAILURE
        }
    }
}

fn show(error: AppError) -> String {
    format!("{:?}: {}", error.code, error.message)
}

fn check(folder: &Path, name: &str, app_data: &Path, run_for: Duration) -> Result<(), String> {
    let folder = folder.canonicalize().map_err(|e| e.to_string())?;
    let file_path: PathBuf = folder.join(name);
    let apart = |app_data: &Path| !app_data.starts_with(&folder) && !folder.starts_with(app_data);
    let overlap = "the app-data folder and the watched folder must not contain each other";
    let absolute = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(app_data);
    // Checked before anything is created, then again on the real path.
    if !apart(&absolute) {
        return Err(overlap.into());
    }
    std::fs::create_dir_all(&absolute).map_err(|e| e.to_string())?;
    let app_data = absolute.canonicalize().map_err(|e| e.to_string())?;
    if !apart(&app_data) {
        return Err(overlap.into());
    }
    if let Some(home) = std::env::var_os("HOME") {
        let real = Path::new(&home).join("Library/Application Support/Traeva");
        if real.canonicalize().is_ok_and(|real| real == app_data) {
            return Err("this is Traeva's real app-data folder; use a separate one".into());
        }
    }
    let service = OrganizeService::start(app_data.clone());
    let started = Instant::now();
    let stamp = || format!("[{:6.1}s]", started.elapsed().as_secs_f64());

    let watches = service.memory_watches().map_err(show)?;
    let watch = match watches
        .into_iter()
        .find(|w| w.status == WatchStatus::Active && w.relative_path == name)
    {
        Some(watch) => {
            println!(
                "{} restart: Memory is already on for {} ({} versions saved)",
                stamp(),
                watch.relative_path,
                watch.versions
            );
            watch
        }
        None => {
            let root = service.authorize_root(&folder).map_err(show)?;
            let file = service
                .register_file(&root.root_id, &file_path)
                .map_err(show)?
                .file;
            let watch = service.start_memory(&file.file_id).map_err(show)?;
            println!(
                "{} Memory on for {} (file {}), first version {}",
                stamp(),
                watch.relative_path,
                watch.file_id,
                watch
                    .latest
                    .as_ref()
                    .map_or("none".to_string(), |s| short(&s.sha256))
            );
            watch
        }
    };

    while started.elapsed() < run_for {
        for check in service.check_memory().map_err(show)? {
            let inode = std::fs::symlink_metadata(&file_path)
                .map(|m| m.ino().to_string())
                .unwrap_or_else(|_| "-".to_string());
            match check.outcome {
                CheckOutcome::Unchanged => {}
                CheckOutcome::Saved { snapshot } => println!(
                    "{} saved v{} sha {} {} bytes (inode now {inode})",
                    stamp(),
                    snapshot.sequence,
                    short(&snapshot.sha256),
                    snapshot.byte_length
                ),
                CheckOutcome::SameBytes { snapshot } => println!(
                    "{} saved again with the same bytes as v{} (inode now {inode})",
                    stamp(),
                    snapshot.sequence
                ),
                CheckOutcome::Problem { error } => println!(
                    "{} not saved: {:?} (inode now {inode})",
                    stamp(),
                    error.code
                ),
                CheckOutcome::Stopped => println!("{} Memory was turned off", stamp()),
                CheckOutcome::CandidateSaved { snapshot, .. } => println!(
                    "{} saved as a candidate: sha {} {} bytes (inode now {inode})",
                    stamp(),
                    short(&snapshot.sha256),
                    snapshot.byte_length
                ),
                CheckOutcome::CandidatePaused => {
                    println!("{} candidate capture paused at its quota", stamp())
                }
                CheckOutcome::LinkedAutomatically { snapshot } => println!(
                    "{} replaced by the app: linked as the next version, new file {} sha {} {} \
                     bytes (inode now {inode})",
                    stamp(),
                    snapshot.file_id,
                    short(&snapshot.sha256),
                    snapshot.byte_length
                ),
            }
        }
        std::thread::sleep(MEMORY_CHECK_INTERVAL);
    }

    // Every saved version reads back with its own hash.
    println!("{} versions of file {}:", stamp(), watch.file_id);
    for snapshot in service.list_snapshots(&watch.file_id).map_err(show)? {
        let (_, bytes) = service.read_snapshot(&snapshot.snapshot_id).map_err(show)?;
        println!(
            "  v{} sha {} {} bytes, read back {} bytes: ok",
            snapshot.sequence,
            short(&snapshot.sha256),
            snapshot.byte_length,
            bytes.len()
        );
    }
    Ok(())
}

fn short(sha256: &str) -> String {
    sha256.chars().take(12).collect()
}
