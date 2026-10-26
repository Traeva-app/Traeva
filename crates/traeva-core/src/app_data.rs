//! Where Traeva keeps its own data (#65).
//!
//! Normally that is `~/Library/Application Support/Traeva/`
//! (architecture.md §4), exactly as before. For development and tests,
//! `TRAEVA_APP_DATA_DIR` can name a separate folder, so a test run of the
//! real app never opens the normal database. Such a test folder:
//!
//! - must be an absolute path without `.` or `..`, in UTF-8;
//! - must not be, contain, or lie inside the normal folder (or the folder
//!   the normal one would be created in), compared by identity: device and
//!   inode, so firmlinks, letter case, Unicode spelling and symbolic links
//!   cannot make the same folder look different;
//! - must not itself be a symbolic link or a file;
//! - must be new, empty, or hold only names Traeva itself uses, so Traeva
//!   never writes its database into a folder that holds other files.
//!
//! The folder is checked once more right after it is created, before
//! anything is written in it.
//!
//! Anything else stops storage with the reason. Traeva never falls back to
//! the normal folder. A test folder also gets its own Keychain account for
//! the API key (`test_key_suffix`), so a test never reads, changes or removes
//! the normal key. The process lock still keeps two instances off one folder.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::contracts::{AppError, ErrorCode};
use crate::observe::sha256_hex;
use crate::platform::macos::{self, Folder, FsError};
use crate::store::{DATABASE_FILE_NAME, LOCK_FILE_NAME};

/// The environment variable that names a separate app-data folder.
pub const APP_DATA_DIR_VAR: &str = "TRAEVA_APP_DATA_DIR";

/// The folder to start with, and where that choice came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Chosen {
    /// The normal folder (`TRAEVA_APP_DATA_DIR` is not set).
    Default(PathBuf),
    /// A test folder, as its real path.
    Test(PathBuf),
}

/// `~/Library/Application Support/Traeva`, from the home folder.
pub(crate) fn default_dir(home: Option<OsString>) -> Result<PathBuf, AppError> {
    match home {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Traeva")),
        _ => Err(AppError::storage(
            ErrorCode::AppDataUnavailable,
            "Cannot find the home folder (HOME is not set), so there is no place to keep Traeva's data.",
        )),
    }
}

/// The folder Traeva starts with, given the values of `TRAEVA_APP_DATA_DIR`
/// and `HOME`. Only reads: nothing is created here.
pub(crate) fn choose(
    override_dir: Option<OsString>,
    home: Option<OsString>,
) -> Result<Chosen, AppError> {
    let normal = default_dir(home)?;
    let Some(value) = override_dir else {
        return Ok(Chosen::Default(normal));
    };
    let shown = value.to_string_lossy().into_owned();
    let refuse = |why: &str| {
        AppError::storage(
            ErrorCode::AppDataUnavailable,
            format!(
                "{APP_DATA_DIR_VAR} is set to “{shown}”, but {why} Traeva opened no database and \
                 does not fall back to its normal data folder."
            ),
        )
    };
    if value.is_empty() {
        return Err(refuse("it is empty."));
    }
    if value.to_str().is_none() {
        return Err(refuse("it is not valid UTF-8."));
    }
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return Err(refuse("it is not an absolute path."));
    }
    // By the text itself: `components()` would quietly drop a "." inside.
    let text = value.to_str().unwrap_or_default();
    if text.split('/').any(|part| part == "." || part == "..") {
        return Err(refuse("it contains “.” or “..”."));
    }
    // By identity (device and inode), not by text: the same folder can be
    // written several ways on macOS (firmlinks such as /System/Volumes/Data,
    // letter case, Unicode spelling, symbolic links). For each path, take
    // the deepest folder that exists.
    let cannot_check = || refuse("Traeva cannot check where it is.");
    let (here, whole) = deepest_folder(&path).map_err(|_| cannot_check())?;
    let (normal_here, _) = deepest_folder(&normal).map_err(|_| cannot_check())?;
    let here_and_above = macos::ancestor_ids(here.fd()).map_err(|_| cannot_check())?;
    // Inside the normal folder, or inside the folder where it would be
    // created when it does not exist yet.
    if here_and_above.contains(&normal_here.id()) {
        return Err(refuse(&format!(
            "it is, or is inside, Traeva's normal data folder {} (or the folder it would be \
             created in).",
            normal.display()
        )));
    }
    if whole {
        let normal_and_above = macos::ancestor_ids(normal_here.fd()).map_err(|_| cannot_check())?;
        if normal_and_above.contains(&here.id()) {
            return Err(refuse(&format!(
                "it contains Traeva's normal data folder {}.",
                normal.display()
            )));
        }
    }
    match fs::symlink_metadata(&path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refuse("Traeva cannot look at it.")),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(refuse("it is a symbolic link. Name the folder itself."));
        }
        Ok(meta) if !meta.is_dir() => return Err(refuse("it is not a folder.")),
        Ok(_) => {
            let names: Vec<OsString> = fs::read_dir(&path)
                .map_err(|_| refuse("Traeva cannot list it."))?
                .map(|entry| entry.map(|e| e.file_name()))
                .collect::<Result<_, _>>()
                .map_err(|_| refuse("Traeva cannot list it."))?;
            if names
                .iter()
                .any(|name| !TRAEVA_NAMES.iter().any(|own| name == own))
            {
                return Err(refuse(
                    "that folder holds other files. Use a new or empty folder, or one only \
                     Traeva has used.",
                ));
            }
        }
    }
    let real = real_location(&path).map_err(|_| cannot_check())?;
    Ok(Chosen::Test(real))
}

/// The only names a folder Traeva has used may hold (with Finder's own
/// `.DS_Store`). A folder with anything else is not a test folder.
const TRAEVA_NAMES: &[&str] = &[
    DATABASE_FILE_NAME,
    "traeva.sqlite3-journal",
    LOCK_FILE_NAME,
    "model-settings.json",
    "model-settings.json.new",
    "diagnostics",
    crate::snapshot::SNAPSHOT_DIR,
    crate::opened::OPENED_DIR,
    ".DS_Store",
];

/// The deepest folder on `path` that exists, opened (following symbolic
/// links, so it is the folder really there), and whether that is `path`
/// itself.
fn deepest_folder(path: &Path) -> Result<(Folder, bool), FsError> {
    let mut at = path.to_path_buf();
    let mut whole = true;
    loop {
        match macos::open_real_folder(&at) {
            Ok(folder) => return Ok((folder, whole)),
            Err(FsError::NotFound | FsError::NotADirectory) if at.pop() => whole = false,
            Err(error) => return Err(error),
        }
    }
}

/// Creates a chosen test folder, then checks it again now that it exists,
/// before anything else is written there: something put in its place
/// meanwhile (a link to the normal folder, say) stops the start.
pub(crate) fn make_test_folder(dir: &Path, home: Option<OsString>) -> Result<(), AppError> {
    fs::create_dir_all(dir).map_err(|err| {
        AppError::storage(
            ErrorCode::AppDataUnavailable,
            format!(
                "Cannot create the test data folder {} ({}). Traeva opened no database.",
                dir.display(),
                err.kind()
            ),
        )
    })?;
    match choose(Some(dir.as_os_str().to_os_string()), home)? {
        Chosen::Test(_) => Ok(()),
        Chosen::Default(_) => Err(AppError::storage(
            ErrorCode::AppDataUnavailable,
            "The test data folder could not be checked again. Traeva opened no database.",
        )),
    }
}

/// Where `path` really is: the deepest part that exists, with every
/// symbolic link resolved, followed by the parts that do not exist yet.
fn real_location(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<OsString> = Vec::new();
    loop {
        match fs::canonicalize(&existing) {
            Ok(real) => {
                return Ok(rest.iter().rev().fold(real, |done, part| done.join(part)));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = existing.file_name() else {
                    return Err(err);
                };
                rest.push(name.to_os_string());
                if !existing.pop() {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// The Keychain account suffix of a test folder: `@test-` and the first 12
/// hex digits of the SHA-256 of its real path. The normal folder has none,
/// so its key stays under the plain provider name (`openai`).
pub(crate) fn test_key_suffix(real_dir: &Path) -> String {
    let digest = sha256_hex(real_dir.as_os_str().as_encoded_bytes());
    format!("@test-{}", &digest[..12])
}

#[cfg(test)]
#[path = "app_data_tests.rs"]
mod tests;
