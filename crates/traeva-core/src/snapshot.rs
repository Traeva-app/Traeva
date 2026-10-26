//! Memory's saved versions (#32): copy the exact bytes of a registered file
//! into Traeva's app-data folder, and read them back checked
//! (docs/development/shared-contract.md §4.2).
//!
//! A capture, step by step. Each step stops at the first problem:
//!   1. check that Memory's watch is still on; then read the file stably
//!      through its root handle (nothing inside the root is written)
//!   2. check that it is the same physical file as the record
//!   3. store the bytes: write them to `snapshots/incoming/`, flush them to
//!      disk, read them back and check the SHA-256, then move them to
//!      `snapshots/<aa>/<sha256>` without replacing anything. A copy already
//!      stored under that SHA-256 is checked and reused instead.
//!   4. in one transaction: check that the watch is still on, the folder is
//!      still authorized, and the file's record is still exactly the one the
//!      capture started from (present, same place, same object, same
//!      revision); then record the observation and insert the `snapshots`
//!      row, unless the bytes equal the file's latest snapshot. If any of
//!      that changed meanwhile, nothing is written.
//!
//! A crash between 3 and 4 leaves a stored copy that no row points at. It is
//! harmless, and a later capture of the same bytes uses it. Unfinished
//! copies in `incoming/` are removed at the next start.
//!
//! The snapshot folders are opened one by one from the app-data folder
//! without following symbolic links, so a link put in their place can never
//! send a write or a removal anywhere else.
//!
//! A capture holds the same lock as a plan's run, so a file is never captured
//! while Traeva moves it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use serde::Serialize;

use crate::contracts::{AppError, ErrorCode};
use crate::memory_contracts::{
    DocxPreview, PreviewText, SnapshotRef, TextExtraction, VersionPreview,
};
use crate::observe::{self, ReadError, StableRead};
use crate::platform::macos::{self, Folder};
use crate::reader::read_text;
use crate::store::{FileEvidence, FileRow, SeenFile, Store, sqlite_error, to_json};
use crate::{OrganizeService, evidence_of, is_canonical_uuid, read_in_root, seen_file};

/// The folder inside app data that holds the stored copies.
pub const SNAPSHOT_DIR: &str = "snapshots";
/// Copies being written; anything left here after a crash is unfinished.
const INCOMING_DIR: &str = "incoming";

/// What one capture did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capture {
    /// The new snapshot; or, when nothing changed, the file's latest one.
    pub snapshot: SnapshotRef,
    /// False when the bytes equal the latest snapshot and nothing was saved.
    pub created: bool,
}

impl OrganizeService {
    /// A capture without a watch, for this crate's tests only. The app
    /// captures a file only while Memory is on for it (capture.rs).
    #[cfg(test)]
    pub(crate) fn capture_snapshot_with(
        &self,
        file_id: &str,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> Result<Capture, AppError> {
        // Without a watch, `capture_for` always ends in a capture or an error.
        self.capture_for(file_id, None, read)?
            .ok_or_else(|| AppError::observation(ErrorCode::FileChanging, "Try again."))
    }

    /// Saves the current bytes of registered file `file_id` as its next
    /// snapshot, unless they equal its latest one. Only the same physical
    /// file is captured: when another file has taken its place (an app that
    /// saves by replacing the file, for example), nothing is recorded and
    /// the error says so. Which file records belong to the same work is #31's
    /// decision, not this function's.
    ///
    /// For Memory's watch `watch_id` (capture.rs), nothing is read once the
    /// watch is off, and nothing is saved if it was turned off while the file
    /// was read: `None`. (`stop_memory` takes the same lock as a capture, so
    /// it waits for a capture under way.)
    pub(crate) fn capture_for(
        &self,
        file_id: &str,
        watch_id: Option<&str>,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> Result<Option<Capture>, AppError> {
        self.store()?;
        check_file_id(file_id)?;
        let _no_run = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.capture_locked(file_id, watch_id, read)
    }

    /// `capture_for` for a caller that already holds the execution lock.
    pub(crate) fn capture_locked(
        &self,
        file_id: &str,
        watch_id: Option<&str>,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> Result<Option<Capture>, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if let Some(watch_id) = watch_id
            && !store.watch_is_on(watch_id, file_id)?
        {
            return Ok(None);
        }
        let (file, present) = store.get_file(file_id)?.ok_or_else(unknown_file)?;
        if let Some(busy) = store.unresolved_action_of_file(file_id)? {
            return Err(crate::execute::unresolved(file_id, None, &busy));
        }
        let (root_row, root) = self.verified_root(&file.root_id)?;
        let shown = format!("{}/{}", root_row.display_path, file.relative_path);
        if !present {
            return Err(with_file(not_there(&shown), file_id));
        }
        let mut segments: Vec<String> = file.relative_path.split('/').map(str::to_string).collect();
        let name = segments.pop().unwrap_or_default();
        // Another object at the place is never read (#31 material §3: with
        // candidate capture off, a replacement is only reported). Looked at
        // under the lock first; the check after the read stays, for an
        // object swapped in between.
        let names: Vec<&str> = segments.iter().map(String::as_str).collect();
        let looked = macos::open_chain(&root, &names)
            .and_then(|chain| macos::observe(chain.last().unwrap_or(&root), &name));
        if let Ok(observed) = looked
            && !evidence_of(&observed).same_object(&file.evidence)
        {
            return Err(with_file(replaced(&shown), file_id));
        }
        let stable = read_in_root(&root, &segments, &name, &shown, read)
            .map_err(|error| with_file(error, file_id))?;
        if !evidence_of(&stable.observed).same_object(&file.evidence) {
            return Err(with_file(replaced(&shown), file_id));
        }

        // The copy is stored (or an existing one checked) before any row.
        store_bytes(store.app_data_dir(), &stable.sha256, &stable.bytes)
            .map_err(|error| with_file(error, file_id))?;
        let seen = seen_file(&root_row.root_id, &segments, &name, &stable, self.now());
        match store.record_capture(&file, &seen, watch_id)? {
            Recorded::Saved(snapshot, created) => Ok(Some(Capture { snapshot, created })),
            Recorded::RecordChanged => Err(with_file(changed_meanwhile(&shown), file_id)),
            Recorded::WatchStopped => Ok(None),
            Recorded::RootRevoked => Err(with_file(
                revoked_meanwhile(&root_row.display_path),
                file_id,
            )),
        }
    }

    /// Every snapshot of file `file_id`, oldest first (by `sequence`).
    pub fn list_snapshots(&self, file_id: &str) -> Result<Vec<SnapshotRef>, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        store.snapshots_of(file_id)
    }

    /// The exact bytes of a snapshot, checked against its SHA-256 and length
    /// on every read. A missing or damaged copy is reported, never replaced.
    pub fn read_snapshot(&self, snapshot_id: &str) -> Result<(SnapshotRef, Vec<u8>), AppError> {
        let store = self.store()?;
        if !is_canonical_uuid(snapshot_id) {
            return Err(AppError::snapshot(
                ErrorCode::InvalidInput,
                "The saved version ID is not valid.",
                None,
            ));
        }
        let snapshot = store.get_snapshot(snapshot_id)?.ok_or_else(|| {
            AppError::snapshot(
                ErrorCode::UnknownSnapshot,
                "There is no saved version with this ID.",
                None,
            )
        })?;
        let bytes = load_bytes(store.app_data_dir(), &snapshot)?;
        Ok((snapshot, bytes))
    }

    /// The text of a snapshot's checked bytes (`reader::read_text`, with the
    /// file name the file had when it was captured).
    pub fn snapshot_text(
        &self,
        snapshot_id: &str,
    ) -> Result<(SnapshotRef, TextExtraction), AppError> {
        let (snapshot, bytes) = self.read_snapshot(snapshot_id)?;
        let name = snapshot
            .relative_path
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let text = read_text(name, &bytes);
        Ok((snapshot, text))
    }

    /// A saved .docx version's text for the window's local preview (#66,
    /// the student's limited Go of 2026-10-11): D's
    /// `docx::preview_docx_snapshot` reads this snapshot's checked bytes,
    /// never the current file, with the reader that also keeps the cached
    /// display text of citation fields (`docx-cache-local-1`). Only here:
    /// changes are compared and sent by `read_text`'s `docx-2`, which leaves
    /// those caches out. Nothing is stored, logged, copied or opened; a
    /// version saved under another name is `unsupported`, not an error.
    pub fn preview_docx_version(&self, snapshot_id: &str) -> Result<DocxPreview, AppError> {
        let (snapshot, extraction) = crate::docx::preview_docx_snapshot(self, snapshot_id)?;
        Ok(preview_of(snapshot, extraction))
    }

    /// A saved version's text for the window's local preview, whatever its
    /// type (contract §4.7, v23): the reader is picked by the name the file
    /// had when it was captured. A .docx gets the reader that keeps
    /// citations' cached text (`docx-cache-local-1`); a .pptx or .xlsx gets
    /// its reader's preview projection, which shows hidden slides and
    /// sheets marked as hidden; TXT/MD what Memory compares (`read_text`);
    /// anything else is `unsupported`. Bytes come only from `read_snapshot`,
    /// never from the current file. Nothing is stored, logged, copied,
    /// opened or sent.
    pub fn preview_version_text(&self, snapshot_id: &str) -> Result<VersionPreview, AppError> {
        let (snapshot, bytes) = self.read_snapshot(snapshot_id)?;
        let name = snapshot
            .relative_path
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let extraction = match extension_of(name).map(str::to_ascii_lowercase).as_deref() {
            Some("docx") => crate::docx::read_docx_cached(&bytes),
            Some("pptx") => crate::pptx::read_pptx_preview(&bytes),
            Some("xlsx") => crate::xlsx::read_xlsx_preview(&bytes),
            _ => read_text(name, &bytes),
        };
        Ok(preview_of(snapshot, extraction))
    }
}

fn preview_of(snapshot: SnapshotRef, extraction: TextExtraction) -> VersionPreview {
    VersionPreview {
        snapshot,
        extraction: PreviewText {
            summary: extraction.summary,
            text: extraction.text,
        },
    }
}

/// The extension of `name.ext`, by the readers' rule: a name before the dot.
fn extension_of(name: &str) -> Option<&str> {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => Some(extension),
        _ => None,
    }
}

/// At start, before anything else runs: removes copies a crash left
/// unfinished in `snapshots/incoming/`. Only plain files named like the
/// UUIDs Traeva gives them are removed; links are never followed. If this
/// fails, the leftovers only take space.
pub fn clear_incoming(app_data_dir: &Path) {
    let Ok(app) = open_app_data(app_data_dir) else {
        return;
    };
    let Ok(incoming) = open_child(&app, SNAPSHOT_DIR, false)
        .and_then(|snapshots| open_child(&snapshots, INCOMING_DIR, false))
    else {
        return;
    };
    let Ok(entries) = rfs::Dir::read_from(&incoming) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        let plain_file = rfs::statat(&incoming, name, AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile);
        if is_canonical_uuid(name) && plain_file {
            let _ = rfs::unlinkat(&incoming, name, AtFlags::empty());
        }
    }
}

// Stored copies

/// Check if SHA-256 is format properly.
fn is_valid_hash(sha256: &str) -> bool {
    if sha256.len() != 64 {
        return false;
    }
    for byte in sha256.bytes() {
        if byte.is_ascii_digit() {
            continue;
        } else if byte < 97 || byte > 102 {
            // Check if byte is not between the byte values of a and f
            return false;
        }
    }
    true
}

/// The app-data folder itself. Startup already checked where it really is
/// (#47), so a link to it is followed here; the folders inside are not.
pub(crate) fn open_app_data(app_data_dir: &Path) -> Result<OwnedFd, Errno> {
    rfs::open(
        app_data_dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Opens folder `name` inside `parent` without following a link (a link or
/// a file there is an error), creating it first when `create` is set.
pub(crate) fn open_child(parent: &OwnedFd, name: &str, create: bool) -> Result<OwnedFd, Errno> {
    if create {
        match rfs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(errno) => return Err(errno),
        }
    }
    rfs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Reads at most `length + 1` bytes of copy `name` in `shard`. Never follows
/// a link, never blocks on a special file, and only reads a plain file.
fn read_copy(shard: &OwnedFd, name: &str, length: u64) -> Result<Vec<u8>, Errno> {
    let fd = rfs::openat(
        shard,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stat = rfs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Errno::INVAL);
    }
    let mut bytes = Vec::new();
    File::from(fd)
        .take(length.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|err| Errno::from_io_error(&err).unwrap_or(Errno::IO))?;
    Ok(bytes)
}

/// Makes sure a checked copy of `bytes` is stored under its SHA-256, and that
/// its name is on disk. An existing copy is reused only if its bytes match;
/// one that does not match is left as it is and reported.
pub(crate) fn store_bytes(app_data_dir: &Path, sha256: &str, bytes: &[u8]) -> Result<(), AppError> {
    if !is_valid_hash(sha256) {
        return Err(cannot_save("not a SHA-256"));
    }
    let fail = |errno: Errno| cannot_save(&errno.to_string());
    let app = open_app_data(app_data_dir).map_err(fail)?;
    let snapshots = open_child(&app, SNAPSHOT_DIR, true).map_err(fail)?;
    let shard = open_child(&snapshots, &sha256[..2], true).map_err(fail)?;

    let stored = match rfs::statat(&shard, sha256, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => true,
        Err(Errno::NOENT) => false,
        Err(errno) => return Err(fail(errno)),
    };
    if stored {
        check_stored(&shard, sha256, bytes.len())?;
    } else {
        let incoming = open_child(&snapshots, INCOMING_DIR, true).map_err(fail)?;
        let temp = uuid::Uuid::new_v4().to_string();
        if let Err(error) = write_checked(&incoming, &temp, sha256, bytes) {
            let _ = rfs::unlinkat(&incoming, temp.as_str(), AtFlags::empty());
            return Err(error);
        }
        match rfs::renameat_with(
            &incoming,
            temp.as_str(),
            &shard,
            sha256,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {}
            Err(Errno::EXIST) => {
                // Another copy of the same bytes got there first.
                let _ = rfs::unlinkat(&incoming, temp.as_str(), AtFlags::empty());
                check_stored(&shard, sha256, bytes.len())?;
            }
            Err(errno) => {
                let _ = rfs::unlinkat(&incoming, temp.as_str(), AtFlags::empty());
                return Err(fail(errno));
            }
        }
    }
    // Make the copy's name durable, from its folder up to app data. Also for
    // a copy that was already there: an earlier capture may have stopped
    // after the move but before this.
    for folder in [&shard, &snapshots, &app] {
        rfs::fcntl_fullfsync(folder).map_err(fail)?;
    }
    Ok(())
}

/// An existing copy must hold exactly these bytes; otherwise it is reported
/// as damaged and left untouched.
fn check_stored(shard: &OwnedFd, sha256: &str, length: usize) -> Result<(), AppError> {
    match read_copy(shard, sha256, length as u64) {
        Ok(found) if found.len() == length && observe::sha256_hex(&found) == sha256 => Ok(()),
        Ok(_) => Err(damaged_copy()),
        Err(errno) => Err(cannot_save(&errno.to_string())),
    }
}

/// Writes a new file `temp` in `incoming`, flushes it to the disk
/// (F_FULLFSYNC), then reads it back and checks the SHA-256 of what is there.
fn write_checked(
    incoming: &OwnedFd,
    temp: &str,
    sha256: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    let fd = rfs::openat(
        incoming,
        temp,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|errno| cannot_save(&errno.to_string()))?;
    let mut file = File::from(fd);
    let fail = |err: std::io::Error| cannot_save(&err.kind().to_string());
    file.write_all(bytes).map_err(fail)?;
    rfs::fcntl_fullfsync(&file).map_err(|errno| cannot_save(&errno.to_string()))?;
    file.seek(SeekFrom::Start(0)).map_err(fail)?;
    let mut written = Vec::with_capacity(bytes.len());
    file.read_to_end(&mut written).map_err(fail)?;
    if observe::sha256_hex(&written) != sha256 {
        return Err(cannot_save("the bytes on disk differ from the bytes read"));
    }
    Ok(())
}

fn load_bytes(app_data_dir: &Path, snapshot: &SnapshotRef) -> Result<Vec<u8>, AppError> {
    let file_id = Some(snapshot.file_id.as_str());
    if !is_valid_hash(&snapshot.sha256) {
        return Err(AppError::snapshot(
            ErrorCode::SnapshotCorrupt,
            "The record of this saved version is damaged. Traeva does not use it.",
            file_id,
        ));
    }
    let bytes = open_app_data(app_data_dir)
        .and_then(|app| open_child(&app, SNAPSHOT_DIR, false))
        .and_then(|snapshots| open_child(&snapshots, &snapshot.sha256[..2], false))
        .and_then(|shard| read_copy(&shard, &snapshot.sha256, snapshot.byte_length))
        .map_err(|errno| {
            AppError::snapshot(
                ErrorCode::SnapshotUnavailable,
                format!(
                    "The saved copy of this version is missing or cannot be read ({errno}). \
                     Traeva does not replace it with the file as it is now."
                ),
                file_id,
            )
        })?;
    if bytes.len() as u64 != snapshot.byte_length || observe::sha256_hex(&bytes) != snapshot.sha256
    {
        return Err(AppError::snapshot(
            ErrorCode::SnapshotCorrupt,
            "The saved copy of this version no longer matches its SHA-256. Traeva does not \
             use it or repair it.",
            file_id,
        ));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// The `snapshots` table
// ---------------------------------------------------------------------------

impl Store {
    pub(crate) fn get_snapshot(&self, snapshot_id: &str) -> Result<Option<SnapshotRef>, AppError> {
        let conn = self.connection()?;
        Ok(query_snapshots(&conn, "WHERE snapshot_id = ?1", [snapshot_id])?.pop())
    }

    pub(crate) fn snapshots_of(&self, file_id: &str) -> Result<Vec<SnapshotRef>, AppError> {
        let conn = self.connection()?;
        query_snapshots(&conn, "WHERE file_id = ?1 ORDER BY sequence", [file_id])
    }

    /// One transaction. When the capture is for a watch, the watch must still
    /// be on; if not, nothing is written (`WatchStopped`). The file's record
    /// must still be exactly `expected` (present, same place, same physical
    /// object, same revision); if not, nothing is written (`RecordChanged`).
    /// Then the observation is recorded the way `record_observation` records
    /// the same object again, and the next snapshot is inserted, unless the
    /// latest one has the same bytes: then that one is returned (`false`).
    /// A new snapshot for a watch also becomes its record chain's newest
    /// version (#31), in the same transaction.
    fn record_capture(
        &self,
        expected: &FileRow,
        seen: &SeenFile,
        watch_id: Option<&str>,
    ) -> Result<Recorded, AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save the version", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if let Some(watch_id) = watch_id {
            let on: Option<bool> = tx
                .query_row(
                    "SELECT status = 'active' FROM memory_watches
                     WHERE watch_id = ?1 AND file_id = ?2",
                    [watch_id, expected.file_id.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(fail)?;
            if on != Some(true) {
                return Ok(Recorded::WatchStopped);
            }
        }
        // Revoking the folder is final, like in `confirm_plan`: a version
        // read just before is not saved after it.
        let root_on: Option<bool> = tx
            .query_row(
                "SELECT status = 'active' FROM authorized_roots WHERE root_id = ?1",
                [&seen.root_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(fail)?;
        if root_on != Some(true) {
            return Ok(Recorded::RootRevoked);
        }
        let now: Option<(bool, String, String, i64, String, i64, String)> = tx
            .query_row(
                "SELECT presence = 'present', root_id, relative_path, record_revision, sha256,
                        byte_length, observation_json
                 FROM files WHERE file_id = ?1",
                [&expected.file_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(fail)?;
        let Some((present, root_id, relative_path, revision, sha256, byte_length, evidence)) = now
        else {
            return Ok(Recorded::RecordChanged);
        };
        let evidence: FileEvidence = serde_json::from_str(&evidence).map_err(|_| {
            AppError::storage(
                ErrorCode::DatabaseCorrupt,
                "A saved file record could not be read. Traeva kept it unchanged.",
            )
        })?;
        let unchanged = present
            && root_id == seen.root_id
            && relative_path == seen.relative_path
            && revision == expected.record_revision
            && evidence.same_object(&seen.evidence);
        if !unchanged {
            return Ok(Recorded::RecordChanged);
        }

        let revision = update_same_object_in(
            &tx,
            &expected.file_id,
            revision,
            &evidence,
            &sha256,
            byte_length,
            seen,
        )?;
        let (snapshot, created) = insert_next_snapshot(&tx, &expected.file_id, revision, seen)?;
        if !created {
            tx.commit().map_err(fail)?;
            return Ok(Recorded::Saved(snapshot, false));
        }
        if let Some(watch_id) = watch_id {
            tx.execute(
                "UPDATE memory_chains SET tail_snapshot_id = ?1
                 WHERE chain_id = (SELECT chain_id FROM memory_chain_watches WHERE watch_id = ?2)",
                params![snapshot.snapshot_id, watch_id],
            )
            .map_err(fail)?;
        }
        tx.commit().map_err(fail)?;
        Ok(Recorded::Saved(snapshot, true))
    }
}

/// Records a new stable read of the same object in its `files` row, the way
/// `record_observation` records the same object again: the revision goes up
/// when anything changed. The row was `revision`, `evidence`, `sha256` and
/// `byte_length` before. Returns the revision now.
pub(crate) fn update_same_object_in(
    tx: &Connection,
    file_id: &str,
    revision: i64,
    evidence: &FileEvidence,
    sha256: &str,
    byte_length: i64,
    seen: &SeenFile,
) -> Result<i64, AppError> {
    let changed = sha256 != seen.sha256
        || u64::try_from(byte_length).ok() != Some(seen.byte_length)
        || !evidence.same_state(&seen.evidence);
    let revision = revision + i64::from(changed);
    tx.execute(
        "UPDATE files SET sha256 = ?2, byte_length = ?3, observation_json = ?4,
             observed_at_ms = ?5, record_revision = ?6, updated_at_ms = ?5
         WHERE file_id = ?1",
        params![
            file_id,
            seen.sha256,
            seen.byte_length as i64,
            to_json(&seen.evidence)?,
            seen.now_ms,
            revision
        ],
    )
    .map_err(|err| sqlite_error("Cannot save the version", &err))?;
    Ok(revision)
}

/// The file's next snapshot for the bytes just read, unless its latest one
/// has the same bytes: then that one, and `false`.
pub(crate) fn insert_next_snapshot(
    tx: &Connection,
    file_id: &str,
    revision: i64,
    seen: &SeenFile,
) -> Result<(SnapshotRef, bool), AppError> {
    let latest = query_snapshots(
        tx,
        "WHERE file_id = ?1 ORDER BY sequence DESC LIMIT 1",
        [file_id],
    )?
    .pop();
    if let Some(latest) = latest.filter(|latest| latest.sha256 == seen.sha256) {
        return Ok((latest, false));
    }
    let sequence: i64 = tx
        .query_row(
            "SELECT coalesce(max(sequence), 0) + 1 FROM snapshots WHERE file_id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .map_err(|err| sqlite_error("Cannot save the version", &err))?;
    let snapshot = SnapshotRef {
        snapshot_id: uuid::Uuid::new_v4().to_string(),
        file_id: file_id.to_string(),
        sequence,
        file_revision: revision,
        root_id: seen.root_id.clone(),
        relative_path: seen.relative_path.clone(),
        sha256: seen.sha256.clone(),
        byte_length: seen.byte_length,
        captured_at_ms: seen.now_ms,
    };
    tx.execute(
        "INSERT INTO snapshots (snapshot_id, file_id, sequence, file_revision, root_id,
             relative_path, sha256, byte_length, observation_json, captured_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            snapshot.snapshot_id,
            snapshot.file_id,
            snapshot.sequence,
            snapshot.file_revision,
            snapshot.root_id,
            snapshot.relative_path,
            snapshot.sha256,
            snapshot.byte_length as i64,
            to_json(&seen.evidence)?,
            snapshot.captured_at_ms
        ],
    )
    .map_err(|err| sqlite_error("Cannot save the version", &err))?;
    Ok((snapshot, true))
}

/// What `record_capture` did.
enum Recorded {
    /// The snapshot, and whether it is new.
    Saved(SnapshotRef, bool),
    /// The file's record changed while the file was read.
    RecordChanged,
    /// The watch the capture was for is off.
    WatchStopped,
    /// The folder was removed from Traeva while the file was read.
    RootRevoked,
}

/// `filter` is always a constant written in this file; values go in `params`.
pub(crate) fn query_snapshots<P: rusqlite::Params>(
    conn: &Connection,
    filter: &str,
    params: P,
) -> Result<Vec<SnapshotRef>, AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot read the saved versions", &err);
    let sql = format!(
        "SELECT snapshot_id, file_id, sequence, file_revision, root_id, relative_path, sha256,
                byte_length, captured_at_ms
         FROM snapshots {filter}"
    );
    let mut stmt = conn.prepare(&sql).map_err(fail)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(SnapshotRef {
                snapshot_id: row.get(0)?,
                file_id: row.get(1)?,
                sequence: row.get(2)?,
                file_revision: row.get(3)?,
                root_id: row.get(4)?,
                relative_path: row.get(5)?,
                sha256: row.get(6)?,
                byte_length: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
                captured_at_ms: row.get(8)?,
            })
        })
        .map_err(fail)?;
    rows.collect::<Result<_, _>>().map_err(fail)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

pub(crate) fn check_file_id(file_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(file_id) {
        Ok(())
    } else {
        Err(AppError::observation(
            ErrorCode::InvalidInput,
            "The file ID is not valid.",
        ))
    }
}

pub(crate) fn unknown_file() -> AppError {
    AppError::observation(
        ErrorCode::UnknownFile,
        "There is no registered file with this ID.",
    )
}

pub(crate) fn not_there(shown: &str) -> AppError {
    AppError::observation(
        ErrorCode::FileNotFound,
        format!("{shown} is no longer where Traeva last saw it. Choose the file again."),
    )
}

pub(crate) fn replaced(shown: &str) -> AppError {
    AppError::observation(
        ErrorCode::FileChanged,
        format!(
            "{shown} is now a different file than the one Traeva recorded (it was replaced). \
             Choose the file again to record the new one."
        ),
    )
}

fn changed_meanwhile(shown: &str) -> AppError {
    AppError::observation(
        ErrorCode::FileChanging,
        format!(
            "{shown} changed, moved or was chosen again while Traeva saved this version, so \
             nothing was saved. Try again."
        ),
    )
}

pub(crate) fn revoked_meanwhile(root: &str) -> AppError {
    AppError::authorization(
        ErrorCode::RootRevoked,
        format!(
            "The folder {root} was removed from Traeva while this version was being saved, so \
             nothing was saved."
        ),
    )
}

pub(crate) fn with_file(mut error: AppError, file_id: &str) -> AppError {
    error.file_id = Some(file_id.to_string());
    error
}

fn cannot_save(why: &str) -> AppError {
    AppError::storage(
        ErrorCode::AppDataUnavailable,
        format!(
            "Traeva could not save this version in its data folder ({why}). Nothing was \
             changed in your folders."
        ),
    )
}

fn damaged_copy() -> AppError {
    AppError::snapshot(
        ErrorCode::SnapshotCorrupt,
        "A saved copy with these bytes already exists but no longer matches its SHA-256. \
         Traeva left it as it is and saved nothing.",
        None,
    )
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
