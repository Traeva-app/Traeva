//! Folder-tree snapshots for zero-write checks (organize-spec.md §2.2).
//!
//! A snapshot records everything Traeva could change: paths, types, file bytes,
//! permissions, extended attributes, modification and change times. Access
//! times are left out on purpose: reading a file may update them.
//!
//! Used by integration tests and, through `#[path]`, by the platform unit tests.

#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: &'static str,
    pub mode: u32,
    pub bytes: Option<Vec<u8>>,
    pub link_target: Option<PathBuf>,
    pub mtime_ns: i128,
    pub ctime_ns: i128,
    pub xattrs: Vec<(String, Vec<u8>)>,
}

/// Every entry below (and including) `root`, sorted by path. Symbolic links
/// are recorded, never followed.
pub fn snapshot(root: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    collect(root, Path::new(""), &mut entries);
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

fn collect(root: &Path, relative: &Path, out: &mut Vec<Entry>) {
    let full = root.join(relative);
    let meta = fs::symlink_metadata(&full).unwrap();
    let file_type = meta.file_type();
    let kind = if file_type.is_dir() {
        "dir"
    } else if file_type.is_file() {
        "file"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    };
    let mut xattrs: Vec<(String, Vec<u8>)> = xattr::list(&full)
        .unwrap()
        .map(|name| {
            let value = xattr::get(&full, &name).unwrap().unwrap_or_default();
            (name.to_string_lossy().into_owned(), value)
        })
        .collect();
    xattrs.sort();
    out.push(Entry {
        path: relative.to_path_buf(),
        kind,
        mode: meta.mode(),
        bytes: file_type.is_file().then(|| fs::read(&full).unwrap()),
        link_target: file_type
            .is_symlink()
            .then(|| fs::read_link(&full).unwrap()),
        mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
        ctime_ns: i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
        xattrs,
    });
    if file_type.is_dir() {
        for child in fs::read_dir(&full).unwrap() {
            collect(root, &relative.join(child.unwrap().file_name()), out);
        }
    }
}

/// A small synthetic tree with the kinds of things a real folder has:
/// nested folders, text files, a symbolic link and an extended attribute.
pub fn fill_sample_tree(root: &Path) {
    fs::create_dir_all(root.join("Notes/2026")).unwrap();
    fs::write(root.join("todo.txt"), "buy milk\n").unwrap();
    fs::write(root.join("Notes/2026/plan.md"), "# Plan\n\n- ship M1\n").unwrap();
    std::os::unix::fs::symlink("Notes/2026/plan.md", root.join("plan-link.md")).unwrap();
    xattr::set(root.join("todo.txt"), "com.traeva.test", b"synthetic").unwrap();
}
