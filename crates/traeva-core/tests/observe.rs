//! Registering files and listing folders through the public service API
//! (organize-spec.md §2.3, §4; O01, O05, O26, O29–O31). Synthetic files only.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use common::*;
use rusqlite::Connection;
use traeva_core::{AppError, ErrorCode, ErrorStage, ExtractionStatus, OrganizeService};

const PLAN_MD_SHA256: &str = "e8cd09ff8c24db7b0ed526948dafdf1c3b55382c6f61504b46a609b0860a5e21";

struct Setup {
    app: tempfile::TempDir,
    root_dir: tempfile::TempDir,
    service: OrganizeService,
    root_id: String,
}

impl Setup {
    /// A ready service and an authorized root holding the sample tree.
    fn new() -> Setup {
        let app = tempfile::Builder::new()
            .prefix("traeva-appdata-")
            .tempdir()
            .unwrap();
        let root_dir = tempfile::Builder::new()
            .prefix("traeva-root-")
            .tempdir()
            .unwrap();
        snapshot::fill_sample_tree(root_dir.path());
        let service = OrganizeService::start(app.path().join("Traeva"));
        let root_id = service.authorize_root(root_dir.path()).unwrap().root_id;
        Setup {
            app,
            root_dir,
            service,
            root_id,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root_dir.path().join(relative)
    }

    fn register(&self, relative: &str) -> Result<traeva_core::ObservedFile, AppError> {
        self.service
            .register_file(&self.root_id, &self.path(relative))
    }

    fn db(&self) -> Connection {
        open_for_inspection(&self.app.path().join("Traeva"))
    }

    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

#[test]
fn registering_reads_only_and_records_identity_location_and_hash() {
    let s = Setup::new();
    let before = snapshot::snapshot(s.root_dir.path());

    let observed = s.register("Notes/2026/plan.md").unwrap();
    let file = &observed.file;

    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "registration writes nothing in the root"
    );
    assert_eq!(file.relative_path, "Notes/2026/plan.md");
    assert_eq!(
        PathBuf::from(&file.display_path),
        fs::canonicalize(s.path("Notes/2026/plan.md")).unwrap()
    );
    assert_eq!(file.sha256, PLAN_MD_SHA256, "checked with `shasum -a 256`");
    assert_eq!(file.byte_length, 18);
    assert_eq!(file.file_revision, 1);
    assert_eq!(file.extraction.status, ExtractionStatus::Ok);
    assert_eq!(observed.extraction.evidence.len(), 2);
    assert_eq!(observed.extraction.evidence[1].text, "- ship M1");

    let db = s.db();
    let (presence, sha256, revision): (String, String, i64) = db
        .query_row(
            "SELECT presence, sha256, record_revision FROM files WHERE file_id = ?1",
            [&file.file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (presence.as_str(), sha256.as_str(), revision),
        ("present", PLAN_MD_SHA256, 1)
    );
    let (reason, from_root, action): (String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT reason, from_root_id, action_id FROM path_history WHERE file_id = ?1",
            [&file.file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((reason.as_str(), from_root, action), ("intake", None, None));
}

#[test]
fn the_same_current_file_keeps_its_id() {
    let s = Setup::new();
    let first = s.register("todo.txt").unwrap().file;
    let again = s.register("todo.txt").unwrap().file;
    assert_eq!(again.file_id, first.file_id);
    assert_eq!(again.file_revision, 1, "nothing changed");
    assert_eq!(s.count("SELECT count(*) FROM path_history"), 1);
}

#[test]
fn editing_in_place_keeps_the_id_and_raises_the_revision() {
    let s = Setup::new();
    let first = s.register("todo.txt").unwrap().file;
    fs::write(s.path("todo.txt"), "buy oat milk\n").unwrap(); // same inode, new bytes
    let edited = s.register("todo.txt").unwrap().file;
    assert_eq!(edited.file_id, first.file_id);
    assert_eq!(edited.file_revision, 2);
    assert_ne!(edited.sha256, first.sha256);
}

#[test]
fn a_copy_with_the_same_content_gets_its_own_id() {
    let s = Setup::new();
    let original = s.register("todo.txt").unwrap().file;
    fs::copy(s.path("todo.txt"), s.path("todo copy.txt")).unwrap();
    let copy = s.register("todo copy.txt").unwrap().file;
    assert_eq!(copy.sha256, original.sha256);
    assert_ne!(
        copy.file_id, original.file_id,
        "same content never merges records"
    );
}

#[test]
fn a_replaced_file_gets_a_new_id_and_the_old_record_goes_missing() {
    let s = Setup::new();
    let original = s.register("todo.txt").unwrap().file;
    // An atomic save: write a new file and rename it over the old one.
    fs::write(s.path("todo.tmp"), "buy milk\n").unwrap();
    fs::rename(s.path("todo.tmp"), s.path("todo.txt")).unwrap();

    let replaced = s.register("todo.txt").unwrap().file;

    assert_eq!(
        replaced.sha256, original.sha256,
        "same bytes, different object"
    );
    assert_ne!(replaced.file_id, original.file_id);
    let presence: String = s
        .db()
        .query_row(
            "SELECT presence FROM files WHERE file_id = ?1",
            [&original.file_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(presence, "missing");
    assert_eq!(
        s.count("SELECT count(*) FROM path_history WHERE reason = 'intake'"),
        2
    );
}

/// A file moved by someone else is a new file at its new place: Traeva only
/// follows moves it made itself, never by inode or hash (O01).
#[test]
fn a_file_moved_outside_traeva_gets_a_new_id_at_its_new_place() {
    let s = Setup::new();
    let original = s.register("todo.txt").unwrap().file;
    fs::rename(s.path("todo.txt"), s.path("Notes/todo.txt")).unwrap();

    let moved = s.register("Notes/todo.txt").unwrap().file;

    assert_eq!(moved.sha256, original.sha256, "the same object and bytes");
    assert_ne!(moved.file_id, original.file_id);
    assert_eq!(s.count("SELECT count(*) FROM files"), 2);
    assert_eq!(
        s.count("SELECT count(*) FROM path_history WHERE reason = 'intake'"),
        2
    );
    assert_eq!(
        s.count("SELECT count(*) FROM path_history WHERE reason <> 'intake'"),
        0,
        "no move is made up"
    );
}

/// A UTF-8 text with a BOM, CRLF line ends, Chinese and an emoji: the hash
/// is over the exact bytes, the count over the text without BOM and CR (O29).
#[test]
fn a_bom_crlf_chinese_text_is_hashed_as_its_raw_bytes() {
    use sha2::{Digest, Sha256};
    let s = Setup::new();
    let text = "第一行\r\n第二行 🙂\r\n";
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    fs::write(s.path("笔记 🙂.txt"), &bytes).unwrap();

    let observed = s.register("笔记 🙂.txt").unwrap();

    let raw: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(observed.file.sha256, raw);
    let stored: String = s
        .db()
        .query_row(
            "SELECT sha256 FROM files WHERE file_id = ?1",
            [&observed.file.file_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, raw);
    let summary = &observed.extraction.summary;
    assert_eq!(summary.status, ExtractionStatus::Ok);
    assert_eq!(summary.total_chars, "第一行\n第二行 🙂\n".chars().count());
    assert_eq!(observed.extraction.excerpt, ["第一行\n第二行 🙂\n"]);
    assert_eq!(fs::read(s.path("笔记 🙂.txt")).unwrap(), bytes, "unchanged");
}

#[test]
fn files_that_cannot_be_organized_are_refused_and_not_recorded() {
    let s = Setup::new();
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    fs::write(outside.path().join("elsewhere.txt"), "outside").unwrap();
    symlink(outside.path(), s.path("outside-link")).unwrap();
    fs::create_dir(s.path("app")).unwrap();
    fs::write(s.path("app/package.json"), "{}").unwrap();
    fs::write(s.path("app/notes.txt"), "project notes").unwrap();
    fs::write(s.path(".env"), "SECRET=synthetic").unwrap();
    fs::File::create(s.path("huge.txt"))
        .unwrap()
        .set_len(20 * 1024 * 1024 + 1)
        .unwrap();
    // A file with two names (a hard link), and a named pipe.
    fs::write(s.path("twice.txt"), "two names").unwrap();
    fs::hard_link(s.path("twice.txt"), s.path("Notes/twice-again.txt")).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(s.path("pipe.txt"))
        .status()
        .unwrap();
    assert!(made.success());
    let before = snapshot::snapshot(s.root_dir.path());

    let code = |path: &Path| s.service.register_file(&s.root_id, path).unwrap_err().code;
    assert_eq!(
        code(&outside.path().join("elsewhere.txt")),
        ErrorCode::OutsideRoot
    );
    assert_eq!(
        code(&s.path("outside-link/elsewhere.txt")),
        ErrorCode::OutsideRoot
    );
    assert_eq!(code(&s.path("plan-link.md")), ErrorCode::UnsupportedFile);
    assert_eq!(code(&s.path("Notes")), ErrorCode::UnsupportedFile);
    assert_eq!(code(&s.path(".env")), ErrorCode::UnsupportedFile);
    assert_eq!(code(&s.path("app/notes.txt")), ErrorCode::ProtectedLocation);
    // A file named like a marker sits in a project by definition (#58).
    assert_eq!(
        code(&s.path("app/package.json")),
        ErrorCode::ProtectedLocation
    );
    assert_eq!(code(&s.path("missing.txt")), ErrorCode::FileNotFound);
    assert_eq!(
        code(&s.path("nowhere/missing.txt")),
        ErrorCode::FileNotFound
    );
    assert_eq!(code(Path::new("relative.txt")), ErrorCode::InvalidInput);
    assert_eq!(code(&s.path("twice.txt")), ErrorCode::UnsupportedFile);
    assert_eq!(code(&s.path("pipe.txt")), ErrorCode::UnsupportedFile);
    let too_large = s.register("huge.txt").unwrap_err();
    assert_eq!(too_large.code, ErrorCode::FileTooLarge);
    assert_eq!(too_large.stage, ErrorStage::Observation);
    assert!(!too_large.retryable);

    assert_eq!(s.count("SELECT count(*) FROM files"), 0);
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
}

#[test]
fn a_file_without_readable_text_can_still_be_registered() {
    let s = Setup::new();
    fs::write(s.path("scan.pdf"), b"%PDF-1.7 synthetic").unwrap();
    fs::write(s.path("legacy.txt"), b"\xFF\xFEh\x00i\x00").unwrap();
    let pdf = s.register("scan.pdf").unwrap();
    let utf16 = s.register("legacy.txt").unwrap();
    assert_eq!(pdf.file.extraction.status, ExtractionStatus::Unsupported);
    assert_eq!(utf16.file.extraction.status, ExtractionStatus::Unsupported);
    assert!(pdf.extraction.evidence.is_empty() && utf16.extraction.evidence.is_empty());
    assert_eq!(
        s.count("SELECT count(*) FROM files"),
        2,
        "the user can still choose a folder by hand"
    );
}

#[test]
fn a_revoked_root_cannot_be_used() {
    let s = Setup::new();
    s.service.revoke_root(&s.root_id).unwrap();
    assert_eq!(
        s.register("todo.txt").unwrap_err().code,
        ErrorCode::RootRevoked
    );
    assert_eq!(
        s.service.list_folders(&s.root_id).unwrap_err().code,
        ErrorCode::RootRevoked
    );
}

#[test]
fn listing_folders_reads_only_and_returns_a_bounded_tree() {
    let s = Setup::new();
    fs::create_dir_all(s.path("Taxes/2025")).unwrap();
    fs::create_dir(s.path("web")).unwrap();
    fs::write(s.path("web/package.json"), "{}").unwrap();
    let before = snapshot::snapshot(s.root_dir.path());

    let catalog = s.service.list_folders(&s.root_id).unwrap();

    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "listing writes nothing"
    );
    let listed: Vec<(&str, &str, usize, bool)> = catalog
        .folders
        .iter()
        .map(|f| {
            (
                f.folder_id.as_str(),
                f.relative_path.as_str(),
                f.depth,
                f.protected.is_some(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("F0", "", 0, false),
            ("F1", "Notes", 1, false),
            ("F2", "Taxes", 1, false),
            ("F3", "web", 1, true),
            ("F4", "Notes/2026", 2, false),
            ("F5", "Taxes/2025", 2, false),
        ]
    );
    assert!(!catalog.truncated);
    assert_eq!(catalog.root_id, s.root_id);
}
