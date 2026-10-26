//! Tests for the filesystem primitives, on real temporary folders only.
//! Nothing here touches the real app data or any personal file.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::macos::*;

use crate::test_snapshot as snapshot;

#[path = "../../tests/common/disk_image.rs"]
mod disk_image;

/// A fresh temporary folder (no leading dot in its name) and an open handle to it.
fn temp_root() -> (tempfile::TempDir, Folder) {
    let dir = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let folder = open_folder_path(dir.path()).unwrap();
    (dir, folder)
}

fn names(chain: &[Folder]) -> Vec<&str> {
    chain.iter().map(Folder::name).collect()
}

/// Whether this volume treats `existing` and `variant` as the same name
/// (APFS is usually case- and normalization-insensitive, but not always).
fn same_name_on_disk(folder: &Path, variant: &str) -> bool {
    fs::symlink_metadata(folder.join(variant)).is_ok()
}

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

#[test]
fn new_folder_names_follow_the_spec() {
    for good in [
        "Notes",
        "2026 Q1",
        "报告",
        "Café",
        "Taxes (old)",
        "🦀 Rust",
        &"a".repeat(255),
    ] {
        assert!(
            NewFolderName::parse(good).is_ok(),
            "{good:?} should be allowed"
        );
    }
    let bad = [
        ("", NameProblem::Empty),
        (".", NameProblem::DotOrDotDot),
        ("..", NameProblem::DotOrDotDot),
        ("a/b", NameProblem::PathSeparator),
        ("/abs", NameProblem::PathSeparator),
        ("a\\b", NameProblem::PathSeparator),
        ("a:b", NameProblem::Colon),
        ("a\u{0}b", NameProblem::ControlCharacter),
        ("line\nbreak", NameProblem::ControlCharacter),
        ("\u{7f}", NameProblem::ControlCharacter),
        (".hidden", NameProblem::LeadingDot),
        (" leading", NameProblem::EdgeWhitespace),
        ("trailing ", NameProblem::EdgeWhitespace),
        ("\u{3000}ideographic", NameProblem::EdgeWhitespace),
        ("Taxes.app", NameProblem::PackageExtension),
        ("Album.photoslibrary", NameProblem::PackageExtension),
    ];
    for (name, problem) in bad {
        assert_eq!(NewFolderName::parse(name), Err(problem), "{name:?}");
    }
    assert_eq!(
        NewFolderName::parse(&"a".repeat(256)),
        Err(NameProblem::TooLong)
    );
    // 86 × 3-byte characters = 258 bytes: counted in UTF-8 bytes, not characters.
    assert_eq!(
        NewFolderName::parse(&"中".repeat(86)),
        Err(NameProblem::TooLong)
    );
}

#[test]
fn project_marker_names_cannot_be_new_folders() {
    // All but the last are the marker itself on a case-insensitive APFS
    // volume, the macOS default (checked on this Mac's volume for the Kelvin
    // sign, the long s and the two "st" ligatures).
    for marker in [
        "package.json",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "CMakeLists.txt",
        "CARGO.TOML",
        "cargo.toml",
        "Package.JSON",
        "pac\u{212A}age.json",  // KELVIN SIGN
        "package.j\u{17F}on",   // LATIN SMALL LETTER LONG S
        "CMakeLi\u{FB06}s.txt", // LATIN SMALL LIGATURE ST
        "CMakeLi\u{FB05}s.txt", // LATIN SMALL LIGATURE LONG S T
        "CMakeL\u{131}sts.txt", // dotless i: APFS keeps it apart; refusing is harmless
    ] {
        assert_eq!(
            NewFolderName::parse(marker),
            Err(NameProblem::ProjectMarker),
            "{marker:?}"
        );
    }
    for plain in [
        "Cargo",
        "package",
        "Cargo.toml notes",
        "package.json.bak",
        "build.gradle.kts",
        "Cargo\u{FF0E}toml", // FULLWIDTH FULL STOP: another name on APFS
    ] {
        assert!(
            NewFolderName::parse(plain).is_ok(),
            "{plain:?} should be allowed"
        );
    }
    // Xcode projects are refused as packages; a folder with this extension
    // would be a project marker too.
    for xcode in ["App.xcodeproj", "Work.xcworkspace"] {
        assert_eq!(
            NewFolderName::parse(xcode),
            Err(NameProblem::PackageExtension)
        );
    }
}

#[test]
fn folding_matches_names_the_way_apfs_does() {
    assert_eq!(fold_name("Café"), fold_name("CAFE\u{301}"));
    assert_eq!(fold_name("Straße"), fold_name("STRASSE"));
    // APFS takes these two orders of the same accents as one name (checked
    // on this Mac's volume). Only the first decomposition puts them in one
    // order, before U+0345 turns into a letter.
    assert_eq!(fold_name("x\u{345}\u{301}"), fold_name("x\u{301}\u{345}"));
    assert_ne!(fold_name("Cargo.toml"), fold_name("Cargo.tom"));
}

#[test]
fn package_names_are_recognized_by_extension() {
    for package in [
        "Foo.app",
        "Photos Library.photoslibrary",
        "Notes.RTFD",
        "App.xcodeproj",
    ] {
        assert!(is_package_name(package), "{package}");
    }
    for plain in [".app", "app", "My.Folder", "report.pages.txt", "Photos"] {
        assert!(!is_package_name(plain), "{plain}");
    }
}

#[test]
fn only_local_writable_apfs_is_supported() {
    let local = libc::MNT_LOCAL as u32;
    let read_only = libc::MNT_RDONLY as u32;
    assert_eq!(check_volume("apfs", local), Ok(()));
    assert!(matches!(
        check_volume("hfs", local),
        Err(FsError::UnsupportedVolume(_))
    ));
    assert!(matches!(
        check_volume("msdos", local),
        Err(FsError::UnsupportedVolume(_))
    ));
    assert!(matches!(
        check_volume("apfs", 0),
        Err(FsError::UnsupportedVolume(_))
    ));
    assert!(matches!(
        check_volume("smbfs", 0),
        Err(FsError::UnsupportedVolume(_))
    ));
    assert!(matches!(
        check_volume("apfs", local | read_only),
        Err(FsError::UnsupportedVolume(_))
    ));
}

#[test]
fn cloud_placeholders_are_refused() {
    // A real SF_DATALESS file needs iCloud or a File Provider, so the flag is
    // set by hand here. Only the kernel can set it on a real file.
    assert!(is_dataless_flags(0x4000_0000));
    assert!(!is_dataless_flags(0));
    let (dir, root) = temp_root();
    fs::write(dir.path().join("doc.txt"), "text").unwrap();
    let mut observed = observe(&root, "doc.txt").unwrap();
    observed.flags |= 0x4000_0000;
    assert_eq!(
        check_movable_file("doc.txt", &observed, root.id().dev),
        Err(FsError::Dataless)
    );
}

// ---------------------------------------------------------------------------
// Walking folders: descriptor-anchored, one name at a time, no symbolic links
// ---------------------------------------------------------------------------

#[test]
fn walk_opens_each_level_and_the_chain_verifies() {
    let (dir, root) = temp_root();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    let chain = open_chain(&root, &["a", "b"]).unwrap();
    assert_eq!(names(&chain), ["a", "b"]);
    assert_eq!(verify_chain(&root, &chain), Ok(()));
}

#[test]
fn walk_never_follows_symbolic_links() {
    let (dir, root) = temp_root();
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    fs::create_dir_all(dir.path().join("a/real")).unwrap();
    symlink(dir.path().join("a/real"), dir.path().join("inside-link")).unwrap();
    symlink(outside.path(), dir.path().join("outside-link")).unwrap();
    symlink(outside.path(), dir.path().join("a/nested-link")).unwrap();

    assert_eq!(
        open_chain(&root, &["inside-link"]).unwrap_err(),
        FsError::SymbolicLink
    );
    assert_eq!(
        open_chain(&root, &["outside-link"]).unwrap_err(),
        FsError::SymbolicLink
    );
    assert_eq!(
        open_chain(&root, &["a", "nested-link"]).unwrap_err(),
        FsError::SymbolicLink
    );
}

#[test]
fn walk_takes_single_names_only() {
    let (dir, root) = temp_root();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    assert_eq!(
        open_chain(&root, &[".."]).unwrap_err(),
        FsError::InvalidName(NameProblem::DotOrDotDot)
    );
    assert_eq!(
        open_chain(&root, &["a/b"]).unwrap_err(),
        FsError::InvalidName(NameProblem::PathSeparator)
    );
    assert_eq!(
        open_chain(&root, &[""]).unwrap_err(),
        FsError::InvalidName(NameProblem::Empty)
    );
}

#[test]
fn walk_refuses_hidden_package_project_and_non_folders() {
    let (dir, root) = temp_root();
    let base = dir.path();
    fs::create_dir(base.join(".config")).unwrap();
    fs::create_dir(base.join("Album.photoslibrary")).unwrap();
    fs::create_dir(base.join("web")).unwrap();
    fs::write(base.join("web/package.json"), "{}").unwrap();
    fs::create_dir_all(base.join("repo/.git")).unwrap();
    fs::create_dir_all(base.join("ios/App.xcodeproj")).unwrap();
    fs::write(base.join("notes.txt"), "text").unwrap();

    let refused = |name: &str| open_chain(&root, &[name]).unwrap_err();
    assert_eq!(refused(".config"), FsError::Protected(Protection::Hidden));
    assert_eq!(
        refused("Album.photoslibrary"),
        FsError::Protected(Protection::Package("Album.photoslibrary".into()))
    );
    assert_eq!(
        refused("web"),
        FsError::Protected(Protection::Project("package.json".into()))
    );
    assert_eq!(
        refused("repo"),
        FsError::Protected(Protection::Project(".git".into()))
    );
    assert_eq!(
        refused("ios"),
        FsError::Protected(Protection::Project("App.xcodeproj".into()))
    );
    assert_eq!(refused("notes.txt"), FsError::NotADirectory);
    assert_eq!(refused("missing"), FsError::NotFound);
}

/// Every project marker of organize-spec.md §4.2, put on disk one at a time
/// (a file or a folder, as each tool makes it): the folder holding it is
/// refused (O12).
#[test]
fn every_project_marker_on_disk_makes_its_folder_a_project() {
    let (dir, root) = temp_root();
    let base = dir.path();
    let files = [
        ".git", // a worktree or submodule has a .git file
        "package.json",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "CMakeLists.txt",
    ];
    let folders = [".git", ".hg", ".svn", "App.xcodeproj", "App.xcworkspace"];
    let mut cases = Vec::new();
    for (n, marker) in files.iter().enumerate() {
        let folder = format!("file-{n}");
        fs::create_dir(base.join(&folder)).unwrap();
        fs::write(base.join(&folder).join(marker), "synthetic").unwrap();
        cases.push((folder, marker.to_string()));
    }
    for (n, marker) in folders.iter().enumerate() {
        let folder = format!("folder-{n}");
        fs::create_dir_all(base.join(&folder).join(marker)).unwrap();
        cases.push((folder, marker.to_string()));
    }
    for (folder, marker) in cases {
        assert_eq!(
            open_chain(&root, &[folder.as_str()]).unwrap_err(),
            FsError::Protected(Protection::Project(marker.clone())),
            "{folder} holds {marker}"
        );
    }
}

#[test]
fn walking_and_checking_write_nothing() {
    let (dir, root) = temp_root();
    snapshot::fill_sample_tree(dir.path());
    fs::create_dir(dir.path().join("web")).unwrap();
    fs::write(dir.path().join("web/package.json"), "{}").unwrap();
    let before = snapshot::snapshot(dir.path());

    let chain = open_chain(&root, &["Notes", "2026"]).unwrap();
    verify_chain(&root, &chain).unwrap();
    observe(&chain[1], "plan.md").unwrap();
    observe(&root, "plan-link.md").unwrap();
    let _ = open_chain(&root, &["web"]);
    let _ = open_chain(&root, &["plan-link.md"]);
    ancestor_ids(root.fd()).unwrap();

    assert_eq!(snapshot::snapshot(dir.path()), before);
}

#[test]
fn verify_chain_notices_a_replaced_folder() {
    let (dir, root) = temp_root();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    let chain = open_chain(&root, &["a", "b"]).unwrap();

    // Another program swaps in a look-alike folder with the same names.
    fs::rename(dir.path().join("a"), dir.path().join("a-old")).unwrap();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();

    assert_eq!(verify_chain(&root, &chain), Err(FsError::ChainChanged));
}

#[test]
fn verify_chain_notices_a_folder_moved_out_of_the_root() {
    let (dir, root) = temp_root();
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::write(dir.path().join("a/doc.txt"), "text").unwrap();
    let chain = open_chain(&root, &["a"]).unwrap();

    fs::rename(dir.path().join("a"), outside.path().join("a")).unwrap();

    assert_eq!(verify_chain(&root, &chain), Err(FsError::ChainChanged));
    // The handle still points at the folder itself, now outside the root.
    // That is why every change is followed by `verify_chain`.
    assert!(observe(&chain[0], "doc.txt").is_ok());
}

#[test]
fn verify_chain_notices_a_folder_turned_into_a_link() {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("a")).unwrap();
    let chain = open_chain(&root, &["a"]).unwrap();

    fs::rename(dir.path().join("a"), dir.path().join("a2")).unwrap();
    symlink(dir.path().join("a2"), dir.path().join("a")).unwrap();

    assert_eq!(verify_chain(&root, &chain), Err(FsError::ChainChanged));
}

#[test]
fn ancestor_ids_walk_up_to_the_filesystem_root() {
    let (dir, root) = temp_root();
    let chain = ancestor_ids(root.fd()).unwrap();
    let id_of = |path: &Path| {
        let folder = open_folder_path(path).unwrap();
        folder.id()
    };
    assert_eq!(chain.first(), Some(&root.id()));
    assert!(chain.contains(&id_of(dir.path().parent().unwrap())));
    assert_eq!(chain.last(), Some(&id_of(Path::new("/"))));
    let seen: Vec<ObjectId> = ancestors(root.fd()).unwrap().iter().map(|o| o.id).collect();
    assert_eq!(seen, chain, "ancestors() walks the same folders");
}

#[test]
fn the_volume_uuid_is_found_by_device_number_and_stays_the_same() {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("child")).unwrap();
    let child = open_folder_path(&dir.path().join("child")).unwrap();

    let uuid = volume_uuid(root.id().dev).expect("the temporary folder's volume has a UUID");
    assert_eq!(uuid.len(), 32);
    assert!(
        uuid.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{uuid}"
    );
    assert_eq!(volume_uuid(child.id().dev), Some(uuid.clone()), "its child");
    assert_eq!(
        volume_uuid(root.id().dev),
        Some(uuid.clone()),
        "asked again"
    );
    // The mount found by device number (f_fsid.val[0] == st_dev) is the
    // volume that holds the folder itself, asked by its own path.
    let path = CString::new(dir.path().as_os_str().as_bytes()).unwrap();
    assert_eq!(read_volume_uuid(&path), Some(uuid));
    assert_eq!(volume_uuid(-1), None, "no volume has this number");
}

// ---------------------------------------------------------------------------
// Root rules
// ---------------------------------------------------------------------------

/// A root and an app-data folder in separate temporary folders.
fn root_and_app_data() -> (tempfile::TempDir, Folder, tempfile::TempDir, Folder) {
    let (root_dir, root) = temp_root();
    let app_dir = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let app_data = open_folder_path(app_dir.path()).unwrap();
    (root_dir, root, app_dir, app_data)
}

#[test]
fn inspect_root_accepts_a_plain_local_folder() {
    let (root_dir, root, _app_dir, app_data) = root_and_app_data();
    let facts = inspect_root(&root, &app_data, &[]).unwrap();
    assert_eq!(facts.fs_type, "apfs");
    assert_eq!(facts.id, root.id());
    assert_eq!(
        PathBuf::from(&facts.real_path),
        fs::canonicalize(root_dir.path()).unwrap()
    );
}

#[test]
fn inspect_root_refuses_folders_inside_a_library() {
    let (_root_dir, _root, _app_dir, app_data) = root_and_app_data();
    let fake_home = tempfile::Builder::new()
        .prefix("traeva-home-")
        .tempdir()
        .unwrap();
    let library = fake_home.path().join("Library");
    fs::create_dir_all(library.join("Mobile Documents")).unwrap();
    let inside = open_folder_path(&library.join("Mobile Documents")).unwrap();
    assert_eq!(
        inspect_root(&inside, &app_data, &[library]),
        Err(FsError::Protected(Protection::AppData))
    );
}

#[test]
fn inspect_root_refuses_when_parents_cannot_be_checked() {
    let (root_dir, root, _app_dir, app_data) = root_and_app_data();
    // Readable but not searchable: ".." cannot be looked up from inside it.
    fs::set_permissions(root_dir.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let result = inspect_root(&root, &app_data, &[]);
    fs::set_permissions(root_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(FsError::OverlapsAppData(Overlap::CannotTell)));
}

// ---------------------------------------------------------------------------
// The two writes: create a folder, move without replacing
// ---------------------------------------------------------------------------

#[test]
fn create_folder_never_claims_an_existing_name() {
    let (dir, root) = temp_root();
    let created = create_folder(&root, &NewFolderName::parse("Taxes").unwrap()).unwrap();
    assert_eq!(created.id(), observe(&root, "Taxes").unwrap().id);
    fs::write(dir.path().join("Taxes/keep.txt"), "keep").unwrap();

    assert_eq!(
        create_folder(&root, &NewFolderName::parse("Taxes").unwrap()).unwrap_err(),
        FsError::AlreadyExists
    );
    assert_eq!(
        fs::read(dir.path().join("Taxes/keep.txt")).unwrap(),
        b"keep"
    );

    // Other spellings are decided by the volume, not by comparing strings.
    for variant in ["taxes", "TAXES"] {
        let result = create_folder(&root, &NewFolderName::parse(variant).unwrap());
        if same_name_on_disk(dir.path(), variant) {
            assert_eq!(result.unwrap_err(), FsError::AlreadyExists, "{variant}");
        } else {
            assert!(result.is_ok(), "{variant}");
        }
    }
}

#[test]
fn create_folder_respects_unicode_equivalent_names() {
    let (dir, root) = temp_root();
    create_folder(&root, &NewFolderName::parse("Caf\u{e9}").unwrap()).unwrap(); // NFC "Café"
    let nfd = "Cafe\u{301}"; // NFD "Café"
    let result = create_folder(&root, &NewFolderName::parse(nfd).unwrap());
    if same_name_on_disk(dir.path(), nfd) {
        assert_eq!(result.unwrap_err(), FsError::AlreadyExists);
    } else {
        assert!(result.is_ok());
    }
}

#[test]
fn create_folder_follows_the_handle_and_verify_catches_the_move() {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("a")).unwrap();
    let chain = open_chain(&root, &["a"]).unwrap();
    fs::rename(dir.path().join("a"), dir.path().join("z")).unwrap();

    create_folder(&chain[0], &NewFolderName::parse("child").unwrap()).unwrap();

    // The folder went into the object Traeva holds, not into whatever path "a" means now.
    assert!(dir.path().join("z/child").is_dir());
    assert!(!dir.path().join("a").exists());
    assert_eq!(verify_chain(&root, &chain), Err(FsError::ChainChanged));
}

/// root/in/doc.txt and an empty root/out, with handles for both folders.
fn move_fixture() -> (tempfile::TempDir, Folder, Folder, Folder) {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("in")).unwrap();
    fs::create_dir(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("in/doc.txt"), "original source bytes").unwrap();
    let input = open_chain(&root, &["in"]).unwrap().remove(0);
    let out = open_chain(&root, &["out"]).unwrap().remove(0);
    (dir, root, input, out)
}

#[test]
fn move_no_replace_moves_the_same_file() {
    let (dir, root, input, out) = move_fixture();
    let before = observe(&input, "doc.txt").unwrap();

    move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap();

    assert_eq!(
        fs::read(dir.path().join("out/doc.txt")).unwrap(),
        b"original source bytes"
    );
    assert!(!dir.path().join("in/doc.txt").exists());
    // It returns what is at the target now: the moved file, unchanged.
    let at_target = confirm_moved(&input, "doc.txt", &out, "doc.txt", before.id).unwrap();
    assert_eq!(at_target.id, before.id);
    assert_eq!(at_target.size, before.size);
    assert_eq!(at_target.mtime_ns, before.mtime_ns);
    assert_eq!(verify_chain(&root, &[out]), Ok(()));
}

#[test]
fn move_no_replace_refuses_a_target_that_appears_late() {
    let (dir, _root, input, out) = move_fixture();
    // Preview time: the target name is free.
    assert_eq!(observe(&out, "doc.txt").unwrap_err(), FsError::NotFound);
    // Then another program creates it before the move.
    fs::write(dir.path().join("out/doc.txt"), "someone else's bytes").unwrap();

    assert_eq!(
        move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap_err(),
        FsError::AlreadyExists
    );
    assert_eq!(
        fs::read(dir.path().join("out/doc.txt")).unwrap(),
        b"someone else's bytes"
    );
    assert_eq!(
        fs::read(dir.path().join("in/doc.txt")).unwrap(),
        b"original source bytes"
    );
}

#[test]
fn move_no_replace_refuses_other_spellings_of_a_taken_name() {
    for occupied in ["DOC.TXT", "Doc.txt"] {
        let (dir, _root, input, out) = move_fixture();
        fs::write(dir.path().join("out").join(occupied), "occupant").unwrap();
        let result = move_no_replace(&input, "doc.txt", &out, "doc.txt");
        if same_name_on_disk(&dir.path().join("out"), "doc.txt") {
            assert_eq!(result.unwrap_err(), FsError::AlreadyExists, "{occupied}");
            assert_eq!(
                fs::read(dir.path().join("out").join(occupied)).unwrap(),
                b"occupant"
            );
            assert!(dir.path().join("in/doc.txt").exists());
        } else {
            assert!(result.is_ok(), "{occupied}");
        }
    }
}

#[test]
fn move_no_replace_refuses_unicode_equivalent_target() {
    let (dir, _root, input, out) = move_fixture();
    fs::write(dir.path().join("in/Caf\u{e9}.txt"), "nfc source").unwrap();
    fs::write(dir.path().join("out/Cafe\u{301}.txt"), "nfd occupant").unwrap();
    let result = move_no_replace(&input, "Caf\u{e9}.txt", &out, "Caf\u{e9}.txt");
    if same_name_on_disk(&dir.path().join("out"), "Caf\u{e9}.txt") {
        assert_eq!(result.unwrap_err(), FsError::AlreadyExists);
        assert_eq!(
            fs::read(dir.path().join("out/Cafe\u{301}.txt")).unwrap(),
            b"nfd occupant"
        );
    } else {
        assert!(result.is_ok());
    }
}

#[test]
fn move_no_replace_refuses_links_and_folders_at_the_target() {
    let (dir, _root, input, out) = move_fixture();
    symlink("/nonexistent/target", dir.path().join("out/doc.txt")).unwrap();
    assert_eq!(
        move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap_err(),
        FsError::AlreadyExists
    );
    assert_eq!(
        fs::read_link(dir.path().join("out/doc.txt")).unwrap(),
        PathBuf::from("/nonexistent/target")
    );

    let (dir, _root, input, out) = move_fixture();
    fs::create_dir(dir.path().join("out/doc.txt")).unwrap();
    assert_eq!(
        move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap_err(),
        FsError::AlreadyExists
    );
    assert!(dir.path().join("in/doc.txt").is_file());
}

#[test]
fn move_no_replace_reports_a_missing_source() {
    let (_dir, _root, input, out) = move_fixture();
    assert_eq!(
        move_no_replace(&input, "missing.txt", &out, "missing.txt").unwrap_err(),
        FsError::NotFound
    );
}

#[test]
fn confirm_moved_rejects_anything_but_the_moved_file() {
    let (dir, _root, input, out) = move_fixture();
    let moved = observe(&input, "doc.txt").unwrap().id;
    move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap();

    // Another program replaces the moved file right after the move.
    fs::rename(
        dir.path().join("out/doc.txt"),
        dir.path().join("out/elsewhere.txt"),
    )
    .unwrap();
    fs::write(dir.path().join("out/doc.txt"), "original source bytes").unwrap();
    assert_eq!(
        confirm_moved(&input, "doc.txt", &out, "doc.txt", moved),
        Err(FsError::ChangedButUnconfirmed)
    );

    // Or a file shows up again at the source name.
    let (dir, _root, input, out) = move_fixture();
    let moved = observe(&input, "doc.txt").unwrap().id;
    move_no_replace(&input, "doc.txt", &out, "doc.txt").unwrap();
    fs::write(dir.path().join("in/doc.txt"), "new").unwrap();
    assert_eq!(
        confirm_moved(&input, "doc.txt", &out, "doc.txt", moved),
        Err(FsError::ChangedButUnconfirmed)
    );
}

#[test]
fn only_the_same_empty_folder_is_ever_removed() {
    let (dir, root) = temp_root();
    let base = dir.path();
    for name in ["Empty", "Store", "Full"] {
        fs::create_dir(base.join(name)).unwrap();
    }
    fs::write(base.join("Store/.DS_Store"), "finder").unwrap();
    fs::write(base.join("Full/a.txt"), "a").unwrap();
    let id = |name: &str| observe(&root, name).unwrap().id;
    let open = |name: &str| open_chain(&root, &[name]).unwrap().remove(0);

    assert!(is_empty_folder(&open("Empty")).unwrap());
    assert!(
        !is_empty_folder(&open("Store")).unwrap(),
        "a .DS_Store counts"
    );
    assert!(!is_empty_folder(&open("Full")).unwrap());

    // rmdir itself refuses a folder that is not empty; nothing inside is touched.
    assert_eq!(
        remove_empty_folder(&root, "Store", id("Store")),
        Err(FsError::NotEmpty)
    );
    assert!(base.join("Store/.DS_Store").exists());
    assert_eq!(
        remove_empty_folder(&root, "Full", id("Full")),
        Err(FsError::NotEmpty)
    );

    // Another folder under the same name is not the one to remove.
    let empty = id("Empty");
    fs::rename(base.join("Empty"), base.join("Empty-old")).unwrap();
    fs::create_dir(base.join("Empty")).unwrap();
    assert_eq!(
        remove_empty_folder(&root, "Empty", empty),
        Err(FsError::ChainChanged)
    );
    assert!(base.join("Empty").is_dir());
    // Nor is a file, or nothing at all.
    fs::write(base.join("plain"), "").unwrap();
    assert_eq!(
        remove_empty_folder(&root, "plain", id("plain")),
        Err(FsError::ChainChanged)
    );
    assert_eq!(
        remove_empty_folder(&root, "gone", empty),
        Err(FsError::ChainChanged)
    );

    // The same folder, still empty: removed.
    assert_eq!(remove_empty_folder(&root, "Empty-old", empty), Ok(()));
    assert!(!base.join("Empty-old").exists());
}

// ---------------------------------------------------------------------------
// Which files may be moved at all
// ---------------------------------------------------------------------------

#[test]
fn only_plain_single_link_visible_files_are_movable() {
    let (dir, root) = temp_root();
    let base = dir.path();
    fs::write(base.join("plain.txt"), "ok").unwrap();
    fs::write(base.join("linked.txt"), "two names").unwrap();
    fs::hard_link(base.join("linked.txt"), base.join("linked-again.txt")).unwrap();
    symlink("plain.txt", base.join("link.txt")).unwrap();
    fs::create_dir(base.join("folder")).unwrap();
    fs::write(base.join(".env"), "SECRET=synthetic").unwrap();
    let fifo = Command::new("mkfifo")
        .arg(base.join("pipe"))
        .status()
        .unwrap();
    assert!(fifo.success());

    let check =
        |name: &str| check_movable_file(name, &observe(&root, name).unwrap(), root.id().dev);
    assert_eq!(check("plain.txt"), Ok(()));
    assert_eq!(
        check("linked.txt"),
        Err(FsError::NotSupportedFile("file with several hard links"))
    );
    assert_eq!(check("link.txt"), Err(FsError::SymbolicLink));
    assert_eq!(check("folder"), Err(FsError::NotSupportedFile("folder")));
    assert_eq!(
        check("pipe"),
        Err(FsError::NotSupportedFile("special file"))
    );
    assert_eq!(check(".env"), Err(FsError::Protected(Protection::Hidden)));

    let plain = observe(&root, "plain.txt").unwrap();
    assert_eq!(
        check_movable_file("plain.txt", &plain, root.id().dev + 1),
        Err(FsError::CrossVolume)
    );
}

// ---------------------------------------------------------------------------
// Real volumes (opt-in: they mount temporary disk images with hdiutil)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "mounts temporary disk images; run with: cargo test -p traeva-core -- --ignored"]
fn walk_does_not_cross_into_another_volume() {
    let (dir, root) = temp_root();
    let _image = disk_image::MountedImage::mount_in(dir.path(), "mounted", "APFS", "16m");
    assert_eq!(
        open_chain(&root, &["mounted"]).unwrap_err(),
        FsError::CrossVolume
    );
}

#[test]
#[ignore = "mounts temporary disk images; run with: cargo test -p traeva-core -- --ignored"]
fn move_across_volumes_is_refused_and_moves_nothing() {
    let (dir, root, input, _out) = move_fixture();
    let holder = tempfile::Builder::new()
        .prefix("traeva-vol-")
        .tempdir()
        .unwrap();
    let image = disk_image::MountedImage::mount_in(holder.path(), "mnt", "APFS", "16m");
    let other_volume = open_folder_path(&image.mount_point).unwrap();
    assert_ne!(other_volume.id().dev, root.id().dev);

    assert_eq!(
        move_no_replace(&input, "doc.txt", &other_volume, "doc.txt").unwrap_err(),
        FsError::CrossVolume
    );
    assert_eq!(
        fs::read(dir.path().join("in/doc.txt")).unwrap(),
        b"original source bytes"
    );
    assert!(!image.mount_point.join("doc.txt").exists());
}

#[test]
#[ignore = "mounts temporary disk images; run with: cargo test -p traeva-core -- --ignored"]
fn roots_on_hfs_volumes_are_refused() {
    let (_root_dir, _root, _app_dir, app_data) = root_and_app_data();
    let holder = tempfile::Builder::new()
        .prefix("traeva-vol-")
        .tempdir()
        .unwrap();
    let image = disk_image::MountedImage::mount_in(holder.path(), "mnt", "HFS+", "8m");
    fs::create_dir(image.mount_point.join("Inbox")).unwrap();
    let on_hfs = open_folder_path(&image.mount_point.join("Inbox")).unwrap();
    assert!(matches!(
        inspect_root(&on_hfs, &app_data, &[]),
        Err(FsError::UnsupportedVolume(why)) if why.starts_with("hfs")
    ));
}

#[test]
fn the_added_time_is_read_for_the_object_that_was_seen() {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("Inbox")).unwrap();
    fs::write(dir.path().join("Inbox/lab.txt"), "lab\n").unwrap();
    // A second after it was written, so the time read is the move's.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let before_move = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    fs::rename(dir.path().join("Inbox/lab.txt"), dir.path().join("lab.txt")).unwrap();
    symlink("lab.txt", dir.path().join("link.txt")).unwrap();
    let seen = observe(&root, "lab.txt").unwrap();

    let added = added_ms(&root, "lab.txt", seen.id).unwrap();
    assert!(
        added >= before_move,
        "moving a file into a folder sets the time it was put there"
    );
    let other = ObjectId {
        dev: seen.id.dev,
        ino: seen.id.ino + 1,
    };
    assert_eq!(added_ms(&root, "lab.txt", other), None, "another object");
    assert_eq!(
        added_ms(&root, "link.txt", seen.id),
        None,
        "a link is not followed"
    );
    assert_eq!(added_ms(&root, "missing.txt", seen.id), None);
}
