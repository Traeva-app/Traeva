//! Tests for find.rs (#198), all offline. The model is a closure standing in
//! for `send_only`; most tests run without one, as Find does with no key.

use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::*;
use crate::model::{SendFailure, SentAnswer};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::store::DATABASE_FILE_NAME;
use crate::test_fixture::{Fixture, START_MS};
use crate::test_snapshot as snapshot;

/// The real limits; the test folders are small anyway.
const LIMITS: FindLimits = FindLimits::SPEC;

fn write(f: &Fixture, relative: &str, text: &str) {
    let path = f.path(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Sets the file's modification time to `days` before the fixture clock.
fn changed_days_ago(f: &Fixture, relative: &str, days: u64) {
    let at =
        UNIX_EPOCH + Duration::from_millis(START_MS as u64) - Duration::from_secs(days * 86_400);
    fs::File::options()
        .write(true)
        .open(f.path(relative))
        .unwrap()
        .set_modified(at)
        .unwrap();
}

/// A transport that must not be used.
fn no_model(
    _: &OutgoingRequest,
    _: &mut dyn FnMut(&Recipient) -> bool,
) -> Result<Option<SentAnswer>, SendFailure> {
    panic!("the model must not be asked");
}

fn recipient() -> Recipient {
    Recipient {
        provider: PROVIDER_OPENAI.to_string(),
        model: DEFAULT_MODEL.to_string(),
    }
}

fn answer(output: Value) -> SentAnswer {
    SentAnswer {
        output,
        latency_ms: 1,
        usage: None,
        provider_request_id: None,
        recipient: recipient(),
    }
}

/// Settings and a key, and Find's agreement when `agree`.
fn model_set_up(f: &Fixture, agree: bool) {
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    if agree {
        f.service().set_find_consent(true).unwrap();
    }
}

fn search_with(f: &Fixture, question: &str, transport: Transport) -> FindView {
    search_limited(f, question, transport, LIMITS)
}

fn search_limited(
    f: &Fixture,
    question: &str,
    transport: Transport,
    limits: FindLimits,
) -> FindView {
    let started = f.service().start_find(question).unwrap();
    assert_eq!(started.state, FindState::Working);
    f.service()
        .run_find_with(&started.find_id, transport, limits)
        .unwrap()
}

fn search(f: &Fixture, question: &str) -> FindView {
    search_with(f, question, &mut no_model)
}

fn names(view: &FindView) -> Vec<&str> {
    view.results.iter().map(|r| r.name.as_str()).collect()
}

fn result<'a>(view: &'a FindView, name: &str) -> &'a FindResult {
    view.results
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {:?}", names(view)))
}

fn database_bytes(f: &Fixture) -> Vec<u8> {
    fs::read(f.app.path().join("Traeva").join(DATABASE_FILE_NAME)).unwrap()
}

/// An `analyses` row as Organize leaves it after a checked answer.
fn describe(f: &Fixture, file_id: &str, summary: &str) {
    let db = f.db();
    let (sha256, revision): (String, i64) = db
        .query_row(
            "SELECT sha256, record_revision FROM files WHERE file_id = ?1",
            [file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    db.execute(
        "INSERT INTO analyses (request_id, file_id, file_sha256, file_revision, provider, model,
             requested_at_ms, prompt_version, output_schema_version, extractor_version,
             app_version, request_meta_json, status, result_json)
         VALUES (?1, ?2, ?3, ?4, 'openai', ?5, ?6, 'p', 's', 'e', 'a', '{}', 'validated', ?7)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            file_id,
            sha256,
            revision,
            DEFAULT_MODEL,
            START_MS,
            json!({ "summary": summary, "disposition": "recommend" }).to_string(),
        ],
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Matching without the model
// ---------------------------------------------------------------------------

#[test]
fn finds_a_file_by_its_text_when_its_name_says_nothing() {
    let f = Fixture::new();
    write(
        &f,
        "notes_0912.txt",
        "Day 3 in Kyoto.\nHotel budget: 40,000 yen.\n",
    );
    write(&f, "other.txt", "Groceries for the week.\n");

    let view = search(&f, "my Kyoto budget");

    assert_eq!(view.state, FindState::Done);
    assert_eq!(
        view.words_by,
        Some(WordsBy::Question {
            reason: "No model is set up.".to_string()
        })
    );
    assert_eq!(
        view.words.as_ref().unwrap().things,
        vec![vec!["Kyoto".to_string()], vec!["budget".to_string()]],
        "\"my\" is left out"
    );
    assert_eq!(names(&view), ["notes_0912.txt"]);
    let found = result(&view, "notes_0912.txt");
    assert_eq!(found.kind, Some(FileKind::Text));
    assert_eq!(found.matches.len(), 2);
    assert!(found.matches.iter().all(|m| m.place == MatchPlace::Text));
    assert_eq!(
        found.matches[0].snippet.as_deref(),
        Some("Day 3 in Kyoto. Hotel budget: 40,000 yen.")
    );
    assert_eq!(found.result_id, "R1");
    assert_eq!(found.relative_path, "notes_0912.txt");
    assert_eq!(view.matched, 1);
}

#[test]
fn case_and_accents_do_not_matter() {
    let f = Fixture::new();
    write(&f, "cv.md", "# Résumé\n\nWork at the CAFÉ.\n");

    let view = search(&f, "resume cafe");

    let found = result(&view, "cv.md");
    assert_eq!(found.matches.len(), 2);
    assert_eq!(found.kind, Some(FileKind::Markdown));
}

#[test]
fn more_things_rank_first_then_the_place_then_the_newest() {
    let f = Fixture::new();
    write(&f, "Trip/kyoto-budget.txt", "numbers\n");
    write(&f, "Trip/a.txt", "kyoto and the budget\n");
    write(&f, "Trip/kyoto.txt", "nothing else\n");
    write(&f, "Trip/b.txt", "kyoto only, older\n");
    write(&f, "Trip/c.txt", "kyoto only, newer\n");
    changed_days_ago(&f, "Trip/b.txt", 30);
    changed_days_ago(&f, "Trip/c.txt", 1);

    let view = search(&f, "kyoto budget");

    assert_eq!(
        names(&view),
        ["kyoto-budget.txt", "a.txt", "kyoto.txt", "c.txt", "b.txt"],
        "two things before one; the name before the text; then the newest"
    );
    assert_eq!(view.matched, 5);
}

#[test]
fn folder_names_count_after_the_name_and_before_the_text() {
    // The student's decision 10/16: a file in a "Kyoto" folder before a
    // file that only uses the word in its text, even when it is older.
    let f = Fixture::new();
    write(&f, "Kyoto/hotel.txt", "a booking\n");
    write(&f, "other/kyoto-hotel.txt", "a booking\n");
    write(&f, "other/words.txt", "a list of words, kyoto among them\n");
    changed_days_ago(&f, "Kyoto/hotel.txt", 30);

    let view = search(&f, "kyoto");

    assert_eq!(names(&view), ["kyoto-hotel.txt", "hotel.txt", "words.txt"]);
    let in_folder = result(&view, "hotel.txt");
    assert_eq!(in_folder.matches[0].place, MatchPlace::Folder);
    assert_eq!(
        in_folder.folder,
        fs::canonicalize(f.path("Kyoto"))
            .unwrap()
            .display()
            .to_string()
    );
}

#[test]
fn a_short_form_in_a_name_or_a_folder_name_counts() {
    // The student's decision 10/16: the folder "Chem" is "chemistry", from
    // 4 characters on, in names and folder names only.
    let f = Fixture::new();
    write(
        &f,
        "School/Chem/AP/final-project.txt",
        "a project on compounds\n",
    );
    write(&f, "School/phys-notes.txt", "forces\n");
    write(&f, "School/Bio/cells.txt", "cells\n");
    write(&f, "School/words.txt", "chem and phys are short\n");

    let view = search(&f, "chemistry physics biology");

    assert_eq!(
        names(&view),
        ["phys-notes.txt", "final-project.txt"],
        "\"Bio\" is too short; a short form in the text does not count"
    );
    let project = result(&view, "final-project.txt");
    assert_eq!(project.matches.len(), 1);
    assert_eq!(project.matches[0].place, MatchPlace::Folder);
    assert_eq!(project.matches[0].wording, "chemistry");
    assert_eq!(
        result(&view, "phys-notes.txt").matches[0].place,
        MatchPlace::Name
    );
}

#[test]
fn a_short_form_is_a_word_of_4_characters_that_starts_a_longer_word() {
    assert!(has_short_form("school/chem/ap", "chemistry"));
    assert!(has_short_form("ap calc ab.pdf", "calculus"));
    assert!(has_short_form("hist/notes.txt", "world history"));
    assert!(has_short_form("化学笔记", "化学笔记本"));
    assert!(
        !has_short_form("化学", "化学反应"),
        "2 characters (6 bytes)"
    );
    assert!(!has_short_form("bio/cells.txt", "biology"), "3 characters");
    assert!(
        !has_short_form("biochem.txt", "chemistry"),
        "the start of a word"
    );
    assert!(
        !has_short_form("world/maps.pdf", "world history"),
        "a whole word is not shortened"
    );
    assert!(!has_short_form("2025 return.pdf", "2025-03-16"), "a number");
    assert!(
        !has_short_form("chemistry.pdf", "chem"),
        "longer words are found as they are"
    );
}

#[test]
fn a_file_in_two_authorized_folders_one_inside_the_other_is_listed_once() {
    // Walked from the inner folder, the file's path sorts first, so that
    // hit is kept and takes the Memory record from the outer one.
    let f = Fixture::new();
    write(&f, "Zoo/Trip/zebra-trip.txt", "zebra\n");
    let file = f.register("Zoo/Trip/zebra-trip.txt");
    f.start_memory(&file.file_id).unwrap();
    let chain_id = f.service().memory_records().unwrap()[0].chain_id.clone();
    opened_ago(&f, "Zoo/Trip/zebra-trip.txt", DAY_MS);
    let alone = search(&f, "zebra");

    f.service().authorize_root(&f.path("Zoo")).unwrap();
    let view = search(&f, "zebra");

    assert_eq!(names(&view), ["zebra-trip.txt"]);
    assert_eq!(view.matched, 1);
    let found = result(&view, "zebra-trip.txt");
    assert_eq!(
        found.memory_chain_id,
        Some(chain_id),
        "the record kept under the outer folder"
    );
    let (once, both) = (alone.coverage.unwrap(), view.coverage.unwrap());
    assert_eq!(both.folders, 2);
    assert_eq!(both.files, once.files, "each file counted once");
    assert_eq!(both.texts_searched, once.texts_searched);
    assert_eq!(names(&recent(&f)), ["zebra-trip.txt"]);
}

#[test]
fn nested_folders_merge_what_each_visit_of_a_file_found() {
    // Organize described the file under the inner folder; only the outer
    // folder's path has "Trip". Both count, for one result.
    let f = Fixture::new();
    write(&f, "Zoo/Trip/zebra.txt", "x\n");
    let inner = f
        .service()
        .authorize_root(&f.path("Zoo/Trip"))
        .unwrap()
        .root_id;
    let file = f
        .service()
        .register_file(&inner, &f.path("Zoo/Trip/zebra.txt"))
        .unwrap()
        .file;
    describe(&f, &file.file_id, "Notes from a safari");

    let view = search(&f, "trip safari");

    assert_eq!(names(&view), ["zebra.txt"]);
    let found = result(&view, "zebra.txt");
    assert_eq!(
        found
            .matches
            .iter()
            .map(|m| (m.wording.as_str(), m.place))
            .collect::<Vec<_>>(),
        [
            ("trip", MatchPlace::Folder),
            ("safari", MatchPlace::Description)
        ]
    );
    assert!(found.described);
    assert_eq!(view.coverage.as_ref().unwrap().described, 1);

    let view = search(&f, "trip");
    assert!(
        result(&view, "zebra.txt").described,
        "the description kept under the inner folder, though only the outer visit matched"
    );
}

#[test]
fn hidden_items_packages_projects_and_links_are_left_out() {
    let f = Fixture::new();
    // The sample tree has plan-link.md, a link to Notes/2026/plan.md.
    write(&f, ".secret-ship.txt", "ship\n");
    write(&f, ".hidden/ship.txt", "ship\n");
    write(&f, "Tool.app/ship.txt", "ship\n");
    write(&f, "code/package.json", "{}\n");
    write(&f, "code/ship.txt", "ship\n");

    let view = search(&f, "ship");

    assert_eq!(names(&view), ["plan.md"], "only the real file, once");
}

#[test]
fn only_active_authorized_folders_are_searched() {
    let f = Fixture::new();
    let folder = |prefix| tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    let outside = folder("traeva-outside-");
    fs::write(outside.path().join("zebra.txt"), "zebra\n").unwrap();
    let second = folder("traeva-second-");
    fs::write(second.path().join("zebra-notes.txt"), "zebra\n").unwrap();
    let second_root = f.service().authorize_root(second.path()).unwrap().root_id;

    let view = search(&f, "zebra");
    assert_eq!(names(&view), ["zebra-notes.txt"]);
    assert_eq!(view.coverage.as_ref().unwrap().folders, 2);

    f.service().revoke_root(&second_root).unwrap();
    let view = search(&f, "zebra");
    assert!(view.results.is_empty());
    assert_eq!(view.coverage.as_ref().unwrap().folders, 1);
}

#[test]
fn organize_descriptions_are_searched_and_marked_when_outdated() {
    let f = Fixture::new();
    write(&f, "scan_001.txt", "Total 18,400\n");
    let file = f.register("scan_001.txt");
    describe(
        &f,
        &file.file_id,
        "A receipt from a ryokan in Kyoto, paid by card.",
    );

    let view = search(&f, "ryokan receipt");
    let found = result(&view, "scan_001.txt");
    assert!(found.described);
    assert!(!found.description_outdated);
    assert!(
        found
            .matches
            .iter()
            .all(|m| m.place == MatchPlace::Description)
    );
    assert_eq!(
        found.matches[0].snippet.as_deref(),
        Some("A receipt from a ryokan in Kyoto, paid by card.")
    );
    assert_eq!(view.coverage.as_ref().unwrap().described, 1);

    fs::write(f.path("scan_001.txt"), "Total 18,400 (tip added)\n").unwrap();
    let view = search(&f, "ryokan");
    let found = result(&view, "scan_001.txt");
    assert!(
        found.description_outdated,
        "the file changed after it was described"
    );
}

#[test]
fn a_file_memory_records_links_to_its_record() {
    let f = Fixture::new();
    write(&f, "essay.txt", "The conclusion is near.\n");
    let file = f.register("essay.txt");
    f.start_memory(&file.file_id).unwrap();
    let chain_id = f.service().memory_records().unwrap()[0].chain_id.clone();

    let view = search(&f, "conclusion");

    assert_eq!(result(&view, "essay.txt").memory_chain_id, Some(chain_id));
}

#[test]
fn a_search_writes_nothing() {
    let f = Fixture::new();
    write(&f, "notes.txt", "kyoto\n");
    let file = f.register("notes.txt");
    describe(&f, &file.file_id, "kyoto notes");
    let tree = snapshot::snapshot(f.root.path());
    let database = database_bytes(&f);

    let view = search(&f, "kyoto");
    assert_eq!(names(&view), ["notes.txt"]);
    f.service().find_result_path(&view.find_id, "R1").unwrap();

    assert_eq!(
        snapshot::snapshot(f.root.path()),
        tree,
        "nothing in the folder changed"
    );
    assert_eq!(
        database_bytes(&f),
        database,
        "nothing was written to the database"
    );
}

// ---------------------------------------------------------------------------
// Limits and what was searched
// ---------------------------------------------------------------------------

#[test]
fn large_files_match_by_name_only() {
    let f = Fixture::new();
    let big = fs::File::create(f.path("kyoto-video-notes.txt")).unwrap();
    big.set_len(MAX_FILE_BYTES + 1).unwrap();

    let view = search(&f, "kyoto");

    assert_eq!(names(&view), ["kyoto-video-notes.txt"]);
    assert_eq!(view.coverage.as_ref().unwrap().texts_too_large, 1);
}

#[test]
fn the_file_limit_and_the_time_limit_are_reported() {
    let f = Fixture::new();
    for i in 0..5 {
        write(&f, &format!("Many/kyoto-{i}.txt"), "kyoto\n");
    }

    let few = FindLimits {
        max_files: 3,
        ..LIMITS
    };
    let view = search_limited(&f, "kyoto", &mut no_model, few);
    let coverage = view.coverage.as_ref().unwrap();
    assert!(coverage.file_limit_reached);
    assert_eq!(coverage.files, 3);

    let no_time = FindLimits {
        text_time: Duration::ZERO,
        ..LIMITS
    };
    write(&f, "later.txt", "zebra\n");
    let view = search_limited(&f, "zebra", &mut no_model, no_time);
    assert!(view.results.is_empty(), "its text was never read");
    assert!(view.coverage.as_ref().unwrap().texts_not_reached >= 1);
}

#[test]
fn texts_read_once_are_kept_for_the_next_search() {
    let f = Fixture::new();
    write(&f, "notes.txt", "zebra crossing\n");
    let first = search(&f, "zebra");
    assert_eq!(names(&first), ["notes.txt"]);

    let no_time = FindLimits {
        text_time: Duration::ZERO,
        ..LIMITS
    };
    let again = search_limited(&f, "crossing", &mut no_model, no_time);
    assert_eq!(names(&again), ["notes.txt"], "the kept text is searched");

    fs::write(f.path("notes.txt"), "zebra crossing, edited\n").unwrap();
    let changed = search_limited(&f, "crossing", &mut no_model, no_time);
    assert!(
        changed.results.is_empty(),
        "a changed file is read again, not taken from memory"
    );
}

#[test]
fn a_pdf_without_the_reader_matches_by_name() {
    let f = Fixture::new();
    write(&f, "kyoto-map.pdf", "%PDF-1.4 not really\n");

    let view = search(&f, "kyoto");

    assert_eq!(result(&view, "kyoto-map.pdf").kind, Some(FileKind::Pdf));
    assert_eq!(view.coverage.as_ref().unwrap().texts_unreadable, 1);
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[test]
fn the_model_gets_only_the_question_and_its_words_are_used() {
    let f = Fixture::new();
    write(&f, "Travel/notes_0912.txt", "Kyoto hotel budget\n");
    let file = f.register("Travel/notes_0912.txt");
    describe(&f, &file.file_id, "Secret description words");
    model_set_up(&f, true);
    let mut sent: Vec<(String, String)> = Vec::new();
    let mut transport =
        |request: &OutgoingRequest, still_allowed: &mut dyn FnMut(&Recipient) -> bool| {
            assert!(still_allowed(&recipient()));
            sent.push((request.instructions.to_string(), request.input.to_string()));
            Ok(Some(answer(json!({
                "things": [["京都", "Kyoto"], ["预算", "budget"]],
                "related": [],
                "kinds": [],
                "when": null
            }))))
        };

    let view = search_with(&f, "我在京都的预算", &mut transport);

    assert_eq!(sent.len(), 1);
    let (instructions, input) = &sent[0];
    assert_eq!(
        serde_json::from_str::<Value>(input).unwrap(),
        json!({ "question": "我在京都的预算" })
    );
    for private in [
        "notes_0912",
        "Travel",
        "Kyoto hotel",
        "Secret description",
        "traeva-root",
    ] {
        assert!(
            !input.contains(private) && !instructions.contains(private),
            "{private} was sent"
        );
    }
    assert_eq!(view.words_by, Some(WordsBy::Model));
    let found = result(&view, "notes_0912.txt");
    assert_eq!(found.matches.len(), 2);
    assert_eq!(found.matches[0].wording, "Kyoto");
}

#[test]
fn kinds_and_the_time_range_from_the_model_filter_the_files() {
    let f = Fixture::new();
    // Put in their folder 30 days before the fixture's now, whatever day the
    // test runs: outside the week asked for.
    let now = clock_ahead_of_real(&f, 30 * DAY_MS);
    write(&f, "kyoto-new.md", "x\n");
    write(&f, "kyoto-old.md", "x\n");
    write(&f, "kyoto-new.txt", "x\n");
    set_times(&f, "kyoto-new.md", now - 2 * DAY_MS, None);
    set_times(&f, "kyoto-old.md", now - 40 * DAY_MS, None);
    set_times(&f, "kyoto-new.txt", now - 2 * DAY_MS, None);
    model_set_up(&f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": [["kyoto"]],
            "related": [],
            "kinds": ["markdown"],
            "when": { "newest": 0, "oldest": 1, "unit": "week" }
        }))))
    };

    let view = search_with(&f, "the kyoto markdown note from this week", &mut transport);

    assert_eq!(names(&view), ["kyoto-new.md"]);
}

// ---------------------------------------------------------------------------
// Time ranges (the student's decisions 10/17)
// ---------------------------------------------------------------------------

/// Sets the fixture clock `ahead_ms` after the real clock, so a file written
/// now was put in its folder `ahead_ms` ago (that time cannot be set by
/// hand). Returns the fixture's now.
fn clock_ahead_of_real(f: &Fixture, ahead_ms: i64) -> i64 {
    let real = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let now = real + ahead_ms;
    f.advance(now - START_MS);
    now
}

/// Sets the file's modification time, and its "last opened" record when
/// given, both in milliseconds since 1970.
fn set_times(f: &Fixture, relative: &str, modified_ms: i64, opened_ms: Option<i64>) {
    fs::File::options()
        .write(true)
        .open(f.path(relative))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_millis(modified_ms as u64))
        .unwrap();
    if let Some(at) = opened_ms {
        let mut value = at.div_euclid(1000).to_le_bytes().to_vec();
        value.extend_from_slice(&(at.rem_euclid(1000) * 1_000_000).to_le_bytes());
        xattr::set(f.path(relative), "com.apple.lastuseddate#PS", &value).unwrap();
    }
}

/// A search whose model answers `things` and the time range `when`.
fn search_at(f: &Fixture, things: Value, when: Value) -> FindView {
    model_set_up(f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": things.clone(),
            "related": vec![json!([]); things.as_array().unwrap().len()],
            "kinds": [],
            "when": when.clone()
        }))))
    };
    search_with(f, "a question", &mut transport)
}

#[test]
fn a_file_organized_two_days_ago_is_found_by_when_it_was_put_in_its_folder() {
    // Written long ago, moved into Physics two days ago: its modification
    // time does not say "two days ago", the time it was put there does.
    let f = Fixture::new();
    let now = clock_ahead_of_real(&f, 2 * DAY_MS);
    write(&f, "Physics/lab-report.txt", "pendulum lab\n");
    set_times(&f, "Physics/lab-report.txt", now - 40 * DAY_MS, None);

    let view = search_at(
        &f,
        json!([["lab report", "lab"]]),
        json!({ "newest": 1, "oldest": 3, "unit": "day" }),
    );

    let found = result(&view, "lab-report.txt");
    assert_eq!(found.time_found, Some(FileTime::Added));
    let added = found.added_at_ms.unwrap();
    assert!((now - 2 * DAY_MS - added).abs() < 10 * 60_000, "{added}");
}

#[test]
fn any_of_the_three_times_can_match_and_the_rest_are_left_out() {
    let f = Fixture::new();
    // Put in their folders 2 days ago: outside the 10 to 20 days asked for.
    let now = clock_ahead_of_real(&f, 2 * DAY_MS);
    for name in ["modified.txt", "opened.txt", "neither.txt"] {
        write(&f, &format!("Lab/{name}"), "lab\n");
    }
    set_times(&f, "Lab/modified.txt", now - 15 * DAY_MS, None);
    set_times(
        &f,
        "Lab/opened.txt",
        now - 40 * DAY_MS,
        Some(now - 15 * DAY_MS),
    );
    set_times(
        &f,
        "Lab/neither.txt",
        now - 40 * DAY_MS,
        Some(now - 40 * DAY_MS),
    );

    let view = search_at(
        &f,
        json!([["lab"]]),
        json!({ "newest": 10, "oldest": 20, "unit": "day" }),
    );

    assert_eq!(names(&view), ["modified.txt", "opened.txt"]);
    assert_eq!(
        result(&view, "modified.txt").time_found,
        Some(FileTime::Modified)
    );
    assert_eq!(
        result(&view, "opened.txt").time_found,
        Some(FileTime::Opened)
    );
}

#[test]
fn minutes_count() {
    // Written 20 minutes before the fixture's now.
    let f = Fixture::new();
    clock_ahead_of_real(&f, 20 * 60_000);
    write(&f, "Notes/draft.txt", "lab\n");

    let half_an_hour = search_at(
        &f,
        json!([["lab"]]),
        json!({ "newest": 15, "oldest": 60, "unit": "minute" }),
    );
    let a_few_minutes = search_at(
        &f,
        json!([["lab"]]),
        json!({ "newest": 0, "oldest": 15, "unit": "minute" }),
    );

    assert_eq!(names(&half_an_hour), ["draft.txt"]);
    assert!(a_few_minutes.results.is_empty());
}

#[test]
fn a_time_alone_lists_the_files_of_that_time() {
    let f = Fixture::new();
    let now = clock_ahead_of_real(&f, 0);
    write(&f, "Inbox/new.txt", "x\n");
    write(&f, "Inbox/old.txt", "x\n");
    set_times(&f, "Inbox/old.txt", now - 400 * DAY_MS, None);

    let view = search_at(
        &f,
        json!([]),
        json!({ "newest": 0, "oldest": 15, "unit": "minute" }),
    );

    assert!(names(&view).contains(&"new.txt"), "{:?}", names(&view));
    assert!(
        names(&view).contains(&"old.txt"),
        "old.txt was also written into its folder just now"
    );
}

#[test]
fn the_model_s_time_range_is_checked() {
    let range = |newest: i64, oldest: i64, unit: &str| {
        time_ago_from(&json!({ "newest": newest, "oldest": oldest, "unit": unit }))
    };
    let ago = |newest, oldest, unit| {
        Some(Some(TimeAgo {
            newest,
            oldest,
            unit,
        }))
    };
    assert_eq!(time_ago_from(&Value::Null), Some(None));
    assert_eq!(range(1, 3, "day"), ago(1, 3, TimeUnit::Day));
    assert_eq!(range(2, 2, "day"), ago(1, 3, TimeUnit::Day), "widened");
    assert_eq!(range(0, 0, "minute"), ago(0, 1, TimeUnit::Minute));
    assert_eq!(range(3, 1, "day"), None, "newest after oldest");
    assert_eq!(
        range(24, 26, "year"),
        Some(None),
        "more than 20 years back: searched without a time"
    );
    assert_eq!(range(0, 300_000_000, "year"), Some(None), "no overflow");
    assert_eq!(range(0, 4_294_967_295, "month"), Some(None));
    assert_eq!(range(-1, 3, "day"), None);
    assert_eq!(range(0, 3, "decade"), None);
    assert_eq!(time_ago_from(&json!("yesterday")), None);
}

#[test]
fn no_things_lists_every_file_of_the_kinds_asked_for() {
    let f = Fixture::new();
    write(&f, "a.md", "x\n");
    model_set_up(&f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(
            json!({ "things": [], "related": [], "kinds": ["markdown"], "when": null }),
        )))
    };

    let view = search_with(&f, "my markdown notes", &mut transport);

    let mut found = names(&view);
    found.sort_unstable();
    assert_eq!(found, ["a.md", "plan.md"]);
}

#[test]
fn without_find_s_agreement_the_model_is_not_asked() {
    let f = Fixture::new();
    model_set_up(&f, false);
    f.service().set_analysis_consent(true).unwrap();
    write(&f, "zebra.txt", "x\n");

    let view = search(&f, "zebra");

    assert_eq!(
        view.words_by,
        Some(WordsBy::Question {
            reason: "Searching with the model is not turned on.".to_string()
        }),
        "Organize's agreement does not count for Find"
    );
    assert_eq!(names(&view), ["zebra.txt"]);
}

#[test]
fn a_failed_or_unusable_answer_falls_back_to_the_question_s_words() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");
    model_set_up(&f, true);

    let mut failing = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Err(SendFailure {
            code: ErrorCode::ModelTimeout,
            retryable: true,
        })
    };
    let view = search_with(&f, "zebra", &mut failing);
    assert_eq!(names(&view), ["zebra.txt"]);
    assert!(matches!(
        view.words_by,
        Some(WordsBy::Question { ref reason }) if reason.starts_with("The model did not answer")
    ));

    let mut unusable = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": [["a1"], ["a2"], ["a3"], ["a4"], ["a5"], ["a6"], ["a7"]],
            "related": [],
            "kinds": [],
            "when": null
        }))))
    };
    let view = search_with(&f, "zebra", &mut unusable);
    assert_eq!(names(&view), ["zebra.txt"]);
    assert_eq!(
        view.words_by,
        Some(WordsBy::Question {
            reason: "The model's answer could not be used.".to_string()
        })
    );
}

#[test]
fn an_answer_after_the_agreement_was_withdrawn_is_not_used() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");
    model_set_up(&f, true);
    let service = f.service();
    let mut transport = |_: &OutgoingRequest, still_allowed: &mut dyn FnMut(&Recipient) -> bool| {
        assert!(still_allowed(&recipient()));
        service.set_find_consent(false).unwrap(); // while the model works
        Ok(Some(answer(
            json!({ "things": [["giraffe"]], "related": [], "kinds": [], "when": null }),
        )))
    };

    let view = search_with(&f, "zebra", &mut transport);

    assert_eq!(
        view.words_by,
        Some(WordsBy::Question {
            reason: "Searching with the model was turned off during the search.".to_string()
        })
    );
    assert_eq!(names(&view), ["zebra.txt"]);
}

#[test]
fn a_cancelled_or_replaced_search_drops_what_comes_late() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");
    model_set_up(&f, true);
    let service = f.service();

    let first = service.start_find("zebra").unwrap();
    let mut transport = |_: &OutgoingRequest, still_allowed: &mut dyn FnMut(&Recipient) -> bool| {
        service.cancel_find(&first.find_id).unwrap();
        assert!(
            !still_allowed(&recipient()),
            "nothing goes out for a cancelled search"
        );
        Ok(None)
    };
    let view = service
        .run_find_with(&first.find_id, &mut transport, LIMITS)
        .unwrap();
    assert_eq!(view.state, FindState::Cancelled);
    assert!(view.results.is_empty());

    let older = service.start_find("zebra").unwrap();
    let newer = service.start_find("zebra").unwrap();
    assert_eq!(
        service.find(&older.find_id).unwrap().state,
        FindState::Cancelled
    );
    let late = service
        .run_find_with(&older.find_id, &mut no_model, LIMITS)
        .unwrap();
    assert_eq!(late.state, FindState::Cancelled);
    assert!(late.results.is_empty());
    assert_eq!(
        service.find(&newer.find_id).unwrap().state,
        FindState::Working
    );
}

// ---------------------------------------------------------------------------
// Showing and opening a result
// ---------------------------------------------------------------------------

#[test]
fn a_result_s_path_is_given_only_while_the_same_file_is_there() {
    let f = Fixture::new();
    write(&f, "Trip/zebra.txt", "x\n");
    let view = search(&f, "zebra");

    let path = f.service().find_result_path(&view.find_id, "R1").unwrap();
    assert_eq!(path, fs::canonicalize(f.path("Trip/zebra.txt")).unwrap());

    fs::remove_file(f.path("Trip/zebra.txt")).unwrap();
    fs::write(f.path("Trip/zebra.txt"), "a new file at the same place\n").unwrap();
    let error = f
        .service()
        .find_result_path(&view.find_id, "R1")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanged);

    let error = f
        .service()
        .find_result_path(&view.find_id, "R9")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

#[test]
fn a_question_must_have_1_to_200_characters() {
    let f = Fixture::new();
    for question in ["", "   ", &"x".repeat(201)] {
        let error = f.service().start_find(question).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }
}

#[test]
fn the_question_s_own_words() {
    let words = question_words("Find the CV, my cv and 税 file a b 1 2 3 4 5 6 7");
    assert_eq!(
        words.things,
        vec![vec!["CV".to_string()], vec!["税".to_string()]],
        "common words, one ASCII letter or digit and repeats are left out"
    );
    assert_eq!(
        words.related,
        vec![Vec::<String>::new(); 2],
        "no related words"
    );
    let many = question_words("one two three four five six seven");
    assert_eq!(many.things.len(), 6);
}

#[test]
fn the_model_s_answer_is_checked() {
    let good = json!({
        "things": [["Kyoto", "kyoto", " 京都 "]],
        "related": [],
        "kinds": ["pdf", "pdf"],
        "when": { "newest": 0, "oldest": 1, "unit": "day" }
    });
    let words = words_from(&good).unwrap();
    assert_eq!(
        words.things,
        vec![vec!["Kyoto".to_string(), "京都".to_string()]]
    );
    assert_eq!(words.kinds, vec![FileKind::Pdf]);
    assert_eq!(
        words.when,
        Some(TimeAgo {
            newest: 0,
            oldest: 1,
            unit: TimeUnit::Day
        })
    );

    for bad in [
        json!({ "things": [["a1"]], "related": [], "kinds": [], "when": null, "extra": 1 }),
        json!({ "things": [["ab", "cd", "ef", "gh", "ij", "kl"]], "related": [], "kinds": [], "when": null }),
        json!({ "things": [[]], "related": [], "kinds": [], "when": null }),
        json!({ "things": [["x".repeat(41)]], "related": [], "kinds": [], "when": null }),
        json!({ "things": [["a\nb"]], "related": [], "kinds": [], "when": null }),
        json!({ "things": [], "related": [], "kinds": ["image"], "when": null }),
        json!({ "things": [], "related": [], "kinds": [], "when": { "newest": 3, "oldest": 1, "unit": "day" } }),
        json!({ "things": [["a1"]], "kinds": [], "when": null }),
    ] {
        assert_eq!(words_from(&bad), None, "{bad}");
    }
}

#[test]
fn related_words_are_kept_or_left_out_one_by_one() {
    let words = words_from(&json!({
        "things": [["chemistry", "化学"], ["  "], ["kyoto"]],
        "related": [
            ["reaction", "Mole", "mole", "ab", "Chemistry", "x".repeat(41), "a\nb", "化学反应",
             "酸", "acid", "molecule", "titration", "ion", "bond", "electron"],
            ["of the dropped thing"],
        ],
        "kinds": [],
        "when": null
    }))
    .unwrap();

    assert_eq!(words.things, [vec!["chemistry", "化学"], vec!["kyoto"]]);
    assert_eq!(
        words.related,
        [
            vec![
                "reaction",
                "Mole",
                "化学反应",
                "acid",
                "molecule",
                "titration",
                "ion",
                "bond"
            ],
            vec![],
        ],
        "repeats, wordings, too short, too long and control characters are left \
         out, at most 8; the dropped thing's list goes with it; a missing list is empty"
    );
}

#[test]
fn a_related_word_counts_at_the_start_of_a_word() {
    assert_eq!(word_start("ionic bonds", "ion"), Some(0));
    assert_eq!(word_start("a question, then ionic", "ion"), Some(17));
    assert_eq!(word_start("the moles of acid", "mole"), Some(4));
    assert_eq!(word_start("a question about a nation", "ion"), None);
    assert!(
        word_start("这是化学反应的练习", "化学反应").is_some(),
        "no spaces in Chinese"
    );
    assert!(
        word_start("求反应物的mole数", "mole").is_some(),
        "after a Chinese character"
    );
    assert_eq!(word_start("региона", "ион"), None, "Russian has spaces");
    assert_eq!(word_start("anything", ""), None);
}

#[test]
fn one_word_does_not_count_as_two_related_words() {
    // "mole" only starts "molecule", and 反应 is inside 化学反应: one word in
    // the file, so not the 2 related words a match needs.
    let f = Fixture::new();
    write(
        &f,
        "Bio/cell.txt",
        "Each water molecule crosses the membrane.\n",
    );
    write(&f, "Bio/lab.txt", "Water molecules and an acid.\n");
    write(&f, "Notes/react.txt", "这是化学反应。\n");
    model_set_up(&f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": [["chemistry"]],
            "related": [["molecule", "mole", "acid", "化学反应", "反应"]],
            "kinds": [],
            "when": null
        }))))
    };

    let view = search_with(&f, "chemistry", &mut transport);

    assert_eq!(names(&view), ["lab.txt"]);
    assert_eq!(
        result(&view, "lab.txt").matches[0].related_words,
        ["molecule", "mole", "acid"]
    );
}

#[test]
fn a_related_word_s_snippet_is_where_it_starts_a_word() {
    let f = Fixture::new();
    write(
        &f,
        "notes.txt",
        "Answer each question first.\nIonic bonds form between ions.\n",
    );
    model_set_up(&f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": [["chemistry"]],
            "related": [["ion", "bond"]],
            "kinds": [],
            "when": null
        }))))
    };

    let view = search_with(&f, "chemistry", &mut transport);

    let found = &result(&view, "notes.txt").matches[0];
    assert_eq!(found.wording, "ion");
    let shown = found.snippet.as_deref().unwrap();
    assert!(shown.contains("first. Ionic bonds"), "{shown}");
}

#[test]
fn a_file_with_2_related_words_matches_after_the_files_with_a_wording() {
    // The student's decision 10/16: a chemistry problem set that never says
    // "chemistry" is found through the words such a file uses.
    let f = Fixture::new();
    write(
        &f,
        "Homework/set-4.txt",
        "Balance the reaction, then find the moles of acid.\n",
    );
    write(&f, "Homework/words.txt", "chemistry is one of the words\n");
    write(&f, "Homework/one.txt", "a chemical reaction\n");
    write(
        &f,
        "Homework/quiz.txt",
        "Which question is about the nation?\n",
    );
    model_set_up(&f, true);
    let mut transport = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
        Ok(Some(answer(json!({
            "things": [["chemistry", "化学"]],
            "related": [["reaction", "mole", "acid", "ion", "化学反应"]],
            "kinds": [],
            "when": null
        }))))
    };

    let view = search_with(&f, "chemistry", &mut transport);

    assert_eq!(
        names(&view),
        ["words.txt", "set-4.txt"],
        "one related word is not enough; \"ion\" inside a word does not count"
    );
    let set = result(&view, "set-4.txt");
    assert_eq!(set.matches.len(), 1);
    let found = &set.matches[0];
    assert_eq!(found.place, MatchPlace::Text);
    assert_eq!(found.related_words, ["reaction", "mole", "acid"]);
    assert_eq!(found.wording, "reaction");
    assert!(
        found
            .snippet
            .as_deref()
            .unwrap()
            .contains("Balance the reaction")
    );
    assert!(
        result(&view, "words.txt").matches[0]
            .related_words
            .is_empty()
    );
}

#[test]
fn a_snippet_is_cut_around_the_match_on_one_line() {
    let text = format!("{}Café\n\nopens at nine{}", "a".repeat(80), "b".repeat(200));
    let folded = fold(&text);
    let shown = snippet(&text, &folded, folded.find(&fold("CAFE")).unwrap()).unwrap();
    assert!(shown.starts_with('…') && shown.ends_with('…'));
    assert!(shown.contains("Café opens at nine"));
    assert_eq!(
        shown.chars().filter(|c| *c == 'a').count(),
        SNIPPET_BEFORE + 2,
        "the 50 before, Café and at"
    );
}

#[test]
fn find_s_agreement_is_cleared_with_the_model_and_kept_apart() {
    let f = Fixture::new();
    model_set_up(&f, true);
    let setup = f.service().find_setup().unwrap();
    assert!(setup.model_ready);
    assert!(setup.consent_agreed_at_ms.is_some());
    assert!(
        f.service()
            .model_settings()
            .unwrap()
            .consent_agreed_at_ms
            .is_none(),
        "Organize's agreement is not given by Find's"
    );

    f.service()
        .save_model_settings(&openai("gpt-5.6-luna", None))
        .unwrap();
    assert!(
        f.service()
            .find_setup()
            .unwrap()
            .consent_agreed_at_ms
            .is_none()
    );
}

#[test]
fn without_settings_the_setup_says_so() {
    let f = Fixture::new();
    let setup = f.service().find_setup().unwrap();
    assert!(!setup.model_ready);
    assert_eq!(setup.consent_agreed_at_ms, None);
    assert!(setup.notice.contains("only what you type"));
    let error = f.service().set_find_consent(true).unwrap_err();
    assert_eq!(error.code, ErrorCode::ModelNotConfigured);
}

// ---------------------------------------------------------------------------
// Recent files (#198)
// ---------------------------------------------------------------------------

/// Records, as macOS does, that the file was last opened `ago_ms` before
/// the fixture clock: 16 bytes, seconds then nanoseconds, little-endian.
fn opened_ago(f: &Fixture, relative: &str, ago_ms: i64) {
    let at = START_MS - ago_ms;
    let mut value = at.div_euclid(1000).to_le_bytes().to_vec();
    value.extend_from_slice(&(at.rem_euclid(1000) * 1_000_000).to_le_bytes());
    xattr::set(f.path(relative), "com.apple.lastuseddate#PS", &value).unwrap();
}

fn recent(f: &Fixture) -> FindView {
    f.service().recent_files_with(LIMITS).unwrap()
}

#[test]
fn recent_files_are_the_ones_opened_in_the_last_30_days_newest_first() {
    let f = Fixture::new();
    for (name, days) in [("old.txt", 31), ("c.txt", 29), ("a.txt", 1), ("b.md", 5)] {
        write(&f, &format!("School/{name}"), "x\n");
        opened_ago(&f, &format!("School/{name}"), days * DAY_MS);
    }
    write(&f, "School/never.txt", "x\n");

    let view = recent(&f);

    assert_eq!(view.state, FindState::Done);
    assert_eq!(view.words, None);
    assert_eq!(names(&view), ["a.txt", "b.md", "c.txt"]);
    assert_eq!(view.matched, 3);
    let a = result(&view, "a.txt");
    assert_eq!(a.opened_at_ms, Some(START_MS - DAY_MS));
    assert_eq!(a.relative_path, "School/a.txt");
    assert!(a.matches.is_empty());
    assert!(view.coverage.as_ref().unwrap().files > 4);
}

#[test]
fn at_most_twenty_recent_files() {
    let f = Fixture::new();
    for n in 0..22 {
        let name = format!("Drafts/d{n:02}.txt");
        write(&f, &name, "x\n");
        opened_ago(&f, &name, n * 60_000);
    }
    let view = recent(&f);
    assert_eq!(view.results.len(), 20);
    assert_eq!(view.matched, 22);
    assert_eq!(view.results[0].name, "d00.txt");
    assert_eq!(view.results[19].name, "d19.txt");
}

#[test]
fn hidden_items_packages_projects_and_links_are_not_recent() {
    let f = Fixture::new();
    // The sample tree has plan-link.md, a link to Notes/2026/plan.md.
    for relative in [
        ".secret.txt",
        ".hidden/x.txt",
        "Tool.app/x.txt",
        "code/x.txt",
    ] {
        write(&f, relative, "x\n");
        opened_ago(&f, relative, DAY_MS);
    }
    write(&f, "code/package.json", "{}\n");
    opened_ago(&f, "Notes/2026/plan.md", DAY_MS);

    let view = recent(&f);

    assert_eq!(names(&view), ["plan.md"], "only the real file, once");
}

/// A file only in iCloud (dataless) is never opened, so nothing is
/// downloaded (`SF_DATALESS` cannot be set on a test file: the check is
/// covered on its flags).
#[test]
fn a_file_only_in_icloud_is_not_opened_for_its_record() {
    let seen = |kind: Kind, flags: u32| Observation {
        id: ObjectId { dev: 1, ino: 2 },
        kind,
        link_count: 1,
        size: 10,
        mtime_ns: 0,
        ctime_ns: 0,
        birthtime_ns: 0,
        flags,
    };
    assert!(may_read_last_opened(&seen(Kind::File, 0)));
    assert!(!may_read_last_opened(&seen(Kind::File, 0x4000_0000)));
    assert!(!may_read_last_opened(&seen(Kind::SymbolicLink, 0)));
}

#[test]
fn listing_recent_files_writes_nothing() {
    let f = Fixture::new();
    write(&f, "School/a.txt", "x\n");
    opened_ago(&f, "School/a.txt", DAY_MS);
    let tree = snapshot::snapshot(f.root.path());
    let database = database_bytes(&f);

    let view = recent(&f);

    assert_eq!(names(&view), ["a.txt"]);
    assert_eq!(snapshot::snapshot(f.root.path()), tree);
    assert_eq!(database_bytes(&f), database);
}

#[test]
fn opening_a_recent_file_checks_it_is_still_the_same_file() {
    let f = Fixture::new();
    write(&f, "School/a.txt", "x\n");
    opened_ago(&f, "School/a.txt", DAY_MS);
    let view = recent(&f);

    let path = f.service().find_result_path(&view.find_id, "R1").unwrap();
    assert_eq!(path, fs::canonicalize(f.path("School/a.txt")).unwrap());

    fs::remove_file(f.path("School/a.txt")).unwrap();
    write(&f, "School/a.txt", "x\n");
    let error = f
        .service()
        .find_result_path(&view.find_id, "R1")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileChanged);
}

#[test]
fn a_search_result_says_when_it_was_last_opened() {
    let f = Fixture::new();
    write(&f, "School/ship-log.txt", "x\n");
    write(&f, "School/ship-plan.txt", "x\n");
    opened_ago(&f, "School/ship-log.txt", 2 * DAY_MS);

    let view = search(&f, "ship");

    assert_eq!(
        result(&view, "ship-log.txt").opened_at_ms,
        Some(START_MS - 2 * DAY_MS)
    );
    assert_eq!(result(&view, "ship-plan.txt").opened_at_ms, None);
}

#[test]
fn recent_files_leave_a_search_under_way_alone() {
    let f = Fixture::new();
    let started = f.service().start_find("ship").unwrap();

    recent(&f);

    assert_eq!(
        f.service().find(&started.find_id).unwrap().state,
        FindState::Working
    );
}

#[test]
fn the_question_is_not_kept_after_the_run_takes_it() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");
    let view = search(&f, "zebra stripes");
    assert_eq!(names(&view), ["zebra.txt"]);
    let finds = f.service().finds();
    assert_eq!(finds[&view.find_id].question, "", "the text is gone");
}

#[test]
fn a_search_stopped_before_its_run_does_not_keep_the_question() {
    let f = Fixture::new();
    let replaced = f.service().start_find("zebra stripes").unwrap();
    let cancelled = f.service().start_find("lion mane").unwrap();
    f.service().cancel_find(&cancelled.find_id).unwrap();
    let finds = f.service().finds();
    for id in [&replaced.find_id, &cancelled.find_id] {
        assert_eq!(finds[id].state, FindState::Cancelled);
        assert_eq!(finds[id].question, "", "the text is gone");
    }
}

#[test]
fn a_question_with_nothing_to_look_for_lists_no_file() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");

    let view = search(&f, "find the file");

    assert_eq!(view.state, FindState::Done);
    assert!(view.words.as_ref().unwrap().things.is_empty());
    assert!(view.results.is_empty());
    assert_eq!(view.matched, 0);
    assert_eq!(view.coverage, None, "the files were not searched");
}

#[test]
fn a_search_that_cannot_read_the_database_says_why() {
    let f = Fixture::new();
    write(&f, "zebra.txt", "x\n");
    let started = f.service().start_find("zebra").unwrap();
    f.db()
        .execute_batch("ALTER TABLE analyses RENAME TO analyses_moved_away;")
        .unwrap();

    let view = f
        .service()
        .run_find_with(&started.find_id, &mut no_model, LIMITS)
        .unwrap();

    assert_eq!(view.state, FindState::Done);
    assert!(view.results.is_empty());
    assert!(view.problem.is_some(), "{view:?}");
}
