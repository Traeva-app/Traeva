//! Exporting an old version to the user's folder (#35): a new file holding a
//! saved version's exact bytes, in a folder the user authorized, after the
//! user saw where it goes and confirmed it.
//!
//! Nothing new writes into a user's folder here. The copy is made in
//! Traeva's data folder first (`opened.rs`: created new, never through a
//! link, flushed, read back and checked against the saved version's
//! SHA-256), and then moved into the chosen folder by Gate 1's own
//! primitives (`platform/macos.rs`):
//!   - the folders are opened one name at a time from the authorized root,
//!     never following a link, never into a hidden folder, a package or a
//!     project folder (`open_chain`);
//!   - the one move never replaces anything (`move_no_replace`: the kernel
//!     refuses when the name is taken, in any case or spelling), with no
//!     fallback to copying;
//!   - after the move the root must still be the authorized one, the folders
//!     the ones opened, and the name must hold the very file that was moved
//!     (`verified_root`, `verify_chain`, `confirm_moved`, as Organize's
//!     check after a move); otherwise the result is reported as
//!     unconfirmed, never as done, and nothing is moved back.
//!
//! The order is relocate.rs's: the execution lock first (no Organize move
//! and no Memory capture runs meanwhile), then no unsettled Organize action
//! in the root, then the root itself, so a root removed or replaced while
//! the export waited for the lock is never written into. The export carries
//! the preview's token, which covers the version, the root and every folder
//! on the way: a place that changed since it was shown is refused.
//!
//! The folder must be on the same disk as Traeva's data folder (a move never
//! crosses disks); that is checked before anything is written. Only the text
//! files Memory reads are exported, under a name with the original
//! extension: a file Traeva writes has no quarantine mark, so another kind
//! could skip the system's first-open check. The new file gets the
//! permissions any new file gets here (the user's umask). Nothing is
//! recorded in the database: the new file is the user's.

use std::path::{Path, PathBuf};

use rustix::fs::{self as rfs, AtFlags};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode};
use crate::memory_consent::memory_error;
use crate::memory_contracts::SnapshotRef;
use crate::observe::{is_text_file_name, sha256_hex};
use crate::opened::{OPENED_DIR, new_folder, write_checked};
use crate::platform::macos::{self, Folder, FsError, NameProblem, NewFolderName};
use crate::snapshot::{open_app_data, open_child};

/// An exported file is an ordinary new file: created with 0666 less the
/// process umask, as any app's new file, and never made more open.
const EXPORTED_CREATE_MODE: u16 = 0o666;

/// Where a copy of a version would go. Nothing is written for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPreview {
    pub snapshot: SnapshotRef,
    pub root_id: String,
    /// The folder inside the root, names joined by "/"; empty for the root.
    pub folder: String,
    pub name: String,
    /// The whole place, for the page.
    pub display_path: String,
    pub byte_length: i64,
    /// Sent back unchanged with the export: it covers the version, the root
    /// and every folder on the way.
    pub page_token: String,
}

/// Exporting what a preview showed: the window sends its parts back, and
/// everything is checked again.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub snapshot_id: String,
    pub root_id: String,
    pub folder: String,
    pub name: String,
    pub page_token: String,
}

/// A copy written to the user's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportedVersion {
    pub snapshot: SnapshotRef,
    pub display_path: String,
    /// The copy's SHA-256 as read back before the move: the saved version's.
    pub sha256: String,
    pub byte_length: i64,
}

impl OrganizeService {
    /// Where a copy of version `snapshot_id` would go, for the place
    /// `picked` that the native save panel returned in root `root_id`. Only
    /// looks, with the export's own checks, so a place the export would
    /// refuse is refused here already.
    pub fn preview_export(
        &self,
        snapshot_id: &str,
        root_id: &str,
        picked: &Path,
    ) -> Result<ExportPreview, AppError> {
        let store = self.store()?;
        let (snapshot, bytes) = self.read_snapshot(snapshot_id)?;
        let (row, root) = self.verified_root(root_id)?;
        let (segments, name) = crate::relative_location(&row.display_path, picked)?;
        let target = Target::new(root_id, &row.display_path, segments, name);
        target.check_name(&snapshot)?;
        self.no_unsettled_action(store, root_id, &snapshot.file_id)?;
        let chain = target.open(&root)?;
        let into = chain.last().unwrap_or(&root);
        target.check_free(into)?;
        target.check_same_disk(store.app_data_dir(), into)?;
        Ok(ExportPreview {
            root_id: root_id.to_string(),
            folder: target.folder(),
            display_path: target.display_path(),
            page_token: target.token(&snapshot, &root, &chain),
            name: target.name,
            byte_length: bytes.len() as i64,
            snapshot,
        })
    }

    /// Writes a copy of a version where the preview showed. Errors, with
    /// nothing written to the user's folder: the ones of `read_snapshot`
    /// and of the root; `invalid_input` for a folder or name that is not a
    /// plain one, or a name without the original extension;
    /// `unsupported_file` for a version that is not a text file;
    /// `unsupported_location` for a folder on another disk;
    /// `protected_location`, `file_not_found` for a folder that is hidden,
    /// a package or a project, or gone; `target_conflict` when the name is
    /// taken; `unresolved_action` while an Organize action in the root is
    /// not settled; `page_outdated` when the root or a folder is not the one
    /// the preview showed; `app_data_unavailable` when the copy cannot be
    /// made; `operation_refused` when macOS refuses the move. After the
    /// move, `result_unconfirmed` when Traeva cannot confirm where the copy
    /// is.
    pub fn export_version(&self, request: &ExportRequest) -> Result<ExportedVersion, AppError> {
        let store = self.store()?;
        check_token(&request.page_token)?;
        let segments = plain_folder(&request.folder)?;
        let (snapshot, bytes) = self.read_snapshot(&request.snapshot_id)?;
        // No Organize move and no Memory capture runs while the copy goes
        // in; the root is checked only once the lock is held.
        let _no_move = self.lock_execution();
        self.no_unsettled_action(store, &request.root_id, &snapshot.file_id)?;
        let (row, root) = self.verified_root(&request.root_id)?;
        let target = Target::new(
            &request.root_id,
            &row.display_path,
            segments,
            request.name.clone(),
        );
        target.check_name(&snapshot)?;
        let chain = target.open(&root)?;
        let into = chain.last().unwrap_or(&root);
        target.check_free(into)?;
        target.check_same_disk(store.app_data_dir(), into)?;
        if target.token(&snapshot, &root, &chain) != request.page_token {
            return Err(memory_error(
                ErrorCode::PageOutdated,
                format!(
                    "{} is not the place the preview showed any more (its folder or root \
                     changed). Nothing was written. Export again.",
                    target.display_path()
                ),
                true,
                None,
            ));
        }

        let staged = Staged::write(store.app_data_dir(), &target.name, &bytes, &snapshot)?;
        let moved = macos::move_no_replace(&staged.folder, &target.name, into, &target.name);
        if let Err(err) = moved {
            if matches!(err, FsError::ChangedButUnconfirmed) {
                return Err(target.unconfirmed());
            }
            staged.remove_copy();
            return Err(target.move_error(err));
        }
        let confirmed = macos::verify_chain(&root, &chain).and_then(|()| {
            macos::confirm_moved(
                &staged.folder,
                &target.name,
                into,
                &target.name,
                staged.file,
            )
        });
        // The root must still be the authorized folder it was (Organize's
        // `check_after_move`): the copy went into the folders that handle
        // reached, which the path may no longer name.
        let same_root = matches!(
            self.verified_root(&request.root_id),
            Ok((_, now)) if now.id() == root.id()
        );
        if confirmed.is_err() || !same_root {
            return Err(target.unconfirmed());
        }
        staged.remove_folder();
        Ok(ExportedVersion {
            display_path: target.display_path(),
            sha256: snapshot.sha256.clone(),
            byte_length: bytes.len() as i64,
            snapshot,
        })
    }
}

/// The folders and name a copy goes to, inside one root.
struct Target {
    root_id: String,
    root_path: String,
    segments: Vec<String>,
    name: String,
}

impl Target {
    fn new(root_id: &str, root_path: &str, segments: Vec<String>, name: String) -> Target {
        Target {
            root_id: root_id.to_string(),
            root_path: root_path.to_string(),
            segments,
            name,
        }
    }

    fn folder(&self) -> String {
        self.segments.join("/")
    }

    fn display_path(&self) -> String {
        let mut path = PathBuf::from(&self.root_path);
        path.extend(&self.segments);
        path.join(&self.name).display().to_string()
    }

    /// A name Organize would accept for something new, ending in the
    /// version's own text extension.
    fn check_name(&self, snapshot: &SnapshotRef) -> Result<(), AppError> {
        let original = snapshot
            .relative_path
            .rsplit('/')
            .next()
            .unwrap_or_default();
        if !is_text_file_name(original) {
            return Err(memory_error(
                ErrorCode::UnsupportedFile,
                "Only versions of text files (.txt, .md, .markdown) can be exported for now. \
                 Nothing was written.",
                false,
                Some(&snapshot.file_id),
            ));
        }
        let wanted = extension(original);
        if let Err(problem) = NewFolderName::parse(&self.name) {
            return Err(invalid(&format!(
                "“{}” cannot be used as a file name: {}. Nothing was written.",
                self.name,
                name_problem(&problem)
            )));
        }
        if extension(&self.name) != wanted {
            return Err(invalid(&format!(
                "The name must end in .{wanted}, like the file this version is of. Nothing was \
                 written."
            )));
        }
        Ok(())
    }

    /// The move cannot cross disks: refused before anything is written.
    fn check_same_disk(&self, app_data: &Path, into: &Folder) -> Result<(), AppError> {
        let app = open_app_data(app_data).and_then(|fd| rfs::fstat(&fd));
        match app {
            Ok(app) if app.st_dev as i64 == into.id().dev => Ok(()),
            Ok(_) => Err(self.move_error(FsError::CrossVolume)),
            Err(errno) => Err(memory_error(
                ErrorCode::AppDataUnavailable,
                format!("Traeva could not check its data folder ({errno}). Nothing was written."),
                true,
                None,
            )),
        }
    }

    /// What the preview showed, for the export to check it is still so: the
    /// version, the root and each folder on the way (by identity, not only
    /// by name), and the name.
    fn token(&self, snapshot: &SnapshotRef, root: &Folder, chain: &[Folder]) -> String {
        let folder = |f: &Folder| {
            format!(
                "{}:{}:{}:{}",
                f.name(),
                f.id().dev,
                f.id().ino,
                f.observation().birthtime_ns
            )
        };
        let mut lines = vec![
            "memory-export-1".to_string(),
            format!("version:{}:{}", snapshot.snapshot_id, snapshot.sha256),
            format!("root:{}:{}", self.root_id, folder(root)),
        ];
        lines.extend(chain.iter().map(|f| format!("folder:{}", folder(f))));
        lines.push(format!("name:{}", self.name));
        sha256_hex(lines.join("\n").as_bytes())
    }

    /// The folders, opened one by one from the root.
    fn open(&self, root: &Folder) -> Result<Vec<Folder>, AppError> {
        let names: Vec<&str> = self.segments.iter().map(String::as_str).collect();
        macos::open_chain(root, &names).map_err(|err| self.folder_error(err))
    }

    /// The name is free now (the move checks again, atomically).
    fn check_free(&self, into: &Folder) -> Result<(), AppError> {
        match macos::observe(into, &self.name) {
            Err(FsError::NotFound) => Ok(()),
            Ok(_) => Err(self.taken()),
            Err(err) => Err(self.folder_error(err)),
        }
    }

    fn taken(&self) -> AppError {
        memory_error(
            ErrorCode::TargetConflict,
            format!(
                "{} already exists. Traeva never replaces a file: choose another name. Nothing \
                 was written.",
                self.display_path()
            ),
            false,
            None,
        )
    }

    fn unconfirmed(&self) -> AppError {
        memory_error(
            ErrorCode::ResultUnconfirmed,
            format!(
                "The copy was moved toward {}, but Traeva could not confirm where it is now. \
                 Look in the folder; nothing will be moved back.",
                self.display_path()
            ),
            false,
            None,
        )
    }

    fn move_error(&self, err: FsError) -> AppError {
        match err {
            FsError::AlreadyExists => self.taken(),
            FsError::CrossVolume => memory_error(
                ErrorCode::UnsupportedLocation,
                format!(
                    "{} is on another disk than Traeva's data folder, so the copy cannot be \
                     moved there. Choose a folder on this disk. Nothing was written.",
                    self.root_path
                ),
                false,
                None,
            ),
            other => memory_error(
                ErrorCode::OperationRefused,
                format!(
                    "macOS refused to put the copy at {} ({other:?}). Nothing was written.",
                    self.display_path()
                ),
                false,
                None,
            ),
        }
    }

    fn folder_error(&self, err: FsError) -> AppError {
        let mut shown = PathBuf::from(&self.root_path);
        shown.extend(&self.segments);
        let shown = shown.display();
        let (code, why) = match err {
            FsError::NotFound | FsError::ChainChanged => {
                (ErrorCode::FileNotFound, "is not there any more")
            }
            FsError::Protected(_) => (
                ErrorCode::ProtectedLocation,
                "is hidden, a package or a project folder, where Traeva never writes",
            ),
            FsError::SymbolicLink | FsError::NotADirectory => (
                ErrorCode::UnsupportedLocation,
                "is reached through a link or is not a plain folder",
            ),
            FsError::CrossVolume => (ErrorCode::UnsupportedLocation, "is on another disk"),
            FsError::Dataless => (
                ErrorCode::UnsupportedLocation,
                "is a cloud placeholder that is not on this Mac",
            ),
            _ => (ErrorCode::OperationRefused, "cannot be opened"),
        };
        memory_error(
            code,
            format!(
                "The folder {shown} {why} (in root {}). Nothing was written.",
                self.root_id
            ),
            false,
            None,
        )
    }
}

/// The copy made in Traeva's data folder, before it moves.
struct Staged {
    /// `opened` in app data, which holds `id`.
    opened: std::os::fd::OwnedFd,
    id: String,
    folder: Folder,
    name: String,
    /// The copy itself, to check it is the file that arrived.
    file: macos::ObjectId,
}

impl Staged {
    fn write(
        app_data: &Path,
        name: &str,
        bytes: &[u8],
        snapshot: &SnapshotRef,
    ) -> Result<Staged, AppError> {
        let fail = |why: &str| {
            memory_error(
                ErrorCode::AppDataUnavailable,
                format!(
                    "Traeva could not make the copy in its data folder ({why}). Nothing was \
                     written to your folder."
                ),
                true,
                None,
            )
        };
        let app = open_app_data(app_data).map_err(|e| fail(&e.to_string()))?;
        let opened = open_child(&app, OPENED_DIR, true).map_err(|e| fail(&e.to_string()))?;
        let (id, fd) = new_folder(&opened).map_err(|why| fail(&why))?;
        let cleanup = |opened: &std::os::fd::OwnedFd| {
            let _ = rfs::unlinkat(&fd, name, AtFlags::empty());
            let _ = rfs::unlinkat(opened, id.as_str(), AtFlags::REMOVEDIR);
        };
        if let Err(why) = write_checked(
            &fd,
            name,
            bytes,
            &snapshot.sha256,
            EXPORTED_CREATE_MODE,
            None,
        ) {
            cleanup(&opened);
            return Err(fail(&why));
        }
        // A handle the Gate 1 primitives take, checked to be the folder just
        // made.
        let path = app_data.join(OPENED_DIR).join(&id);
        let made = rfs::fstat(&fd).map_err(|e| fail(&e.to_string()))?;
        let folder = match macos::open_folder_path(&path) {
            Ok(folder)
                if folder.id().dev == made.st_dev as i64
                    && folder.id().ino == made.st_ino as u64 =>
            {
                folder
            }
            _ => {
                cleanup(&opened);
                return Err(fail("its folder changed"));
            }
        };
        let file = match macos::observe(&folder, name) {
            Ok(observed) => observed.id,
            Err(err) => {
                cleanup(&opened);
                return Err(fail(&format!("{err:?}")));
            }
        };
        Ok(Staged {
            opened,
            id,
            folder,
            name: name.to_string(),
            file,
        })
    }

    /// The move did not happen: the copy and its folder go.
    fn remove_copy(&self) {
        let _ = rfs::unlinkat(self.folder.fd(), self.name.as_str(), AtFlags::empty());
        self.remove_folder();
    }

    /// The copy moved out: only its folder is left, empty. A folder that is
    /// not empty is kept (rmdir never removes anything in it).
    fn remove_folder(&self) {
        let _ = rfs::unlinkat(&self.opened, self.id.as_str(), AtFlags::REMOVEDIR);
        let _ = rfs::fcntl_fullfsync(&self.opened);
    }
}

/// A token is a SHA-256 in lowercase hex.
fn check_token(token: &str) -> Result<(), AppError> {
    if token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(invalid("The page token is not valid. Nothing was written."))
    }
}

/// The folder part of a request: names joined by "/", or empty for the
/// root. Nothing that could step out of the root or be read two ways.
fn plain_folder(folder: &str) -> Result<Vec<String>, AppError> {
    if folder.is_empty() {
        return Ok(Vec::new());
    }
    // A leading or doubled "/" gives an empty name.
    if folder.contains('\0')
        || folder
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid(
            "The folder is not the one the page showed. Nothing was written.",
        ));
    }
    Ok(folder.split('/').map(str::to_string).collect())
}

/// Why a name breaks the naming rules, in the user's words.
fn name_problem(problem: &NameProblem) -> &'static str {
    match problem {
        NameProblem::Empty => "it is empty",
        NameProblem::DotOrDotDot => "it is only dots",
        NameProblem::PathSeparator => "it holds a slash",
        NameProblem::Colon => "it holds a colon",
        NameProblem::ControlCharacter => "it holds a control character",
        NameProblem::LeadingDot => "it starts with a dot, which hides a file",
        NameProblem::EdgeWhitespace => "it starts or ends with a space",
        NameProblem::TooLong => "it is longer than 255 bytes",
        NameProblem::PackageExtension => "it ends like a package (an app or bundle)",
        NameProblem::ProjectMarker => "it is the name of a project file",
    }
}

/// The extension after the last dot, lowercased; empty when there is none.
fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default()
}

fn invalid(message: &str) -> AppError {
    memory_error(ErrorCode::InvalidInput, message, false, None)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
