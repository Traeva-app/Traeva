//! Temporary disk images for volume tests: an HFS+ volume (not APFS) and a
//! second APFS volume (another device). Mounted with `-nobrowse` so Finder
//! does not show them, and detached when the value is dropped.
//!
//! Tests that use this are opt-in: `cargo test -p traeva-core -- --ignored`.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct MountedImage {
    pub mount_point: PathBuf,
    _image_dir: tempfile::TempDir,
}

impl MountedImage {
    /// Creates a `size` image with filesystem `fs` ("HFS+" or "APFS") and
    /// mounts it at `mount_point`, which must be an empty folder.
    pub fn mount_at(mount_point: &Path, fs: &str, size: &str) -> MountedImage {
        let image_dir = tempfile::Builder::new()
            .prefix("traeva-image-")
            .tempdir()
            .unwrap();
        let image = image_dir.path().join("volume.dmg");
        run(&[
            "create",
            "-quiet",
            "-size",
            size,
            "-fs",
            fs,
            "-volname",
            "TraevaTest",
            image.to_str().unwrap(),
        ]);
        run(&[
            "attach",
            "-quiet",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
            mount_point.to_str().unwrap(),
            image.to_str().unwrap(),
        ]);
        MountedImage {
            mount_point: mount_point.to_path_buf(),
            _image_dir: image_dir,
        }
    }

    /// Like `mount_at`, with a fresh empty mount folder in `parent`.
    pub fn mount_in(parent: &Path, name: &str, fs: &str, size: &str) -> MountedImage {
        let mount_point = parent.join(name);
        fs::create_dir(&mount_point).unwrap();
        MountedImage::mount_at(&mount_point, fs, size)
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&self.mount_point)
            .status();
    }
}

fn run(args: &[&str]) {
    let status = Command::new("hdiutil").args(args).status().unwrap();
    assert!(status.success(), "hdiutil {args:?} failed");
}
