//! Opening an old version (#35): a new copy of a saved version's exact
//! bytes, for the user to look at in the app that opens such files. The
//! saved version itself is never handed to an app, so no app can change it.
//!
//! Each open writes a new copy to `opened/<random ID>/<name>` in Traeva's
//! data folder: never inside a folder the user authorized, and never over
//! anything (shared-contract.md §4.7).
//!   - The bytes come only from `read_snapshot`, which checks their
//!     SHA-256 (§4.2); the snapshot folders are not opened here.
//!   - The folders are opened one by one from the data folder without
//!     following links (snapshot.rs's `open_app_data` and `open_child`).
//!     The ID folder is new: one already there means another ID.
//!   - The copy is created new (it must not exist), flushed to the disk,
//!     read back and checked against the same SHA-256, and then made
//!     read-only, so the app asks before saving anything (to another file).
//!   - Only copies of the text files Memory reads may be opened by the
//!     system (`copy_name`); any other kind is only shown by its place.
//!
//! Copies are kept: CAC keeps everything, cleaning up comes later.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;

use rustix::fs::{self as rfs, AtFlags, Mode, OFlags};
use rustix::io::Errno;
use serde::Serialize;

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode};
use crate::memory_contracts::SnapshotRef;
use crate::observe::{is_text_file_name, sha256_hex};
use crate::snapshot::{open_app_data, open_child};

/// The folder in app data that holds the copies.
pub const OPENED_DIR: &str = "opened";

/// The longest copy name kept, in bytes (a file name may have 255).
const MAX_NAME_BYTES: usize = 200;

/// A copy made for opening.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedCopy {
    /// The saved version it was made from.
    pub snapshot: SnapshotRef,
    /// Where the copy is, for the system to open. It is not the user's file.
    pub path: String,
    /// The SHA-256 of the copy as read back from the disk: the saved
    /// version's.
    pub sha256: String,
    pub byte_length: i64,
    /// The system may open it: a text file Memory reads (.txt, .md,
    /// .markdown). Other copies are only shown by their place.
    pub can_open: bool,
    /// The number in the copy's name: the version's number on its record
    /// line when the copy was made; null when the name has none. The page
    /// says this one, not the number the history shows later, which can
    /// change when the line changes (R2-MEM-11).
    pub number: Option<usize>,
}

/// What opening an old version gave: the copy, and whether the system
/// opened it (the window's `open_memory_version` asks it to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedVersion {
    pub copy: OpenedCopy,
    /// False when the copy may not be opened (`copy.can_open`) or macOS
    /// could not open it. The copy is still there, at `copy.path`.
    pub opened: bool,
}

impl OrganizeService {
    /// Makes a new read-only copy of saved version `snapshot_id` to open.
    /// Errors: the ones of `read_snapshot` (an ID that is not valid or not
    /// known, a missing or damaged saved copy), or `app_data_unavailable`
    /// when the copy cannot be written. A copy not written whole is removed.
    pub fn copy_for_opening(&self, snapshot_id: &str) -> Result<OpenedCopy, AppError> {
        let (snapshot, bytes) = self.read_snapshot(snapshot_id)?;
        let app_data = self.store()?.app_data_dir().to_path_buf();
        let number = self.line_number(&snapshot)?;
        let (name, can_open) = copy_name(&snapshot, number);
        let fail = |errno: Errno| cannot_write(&errno.to_string());
        let app = open_app_data(&app_data).map_err(fail)?;
        let opened = open_child(&app, OPENED_DIR, true).map_err(fail)?;
        let (id, folder) = new_folder(&opened).map_err(|why| cannot_write(&why))?;
        if let Err(why) = write_checked(
            &folder,
            &name,
            &bytes,
            &snapshot.sha256,
            0o600,
            Some(READ_ONLY),
        ) {
            let _ = rfs::unlinkat(&folder, name.as_str(), AtFlags::empty());
            let _ = rfs::unlinkat(&opened, id.as_str(), AtFlags::REMOVEDIR);
            return Err(cannot_write(&why));
        }
        for fd in [&folder, &opened, &app] {
            rfs::fcntl_fullfsync(fd).map_err(fail)?;
        }
        Ok(OpenedCopy {
            path: app_data
                .join(OPENED_DIR)
                .join(&id)
                .join(&name)
                .to_string_lossy()
                .into_owned(),
            sha256: snapshot.sha256.clone(),
            byte_length: bytes.len() as i64,
            can_open,
            number,
            snapshot,
        })
    }
}

/// A copy for opening is read-only, so the app asks before saving it.
const READ_ONLY: u16 = 0o444;

/// A new folder with a random ID in `opened`. An ID already there is never
/// reused: another one is drawn. The error is the reason, for the caller's
/// message.
pub(crate) fn new_folder(opened: &OwnedFd) -> Result<(String, OwnedFd), String> {
    for _ in 0..5 {
        let id = uuid::Uuid::new_v4().to_string();
        match rfs::mkdirat(opened, id.as_str(), Mode::from_raw_mode(0o700)) {
            Ok(()) => {
                let folder = open_child(opened, &id, false).map_err(|e| e.to_string())?;
                return Ok((id, folder));
            }
            Err(Errno::EXIST) => continue,
            Err(errno) => return Err(errno.to_string()),
        }
    }
    Err("no new folder could be made".to_string())
}

impl OrganizeService {
    /// The version's number as the history and search number it: its place
    /// on its file's record line, in save order (`usable_pairs_of_line`,
    /// like `lineNumbers` in history.ts); a file's only version is 1. None
    /// when the line does not list it: the page then names it by its time,
    /// and the copy by no number. Not the file's own `sequence`: an editor
    /// that saves by writing a new file starts that at 1 again (R2-MEM-10).
    fn line_number(&self, snapshot: &SnapshotRef) -> Result<Option<usize>, AppError> {
        let pairs = self.usable_pairs_of_line(&snapshot.file_id)?;
        if pairs.is_empty() {
            let only = self.list_snapshots(&snapshot.file_id)?.len() == 1;
            return Ok(only.then_some(1));
        }
        let mut line: Vec<&str> = Vec::new();
        for version in pairs.iter().flat_map(|pair| [&pair.before, &pair.after]) {
            if !line.contains(&version.snapshot_id.as_str()) {
                line.push(&version.snapshot_id);
            }
        }
        Ok(line
            .iter()
            .position(|id| *id == snapshot.snapshot_id)
            .map(|index| index + 1))
    }
}

/// The copy's name, and whether the system may open it. `number` is the
/// version's number on the history page (`line_number`), so the copy and
/// the page say the same; without one the name has no number. Only the
/// text files Memory reads (`observe::is_text_file_name`: .txt, .md,
/// .markdown) keep their extension and may be opened: "essay (version 3).md"
/// for version 3 of "Drafts/essay.md". Anything else becomes "version-3"
/// with no extension and is never handed to the system: `/usr/bin/open`
/// picks an app by the extension (a `.terminal` file runs in Terminal, a
/// `.dmg` is mounted), and a copy Traeva writes has no quarantine mark, so
/// the system's first-open check would be skipped. A text name that is
/// hidden (starts with "."), holds "/" or a NUL, or is too long becomes
/// "version-3.md".
fn copy_name(snapshot: &SnapshotRef, number: Option<usize>) -> (String, bool) {
    let original = snapshot
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let (label, fallback) = match number {
        Some(n) => (format!("version {n}"), format!("version-{n}")),
        None => ("an older version".to_string(), "older-version".to_string()),
    };
    let Some((stem, extension)) = original
        .rsplit_once('.')
        .filter(|_| is_text_file_name(original))
    else {
        return (fallback, false);
    };
    let named = format!("{stem} ({label}).{extension}");
    let usable = !original.starts_with('.')
        && !original.contains(['/', '\0'])
        && named.len() <= MAX_NAME_BYTES;
    if usable {
        (named, true)
    } else {
        (format!("{fallback}.{extension}"), true)
    }
}

fn cannot_write(why: &str) -> AppError {
    AppError::storage(
        ErrorCode::AppDataUnavailable,
        format!(
            "Traeva could not make a copy of this version to open ({why}). Nothing was changed \
             in your folders or in the saved version."
        ),
    )
}

/// Creates file `name` in `folder` holding `bytes` (it must not exist; a
/// link is never followed; `create_mode` less the umask), flushes it, reads
/// it back, checks it against `sha256`, and then gives it permissions
/// `final_mode` when there is one. The error is the reason, for the
/// caller's message.
pub(crate) fn write_checked(
    folder: &OwnedFd,
    name: &str,
    bytes: &[u8],
    sha256: &str,
    create_mode: u16,
    final_mode: Option<u16>,
) -> Result<(), String> {
    let fd = rfs::openat(
        folder,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(create_mode),
    )
    .map_err(|errno| errno.to_string())?;
    let mut file = File::from(fd);
    let fail = |err: std::io::Error| err.kind().to_string();
    file.write_all(bytes).map_err(fail)?;
    rfs::fcntl_fullfsync(&file).map_err(|errno| errno.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(fail)?;
    let mut written = Vec::with_capacity(bytes.len());
    file.read_to_end(&mut written).map_err(fail)?;
    if sha256_hex(&written) != sha256 {
        return Err("the bytes on disk differ from the saved version".to_string());
    }
    if let Some(mode) = final_mode {
        rfs::fchmod(&file, Mode::from_raw_mode(mode)).map_err(|errno| errno.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "opened_tests.rs"]
mod tests;
