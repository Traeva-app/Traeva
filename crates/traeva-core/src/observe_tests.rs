//! Tests for observation: text extraction (pure), stable reads, the folder
//! catalog, and registration while a file changes. Synthetic temporary files only.

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::*;
use crate::contracts::{ErrorCode, ExtractionStatus};
use crate::platform::macos::{Folder, FsError, Kind, Protection, open_folder_path};

use crate::test_snapshot as snapshot;

// ---------------------------------------------------------------------------
// Text extraction
// ---------------------------------------------------------------------------

/// Every evidence piece must be exactly the slice of the normalized text it
/// claims to be, and IDs must count up from E1.
fn assert_evidence_is_exact(normalized: &str, extraction: &Extraction) {
    let chars: Vec<char> = normalized.chars().collect();
    for (index, piece) in extraction.evidence.iter().enumerate() {
        assert_eq!(piece.id, format!("E{}", index + 1));
        let slice: String = chars[piece.start_char..piece.end_char].iter().collect();
        assert_eq!(piece.text, slice, "{}", piece.id);
        assert!(piece.end_char - piece.start_char <= EVIDENCE_MAX_CHARS);
        let line_of =
            |char_index: usize| 1 + chars[..char_index].iter().filter(|c| **c == '\n').count();
        assert_eq!(piece.first_line, line_of(piece.start_char), "{}", piece.id);
    }
}

#[test]
fn short_text_is_used_whole_with_paragraph_evidence() {
    let text = "Groceries\nmilk\n\nTaxes 2026\n";
    let e = extract_text("note.txt", text.as_bytes());
    assert_eq!(e.summary.status, ExtractionStatus::Ok);
    assert_eq!(e.summary.total_chars, 27);
    assert!(!e.summary.truncated && !e.summary.blank);
    assert_eq!(e.omitted, None);
    let pieces: Vec<_> = e
        .evidence
        .iter()
        .map(|p| {
            (
                p.id.as_str(),
                p.first_line,
                p.last_line,
                p.start_char,
                p.end_char,
                p.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        pieces,
        [
            ("E1", 1, 2, 0, 14, "Groceries\nmilk"),
            ("E2", 4, 4, 16, 26, "Taxes 2026")
        ]
    );
    assert_evidence_is_exact(text, &e);
}

#[test]
fn a_utf8_bom_is_allowed_and_line_endings_are_normalized() {
    let e = extract_text("a.md", b"\xEF\xBB\xBFone\r\ntwo\rthree\n");
    assert_eq!(e.summary.status, ExtractionStatus::Ok);
    assert_eq!(e.summary.total_chars, 14); // "one\ntwo\nthree\n"
    assert_eq!(e.evidence.len(), 1);
    assert_eq!(e.evidence[0].text, "one\ntwo\nthree");
    assert_eq!((e.evidence[0].first_line, e.evidence[0].last_line), (1, 3));
}

#[test]
fn characters_are_counted_as_unicode_scalar_values() {
    // 😀, space, café (NFC é = 1), space, 中文, newline
    assert_eq!(
        extract_text("u.txt", "😀 café 中文\n".as_bytes())
            .summary
            .total_chars,
        10
    );
    // The same word in NFD: "e" + combining accent are two scalar values.
    assert_eq!(
        extract_text("u.txt", "cafe\u{301}".as_bytes())
            .summary
            .total_chars,
        5
    );
}

#[test]
fn exactly_8000_characters_are_used_whole() {
    let text = "x".repeat(MAX_TEXT_CHARS);
    let e = extract_text("long.txt", text.as_bytes());
    assert_eq!(e.summary.status, ExtractionStatus::Ok);
    assert!(!e.summary.truncated);
    assert_eq!(e.evidence.last().unwrap().end_char, MAX_TEXT_CHARS);
}

#[test]
fn longer_texts_use_head_and_tail_with_the_middle_marked() {
    // 80 lines of 99 'x' + '\n' (8,000 chars), then "y": 8,001 chars in 81 lines.
    let mut text = format!("{}\n", "x".repeat(99)).repeat(80);
    text.push('y');
    let e = extract_text("long.txt", text.as_bytes());
    assert_eq!(e.summary.status, ExtractionStatus::Partial);
    assert!(e.summary.truncated);
    assert_eq!(e.summary.total_chars, 8_001);
    let omitted = e.omitted.clone().unwrap();
    assert_eq!((omitted.start_char, omitted.end_char), (6_000, 6_001));
    assert_eq!((omitted.first_line, omitted.last_line), (61, 61));
    // Nothing from the left-out middle, and the tail keeps its real position.
    for piece in &e.evidence {
        assert!(
            piece.end_char <= 6_000 || piece.start_char >= 6_001,
            "{}",
            piece.id
        );
    }
    assert_eq!(e.evidence.first().unwrap().start_char, 0);
    assert_eq!(e.evidence.last().unwrap().end_char, 8_001);
    assert_eq!(e.evidence.last().unwrap().last_line, 81);
    assert_evidence_is_exact(&text, &e);
}

#[test]
fn a_long_text_keeps_true_line_numbers_in_the_tail() {
    let text: String = (1..=4_000).map(|n| format!("line {n}\n")).collect();
    let e = extract_text("log.md", text.as_bytes());
    let total = text.chars().count();
    assert_eq!(e.summary.total_chars, total);
    let tail: Vec<_> = e
        .evidence
        .iter()
        .filter(|p| p.start_char >= total - TAIL_CHARS)
        .collect();
    assert!(!tail.is_empty());
    assert_eq!(tail.last().unwrap().last_line, 4_000);
    assert!(tail.last().unwrap().text.ends_with("line 4000"));
    assert_evidence_is_exact(&text, &e);
}

#[test]
fn evidence_pieces_have_a_size_limit() {
    let line = "w".repeat(300);
    let text = format!("{line}\n{line}\n{line}\n\n{}\n", "z".repeat(2_000));
    let e = extract_text("p.txt", text.as_bytes());
    let pieces: Vec<_> = e
        .evidence
        .iter()
        .map(|p| (p.first_line, p.last_line, p.end_char - p.start_char))
        .collect();
    assert_eq!(
        pieces,
        [
            (1, 2, 601),
            (3, 3, 300),
            (5, 5, 800),
            (5, 5, 800),
            (5, 5, 400)
        ]
    );
    assert_evidence_is_exact(&text, &e);
}

#[test]
fn only_txt_and_markdown_files_are_read() {
    for name in ["note.txt", "README.MD", "Draft.Markdown"] {
        assert!(is_text_file_name(name), "{name}");
    }
    for name in [
        "scan.pdf",
        "Notes",
        ".txt",
        "report.docx",
        "archive.txt.zip",
    ] {
        let e = extract_text(name, b"plain words");
        assert_eq!(e.summary.status, ExtractionStatus::Unsupported, "{name}");
        assert!(e.evidence.is_empty());
    }
}

#[test]
fn utf16_nul_and_invalid_utf8_are_never_guessed() {
    let unsupported: [&[u8]; 3] = [b"\xFF\xFEh\x00i\x00", b"\xFE\xFF\x00h\x00i", b"ab\x00cd"];
    for bytes in unsupported {
        let e = extract_text("a.txt", bytes);
        assert_eq!(e.summary.status, ExtractionStatus::Unsupported, "{bytes:?}");
        assert!(e.evidence.is_empty());
    }
    let invalid: [&[u8]; 3] = [b"fo\xFF", b"\xE4\xB8", b"\xC0\xAF"];
    for bytes in invalid {
        let e = extract_text("a.txt", bytes);
        assert_eq!(e.summary.status, ExtractionStatus::Failed, "{bytes:?}");
        assert!(e.evidence.is_empty(), "no lossy text");
        assert_eq!(e.summary.total_chars, 0);
        // Said, not hidden (O26).
        assert!(e.summary.note.is_some(), "{bytes:?}");
    }
}

/// Over 8,000 characters of Chinese and emoji: the cut is counted in
/// characters, never in bytes, and every piece is an exact slice (O29, O30).
#[test]
fn long_multibyte_texts_are_cut_by_characters() {
    for unit in ["中", "🙂", "é"] {
        let mut text = String::new();
        for line in 0..90 {
            text.push_str(&unit.repeat(99));
            text.push_str(&format!("{line}\n"));
        }
        let total = text.chars().count();
        assert!(total > MAX_TEXT_CHARS);
        let e = extract_text("long.txt", text.as_bytes());
        assert_eq!(e.summary.status, ExtractionStatus::Partial, "{unit}");
        assert_eq!(e.summary.total_chars, total);
        let omitted = e.omitted.clone().unwrap();
        assert_eq!(
            (omitted.start_char, omitted.end_char),
            (6_000, total - 2_000),
            "{unit}"
        );
        assert_eq!(e.excerpt.len(), 2, "head and tail, never joined");
        assert_eq!(e.excerpt[0].chars().count(), 6_000);
        assert_eq!(e.excerpt[1].chars().count(), 2_000);
        let head: String = text.chars().take(6_000).collect();
        assert_eq!(e.excerpt[0], head);
        assert_evidence_is_exact(&text, &e);
    }
}

/// Markdown is only text: a script, a link and an image link stay plain
/// characters in the text pieces, and nothing is fetched or run (O25).
#[test]
fn markdown_scripts_and_links_stay_plain_text() {
    let text = "# Notes\n\n<script>alert('x')</script>\n\n[a link](https://example.invalid/x) and ![](file:///etc/hosts)\n";
    let e = extract_text("notes.md", text.as_bytes());
    assert_eq!(e.summary.status, ExtractionStatus::Ok);
    assert_eq!(e.excerpt, [text]);
    let pieces: Vec<&str> = e.evidence.iter().map(|p| p.text.as_str()).collect();
    assert!(
        pieces.contains(&"<script>alert('x')</script>"),
        "{pieces:?}"
    );
    assert!(
        pieces
            .iter()
            .any(|p| p.contains("(https://example.invalid/x)") && p.contains("(file:///etc/hosts)")),
        "{pieces:?}"
    );
    assert_evidence_is_exact(text, &e);
}

/// The read budget of organize-spec.md §4.1, pinned (O31).
#[test]
fn the_read_budget_is_the_specs() {
    assert_eq!(PAUSE_BETWEEN_READS, Duration::from_millis(350));
    assert_eq!(MAX_READ_ATTEMPTS, 3);
    assert_eq!(MAX_READ_TIME, Duration::from_secs(5));
    assert_eq!(MAX_FILE_BYTES, 20 * 1024 * 1024);
    assert_eq!(
        (MAX_TEXT_CHARS, HEAD_CHARS, TAIL_CHARS),
        (8_000, 6_000, 2_000)
    );
}

#[test]
fn usable_text_means_at_least_one_text_piece() {
    // Whitespace where the head and tail are read, words only in the middle
    // that is left out: not blank, yet nothing can be sent.
    let middle_only = format!("{}middle words{}", " ".repeat(6_000), " ".repeat(2_000));
    let cases: [(&str, &[u8], bool); 5] = [
        ("note.txt", b"A short note.", true),
        ("empty.txt", b"  \n", false),
        ("scan.pdf", b"%PDF-1.7", false),
        ("bad.txt", b"\xFF\xFE", false),
        ("long.txt", middle_only.as_bytes(), false),
    ];
    for (name, bytes, usable) in cases {
        let e = extract_text(name, bytes);
        assert_eq!(e.summary.usable_text, usable, "{name}");
        assert_eq!(e.summary.usable_text, !e.evidence.is_empty(), "{name}");
    }
    let e = extract_text("long.txt", middle_only.as_bytes());
    assert!(e.summary.truncated && !e.summary.blank);
}

#[test]
fn empty_and_whitespace_only_texts_are_blank() {
    for bytes in [&b""[..], b"   \n\t\n", b"\xEF\xBB\xBF\r\n"] {
        let e = extract_text("empty.txt", bytes);
        assert_eq!(e.summary.status, ExtractionStatus::Ok);
        assert!(e.summary.blank, "{bytes:?}");
        assert!(e.evidence.is_empty());
    }
}

#[test]
fn sha256_matches_a_known_value() {
    assert_eq!(
        sha256_hex(b"hello\n"),
        "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
    );
}

// ---------------------------------------------------------------------------
// Stable reads
// ---------------------------------------------------------------------------

fn temp_root() -> (tempfile::TempDir, Folder) {
    let dir = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let folder = open_folder_path(dir.path()).unwrap();
    (dir, folder)
}

fn no_pause() -> impl FnMut(u32) {
    |_| {}
}

#[test]
fn a_stable_read_returns_the_exact_bytes_and_their_hash() {
    let (dir, root) = temp_root();
    let raw = b"\xEF\xBB\xBFhello\r\n";
    fs::write(dir.path().join("doc.txt"), raw).unwrap();
    let pauses = Cell::new(0);
    let read = read_stable_with(
        &root,
        "doc.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut |_| pauses.set(pauses.get() + 1),
    )
    .unwrap();
    assert_eq!(read.bytes, raw);
    // The hash is of the raw bytes, BOM and CRLF included (checked with `shasum -a 256`).
    assert_eq!(
        read.sha256,
        "ce1147fdc2c0b2bae8886109c9ea731213a5bd591e0755f1fc3ec699b5f0b003"
    );
    assert_eq!(read.observed.kind, Kind::File);
    assert_eq!(pauses.get(), 1);
}

#[test]
fn a_file_that_keeps_changing_is_not_used() {
    let (dir, root) = temp_root();
    let path = dir.path().join("doc.txt");
    fs::write(&path, "v0").unwrap();
    let result = read_stable_with(
        &root,
        "doc.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut |attempt| fs::write(&path, format!("v{attempt}")).unwrap(),
    );
    assert_eq!(
        result.unwrap_err(),
        ReadError::StillChanging { attempts: 3 }
    );
}

#[test]
fn an_equal_length_change_with_restored_mtime_is_still_caught() {
    let (dir, root) = temp_root();
    let path = dir.path().join("doc.txt");
    fs::write(&path, "AAAA").unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let read = read_stable_with(
        &root,
        "doc.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut |attempt| {
            if attempt == 1 {
                fs::write(&path, "BBBB").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(mtime)
                    .unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(read.bytes, b"BBBB");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);

    // Even if every attribute looked identical, different contents are a change.
    let same = read.observed.clone();
    assert!(!is_stable("aaa", "bbb", &same, &same, &same, Some(&same)));
    assert!(is_stable("aaa", "aaa", &same, &same, &same, Some(&same)));
}

#[test]
fn the_name_must_still_point_at_the_file_that_was_read() {
    let (dir, root) = temp_root();
    fs::write(dir.path().join("a.txt"), "a").unwrap();
    fs::write(dir.path().join("b.txt"), "b").unwrap();
    let a = macos::observe(&root, "a.txt").unwrap();
    let b = macos::observe(&root, "b.txt").unwrap();
    // Same hash, unchanged open file, but the name now means something else.
    assert!(!is_stable("h", "h", &a, &a, &a, Some(&b)));
    assert!(!is_stable("h", "h", &a, &a, &a, None));
    assert!(is_stable("h", "h", &a, &a, &a, Some(&a)));
}

#[test]
fn a_file_replaced_by_an_atomic_save_is_read_again() {
    let (dir, root) = temp_root();
    let path = dir.path().join("doc.txt");
    fs::write(&path, "old").unwrap();
    let old_id = macos::observe(&root, "doc.txt").unwrap().id;
    let read = read_stable_with(
        &root,
        "doc.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut |attempt| {
            if attempt == 1 {
                fs::write(dir.path().join("doc.txt.tmp"), "new").unwrap();
                fs::rename(dir.path().join("doc.txt.tmp"), &path).unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(read.bytes, b"new");
    assert_ne!(read.observed.id, old_id);
}

#[test]
fn the_time_budget_stops_further_attempts() {
    let (dir, root) = temp_root();
    let path = dir.path().join("doc.txt");
    fs::write(&path, "v0").unwrap();
    let limits = ReadLimits {
        max_time: Duration::from_millis(100),
        ..ReadLimits::SPEC
    };
    let result = read_stable_with(&root, "doc.txt", root.id().dev, limits, &mut |attempt| {
        std::thread::sleep(Duration::from_millis(120));
        fs::write(&path, format!("v{attempt}")).unwrap();
    });
    assert_eq!(
        result.unwrap_err(),
        ReadError::StillChanging { attempts: 1 }
    );
}

#[test]
fn files_over_20_mib_are_refused_without_reading() {
    let (dir, root) = temp_root();
    let at_limit = fs::File::create(dir.path().join("at-limit.txt")).unwrap();
    at_limit.set_len(MAX_FILE_BYTES).unwrap();
    let over = fs::File::create(dir.path().join("over.txt")).unwrap();
    over.set_len(MAX_FILE_BYTES + 1).unwrap();

    let read = read_stable_with(
        &root,
        "at-limit.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut no_pause(),
    )
    .unwrap();
    assert_eq!(read.bytes.len() as u64, MAX_FILE_BYTES);
    let pauses = Cell::new(0);
    let result = read_stable_with(
        &root,
        "over.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut |_| pauses.set(pauses.get() + 1),
    );
    assert_eq!(result.unwrap_err(), ReadError::TooLarge);
    assert_eq!(pauses.get(), 0, "nothing was read");
}

#[test]
fn refused_files_are_never_opened() {
    let (dir, root) = temp_root();
    let base = dir.path();
    fs::write(base.join("plain.txt"), "ok").unwrap();
    symlink("plain.txt", base.join("link.txt")).unwrap();
    fs::write(base.join("twin.txt"), "two names").unwrap();
    fs::hard_link(base.join("twin.txt"), base.join("twin-again.txt")).unwrap();
    fs::write(base.join(".env"), "SECRET=synthetic").unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(base.join("pipe.txt"))
            .status()
            .unwrap()
            .success()
    );

    let refused = |name: &str| {
        read_stable_with(&root, name, root.id().dev, ReadLimits::SPEC, &mut |_| {
            panic!("{name} must not be read")
        })
        .unwrap_err()
    };
    assert_eq!(refused("link.txt"), ReadError::Fs(FsError::SymbolicLink));
    assert_eq!(
        refused("twin.txt"),
        ReadError::Fs(FsError::NotSupportedFile("file with several hard links"))
    );
    assert_eq!(
        refused(".env"),
        ReadError::Fs(FsError::Protected(Protection::Hidden))
    );
    assert_eq!(
        refused("pipe.txt"),
        ReadError::Fs(FsError::NotSupportedFile("special file"))
    );
    assert_eq!(refused("missing.txt"), ReadError::Fs(FsError::NotFound));
}

#[test]
fn stable_reads_write_nothing() {
    let (dir, root) = temp_root();
    snapshot::fill_sample_tree(dir.path());
    let before = snapshot::snapshot(dir.path());
    read_stable_with(
        &root,
        "todo.txt",
        root.id().dev,
        ReadLimits::SPEC,
        &mut no_pause(),
    )
    .unwrap();
    assert_eq!(snapshot::snapshot(dir.path()), before);
}

// ---------------------------------------------------------------------------
// Folder catalog
// ---------------------------------------------------------------------------

fn catalog_paths(catalog: &FolderCatalog) -> Vec<String> {
    catalog
        .folders
        .iter()
        .map(|f| f.segments.join("/"))
        .collect()
}

#[test]
fn the_catalog_is_breadth_first_and_skips_what_it_must() {
    let (dir, root) = temp_root();
    let base = dir.path();
    for folder in [
        "b",
        "a/x",
        ".hidden/inside",
        "Lib.photoslibrary/inside",
        "proj/inner",
    ] {
        fs::create_dir_all(base.join(folder)).unwrap();
    }
    fs::write(base.join("proj/package.json"), "{}").unwrap();
    fs::write(base.join("c.txt"), "a file, not a folder").unwrap();
    symlink(base.join("a"), base.join("link-to-a")).unwrap();
    let before = snapshot::snapshot(base);

    let catalog = build_catalog(&root).unwrap();

    assert_eq!(catalog_paths(&catalog), ["", "a", "b", "proj", "a/x"]);
    let ids: Vec<_> = catalog
        .folders
        .iter()
        .map(|f| f.folder_id.as_str())
        .collect();
    assert_eq!(ids, ["F0", "F1", "F2", "F3", "F4"]);
    assert_eq!(catalog.folders[4].parent_id.as_deref(), Some("F1"));
    assert_eq!(
        catalog.folders[3].protected,
        Some(Protection::Project("package.json".to_string()))
    );
    assert!(!catalog.truncated);
    assert_eq!(
        snapshot::snapshot(base),
        before,
        "listing folders writes nothing"
    );
}

#[test]
fn the_catalog_stops_at_six_levels() {
    let (dir, root) = temp_root();
    fs::create_dir_all(dir.path().join("d1/d2/d3/d4/d5/d6/d7/d8")).unwrap();
    let catalog = build_catalog(&root).unwrap();
    assert_eq!(catalog.folders.len(), 1 + MAX_CATALOG_DEPTH);
    assert_eq!(
        catalog.folders.last().unwrap().segments.len(),
        MAX_CATALOG_DEPTH
    );
    assert!(catalog.truncated);
}

#[test]
fn the_catalog_stops_at_500_folders() {
    let (dir, root) = temp_root();
    for n in 0..600 {
        fs::create_dir(dir.path().join(format!("folder-{n:03}"))).unwrap();
    }
    let catalog = build_catalog(&root).unwrap();
    assert_eq!(catalog.folders.len(), MAX_CATALOG_FOLDERS);
    assert!(catalog.truncated);
}

#[test]
fn the_model_gets_at_most_100_unprotected_candidates() {
    let (dir, root) = temp_root();
    fs::create_dir(dir.path().join("aaa-project")).unwrap();
    fs::write(dir.path().join("aaa-project/Cargo.toml"), "[package]").unwrap();
    for n in 0..150 {
        fs::create_dir(dir.path().join(format!("topic-{n:03}"))).unwrap();
    }
    let catalog = build_catalog(&root).unwrap();
    let candidates = catalog.model_candidates();
    assert_eq!(candidates.len(), MAX_MODEL_CANDIDATES);
    assert!(candidates.iter().all(|f| f.protected.is_none()));
    assert_eq!(
        candidates[0].folder_id, "F0",
        "the root itself can be a target"
    );
}

// ---------------------------------------------------------------------------
// Registration while the file changes (through the service)
// ---------------------------------------------------------------------------

#[test]
fn a_file_that_keeps_changing_is_not_registered() {
    let app = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let (dir, _root) = temp_root();
    let path = dir.path().join("draft.txt");
    fs::write(&path, "v0").unwrap();
    let service = crate::OrganizeService::start(app.path().join("Traeva"));
    let root = service.authorize_root(dir.path()).unwrap();

    let error = service
        .register_file_with(&root.root_id, &path, &mut |_root, parent, name| {
            read_stable_with(
                parent,
                name,
                parent.id().dev,
                ReadLimits::SPEC,
                &mut |attempt| fs::write(&path, format!("v{attempt}")).unwrap(),
            )
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::FileChanging);
    assert!(error.retryable);
    let conn =
        rusqlite::Connection::open(app.path().join("Traeva").join(crate::DATABASE_FILE_NAME))
            .unwrap();
    let files: i64 = conn
        .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(files, 0, "nothing is recorded for an unstable file");
}

#[test]
fn moving_a_parent_folder_while_reading_is_caught() {
    let app = tempfile::Builder::new()
        .prefix("traeva-appdata-")
        .tempdir()
        .unwrap();
    let (dir, _root) = temp_root();
    fs::create_dir(dir.path().join("Inbox")).unwrap();
    let path = dir.path().join("Inbox/note.txt");
    fs::write(&path, "text").unwrap();
    let service = crate::OrganizeService::start(app.path().join("Traeva"));
    let root = service.authorize_root(dir.path()).unwrap();

    let error = service
        .register_file_with(&root.root_id, &path, &mut |_root, parent, name| {
            let read = read_stable_with(
                parent,
                name,
                parent.id().dev,
                ReadLimits::SPEC,
                &mut no_pause(),
            );
            // Another program renames the folder right after the read.
            fs::rename(dir.path().join("Inbox"), dir.path().join("Inbox-moved")).unwrap();
            read
        })
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::FileChanging);
    assert!(Path::new(&dir.path().join("Inbox-moved/note.txt")).exists());
}
