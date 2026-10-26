//! macOS filesystem safety primitives (architecture.md §4, organize-spec.md §2.1 and §4.2).
//!
//! The idea behind this file: Traeva never makes a safety decision from a path
//! string. It opens a folder once, keeps the open handle (a file descriptor),
//! and reaches everything below it one name at a time with `openat`-style
//! calls that never follow symbolic links. Whatever happens to path strings in
//! the meantime, a handle keeps pointing at the folder Traeva checked.
//!
//! What handles do not give us: the folder tree is not locked. Another program
//! can still move a folder while Traeva works. That is why callers re-check
//! the chain after a change (`verify_chain`, `confirm_moved`) and report
//! `needs_user_decision` instead of success when something moved.
//!
//! Nothing in this file writes anything except `create_folder`,
//! `move_no_replace` and `remove_empty_folder`. The first two run only after
//! a confirmed plan; the last only for an empty folder Traeva has a receipt for.

use std::ffi::{CStr, CString, c_int};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Component, Path, PathBuf};

use icu_normalizer::DecomposingNormalizerBorrowed;
use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;

/// `SF_DATALESS` from <sys/stat.h>: the file is an iCloud / File Provider
/// placeholder whose content is not on this Mac. (The libc crate lacks it.)
const SF_DATALESS: u32 = 0x4000_0000;

/// Every folder is opened read-only, must be a folder, and must not be a
/// symbolic link. `CLOEXEC` keeps the handle out of child processes.
const FOLDER_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Upper bound when walking up through parent folders; real paths are far shorter.
const MAX_ANCESTORS: usize = 256;

/// Names whose presence marks a folder as a project that Traeva must not
/// reorganize (organize-spec.md §4.2).
const PROJECT_MARKER_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "package.json",
    "Cargo.toml",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
];

/// Xcode projects are folders with these extensions.
const PROJECT_MARKER_EXTENSIONS: &[&str] = &["xcodeproj", "xcworkspace"];

/// Folder extensions that macOS shows as a single item (a package or bundle).
/// Traeva never walks into one. This is the basic list the spec asks for,
/// not every package type macOS knows.
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "xpc",
    "photoslibrary",
    "musiclibrary",
    "imovielibrary",
    "tvlibrary",
    "fcpbundle",
    "logicx",
    "band",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "xcodeproj",
    "xcworkspace",
    "xcarchive",
    "playground",
    "dsym",
];

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// Why a filesystem step was refused or failed. Callers turn this into an
/// `AppError` for their stage, roughly:
///
/// - not allowed here (`Protected`, `UnsupportedVolume`, `CrossVolume`,
///   `Dataless`, `NotSupportedFile`, `SymbolicLink`, `InvalidName`) → blocked
/// - `AlreadyExists` → conflict: show a new preview, never overwrite
/// - `ChainChanged` before any change → stale; after a change → needs_user_decision
/// - `ChangedButUnconfirmed` → needs_user_decision, never "nothing happened"
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// The name does not exist.
    NotFound,
    /// The target name is taken, including by a differently-cased or
    /// differently-normalized spelling, or by a dangling symbolic link.
    AlreadyExists,
    /// The item is a symbolic link. Traeva never follows one.
    SymbolicLink,
    /// The item had to be a folder but is something else.
    NotADirectory,
    /// Not a plain single-link file (a folder, special file or hard link).
    NotSupportedFile(&'static str),
    /// The step would leave the root's volume.
    CrossVolume,
    /// The volume is not local, writable APFS.
    UnsupportedVolume(String),
    /// iCloud / File Provider placeholder whose content is not on this Mac.
    Dataless,
    /// Hidden item, package, project folder or app data.
    Protected(Protection),
    /// The root and Traeva's app-data folder overlap, or that cannot be ruled out.
    OverlapsAppData(Overlap),
    InvalidName(NameProblem),
    PermissionDenied,
    /// A folder Traeva had checked is no longer the object at its name.
    ChainChanged,
    /// A change (mkdir or move) happened, or may have happened, but could not
    /// be confirmed or made durable. Never treat this as "nothing happened".
    ChangedButUnconfirmed,
    /// The folder is not empty (a `.DS_Store` counts), so it was kept.
    NotEmpty,
    /// Any other OS error, by number.
    Other(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protection {
    /// The name starts with "." (hidden configuration).
    Hidden,
    /// A package such as `Photos Library.photoslibrary`.
    Package(String),
    /// The folder contains this project marker, e.g. `.git` or `package.json`.
    Project(String),
    /// Inside a Library folder, which holds application data.
    AppData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    RootContainsAppData,
    RootInsideAppData,
    CannotTell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    DotOrDotDot,
    PathSeparator,
    Colon,
    ControlCharacter,
    LeadingDot,
    EdgeWhitespace,
    TooLong,
    /// Ends like a package (e.g. ".app"): Finder would show the new folder as
    /// a package, and Traeva itself never walks into packages.
    PackageExtension,
    /// A project marker's name (e.g. "Cargo.toml"): once the folder exists,
    /// the folder around it counts as a project, which Traeva leaves alone
    /// (#58).
    ProjectMarker,
}

fn from_errno(errno: Errno) -> FsError {
    match errno {
        Errno::NOENT => FsError::NotFound,
        Errno::EXIST => FsError::AlreadyExists,
        Errno::LOOP => FsError::SymbolicLink,
        Errno::NOTDIR => FsError::NotADirectory,
        Errno::XDEV => FsError::CrossVolume,
        Errno::ACCESS | Errno::PERM => FsError::PermissionDenied,
        Errno::NAMETOOLONG => FsError::InvalidName(NameProblem::TooLong),
        Errno::ROFS => FsError::UnsupportedVolume("read-only volume".to_string()),
        Errno::NOTEMPTY => FsError::NotEmpty,
        other => FsError::Other(other.raw_os_error()),
    }
}

// ---------------------------------------------------------------------------
// Observations and folder handles
// ---------------------------------------------------------------------------

/// Device and inode number. This is physical evidence only: inode numbers can
/// be reused, so Traeva never uses it as a file's identity. `dev` is only
/// valid while its volume stays mounted (see `volume_uuid`): compare two live
/// values, never a saved one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId {
    pub dev: i64,
    pub ino: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    SymbolicLink,
    Other,
}

/// What `stat` reports about one object, without following symbolic links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub id: ObjectId,
    pub kind: Kind,
    pub link_count: u64,
    pub size: u64,
    pub mtime_ns: i64,
    /// Changes whenever content or metadata changes, and cannot be set back by hand.
    pub ctime_ns: i64,
    pub birthtime_ns: i64,
    pub flags: u32,
}

impl Observation {
    fn from_stat(st: &rfs::Stat) -> Observation {
        let kind = match FileType::from_raw_mode(st.st_mode) {
            FileType::RegularFile => Kind::File,
            FileType::Directory => Kind::Directory,
            FileType::Symlink => Kind::SymbolicLink,
            _ => Kind::Other,
        };
        Observation {
            id: ObjectId {
                dev: i64::from(st.st_dev),
                ino: st.st_ino,
            },
            kind,
            link_count: u64::from(st.st_nlink),
            size: u64::try_from(st.st_size).unwrap_or(0),
            mtime_ns: nanoseconds(st.st_mtime, st.st_mtime_nsec),
            ctime_ns: nanoseconds(st.st_ctime, st.st_ctime_nsec),
            birthtime_ns: nanoseconds(st.st_birthtime, st.st_birthtime_nsec),
            flags: st.st_flags,
        }
    }

    pub fn is_dataless(&self) -> bool {
        is_dataless_flags(self.flags)
    }
}

fn nanoseconds(seconds: i64, nanos: i64) -> i64 {
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

pub fn is_dataless_flags(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

/// An open folder that Traeva has checked. The handle keeps pointing at that
/// same folder even if someone renames or moves it.
#[derive(Debug)]
pub struct Folder {
    fd: OwnedFd,
    observed: Observation,
    /// The name inside its parent; empty for a folder opened by absolute path.
    name: String,
}

impl Folder {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn id(&self) -> ObjectId {
        self.observed.id
    }

    pub fn observation(&self) -> &Observation {
        &self.observed
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Opens an absolute folder path: the authorized root or the app-data folder.
/// The last component must be a real folder, not a symbolic link.
pub fn open_folder_path(path: &Path) -> Result<Folder, FsError> {
    let fd = match rfs::open(path, FOLDER_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOTDIR | Errno::LOOP) => {
            return match rfs::lstat(path) {
                Ok(st) if Observation::from_stat(&st).kind == Kind::SymbolicLink => {
                    Err(FsError::SymbolicLink)
                }
                _ => Err(FsError::NotADirectory),
            };
        }
        Err(errno) => return Err(from_errno(errno)),
    };
    let observed = Observation::from_stat(&rfs::fstat(&fd).map_err(from_errno)?);
    Ok(Folder {
        fd,
        observed,
        name: String::new(),
    })
}

/// Opens the folder at `path`, following symbolic links anywhere in it. Only
/// for Traeva's own app-data folder at startup (#47), where the folder's real
/// place is what counts, wherever a link points.
pub fn open_real_folder(path: &Path) -> Result<Folder, FsError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let fd = rfs::open(path, flags, Mode::empty()).map_err(from_errno)?;
    let observed = Observation::from_stat(&rfs::fstat(&fd).map_err(from_errno)?);
    Ok(Folder {
        fd,
        observed,
        name: String::new(),
    })
}

/// Looks at one name inside `parent` without following symbolic links and
/// without opening or reading the item.
pub fn observe(parent: &Folder, name: &str) -> Result<Observation, FsError> {
    check_existing_name(name)?;
    rfs::statat(parent.fd(), name, AtFlags::SYMLINK_NOFOLLOW)
        .map(|st| Observation::from_stat(&st))
        .map_err(from_errno)
}

/// Opens file `name` inside `parent` for reading only. Never follows a symbolic
/// link. `NONBLOCK` keeps a named pipe swapped in at the last moment from
/// blocking the open; the caller still checks the opened object is a plain file.
pub fn open_file(parent: &Folder, name: &str) -> Result<File, FsError> {
    check_existing_name(name)?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rfs::openat(parent.fd(), name, flags, Mode::empty()).map_err(from_errno)?;
    Ok(File::from(fd))
}

/// What `fstat` reports about an open file.
pub fn observe_open(file: &File) -> Result<Observation, FsError> {
    rfs::fstat(file)
        .map(|st| Observation::from_stat(&st))
        .map_err(from_errno)
}

/// The extended attribute macOS keeps a file's "last opened" time in: 16
/// bytes, seconds then nanoseconds since 1970, each a little-endian i64.
const LAST_OPENED_ATTR: &str = "com.apple.lastuseddate#PS";

/// When file `name` inside `parent` was last opened, as macOS recorded it,
/// in milliseconds since 1970; None when there is no such record (#198).
/// `expected` is the object listed there: it is checked again right before
/// the open and on the open file. A file only in iCloud (dataless) is never
/// opened, so nothing is downloaded; the content is never read.
pub fn last_opened_ms(parent: &Folder, name: &str, expected: ObjectId) -> Option<i64> {
    let readable =
        |seen: &Observation| seen.kind == Kind::File && seen.id == expected && !seen.is_dataless();
    if !readable(&observe(parent, name).ok()?) {
        return None;
    }
    let file = open_file(parent, name).ok()?;
    if !readable(&observe_open(&file).ok()?) {
        return None;
    }
    let mut value = [0u8; 32];
    let length = rfs::fgetxattr(&file, LAST_OPENED_ATTR, &mut value[..]).ok()?;
    if length != 16 {
        return None;
    }
    let seconds = i64::from_le_bytes(value[..8].try_into().ok()?);
    let nanos = i64::from_le_bytes(value[8..16].try_into().ok()?);
    Some(
        seconds
            .saturating_mul(1000)
            .saturating_add(nanos.div_euclid(1_000_000)),
    )
}

/// What getattrlistat returns for the added time: the reply's length, which
/// attributes came back, then the device, the inode and the time, packed
/// at 4 bytes as the kernel writes them.
#[repr(C, packed(4))]
struct AddedReply {
    length: u32,
    returned: libc::attribute_set_t,
    dev: i32,
    ino: u64,
    added: libc::timespec,
}

/// When file `name` in `parent` was put in that folder (created there, or
/// moved or renamed into it), as the volume records it, in milliseconds
/// since 1970; None when it has no record or `name` is no longer the object
/// `expected`. Only attributes are read: the file is not opened, so a file
/// only in iCloud is not downloaded. Find uses it for "the file I organized
/// two days ago" (the student's decision 10/17).
pub fn added_ms(parent: &Folder, name: &str, expected: ObjectId) -> Option<i64> {
    check_existing_name(name).ok()?;
    let name = CString::new(name).ok()?;
    // SAFETY: attrlist is plain data, for which all zeros is a valid value.
    let mut request: libc::attrlist = unsafe { std::mem::zeroed() };
    request.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    request.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
        | libc::ATTR_CMN_DEVID
        | libc::ATTR_CMN_FILEID
        | libc::ATTR_CMN_ADDEDTIME;
    // SAFETY: AddedReply is plain data, for which all zeros is a valid value.
    let mut reply: AddedReply = unsafe { std::mem::zeroed() };
    // SAFETY: the folder's descriptor is open, `name` is a C string, and
    // `reply` is as large as the size given.
    let done = unsafe {
        libc::getattrlistat(
            std::os::fd::AsRawFd::as_raw_fd(&parent.fd()),
            name.as_ptr(),
            (&raw mut request).cast(),
            (&raw mut reply).cast(),
            std::mem::size_of::<AddedReply>(),
            libc::FSOPT_NOFOLLOW.into(),
        )
    };
    let returned = reply.returned.commonattr;
    let wanted = libc::ATTR_CMN_DEVID | libc::ATTR_CMN_FILEID | libc::ATTR_CMN_ADDEDTIME;
    let (dev, ino, added) = (reply.dev, reply.ino, reply.added);
    if done != 0 || returned & wanted != wanted {
        return None;
    }
    if (ObjectId {
        dev: i64::from(dev),
        ino,
    }) != expected
    {
        return None;
    }
    Some(
        added
            .tv_sec
            .saturating_mul(1000)
            .saturating_add(added.tv_nsec.div_euclid(1_000_000)),
    )
}

/// Reads the whole file from the start, but never more than `limit + 1` bytes,
/// so a caller can tell "exactly at the limit" from "over the limit".
pub fn read_whole_file(file: &mut File, limit: u64) -> Result<Vec<u8>, FsError> {
    file.seek(SeekFrom::Start(0)).map_err(from_io)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(from_io)?;
    Ok(bytes)
}

fn from_io(error: std::io::Error) -> FsError {
    match error.raw_os_error() {
        Some(code) => from_errno(Errno::from_raw_os_error(code)),
        None => FsError::Other(0),
    }
}

// ---------------------------------------------------------------------------
// Names (organize-spec.md §4.2)
// ---------------------------------------------------------------------------

/// New folder's name, checked for eligibility by following the spec's naming rules.
/// Only `NewFolderName::parse` can make a NewFolderName, so no one can skip the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFolderName(String);

impl NewFolderName {
    pub fn parse(name: &str) -> Result<NewFolderName, NameProblem> {
        if name.is_empty() {
            return Err(NameProblem::Empty);
        }
        if name == "." || name == ".." {
            return Err(NameProblem::DotOrDotDot);
        }
        if name.contains('/') || name.contains('\\') {
            return Err(NameProblem::PathSeparator);
        }
        if name.contains(':') {
            return Err(NameProblem::Colon);
        }
        if name.chars().any(char::is_control) {
            return Err(NameProblem::ControlCharacter);
        }
        if name.starts_with('.') {
            return Err(NameProblem::LeadingDot);
        }
        if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
            return Err(NameProblem::EdgeWhitespace);
        }
        // APFS allows 255 bytes of UTF-8; mkdir reports any stricter limit.
        if name.len() > 255 {
            return Err(NameProblem::TooLong);
        }
        if is_package_name(name) {
            return Err(NameProblem::PackageExtension);
        }
        if is_project_marker_name(name) {
            return Err(NameProblem::ProjectMarker);
        }
        Ok(NewFolderName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Names already on disk are kept exactly as they are, but must still be one
/// single name, so they cannot change the meaning of a path.
fn check_existing_name(name: &str) -> Result<(), FsError> {
    if name.is_empty() {
        return Err(FsError::InvalidName(NameProblem::Empty));
    }
    if name == "." || name == ".." {
        return Err(FsError::InvalidName(NameProblem::DotOrDotDot));
    }
    if name.contains('/') {
        return Err(FsError::InvalidName(NameProblem::PathSeparator));
    }
    if name.contains('\0') {
        return Err(FsError::InvalidName(NameProblem::ControlCharacter));
    }
    Ok(())
}

pub fn is_package_name(name: &str) -> bool {
    has_extension_in(name, PACKAGE_EXTENSIONS)
}

/// True when APFS would take `name` for one of the project marker names
/// (#58): "CARGO.TOML", or "pacKage.json" with the Kelvin sign, is
/// "Cargo.toml" or "package.json" there. Only names that do not start with
/// "." are left to check here; the others are already hidden names.
fn is_project_marker_name(name: &str) -> bool {
    let folded = fold_name(name);
    PROJECT_MARKER_NAMES
        .iter()
        .any(|marker| fold_name(marker) == folded)
}

/// A name or path spelled the one way APFS compares names (#55). A
/// case-insensitive APFS volume (the macOS default) treats two names as the
/// same when they differ only in how accents are written ("é" as one
/// character, or "e" plus a combining accent) or in case, including "ß" and
/// "ss", "ﬁ" and "fi", "µ" and "μ". So: split accents off (NFD), turn every
/// letter to lower case, upper case and lower case again (so all case forms
/// of a letter end the same), then NFD again. With today's Unicode tables
/// the last step changes nothing; it is there because Unicode defines
/// canonical caseless matching this way, so newer tables cannot break it.
///
/// It must never keep apart two names APFS treats as the same: that would
/// let a marker-named folder through (#58) or skip a wait for an unsettled
/// action (#55). It may join a few more names than the volume does (for
/// example "ı" and "i", or two case forms on a case-sensitive APFS volume);
/// that only refuses a name or makes Traeva wait.
pub fn fold_name(text: &str) -> String {
    let nfd = DecomposingNormalizerBorrowed::new_nfd();
    let folded: String = nfd
        .normalize(text)
        .chars()
        .flat_map(char::to_lowercase)
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    nfd.normalize(&folded).into_owned()
}

fn has_extension_in(name: &str, extensions: &[&str]) -> bool {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extensions
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Volume, file and folder rules
// ---------------------------------------------------------------------------

/// The first release supports local, writable APFS only (organize-spec.md §4.2).
pub fn check_volume(fs_type: &str, mount_flags: u32) -> Result<(), FsError> {
    if fs_type != "apfs" {
        return Err(FsError::UnsupportedVolume(format!(
            "{fs_type} volume; Traeva needs local APFS"
        )));
    }
    if mount_flags & libc::MNT_LOCAL as u32 == 0 {
        return Err(FsError::UnsupportedVolume(
            "network or non-local volume".to_string(),
        ));
    }
    if mount_flags & libc::MNT_RDONLY as u32 != 0 {
        return Err(FsError::UnsupportedVolume("read-only volume".to_string()));
    }
    Ok(())
}

fn volume_of(fd: BorrowedFd<'_>) -> Result<String, FsError> {
    let stats = rfs::fstatfs(fd).map_err(from_errno)?;
    let name: Vec<u8> = stats
        .f_fstypename
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let fs_type = String::from_utf8_lossy(&name).into_owned();
    check_volume(&fs_type, stats.f_flags)?;
    Ok(fs_type)
}

/// The UUID of the mounted volume with device number `dev` (`st_dev`), as 32
/// lowercase hex characters; None when it cannot be read. Call it with a
/// device number just observed, never with a saved one.
///
/// Why saved evidence needs it: macOS numbers volumes when it mounts them,
/// so after a restart the same volume can have another `st_dev` (seen on
/// 10/15: 16777232 became 16777234). The UUID is stored on the volume
/// itself and stays the same. A `dev` is only valid while its volume stays
/// mounted.
///
/// Nothing is cached: after a disk is ejected, the next disk mounted can get
/// the same `st_dev` (seen on this Mac on 10/15), so a cache by `dev` could
/// save the old disk's UUID. Each call is one `getfsstat` and one
/// `getattrlist`.
pub fn volume_uuid(dev: i64) -> Option<String> {
    mount_point_of(dev).and_then(|path| read_volume_uuid(&path))
}

/// The mount point of the volume with device number `dev`. On macOS a
/// volume's file system ID (`f_fsid.val[0]`) is its device number.
fn mount_point_of(dev: i64) -> Option<CString> {
    // SAFETY: with no buffer, getfsstat only counts the mounted volumes.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    // A little room for a volume mounted in between.
    let room = usize::try_from(count).ok()? + 4;
    // SAFETY: statfs is plain data, for which all zeros is a valid value.
    let mut mounts: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; room];
    let size = c_int::try_from(room * std::mem::size_of::<libc::statfs>()).ok()?;
    // SAFETY: `size` is the size in bytes of `mounts`.
    let filled = unsafe { libc::getfsstat(mounts.as_mut_ptr(), size, libc::MNT_NOWAIT) };
    mounts.truncate(usize::try_from(filled).ok()?);
    let mount = mounts.iter().find(|mount| {
        // SAFETY: fsid_t is two i32 values; the libc crate keeps them private.
        let fsid = unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(mount.f_fsid) };
        i64::from(fsid[0]) == dev
    })?;
    let path: Vec<u8> = mount
        .f_mntonname
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    CString::new(path).ok()
}

/// What getattrlist returns for ATTR_VOL_UUID: the reply's length, then the UUID.
#[repr(C)]
struct VolumeUuidReply {
    length: u32,
    uuid: [u8; 16],
}

/// The UUID of the volume that holds `path`, read with getattrlist.
pub(crate) fn read_volume_uuid(path: &CStr) -> Option<String> {
    // SAFETY: attrlist is plain data, for which all zeros is a valid value.
    let mut request: libc::attrlist = unsafe { std::mem::zeroed() };
    request.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    // ATTR_VOL_INFO must come with any volume attribute; it returns nothing.
    request.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID;
    let mut reply = VolumeUuidReply {
        length: 0,
        uuid: [0; 16],
    };
    // SAFETY: both pointers point at live values, `reply` of the given size.
    let done = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            std::ptr::from_mut(&mut request).cast(),
            std::ptr::from_mut(&mut reply).cast(),
            std::mem::size_of::<VolumeUuidReply>(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    let complete = reply.length as usize == std::mem::size_of::<VolumeUuidReply>();
    if done != 0 || !complete || reply.uuid == [0; 16] {
        return None;
    }
    Some(
        reply
            .uuid
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// The first release moves only plain, local, single-link files that are not
/// hidden (organize-spec.md §4.2).
pub fn check_movable_file(
    name: &str,
    observed: &Observation,
    root_dev: i64,
) -> Result<(), FsError> {
    if name.starts_with('.') {
        return Err(FsError::Protected(Protection::Hidden));
    }
    match observed.kind {
        Kind::File => {}
        Kind::SymbolicLink => return Err(FsError::SymbolicLink),
        Kind::Directory => return Err(FsError::NotSupportedFile("folder")),
        Kind::Other => return Err(FsError::NotSupportedFile("special file")),
    }
    if observed.link_count != 1 {
        return Err(FsError::NotSupportedFile("file with several hard links"));
    }
    if observed.id.dev != root_dev {
        return Err(FsError::CrossVolume);
    }
    if observed.is_dataless() {
        return Err(FsError::Dataless);
    }
    Ok(())
}

/// Returns the first project marker found in the folder at `relative`
/// (".", "..", "../..", ...) below `dir`. Only names are looked at; no file is
/// opened or read.
fn find_project_marker(dir: BorrowedFd<'_>, relative: &str) -> Result<Option<String>, FsError> {
    for marker in PROJECT_MARKER_NAMES {
        let path = format!("{relative}/{marker}");
        match rfs::statat(dir, path.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Ok(Some(marker.to_string())),
            Err(Errno::NOENT) => {}
            Err(errno) => return Err(from_errno(errno)),
        }
    }
    // Xcode projects are folders named *.xcodeproj or *.xcworkspace: list the names.
    let listing = rfs::openat(
        dir,
        relative,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(from_errno)?;
    for entry in rfs::Dir::read_from(&listing).map_err(from_errno)? {
        let entry = entry.map_err(from_errno)?;
        let name = String::from_utf8_lossy(entry.file_name().to_bytes()).into_owned();
        if has_extension_in(&name, PROJECT_MARKER_EXTENSIONS) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// The identities of `dir` and of every folder above it, up to `/`.
/// Walks with "..", "../..", ... from the same handle, so path strings and
/// symbolic links play no part. Fails if any parent cannot be looked at.
pub fn ancestor_ids(dir: BorrowedFd<'_>) -> Result<Vec<ObjectId>, FsError> {
    Ok(ancestors(dir)?.iter().map(|seen| seen.id).collect())
}

/// `ancestor_ids` with the whole observation of each folder (its birth time
/// too), for comparing with evidence saved earlier.
pub fn ancestors(dir: BorrowedFd<'_>) -> Result<Vec<Observation>, FsError> {
    let mut chain = vec![Observation::from_stat(
        &rfs::fstat(dir).map_err(from_errno)?,
    )];
    let mut relative = String::from("..");
    for _ in 0..MAX_ANCESTORS {
        let parent = Observation::from_stat(
            &rfs::statat(dir, relative.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(from_errno)?,
        );
        if chain.last().map(|last| last.id) == Some(parent.id) {
            return Ok(chain); // `/` is its own parent.
        }
        chain.push(parent);
        relative.push_str("/..");
    }
    Err(FsError::Other(libc::ELOOP))
}

fn up(levels: usize) -> String {
    if levels == 0 {
        ".".to_string()
    } else {
        vec![".."; levels].join("/")
    }
}

/// What Traeva records about an authorized root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootFacts {
    /// The folder's actual path as the kernel reports it, for display and reopening.
    pub real_path: String,
    pub id: ObjectId,
    pub birthtime_ns: i64,
    pub fs_type: String,
}

/// All checks for a folder the user wants to authorize, run again every time
/// the root is used. `library_folders` are the folders that hold app data
/// (`~/Library` and `/Library`).
pub fn inspect_root(
    root: &Folder,
    app_data: &Folder,
    library_folders: &[PathBuf],
) -> Result<RootFacts, FsError> {
    let observed = root.observation();
    if observed.kind != Kind::Directory {
        return Err(FsError::NotADirectory);
    }
    if observed.is_dataless() {
        return Err(FsError::Dataless);
    }
    let fs_type = volume_of(root.fd())?;

    let real_path = rfs::getpath(root.fd())
        .map_err(from_errno)?
        .into_string()
        .map_err(|_| FsError::InvalidName(NameProblem::ControlCharacter))?;
    for component in Path::new(&real_path).components() {
        if let Component::Normal(name) = component {
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                return Err(FsError::Protected(Protection::Hidden));
            }
            if is_package_name(&name) {
                return Err(FsError::Protected(Protection::Package(name.into_owned())));
            }
        }
    }

    // App-data isolation, by identity: neither folder may be inside the other.
    let root_chain =
        ancestor_ids(root.fd()).map_err(|_| FsError::OverlapsAppData(Overlap::CannotTell))?;
    let app_data_chain =
        ancestor_ids(app_data.fd()).map_err(|_| FsError::OverlapsAppData(Overlap::CannotTell))?;
    if root_chain.contains(&app_data.id()) {
        return Err(FsError::OverlapsAppData(Overlap::RootInsideAppData));
    }
    if app_data_chain.contains(&root.id()) {
        return Err(FsError::OverlapsAppData(Overlap::RootContainsAppData));
    }

    for library in library_folders {
        if let Ok(st) = rfs::stat(library)
            && root_chain.contains(&Observation::from_stat(&st).id)
        {
            return Err(FsError::Protected(Protection::AppData));
        }
    }

    // A root may not be, or be inside, a project folder. Ancestors that cannot
    // be read are skipped; problems reading the root itself are not.
    for level in 0..root_chain.len() {
        match find_project_marker(root.fd(), &up(level)) {
            Ok(Some(marker)) => return Err(FsError::Protected(Protection::Project(marker))),
            Ok(None) => {}
            Err(error) if level == 0 => return Err(error),
            Err(_) => {}
        }
    }

    Ok(RootFacts {
        real_path,
        id: root.id(),
        birthtime_ns: observed.birthtime_ns,
        fs_type,
    })
}

// ---------------------------------------------------------------------------
// Walking folders inside a root
// ---------------------------------------------------------------------------

/// Opens `names` one at a time below `root`, never following symbolic links,
/// and checks every folder on the way. Returns one handle per name so the
/// caller can re-check the whole chain later with `verify_chain`.
pub fn open_chain(root: &Folder, names: &[&str]) -> Result<Vec<Folder>, FsError> {
    let mut chain: Vec<Folder> = Vec::new();
    for name in names {
        let parent = chain.last().unwrap_or(root);
        let folder = open_child_folder(parent, name, root.id().dev)?;
        chain.push(folder);
    }
    Ok(chain)
}

fn open_child_folder(parent: &Folder, name: &str, root_dev: i64) -> Result<Folder, FsError> {
    check_existing_name(name)?;
    if name.starts_with('.') {
        return Err(FsError::Protected(Protection::Hidden));
    }
    if is_package_name(name) {
        return Err(FsError::Protected(Protection::Package(name.to_string())));
    }
    let fd = match rfs::openat(parent.fd(), name, FOLDER_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        // macOS reports a symbolic link as "not a directory" here; find out which.
        Err(Errno::NOTDIR | Errno::LOOP) => {
            return match observe(parent, name) {
                Ok(o) if o.kind == Kind::SymbolicLink => Err(FsError::SymbolicLink),
                _ => Err(FsError::NotADirectory),
            };
        }
        Err(errno) => return Err(from_errno(errno)),
    };
    let observed = Observation::from_stat(&rfs::fstat(&fd).map_err(from_errno)?);
    if observed.id.dev != root_dev {
        return Err(FsError::CrossVolume);
    }
    if observed.is_dataless() {
        return Err(FsError::Dataless);
    }
    if let Some(marker) = find_project_marker(fd.as_fd(), ".")? {
        return Err(FsError::Protected(Protection::Project(marker)));
    }
    Ok(Folder {
        fd,
        observed,
        name: name.to_string(),
    })
}

/// A folder found directly inside another one, for the folder catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildFolder {
    pub name: String,
    pub id: ObjectId,
    /// Set for project folders: shown, but never a target and never entered.
    pub protected: Option<Protection>,
}

/// The folders directly inside `parent`, sorted by name. Only names and folder
/// attributes are read. Left out completely: files, symbolic links, hidden
/// folders, packages, other volumes, cloud placeholders and folders Traeva
/// cannot open. Project folders are listed as protected.
pub fn list_child_folders(parent: &Folder, root_dev: i64) -> Result<Vec<ChildFolder>, FsError> {
    let mut children = Vec::new();
    for entry in rfs::Dir::read_from(parent.fd()).map_err(from_errno)? {
        let entry = entry.map_err(from_errno)?;
        let Ok(name) = entry.file_name().to_str() else {
            continue; // APFS names are UTF-8; skip anything else rather than guess.
        };
        if name == "." || name == ".." || name.starts_with('.') || is_package_name(name) {
            continue;
        }
        let is_folder = match entry.file_type() {
            FileType::Directory => true,
            FileType::Unknown => {
                matches!(observe(parent, name), Ok(o) if o.kind == Kind::Directory)
            }
            _ => false,
        };
        if !is_folder {
            continue;
        }
        match open_child_folder(parent, name, root_dev) {
            Ok(folder) => children.push(ChildFolder {
                name: name.to_string(),
                id: folder.id(),
                protected: None,
            }),
            Err(FsError::Protected(why @ Protection::Project(_))) => {
                if let Ok(observed) = observe(parent, name) {
                    children.push(ChildFolder {
                        name: name.to_string(),
                        id: observed.id,
                        protected: Some(why),
                    });
                }
            }
            Err(_) => {}
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(children)
}

/// The plain files directly inside `parent`, sorted by name, for Find
/// (#198). Only names and attributes are read, never content. Left out:
/// folders, symbolic links, special files, hidden names and packages.
/// Cloud placeholders are listed; their `flags` say so.
pub fn list_child_files(parent: &Folder) -> Result<Vec<(String, Observation)>, FsError> {
    let mut files = Vec::new();
    for entry in rfs::Dir::read_from(parent.fd()).map_err(from_errno)? {
        let entry = entry.map_err(from_errno)?;
        let Ok(name) = entry.file_name().to_str() else {
            continue; // APFS names are UTF-8; skip anything else rather than guess.
        };
        if name == "." || name == ".." || name.starts_with('.') || is_package_name(name) {
            continue;
        }
        if !matches!(entry.file_type(), FileType::RegularFile | FileType::Unknown) {
            continue;
        }
        if let Ok(observed) = observe(parent, name)
            && observed.kind == Kind::File
        {
            files.push((name.to_string(), observed));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Checks that every folder in `chain` is still the object at its name,
/// starting again from `root`. Renamed, replaced, moved out of the root or
/// turned into a link: all of these are `ChainChanged`.
pub fn verify_chain(root: &Folder, chain: &[Folder]) -> Result<(), FsError> {
    let mut parent = root;
    for folder in chain {
        match rfs::statat(parent.fd(), folder.name(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => {
                let now = Observation::from_stat(&st);
                if now.kind != Kind::Directory || now.id != folder.id() {
                    return Err(FsError::ChainChanged);
                }
            }
            Err(Errno::NOENT | Errno::NOTDIR) => return Err(FsError::ChainChanged),
            Err(errno) => return Err(from_errno(errno)),
        }
        parent = folder;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The only three writes: create a folder, move without replacing, and remove
// an empty folder Traeva created
// ---------------------------------------------------------------------------

/// Creates folder `name` inside `parent` and returns a handle to it. If
/// anything already uses that name (any case or Unicode spelling), the kernel
/// refuses with `AlreadyExists`, so an existing folder is never claimed.
///
/// Remaining race: between mkdir and opening the new folder another program
/// could swap it. Traeva cannot rule that out, so later checks still compare
/// the recorded identity before trusting the folder.
pub fn create_folder(parent: &Folder, name: &NewFolderName) -> Result<Folder, FsError> {
    rfs::mkdirat(parent.fd(), name.as_str(), Mode::from_raw_mode(0o777)).map_err(from_errno)?;
    // From here on the folder exists: any problem means "changed, but unconfirmed".
    let fd = rfs::openat(parent.fd(), name.as_str(), FOLDER_FLAGS, Mode::empty())
        .map_err(|_| FsError::ChangedButUnconfirmed)?;
    let observed =
        Observation::from_stat(&rfs::fstat(&fd).map_err(|_| FsError::ChangedButUnconfirmed)?);
    if observed.kind != Kind::Directory || observed.id.dev != parent.id().dev {
        return Err(FsError::ChangedButUnconfirmed);
    }
    // Make the new directory entry durable before anyone records it.
    rfs::fcntl_fullfsync(parent.fd()).map_err(|_| FsError::ChangedButUnconfirmed)?;
    Ok(Folder {
        fd,
        observed,
        name: name.as_str().to_string(),
    })
}

/// Moves `from_name` in `from` to `to_name` in `to` in one atomic step that
/// never replaces anything: `renameatx_np(..., RENAME_EXCL)`. If the target
/// name is taken at that moment, the kernel refuses with `AlreadyExists` and
/// nothing moves. There is no fallback to a plain rename or to copy-and-delete.
pub fn move_no_replace(
    from: &Folder,
    from_name: &str,
    to: &Folder,
    to_name: &str,
) -> Result<(), FsError> {
    check_existing_name(from_name)?;
    check_existing_name(to_name)?;
    rfs::renameat_with(
        from.fd(),
        from_name,
        to.fd(),
        to_name,
        RenameFlags::NOREPLACE,
    )
    .map_err(from_errno)?;
    // The move happened. Make both directory entries durable.
    let synced = rfs::fcntl_fullfsync(to.fd()).and_then(|()| {
        if from.id() == to.id() {
            Ok(())
        } else {
            rfs::fcntl_fullfsync(from.fd())
        }
    });
    synced.map_err(|_| FsError::ChangedButUnconfirmed)
}

/// True when the folder has no entries at all. A `.DS_Store` counts as an
/// entry, so a folder holding only one is not empty.
pub fn is_empty_folder(folder: &Folder) -> Result<bool, FsError> {
    for entry in rfs::Dir::read_from(folder.fd()).map_err(from_errno)? {
        let entry = entry.map_err(from_errno)?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Removes folder `name` inside `parent`, only if that name is still the
/// folder `expected` and the folder is empty. The kernel's rmdir refuses a
/// folder that is not empty (`NotEmpty`), so something added at the last
/// moment is never deleted, and nothing is ever removed recursively.
///
/// Remaining race: between the identity check and rmdir another program
/// could swap in a different, empty folder of the same name. Only an empty
/// folder can be removed, so no file can be lost that way.
pub fn remove_empty_folder(parent: &Folder, name: &str, expected: ObjectId) -> Result<(), FsError> {
    check_existing_name(name)?;
    match rfs::statat(parent.fd(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => {
            let now = Observation::from_stat(&st);
            if now.kind != Kind::Directory || now.id != expected {
                return Err(FsError::ChainChanged);
            }
        }
        Err(Errno::NOENT) => return Err(FsError::ChainChanged),
        Err(errno) => return Err(from_errno(errno)),
    }
    rfs::unlinkat(parent.fd(), name, AtFlags::REMOVEDIR).map_err(from_errno)?;
    // The folder is gone. Make the removal durable.
    rfs::fcntl_fullfsync(parent.fd()).map_err(|_| FsError::ChangedButUnconfirmed)
}

/// After a move: the source name must be gone and the target name must be the
/// very file that was moved (same device and inode, still a plain file).
/// Anything else means the result is uncertain. Returns what the target name
/// shows now, so a caller that has read the file can check that it is still
/// the version it read.
pub fn confirm_moved(
    from: &Folder,
    from_name: &str,
    to: &Folder,
    to_name: &str,
    moved: ObjectId,
) -> Result<Observation, FsError> {
    match observe(from, from_name) {
        Err(FsError::NotFound) => {}
        _ => return Err(FsError::ChangedButUnconfirmed),
    }
    match observe(to, to_name) {
        Ok(target) if target.kind == Kind::File && target.id == moved => Ok(target),
        _ => Err(FsError::ChangedButUnconfirmed),
    }
}
