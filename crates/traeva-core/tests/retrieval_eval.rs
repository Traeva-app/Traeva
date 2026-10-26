//! Saves the #34 evaluation documents with Memory (#32's real capture), one
//! version at a time, and prints what was saved. It sends nothing: no
//! agreement is given and no model is asked, so no key is needed. It never
//! runs by itself:
//!   - it is `#[ignore]`d, and also needs `TRAEVA_MEMORY_EVAL=1` (without
//!     it, a run of the ignored tests skips this one);
//!   - `TRAEVA_APP_DATA_DIR` must name a separate test folder (#65);
//!   - `TRAEVA_MEMORY_EVAL_CORPUS` is the corpus: one folder per document,
//!     holding `v1.md`, `v2.md`, ... (`make_corpus.py` writes it);
//!   - `TRAEVA_MEMORY_EVAL_ROOT` is an empty folder outside the test folder.
//!     It is authorized, and each document is written into it as the file
//!     `<folder>.md`, then rewritten in place with each next version, the
//!     way an editor saves in place.
//!
//! After each write it waits until Memory's watcher has saved the new
//! version, then checks every saved version's SHA-256 against its file in
//! the corpus.
//!
//! ```text
//! TRAEVA_MEMORY_EVAL=1 TRAEVA_APP_DATA_DIR=… TRAEVA_MEMORY_EVAL_CORPUS=… \
//!   TRAEVA_MEMORY_EVAL_ROOT=… \
//!   cargo test -p traeva-core --test retrieval_eval save_ -- --ignored --nocapture
//! ```
//!
//! `answer_the_held_out_questions` is the #34 evaluation in one run: it
//! saves the documents the same way into a new empty folder, gives Memory's
//! agreement for them, has the real model describe every pair, then asks
//! each held-out question once. It does not answer a question the search
//! asks back: that question is the outcome. It also needs:
//!   - `TRAEVA_MEMORY_EVAL_QUESTIONS`: the student's file, one question per
//!     line as `document.md | question | expected answer`;
//!   - `TRAEVA_MEMORY_EVAL_SHA256`: the SHA-256 the student posted on #34.
//!     The file must match it before anything is saved or sent;
//!   - a key in the test folder's own Keychain account, saved by the
//!     student (as for `changes_live`).
//!
//! At most `BUDGET` requests go out, or `TRAEVA_MEMORY_EVAL_BUDGET` if lower.
//!
//! ```text
//! TRAEVA_MEMORY_EVAL=1 TRAEVA_APP_DATA_DIR=… TRAEVA_MEMORY_EVAL_CORPUS=… \
//!   TRAEVA_MEMORY_EVAL_ROOT=… TRAEVA_MEMORY_EVAL_QUESTIONS=… \
//!   TRAEVA_MEMORY_EVAL_SHA256=… \
//!   cargo test -p traeva-core --test retrieval_eval answer_ -- --ignored --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use traeva_core::{
    AppDataSource, CheckOutcome, ModelSettingsInput, OrganizeService, PairState, PassStep,
    SearchCandidate, SearchState, SearchView, StorageStatus, VersionPairView,
};

/// Requests the evaluation may send, all in all (the student's limit for
/// one milestone).
const BUDGET: usize = 20;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

#[test]
#[ignore = "saves the evaluation documents in a test folder; see the module docs"]
fn save_the_evaluation_documents_with_memory() {
    if std::env::var("TRAEVA_MEMORY_EVAL").as_deref() != Ok("1") {
        println!("skipped: TRAEVA_MEMORY_EVAL is not 1, so nothing was saved");
        return;
    }
    let service = start_in_test_folder();
    save_corpus(&service);
}

/// A service on the separate test folder, never the normal one.
fn start_in_test_folder() -> OrganizeService {
    env(traeva_core::APP_DATA_DIR_VAR);
    let service = OrganizeService::start_for_app();
    let storage = service.get_app_state().unwrap().storage;
    let StorageStatus::Ready(info) = &storage else {
        panic!("storage is not ready: {storage:?}");
    };
    assert_eq!(
        info.app_data_source,
        AppDataSource::Test,
        "not a separate test folder"
    );
    println!("app data: {}", info.app_data_dir);
    service
}

/// Writes every document of the corpus into the empty root, version by
/// version, and checks what Memory saved. Returns each file's name and ID.
fn save_corpus(service: &OrganizeService) -> Vec<(String, String)> {
    let corpus = PathBuf::from(env("TRAEVA_MEMORY_EVAL_CORPUS"));
    let root = PathBuf::from(env("TRAEVA_MEMORY_EVAL_ROOT"));
    assert!(
        fs::read_dir(&root).unwrap().next().is_none(),
        "{} must be empty",
        root.display()
    );
    let root_id = service.authorize_root(&root).unwrap().root_id;

    let mut documents: Vec<PathBuf> = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    documents.sort();
    let mut files = Vec::new();
    for folder in documents {
        let versions = versions_in(&folder);
        let name = format!("{}.md", folder.file_name().unwrap().to_string_lossy());
        let file = root.join(&name);
        fs::write(&file, fs::read(&versions[0]).unwrap()).unwrap();
        let file_id = service.register_file(&root_id, &file).unwrap().file.file_id;
        service.start_memory(&file_id).unwrap();
        for version in &versions[1..] {
            // Never two writes within one watcher pass.
            thread::sleep(Duration::from_millis(1100));
            fs::write(&file, fs::read(version).unwrap()).unwrap();
            wait_until_saved(service);
        }

        let saved = service.list_snapshots(&file_id).unwrap();
        assert_eq!(
            saved.len(),
            versions.len(),
            "{name}: one saved version each"
        );
        println!("\n{name} (file_id {file_id})");
        println!("| version | snapshot_id | sha256 | matches the corpus |");
        println!("|---|---|---|---|");
        for (snapshot, version) in saved.iter().zip(&versions) {
            let expected = traeva_core::read_text("x.md", &fs::read(version).unwrap());
            let bytes = service.read_snapshot(&snapshot.snapshot_id).unwrap().1;
            let same = bytes == fs::read(version).unwrap();
            assert!(
                same,
                "{name} v{} differs from the corpus",
                snapshot.sequence
            );
            assert!(expected.summary.status == traeva_core::TextStatus::Complete);
            println!(
                "| v{} | {} | {} | {} |",
                snapshot.sequence, snapshot.snapshot_id, snapshot.sha256, same
            );
        }
        files.push((name, file_id));
    }
    files
}

/// One held-out question: `document.md | question | expected answer`.
struct Question {
    document: String,
    text: String,
    expected: String,
}

fn parse_questions(text: &str) -> Vec<Question> {
    let questions: Vec<Question> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(n, line)| {
            let parts: Vec<&str> = line.splitn(3, '|').map(str::trim).collect();
            assert!(
                parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty(),
                "line {} is not `document.md | question | expected`",
                n + 1
            );
            Question {
                document: parts[0].trim_end_matches(".md").to_string(),
                text: parts[1].to_string(),
                expected: parts.get(2).copied().unwrap_or("").to_string(),
            }
        })
        .collect();
    assert!(!questions.is_empty(), "no question in the file");
    questions
}

#[test]
#[ignore = "sends to the real model; see the module docs"]
fn answer_the_held_out_questions() {
    if std::env::var("TRAEVA_MEMORY_EVAL").as_deref() != Ok("1") {
        println!("skipped: TRAEVA_MEMORY_EVAL is not 1, so nothing was saved or sent");
        return;
    }
    // The commitment first: nothing is saved or sent for another file.
    let bytes = fs::read(env("TRAEVA_MEMORY_EVAL_QUESTIONS")).unwrap();
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        actual,
        env("TRAEVA_MEMORY_EVAL_SHA256").trim().to_lowercase(),
        "the questions file is not the one committed on #34"
    );
    println!("questions file SHA-256 matches the commitment on #34: {actual}");
    let questions = parse_questions(std::str::from_utf8(&bytes).unwrap());
    let budget = std::env::var("TRAEVA_MEMORY_EVAL_BUDGET")
        .map(|left| left.parse::<usize>().expect("a number of requests"))
        .unwrap_or(BUDGET)
        .min(BUDGET);
    println!("{} questions, budget {budget} requests", questions.len());

    let service = start_in_test_folder();
    let files = save_corpus(&service);
    let file_of = |document: &str| {
        files
            .iter()
            .find(|(name, _)| name.trim_end_matches(".md") == document)
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| panic!("no document named {document}.md in the corpus"))
    };
    // Every question names a document of the corpus, before anything is sent.
    for question in &questions {
        file_of(&question.document);
    }

    // Provider and model (the key stays the one in the Keychain); a settings
    // change ends a hold left by an earlier refused key (see changes_live);
    // only this run's files may be sent.
    service
        .save_model_settings(&ModelSettingsInput {
            provider: traeva_core::PROVIDER_OPENAI.to_string(),
            model: traeva_core::DEFAULT_MODEL.to_string(),
            api_key: None,
        })
        .unwrap();
    service.set_analysis_consent(false).unwrap();
    let ours: Vec<&String> = files.iter().map(|(_, id)| id).collect();
    for watch in service.memory_watches().unwrap() {
        if !ours.contains(&&watch.file_id) && service.memory_consent(&watch.file_id).unwrap().agreed
        {
            service.set_memory_consent(&watch.file_id, false).unwrap();
        }
    }
    for id in &ours {
        service.set_memory_consent(id, true).unwrap();
    }

    // Every pair described, one per pass; a failure is tried again later.
    let mut sent = 0;
    let mut held = None;
    let started = Instant::now();
    while sent < budget && started.elapsed() < Duration::from_secs(900) {
        let steps = service.analyze_memory().unwrap();
        sent += steps
            .iter()
            .filter(|step| matches!(step, PassStep::Described { .. }))
            .count();
        let pairs: Vec<VersionPairView> = ours
            .iter()
            .flat_map(|id| service.memory_changes(id).unwrap())
            .collect();
        if pairs.iter().all(settled) {
            break;
        }
        held = pairs.iter().find_map(|pair| match &pair.state {
            PairState::Failed {
                error_code,
                will_retry: false,
            } => Some(*error_code),
            _ => None,
        });
        if let Some(code) = held {
            println!("stopped: {code:?} holds every request until the settings change");
            break;
        }
        if steps.is_empty() {
            thread::sleep(Duration::from_secs(5));
        }
    }
    println!("\ndescriptions: {sent} requests");
    for (name, id) in &files {
        println!("\n{name}");
        for pair in service.memory_changes(id).unwrap() {
            println!(
                "v{} → v{}: {:?}",
                pair.before.sequence, pair.after.sequence, pair.state
            );
            for change in &pair.changes {
                println!(
                    "  {:?} [{}] {}",
                    change.action, change.topic, change.description
                );
            }
        }
    }

    println!("\n| # | document | question | expected | result | coverage |");
    println!("|---|---|---|---|---|---|");
    for (n, question) in questions.iter().enumerate() {
        let n = n + 1;
        if let Some(code) = held {
            println!("| Q{n} | | | | not asked: {code:?} | |");
            continue;
        }
        if sent >= budget {
            println!("| Q{n} | | | | not asked: budget used up | |");
            continue;
        }
        let mut view = service
            .start_memory_search(&file_of(&question.document), &question.text)
            .unwrap();
        while view.state == SearchState::Working && sent < budget {
            sent += 1;
            view = service.run_memory_search(&view.search_id).unwrap();
        }
        println!(
            "| Q{n} | {}.md | {} | {} | {} | {} |",
            question.document,
            question.text,
            question.expected,
            outcome(&view.state),
            coverage(&view)
        );
        service.cancel_memory_search(&view.search_id).ok();
    }
    println!("\nrequests sent: {sent} of {budget}");
    for id in &ours {
        service.set_memory_consent(id, false).unwrap();
    }
}

fn settled(pair: &VersionPairView) -> bool {
    matches!(
        pair.state,
        PairState::Analyzed { .. }
            | PairState::NoTextChange
            | PairState::Unreadable { .. }
            | PairState::Failed {
                will_retry: false,
                ..
            }
            | PairState::InvalidOutput {
                will_retry: false,
                ..
            }
    )
}

/// The result in a few words: versions by their number in the document.
fn outcome(state: &SearchState) -> String {
    let version = |candidate: &SearchCandidate| match &candidate.version {
        Some(version) => format!("v{}", version.sequence),
        None => format!(
            "v{}→v{} (side not known)",
            candidate.before.sequence, candidate.after.sequence
        ),
    };
    match state {
        SearchState::Found {
            candidate,
            reached_by,
            ..
        } => format!(
            "Found {} ({reached_by:?}; maybe {}, partial {})",
            version(candidate),
            candidate.maybe,
            candidate.partial
        ),
        SearchState::Candidates {
            candidates,
            question,
        } => {
            let listed: Vec<String> = candidates.iter().map(version).collect();
            match question {
                Some(question) => format!(
                    "Candidates {}; asks: {} [{}]",
                    listed.join(", "),
                    question.text,
                    question
                        .options
                        .iter()
                        .map(|option| option.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ),
                None => format!("Candidates {} (no question)", listed.join(", ")),
            }
        }
        other => format!("{other:?}"),
    }
}

fn coverage(view: &SearchView) -> String {
    let coverage = &view.coverage;
    format!(
        "{} of {} pairs described{}{}",
        coverage.described,
        coverage.pairs,
        if coverage.complete {
            ", complete"
        } else {
            ", incomplete"
        },
        if coverage.gap_after_latest {
            ", gap after the newest"
        } else {
            ""
        }
    )
}

/// `v1.md`, `v2.md`, ... in order.
fn versions_in(folder: &Path) -> Vec<PathBuf> {
    let mut versions = Vec::new();
    for n in 1.. {
        let path = folder.join(format!("v{n}.md"));
        if !path.exists() {
            break;
        }
        versions.push(path);
    }
    assert!(
        versions.len() >= 2,
        "{}: at least two versions",
        folder.display()
    );
    versions
}

/// Runs the watcher until it saves a new version.
fn wait_until_saved(service: &OrganizeService) {
    let start = Instant::now();
    loop {
        let checks = service.check_memory().unwrap();
        if checks
            .iter()
            .any(|check| matches!(check.outcome, CheckOutcome::Saved { .. }))
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the new version was not saved: {checks:?}"
        );
        thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn questions_are_read_one_per_line() {
    let questions = parse_questions(
        "heat-island-essay.md | before I fixed a citation | v1 or v4, should ask\n\n  rover-proposal | the draft with the demo date |\n",
    );
    assert_eq!(questions.len(), 2);
    assert_eq!(questions[0].document, "heat-island-essay");
    assert_eq!(questions[0].text, "before I fixed a citation");
    assert_eq!(questions[0].expected, "v1 or v4, should ask");
    assert_eq!(questions[1].document, "rover-proposal");
    assert_eq!(questions[1].expected, "");
}

#[test]
#[should_panic(expected = "line 1 is not")]
fn a_line_without_a_question_is_refused() {
    parse_questions("heat-island-essay.md\n");
}
