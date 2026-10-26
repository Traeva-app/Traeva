//! Tests for retrieval.rs (#34), all offline. The essay below has four
//! saved versions, described by #33 with the fake model:
//!   V1 → V2: Smith citation fixed (C1)
//!   V2 → V3: conclusion added (C2)
//!   V3 → V4: Jones citation fixed (C3)
//! The search then asks the same fake model (see changes_tests.rs).

use std::fs;

use serde_json::{Value, json};

use super::*;
use crate::changes::tests::{
    FakeModel, agree, answer, change, file_with_versions, pass, served_by,
};
use crate::model::tests::{Planned, TestServer, completed};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::store::DATABASE_FILE_NAME;
use crate::test_fixture::Fixture;

const V1: &[u8] = b"Smith 2019 says A.\nJones 2020 says B.\n";
const V2: &[u8] = b"Smith (2019) says A.\nJones 2020 says B.\n";
const V3: &[u8] = b"Smith (2019) says A.\nJones 2020 says B.\nIn conclusion, A.\n";
const V4: &[u8] = b"Smith (2019) says A.\nJones (2020) says B.\nIn conclusion, A.\n";

/// The essay, with Memory's agreement and its three changes described.
fn essay(f: &Fixture) -> String {
    let file = file_with_versions(f, "essay.txt", &[V1, V2, V3, V4]);
    agree(f, &file);
    let mut model = FakeModel::answering(vec![
        Ok(answer(
            vec![change(
                "modified",
                "citations",
                "Put the Smith year in brackets",
                &["H1"],
                Some("Smith 2019"),
                Some("Smith (2019)"),
            )],
            &[],
        )),
        Ok(answer(
            vec![change(
                "added",
                "conclusion",
                "Added a conclusion",
                &["H1"],
                None,
                Some("In conclusion"),
            )],
            &[],
        )),
        Ok(answer(
            vec![change(
                "modified",
                "citations",
                "Put the Jones year in brackets",
                &["H1"],
                Some("Jones 2020"),
                Some("Jones (2020)"),
            )],
            &[],
        )),
    ]);
    for _ in 0..3 {
        pass(f, &mut model);
    }
    file
}

/// An understanding with one judgement per change, C1, C2, ...
fn understood(kind: &str, side: Option<&str>, which: Option<&str>, matches: &[&str]) -> Value {
    let judgements: Vec<Value> = matches
        .iter()
        .enumerate()
        .map(|(i, m)| json!({ "change_id": format!("C{}", i + 1), "match": m }))
        .collect();
    json!({ "kind": kind, "side": side, "which": which, "judgements": judgements })
}

fn citation_question() -> Value {
    json!({
        "question": "Do you mean the Smith citation or the Jones citation?",
        "options": [
            { "text": "Smith", "candidate_ids": ["K1"] },
            { "text": "Jones", "candidate_ids": ["K3"] }
        ]
    })
}

fn run(f: &Fixture, search_id: &str, model: &mut FakeModel) -> SearchView {
    run_with(f, search_id, model, &mut || {})
}

fn run_with(
    f: &Fixture,
    search_id: &str,
    model: &mut FakeModel,
    while_waiting: &mut dyn FnMut(),
) -> SearchView {
    let service = f.service();
    let mut send = |request: &OutgoingRequest, allowed: &mut dyn FnMut(&Recipient) -> bool| {
        model.send(service, request, allowed, &mut || {}, while_waiting)
    };
    service
        .run_memory_search_with(search_id, &mut send)
        .unwrap()
}

fn start(f: &Fixture, file: &str, question: &str) -> SearchView {
    f.service().start_memory_search(file, question).unwrap()
}

/// The sequence number of the version a search found, and how it was
/// reached.
fn found(view: &SearchView) -> (i64, ReachedBy) {
    let SearchState::Found {
        candidate,
        reached_by,
        openable,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert!(openable);
    (candidate.version.as_ref().unwrap().sequence, *reached_by)
}

fn option_texts(question: &SearchQuestion) -> Vec<&str> {
    question.options.iter().map(|o| o.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Finding the version
// ---------------------------------------------------------------------------

#[test]
fn the_same_change_twice_is_narrowed_by_one_question() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    assert_eq!(search.state, SearchState::Working);
    assert!(search.coverage.complete);
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        Ok(citation_question()),
    ]);
    // Two candidates: a question is needed, and the model words it.
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::Working
    );
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: Some(question),
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    let versions: Vec<i64> = candidates
        .iter()
        .map(|c| c.version.as_ref().unwrap().sequence)
        .collect();
    assert_eq!(versions, [1, 3], "before each citation fix");
    assert_eq!(option_texts(question), ["Smith", "Jones"]);
    assert_eq!(question.options[1].candidate_ids, ["K3"]);

    let view = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Option {
                question_id: question.question_id.clone(),
                index: 1,
            },
        )
        .unwrap();
    assert_eq!(found(&view), (3, ReachedBy::Answer));
    assert_eq!(model.sent.len(), 2);
}

#[test]
fn the_last_or_first_time_is_found_without_a_question() {
    let f = Fixture::new();
    let file = essay(&f);
    for (which, version) in [("last", 3), ("first", 1)] {
        let search = start(&f, &file, "before I fixed a citation");
        let mut model = FakeModel::answering(vec![Ok(understood(
            "change",
            Some("before"),
            Some(which),
            &["yes", "no", "yes"],
        ))]);
        assert_eq!(
            found(&run(&f, &search.search_id, &mut model)),
            (version, ReachedBy::Search)
        );
        assert_eq!(model.sent.len(), 1);
    }
}

#[test]
fn before_and_after_a_change_are_its_two_versions() {
    let f = Fixture::new();
    let file = essay(&f);
    for (side, version) in [("before", 2), ("after", 3)] {
        let search = start(&f, &file, "the draft and the conclusion");
        let mut model = FakeModel::answering(vec![Ok(understood(
            "change",
            Some(side),
            Some("any"),
            &["no", "yes", "no"],
        ))]);
        assert_eq!(
            found(&run(&f, &search.search_id, &mut model)),
            (version, ReachedBy::Search)
        );
    }
}

#[test]
fn an_unclear_side_is_asked_without_the_model() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the conclusion one");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        None,
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: Some(question),
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates[0].version, None);
    assert_eq!(
        option_texts(question),
        ["Before the change", "After the change"]
    );
    // Picking a candidate needs the side first.
    let error = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Pick {
                candidate_id: "K2".to_string(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::WrongStage);
    let view = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Option {
                question_id: question.question_id.clone(),
                index: 1,
            },
        )
        .unwrap();
    assert_eq!(found(&view), (3, ReachedBy::Search));
    assert_eq!(model.sent.len(), 1);
}

#[test]
fn a_change_that_never_happened_is_not_found() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the bibliography");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "no", "no"],
    ))]);
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::NotFound
    );
}

#[test]
fn a_maybe_alone_is_a_candidate_not_an_answer() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I touched the ending");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "maybe", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: None,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert!(candidates[0].maybe);
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

/// The essay plus a fifth version that is not described yet.
fn essay_with_a_new_version(f: &Fixture) -> String {
    let file = essay(f);
    fs::write(
        f.path("essay.txt"),
        b"Smith (2019) says A.\nJones (2020) says B.\nIn conclusion, A and B.\n",
    )
    .unwrap();
    f.check_memory();
    file
}

#[test]
fn one_match_in_an_incomplete_history_is_only_a_candidate() {
    let f = Fixture::new();
    let file = essay_with_a_new_version(&f);
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!(
        (
            search.coverage.pairs,
            search.coverage.waiting,
            search.coverage.complete
        ),
        (4, 1, false)
    );
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: None,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates.len(), 1);
    // The user may still pick it; it is then their choice.
    let view = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Pick {
                candidate_id: "K2".to_string(),
            },
        )
        .unwrap();
    assert_eq!(found(&view), (2, ReachedBy::Pick));
}

#[test]
fn the_last_time_in_an_incomplete_history_is_not_claimed() {
    let f = Fixture::new();
    let file = essay_with_a_new_version(&f);
    let search = start(&f, &file, "before I last fixed a citation");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("last"),
        &["yes", "no", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: None,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(
        candidates.len(),
        2,
        "both are shown, neither is called the last"
    );
    assert_eq!(model.sent.len(), 1, "no question for first or last");
}

#[test]
fn no_match_in_an_incomplete_history_cannot_tell() {
    let f = Fixture::new();
    let file = essay_with_a_new_version(&f);
    let search = start(&f, &file, "before I added the bibliography");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "no", "no"],
    ))]);
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::CannotTell
    );
}

#[test]
fn a_period_without_memory_is_a_gap() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    f.service().stop_memory(&file).unwrap();
    fs::write(f.path("essay.txt"), V3).unwrap();
    f.advance(1000);
    f.start_memory(&file).unwrap();
    let search = start(&f, &file, "anything");
    assert_eq!((search.coverage.pairs, search.coverage.gaps), (2, 1));
    assert!(!search.coverage.complete);
}

// ---------------------------------------------------------------------------
// What the search will not do
// ---------------------------------------------------------------------------

#[test]
fn questions_by_time_state_or_name_are_not_supported_and_not_not_found() {
    let f = Fixture::new();
    let file = essay(&f);
    for (kind, asked_by) in [
        ("time", AskedBy::Time),
        ("state", AskedBy::State),
        ("name", AskedBy::Name),
    ] {
        let search = start(&f, &file, "last tuesday's version");
        let mut model =
            FakeModel::answering(vec![Ok(understood(kind, None, None, &["no", "no", "no"]))]);
        assert_eq!(
            run(&f, &search.search_id, &mut model).state,
            SearchState::Unsupported { asked_by }
        );
    }
    let search = start(&f, &file, "hmm");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "unclear",
        None,
        None,
        &["no", "no", "no"],
    ))]);
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::Unclear
    );
}

#[test]
fn without_memorys_agreement_nothing_is_sent() {
    let f = Fixture::new();
    let file = essay(&f);
    f.service().set_memory_consent(&file, false).unwrap();
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!(search.state, SearchState::NotPermitted);
    let mut model = FakeModel::default();
    run(&f, &search.search_id, &mut model);
    assert_eq!(model.sent.len(), 0);
}

#[test]
fn a_withdrawal_while_waiting_drops_the_answer() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let service = f.service();
    let view = run_with(&f, &search.search_id, &mut model, &mut || {
        service.set_memory_consent(&file, false).unwrap();
    });
    assert_eq!(view.state, SearchState::NotPermitted);
}

#[test]
fn a_cancel_while_waiting_drops_the_late_answer() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let service = f.service();
    let id = search.search_id.clone();
    let view = run_with(&f, &search.search_id, &mut model, &mut || {
        service.cancel_memory_search(&id).unwrap();
    });
    assert_eq!(view.state, SearchState::Cancelled);
    // Running it again sends nothing.
    run(&f, &search.search_id, &mut model);
    assert_eq!(model.sent.len(), 1);
}

#[test]
fn an_answer_to_an_old_question_is_refused() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        Ok(citation_question()),
    ]);
    run(&f, &search.search_id, &mut model);
    let view = run(&f, &search.search_id, &mut model);
    let error = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Option {
                question_id: "00000000-0000-4000-8000-000000000000".to_string(),
                index: 0,
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        f.service().memory_search(&search.search_id).unwrap().state,
        view.state
    );
}

#[test]
fn an_understanding_that_breaks_the_rules_fails_and_can_be_run_again() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![
        // C3 is not judged.
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["no", "yes"],
        )),
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["no", "yes", "no"],
        )),
    ]);
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::Failed {
            error_code: ErrorCode::InvalidModelOutput,
            retryable: true,
        }
    );
    assert_eq!(
        found(&run(&f, &search.search_id, &mut model)),
        (2, ReachedBy::Search)
    );
}

#[test]
fn a_question_that_breaks_the_rules_is_not_shown() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        // One option keeps every candidate: answering it narrows nothing.
        Ok(json!({
            "question": "Which one?",
            "options": [
                { "text": "Both", "candidate_ids": ["K1", "K3"] },
                { "text": "Jones", "candidate_ids": ["K3"] }
            ]
        })),
    ]);
    run(&f, &search.search_id, &mut model);
    let view = run(&f, &search.search_id, &mut model);
    assert!(matches!(
        view.state,
        SearchState::Candidates { question: None, .. }
    ));
    let view = f
        .service()
        .answer_memory_search(&search.search_id, &SearchAnswer::NoneOfThese)
        .unwrap();
    assert_eq!(view.state, SearchState::NoneOfThese);
}

#[test]
fn the_same_question_is_never_asked_twice() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I fixed something");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "yes", "yes"],
        )),
        Ok(json!({
            "question": "Was it a citation?",
            "options": [
                { "text": "Yes", "candidate_ids": ["K1", "K3"] },
                { "text": "No", "candidate_ids": ["K2"] }
            ]
        })),
        Ok(json!({
            "question": "Was it a citation?",
            "options": [
                { "text": "Yes", "candidate_ids": ["K1"] },
                { "text": "No", "candidate_ids": ["K3"] }
            ]
        })),
    ]);
    run(&f, &search.search_id, &mut model);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        question: Some(question),
        ..
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    let view = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Option {
                question_id: question.question_id.clone(),
                index: 0,
            },
        )
        .unwrap();
    assert_eq!(view.state, SearchState::Working, "two left: ask again");
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: None,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates.len(), 2);
}

// ---------------------------------------------------------------------------
// Updates, privacy, limits
// ---------------------------------------------------------------------------

#[test]
fn a_new_version_during_a_search_shows_as_an_update() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    assert!(!search.updates_available);
    fs::write(f.path("essay.txt"), b"Something else entirely.\n").unwrap();
    f.check_memory();
    let view = f.service().memory_search(&search.search_id).unwrap();
    assert!(view.updates_available);
    assert_eq!(
        view.coverage.pairs, 3,
        "the search keeps the scope it started with"
    );
}

#[test]
fn the_question_stays_out_of_the_database_and_the_debug_output() {
    let f = Fixture::new();
    let file = essay(&f);
    let canary = "QUESTIONCANARY-91f2";
    let search = start(&f, &file, &format!("before I added {canary}"));
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    run(&f, &search.search_id, &mut model);
    let input = &model.sent[0];
    assert!(input.contains(canary), "the question goes to the model");
    assert!(input.contains("Added a conclusion"));
    for absent in ["essay.txt", "Smith 2019 says A.", search.file_id.as_str()] {
        assert!(!input.contains(absent), "{absent} went out");
    }
    let database = fs::read(f.app.path().join("Traeva").join(DATABASE_FILE_NAME)).unwrap();
    assert!(
        !database
            .windows(canary.len())
            .any(|w| w == canary.as_bytes())
    );
    let searches = f.service().searches();
    let debug = format!("{:?}", searches[&search.search_id]);
    assert!(!debug.contains(canary), "{debug}");
}

#[test]
fn a_found_version_whose_copy_is_gone_is_not_openable() {
    let f = Fixture::new();
    let file = essay(&f);
    let v2 = f.service().list_snapshots(&file).unwrap()[1].clone();
    let copy = f
        .app
        .path()
        .join("Traeva")
        .join(crate::SNAPSHOT_DIR)
        .join(&v2.sha256[..2])
        .join(&v2.sha256);
    fs::rename(&copy, f.app.path().join("aside")).unwrap();
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    assert!(matches!(
        view.state,
        SearchState::Found {
            openable: false,
            ..
        }
    ));
}

#[test]
fn only_the_latest_searches_are_kept() {
    let f = Fixture::new();
    let file = essay(&f);
    let first = start(&f, &file, "first");
    for _ in 0..MAX_SEARCHES {
        f.advance(1);
        start(&f, &file, "again");
    }
    assert!(f.service().memory_search(&first.search_id).is_err());
}

#[test]
fn a_question_must_have_words() {
    let f = Fixture::new();
    let file = essay(&f);
    for question in ["", "   ", &"x".repeat(MAX_QUESTION_CHARS + 1)] {
        let error = f
            .service()
            .start_memory_search(&file, question)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }
}

// ---------------------------------------------------------------------------
// Added after the mutation round
// ---------------------------------------------------------------------------

/// Describes every pair of `file` with the answers given, one pass each.
fn describe(f: &Fixture, answers: Vec<Value>) {
    let mut model = FakeModel::answering(answers.into_iter().map(Ok).collect());
    let passes = model.answers.len();
    for _ in 0..passes {
        pass(f, &mut model);
    }
}

fn conclusion_answer() -> Value {
    answer(
        vec![change(
            "added",
            "conclusion",
            "Added a conclusion",
            &["H1"],
            None,
            Some("In conclusion"),
        )],
        &[],
    )
}

#[test]
fn a_gap_keeps_a_fully_described_history_incomplete() {
    let f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    f.service().stop_memory(&file).unwrap();
    fs::write(f.path("essay.txt"), V3).unwrap();
    f.advance(1000);
    f.start_memory(&file).unwrap();
    agree(&f, &file);
    describe(
        &f,
        vec![
            answer(
                vec![change(
                    "modified",
                    "citations",
                    "Put the Smith year in brackets",
                    &["H1"],
                    Some("Smith 2019"),
                    Some("Smith (2019)"),
                )],
                &[],
            ),
            conclusion_answer(),
        ],
    );
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!(
        (
            search.coverage.described,
            search.coverage.gaps,
            search.coverage.complete
        ),
        (2, 1, false)
    );
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    assert!(
        matches!(view.state, SearchState::Candidates { .. }),
        "{:?}",
        view.state
    );
}

#[test]
fn a_pair_described_in_part_keeps_the_history_incomplete() {
    let f = Fixture::new();
    let huge = "x".repeat(50 * 1024);
    let v2 = format!("Smith (2019) says A.\nJones 2020 says B.\n{huge}\n");
    let v3 = format!("Smith (2019) says A.\nJones 2020 says B.\n{huge}\nIn conclusion, A.\n");
    let file = file_with_versions(&f, "essay.txt", &[V1, v2.as_bytes(), v3.as_bytes()]);
    agree(&f, &file);
    // V1 → V2: the Smith fix is described, the huge line is left out.
    describe(
        &f,
        vec![
            answer(
                vec![change(
                    "modified",
                    "citations",
                    "Put the Smith year in brackets",
                    &["H1"],
                    Some("Smith 2019"),
                    Some("Smith (2019)"),
                )],
                &[],
            ),
            conclusion_answer(),
        ],
    );
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!(
        (search.coverage.described, search.coverage.described_in_part),
        (1, 1)
    );
    assert!(!search.coverage.complete);
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    assert!(
        matches!(view.state, SearchState::Candidates { .. }),
        "{:?}",
        view.state
    );
}

#[test]
fn a_yes_and_a_maybe_in_one_pair_make_one_sure_candidate() {
    let f = Fixture::new();
    let file = file_with_versions(
        &f,
        "essay.txt",
        &[
            b"Smith 2019 says A.\nkeep\nkeep\nkeep\nJones 2020 says B.\n",
            b"Smith (2019) says A.\nkeep\nkeep\nkeep\nJones (2020) says B.\n",
        ],
    );
    agree(&f, &file);
    describe(
        &f,
        vec![answer(
            vec![
                change(
                    "modified",
                    "citations",
                    "Put the Smith year in brackets",
                    &["H1"],
                    Some("Smith 2019"),
                    Some("Smith (2019)"),
                ),
                change(
                    "modified",
                    "citations",
                    "Put the Jones year in brackets",
                    &["H2"],
                    Some("Jones 2020"),
                    Some("Jones (2020)"),
                ),
            ],
            &[],
        )],
    );
    let search = start(&f, &file, "before I fixed the Jones citation");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["maybe", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Found { candidate, .. } = &view.state else {
        panic!("{:?}", view.state)
    };
    assert!(!candidate.maybe);
    assert_eq!(
        candidate.changes.len(),
        2,
        "both matching changes are shown"
    );
}

#[test]
fn a_pair_without_a_text_change_counts_as_described() {
    let f = Fixture::new();
    let file = file_with_versions(
        &f,
        "essay.txt",
        &[V2, b"Smith (2019) says A.\r\nJones 2020 says B.\r\n", V3],
    );
    agree(&f, &file);
    describe(&f, vec![conclusion_answer()]);
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!((search.coverage.pairs, search.coverage.described), (2, 2));
    assert!(search.coverage.complete);
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["yes"],
    ))]);
    assert_eq!(
        found(&run(&f, &search.search_id, &mut model)),
        (2, ReachedBy::Search)
    );
}

// ---------------------------------------------------------------------------
// Added after the independent review
// ---------------------------------------------------------------------------

#[test]
fn the_last_time_never_skips_a_sure_match_for_a_maybe() {
    let f = Fixture::new();
    let file = essay(&f);
    for (which, judgements, kept) in [
        ("last", ["yes", "no", "maybe"], ["K1", "K3"]),
        ("first", ["maybe", "no", "yes"], ["K1", "K3"]),
    ] {
        let search = start(&f, &file, "before I fixed a citation");
        let mut model = FakeModel::answering(vec![
            Ok(understood(
                "change",
                Some("before"),
                Some(which),
                &judgements,
            )),
            Ok(json!({ "question": null, "options": [] })),
        ]);
        run(&f, &search.search_id, &mut model);
        let view = run(&f, &search.search_id, &mut model);
        let SearchState::Candidates { candidates, .. } = &view.state else {
            panic!("{:?}", view.state)
        };
        let ids: Vec<&str> = candidates.iter().map(|c| c.candidate_id.as_str()).collect();
        assert_eq!(ids, kept, "{which}");
    }
}

#[test]
fn a_second_run_while_a_request_is_out_sends_nothing() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let mut second = FakeModel::default();
    let id = search.search_id.clone();
    let view = run_with(&f, &search.search_id, &mut model, &mut || {
        let meanwhile = run(&f, &id, &mut second);
        assert_eq!(meanwhile.state, SearchState::Working);
    });
    assert_eq!(found(&view), (2, ReachedBy::Search));
    assert_eq!((model.sent.len(), second.sent.len()), (1, 0));
}

#[test]
fn when_no_question_can_be_worded_the_user_picks() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        Err(SendFailure {
            code: ErrorCode::ModelRequestTooLarge,
            retryable: false,
        }),
    ]);
    run(&f, &search.search_id, &mut model);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        candidates,
        question: None,
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates.len(), 2);
    let view = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Pick {
                candidate_id: "K3".to_string(),
            },
        )
        .unwrap();
    assert_eq!(found(&view), (3, ReachedBy::Pick));
}

#[test]
fn a_pick_from_an_older_view_cannot_land_on_another_pair() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I fixed something");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "yes", "yes"],
        )),
        Ok(json!({
            "question": "Was it a citation?",
            "options": [
                { "text": "Yes", "candidate_ids": ["K1", "K3"] },
                { "text": "No", "candidate_ids": ["K2"] }
            ]
        })),
        Ok(json!({ "question": null, "options": [] })),
    ]);
    run(&f, &search.search_id, &mut model);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates {
        question: Some(question),
        ..
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    f.service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Option {
                question_id: question.question_id.clone(),
                index: 0,
            },
        )
        .unwrap();
    run(&f, &search.search_id, &mut model);
    // "K2" was the conclusion in the older view; it is gone now.
    let error = f
        .service()
        .answer_memory_search(
            &search.search_id,
            &SearchAnswer::Pick {
                candidate_id: "K2".to_string(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

#[test]
fn minor_edits_can_still_be_found() {
    let f = Fixture::new();
    let file = file_with_versions(
        &f,
        "essay.txt",
        &[
            b"Taxes work\n",
            b"Taxes work.\n",
            b"Taxes work.\nIn conclusion, A.\n",
        ],
    );
    agree(&f, &file);
    describe(&f, vec![answer(vec![], &["H1"]), conclusion_answer()]);
    let search = start(&f, &file, "before I added the full stop");
    assert!(search.coverage.complete);
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["yes", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Found { candidate, .. } = &view.state else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidate.version.as_ref().unwrap().sequence, 1);
    assert_eq!(candidate.changes[0].position, 0, "the minor edits stand in");
    assert!(model.sent[0].contains("spacing or punctuation"));
}

#[test]
fn a_question_by_time_with_a_side_is_still_not_supported() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before last tuesday");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "time",
        Some("before"),
        Some("any"),
        &["yes", "no", "no"],
    ))]);
    assert_eq!(
        run(&f, &search.search_id, &mut model).state,
        SearchState::Unsupported {
            asked_by: AskedBy::Time
        }
    );
}

#[test]
fn a_finished_search_lets_the_question_go() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    run(&f, &search.search_id, &mut model);
    let searches = f.service().searches();
    assert_eq!(searches[&search.search_id].user_question, "");
}

#[test]
fn a_search_that_ends_without_candidates_lets_the_question_go_too() {
    let f = Fixture::new();
    let file = essay(&f);
    for kind in ["time", "unclear"] {
        let search = start(&f, &file, "last tuesday's version");
        let mut model =
            FakeModel::answering(vec![Ok(understood(kind, None, None, &["no", "no", "no"]))]);
        run(&f, &search.search_id, &mut model);
        let searches = f.service().searches();
        assert_eq!(searches[&search.search_id].user_question, "", "{kind}");
    }
}

/// A pair view for `coverage_of`, with `changes` described changes.
fn described_pair(index: i64, changes: usize) -> VersionPairView {
    let snapshot = |sequence: i64| SnapshotRef {
        snapshot_id: format!("00000000-0000-4000-8000-{sequence:012}"),
        file_id: "00000000-0000-4000-8000-0000000000ff".to_string(),
        sequence,
        file_revision: 1,
        root_id: "00000000-0000-4000-8000-0000000000ee".to_string(),
        relative_path: "a.txt".to_string(),
        sha256: "0".repeat(64),
        byte_length: 1,
        captured_at_ms: sequence,
    };
    VersionPairView {
        before: snapshot(index),
        after: snapshot(index + 1),
        pair_id: Some(format!("pair-{index}")),
        state: PairState::Analyzed {
            complete: true,
            hunks: changes as i64,
            left_out_hunks: 0,
            minor_hunks: Vec::new(),
        },
        changes: (0..changes)
            .map(|n| ChangeView {
                change_id: format!("change-{index}-{n}"),
                position: n as i64 + 1,
                action: ChangeAction::Added,
                topic: "t".to_string(),
                description: "d".to_string(),
                evidence: ChangeEvidence {
                    hunks: Vec::new(),
                    before_quote: None,
                    after_quote: None,
                },
            })
            .collect(),
        basis: crate::memory_contracts::PairBasis::SameFile,
        evidence: crate::memory_contracts::PairEvidence::System,
        gap: false,
        gap_kinds: Vec::new(),
        unplaced: false,
        permission_file_ids: vec!["00000000-0000-4000-8000-0000000000ff".to_string()],
        complete: true,
        edge_id: None,
        linked_automatically: false,
    }
}

#[test]
fn beyond_two_hundred_changes_the_oldest_are_left_out() {
    let pairs: Vec<VersionPairView> = (1..=21).map(|i| described_pair(i, 10)).collect();
    let (coverage, sent) = coverage_of(&pairs, false);
    assert_eq!(coverage.changes_left_out, 10);
    assert!(!coverage.complete);
    assert_eq!(sent.len(), MAX_SEARCH_CHANGES);
    assert_eq!(
        sent[0].2.change_id, "change-2-0",
        "the first pair's are left out"
    );
    assert_eq!(sent.last().unwrap().2.change_id, "change-21-9");
    assert_eq!((sent[0].0.as_str(), sent[0].1), ("C1", 1));
}

// ---------------------------------------------------------------------------
// The real send path: `send_only` (model.rs) and a local test server
// ---------------------------------------------------------------------------

#[test]
fn a_search_asks_through_send_only() {
    let server = TestServer::start_sequence(vec![
        Planned::answer(
            200,
            completed(&understood(
                "change",
                Some("before"),
                Some("any"),
                &["yes", "no", "yes"],
            )),
        ),
        Planned::answer(200, completed(&citation_question())),
    ]);
    let f = served_by(&server);
    let file = essay(&f);
    let search = start(&f, &file, "before I fixed the citations");
    run_for_real(&f, &search.search_id);
    let view = run_for_real(&f, &search.search_id);
    let SearchState::Candidates {
        question: Some(question),
        ..
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(option_texts(question), ["Smith", "Jones"]);
    assert_eq!(server.count(), 2, "understanding, then the question");

    let first = server.received.lock().unwrap()[0].json().to_string();
    assert!(first.contains("before I fixed the citations"));
    assert!(
        !first.contains("essay.txt") && !first.contains("Smith (2019) says A."),
        "no file name and no document text go out"
    );
}

#[test]
fn a_search_without_a_key_sends_nothing_and_runs_again_once_it_is_saved() {
    let server = TestServer::start(Planned::answer(
        200,
        completed(&understood(
            "change",
            Some("before"),
            Some("any"),
            &["no", "no", "no"],
        )),
    ));
    let f = served_by(&server);
    let file = essay(&f);
    f.service().remove_api_key().unwrap();
    let search = start(&f, &file, "before I fixed the citations");
    let view = run_for_real(&f, &search.search_id);
    assert_eq!(
        view.state,
        SearchState::Failed {
            error_code: ErrorCode::ModelNotConfigured,
            retryable: false,
        }
    );
    assert_eq!(server.count(), 0);

    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    let view = run_for_real(&f, &search.search_id);
    assert_eq!(view.state, SearchState::NotFound);
    assert_eq!(server.count(), 1);
}

fn run_for_real(f: &Fixture, search_id: &str) -> SearchView {
    f.service().run_memory_search(search_id).unwrap()
}

#[test]
fn a_search_answer_from_a_model_the_agreement_does_not_cover_is_dropped() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I fixed the citations");
    let mut model = FakeModel {
        answered_by: Some("gpt-other".to_string()),
        ..FakeModel::answering(vec![Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "no"],
        ))])
    };
    let view = run(&f, &search.search_id, &mut model);
    assert_eq!(model.sent.len(), 1);
    assert_eq!(view.state, SearchState::NotPermitted);
}

#[test]
fn a_failed_search_shows_working_at_once_when_tried_again() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![
        Err(SendFailure {
            code: ErrorCode::ModelTimeout,
            retryable: true,
        }),
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["no", "yes", "no"],
        )),
    ]);
    let failed = run(&f, &search.search_id, &mut model);
    assert!(matches!(failed.state, SearchState::Failed { .. }));
    let again = f.service().retry_memory_search(&search.search_id).unwrap();
    assert_eq!(again.state, SearchState::Working);
    assert_eq!(
        found(&run(&f, &search.search_id, &mut model)),
        (2, ReachedBy::Search)
    );

    // A search that did not fail stays as it is.
    let done = f.service().retry_memory_search(&search.search_id).unwrap();
    assert_eq!(found(&done), (2, ReachedBy::Search));
    assert_eq!(model.sent.len(), 2);
}

// ---------------------------------------------------------------------------
// Gaps from the core (#31 M1a) and the student's 2026-10-09 decision
// ---------------------------------------------------------------------------

#[test]
fn a_stop_of_traeva_after_the_newest_version_keeps_the_answer_from_being_proven() {
    let mut f = Fixture::new();
    let file = essay(&f);
    f.restart();
    f.advance(60_000);
    let search = start(&f, &file, "before I added the conclusion");
    assert!(search.coverage.gap_after_latest);
    assert!(!search.coverage.complete);
    assert_eq!(search.coverage.gaps, 0, "no pair has a gap of its own");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let SearchState::Candidates { candidates, .. } = &view.state else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates.len(), 1, "shown, but not as the only one");
}

#[test]
fn without_a_stop_the_same_search_finds_the_version() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    assert!(!search.coverage.gap_after_latest);
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    assert_eq!(
        found(&run(&f, &search.search_id, &mut model)),
        (2, ReachedBy::Search)
    );
}

#[test]
fn a_stop_of_traeva_between_two_versions_marks_that_pair() {
    let mut f = Fixture::new();
    let file = file_with_versions(&f, "essay.txt", &[V1, V2]);
    f.restart();
    f.advance(60_000);
    fs::write(f.path("essay.txt"), V3).unwrap();
    assert!(
        f.check_memory()
            .iter()
            .any(|c| matches!(c.outcome, crate::CheckOutcome::Saved { .. }))
    );
    let search = start(&f, &file, "before I added the conclusion");
    assert_eq!(search.coverage.pairs, 2);
    assert_eq!(search.coverage.gaps, 1, "only the pair across the stop");
    assert!(!search.coverage.gap_after_latest, "watching again since");
}

#[test]
fn pairs_whose_place_is_unknown_never_decide_the_first_or_last() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before the last time I fixed a citation");
    // As if the user had said the first pair's file is "same work, order
    // unknown" (#31): its order may not be used.
    f.service()
        .searches()
        .get_mut(&search.search_id)
        .unwrap()
        .pairs[0]
        .unplaced = true;
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("last"),
        &["yes", "no", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    assert!(
        !matches!(view.state, SearchState::Found { .. }),
        "{:?}",
        view.state
    );
}

#[test]
fn after_memory_is_turned_off_the_history_ends_there() {
    let mut f = Fixture::new();
    let file = essay(&f);
    f.advance(1_000);
    f.service().stop_memory(&file).unwrap();
    // Traeva is quit right after Memory was turned off for the file, and
    // started again later: that stop is outside the file's recorded history
    // (turning Memory off notes that the watcher was running, #92 dabaf4a).
    f.advance(1_000);
    f.restart();
    f.advance(60_000);
    let search = start(&f, &file, "before I added the conclusion");
    assert!(!search.coverage.gap_after_latest, "{:?}", search.coverage);
    assert!(search.coverage.complete);
}

#[test]
fn a_search_needs_the_agreements_its_pairs_name() {
    let f = Fixture::new();
    let named = file_with_versions(&f, "essay.txt", &[V1, V2]);
    let searched = file_with_versions(&f, "other.txt", &[V1, V2]);
    agree(&f, &named);
    // A pair of the searched file whose agreement is another file's (a
    // continuation keeps its first file's agreement, #31).
    let mut pair = described_pair(1, 1);
    pair.permission_file_ids = vec![named.clone()];
    let service = f.service();
    assert!(
        service
            .scope_consents(&searched, &[pair.clone()])
            .unwrap()
            .is_some()
    );
    pair.permission_file_ids = vec![searched.clone()];
    assert!(
        service
            .scope_consents(&searched, &[pair])
            .unwrap()
            .is_none()
    );
    // No pair yet: the file's own agreement.
    assert!(service.scope_consents(&named, &[]).unwrap().is_some());
}

/// The pair an answer rests on stops being listed after the answer was
/// taken: no answer rests on it. It is shown as a candidate, marked no
/// longer certain (shared-contract.md §4.6, v17).
#[test]
fn no_answer_rests_on_a_pair_the_core_no_longer_lists() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    // As if the relation behind the second pair were corrected while the
    // model answers, after the request's own checks: the search's copy of
    // the pair is one the core does not list (versions 2 and 4).
    let view = run_with(&f, &search.search_id, &mut model, &mut || {
        let mut searches = f.service().searches();
        let stored = searches.get_mut(&search.search_id).unwrap();
        stored.pairs[1].after = stored.pairs[2].after.clone();
    });
    let SearchState::Candidates { candidates, .. } = &view.state else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates.len(), 1, "shown, never given as the answer");
    assert_eq!(
        candidates[0].no_longer_usable,
        Some(PairRefusal::NotAdjacent)
    );
}

/// The candidates of a search, found or not.
fn shown(view: &SearchView) -> Vec<&SearchCandidate> {
    match &view.state {
        SearchState::Found { candidate, .. } => vec![candidate],
        SearchState::Candidates { candidates, .. } => candidates.iter().collect(),
        other => panic!("{other:?}"),
    }
}

/// An answer found across a continuation the user then revokes is shown as
/// no longer certain, with the core's reason; the search is not run again
/// (shared-contract.md §4.6, v17).
#[test]
fn an_answer_whose_relation_is_revoked_is_no_longer_certain() {
    use crate::family_correct::{RelationAction, RelationRequest};

    let f = Fixture::new().without_auto_link();
    let (old, new) = crate::changes::tests::continued_record(&f);
    agree(&f, &old);
    let mut model = FakeModel::answering(vec![
        Ok(answer(
            vec![change(
                "modified",
                "list",
                "Rewrote buy milk as the second version",
                &["H1"],
                Some("buy milk"),
                Some("second version"),
            )],
            &[],
        )),
        Ok(answer(
            vec![change(
                "modified",
                "list",
                "Replaced the second version with the text replaced by an app",
                &["H1"],
                Some("second version"),
                Some("replaced by an app"),
            )],
            &[],
        )),
    ]);
    pass(&f, &mut model);
    pass(&f, &mut model);
    let search = start(&f, &new, "after I replaced the text");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("after"),
        Some("any"),
        &["no", "yes"],
    ))]);
    let view = run(&f, &search.search_id, &mut model);
    let candidate = shown(&view)[0].clone();
    assert_eq!(candidate.after.file_id, new);
    assert_eq!(candidate.no_longer_usable, None);

    let edge: String = f
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE kind = 'continuation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let preview = f.service().relation_preview(&edge).unwrap();
    f.service()
        .correct_relation(&RelationRequest {
            edge_id: edge,
            action: RelationAction::Revoke,
            page_token: preview.page_token,
        })
        .unwrap();

    let again = f.service().memory_search(&search.search_id).unwrap();
    assert_eq!(again.state.clone(), {
        let mut state = view.state.clone();
        match &mut state {
            SearchState::Found { candidate, .. } => {
                candidate.no_longer_usable = Some(PairRefusal::RelationSuperseded)
            }
            SearchState::Candidates { candidates, .. } => {
                candidates[0].no_longer_usable = Some(PairRefusal::RelationSuperseded)
            }
            other => panic!("{other:?}"),
        }
        state
    });
    assert!(again.updates_available, "the history changed");
    let json = serde_json::to_value(&again).unwrap();
    let state = &json["state"];
    let candidate = if state["kind"] == "found" {
        &state["candidate"]
    } else {
        &state["candidates"][0]
    };
    assert_eq!(candidate["no_longer_usable"], "relation_superseded");
}

/// A relation revoked after a search started cuts the line it read: the
/// cut-off part needs its own first file's agreement, so the search sends
/// nothing more and says the history changed, not that Memory has no
/// agreement (shared-contract.md §4.6, v17).
#[test]
fn a_search_sends_nothing_once_a_pair_it_read_is_corrected() {
    use crate::family_correct::{RelationAction, RelationRequest};

    let f = Fixture::new().without_auto_link();
    let (old, new) = crate::changes::tests::continued_record(&f);
    agree(&f, &old);
    let search = start(&f, &new, "after I replaced the text");
    let edge: String = f
        .db()
        .query_row(
            "SELECT edge_id FROM memory_edges WHERE kind = 'continuation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let preview = f.service().relation_preview(&edge).unwrap();
    f.service()
        .correct_relation(&RelationRequest {
            edge_id: edge,
            action: RelationAction::Revoke,
            page_token: preview.page_token,
        })
        .unwrap();

    let mut model = FakeModel::default();
    let view = run(&f, &search.search_id, &mut model);
    assert!(model.sent.is_empty(), "nothing went out");
    assert_eq!(model.refused, 1);
    assert_eq!(
        view.state,
        SearchState::Failed {
            error_code: ErrorCode::PageOutdated,
            retryable: false,
        }
    );
    assert!(view.updates_available);
}

/// A search started from any file of a continued record covers the whole
/// line, and the time after its newest version is judged on the file that
/// holds it, not on the older file whose watch ended at the continuation.
#[test]
fn a_search_of_a_continued_record_covers_its_whole_line() {
    let f = Fixture::new().without_auto_link();
    let (old, new) = crate::changes::tests::continued_record(&f);
    for file_id in [&old, &new] {
        let view = f
            .service()
            .start_memory_search(file_id, "the first version")
            .unwrap();
        assert_eq!(view.coverage.pairs, 2, "{file_id}");
        assert!(!view.coverage.gap_after_latest, "{file_id}");
        f.service().cancel_memory_search(&view.search_id).unwrap();
    }
}

/// Traeva stopped after the newest version, which is on the file the record
/// went on as: a search started from the older file of the line sees it too.
#[test]
fn a_stop_after_the_newest_version_of_a_line_counts_from_any_file() {
    let mut f = Fixture::new().without_auto_link();
    let (old, new) = crate::changes::tests::continued_record(&f);
    f.restart();
    f.advance(60_000);
    for file_id in [&old, &new] {
        let view = f
            .service()
            .start_memory_search(file_id, "the first version")
            .unwrap();
        assert!(view.coverage.gap_after_latest, "{file_id}");
        f.service().cancel_memory_search(&view.search_id).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Running a search to its end, as the window does in the background
// (R2-MEM-03)
// ---------------------------------------------------------------------------

fn run_to_end_with(
    f: &Fixture,
    search_id: &str,
    model: &mut FakeModel,
    while_waiting: &mut dyn FnMut(),
) -> SearchView {
    let service = f.service();
    let mut send = |request: &OutgoingRequest, allowed: &mut dyn FnMut(&Recipient) -> bool| {
        model.send(service, request, allowed, &mut || {}, while_waiting)
    };
    service
        .run_memory_search_to_end_with(search_id, &mut send)
        .unwrap()
}

#[test]
fn a_run_to_the_end_goes_on_from_understanding_to_the_question() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        Ok(citation_question()),
    ]);
    let view = run_to_end_with(&f, &search.search_id, &mut model, &mut || {});
    let SearchState::Candidates {
        question: Some(question),
        ..
    } = &view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(option_texts(question), ["Smith", "Jones"]);
    assert_eq!(model.sent.len(), 2, "one request per step");
    // Nothing is left to run: another run sends nothing.
    let again = run_to_end_with(&f, &search.search_id, &mut model, &mut || {});
    assert_eq!(again.state, view.state);
    assert_eq!(model.sent.len(), 2);
}

#[test]
fn a_run_to_the_end_with_one_match_asks_the_model_once() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I added the conclusion");
    let mut model = FakeModel::answering(vec![Ok(understood(
        "change",
        Some("before"),
        Some("any"),
        &["no", "yes", "no"],
    ))]);
    let view = run_to_end_with(&f, &search.search_id, &mut model, &mut || {});
    assert!(
        matches!(view.state, SearchState::Found { .. }),
        "{:?}",
        view.state
    );
    assert_eq!(model.sent.len(), 1);
}

#[test]
fn a_run_to_the_end_leaves_a_step_another_run_has_out() {
    let f = Fixture::new();
    let file = essay(&f);
    let search = start(&f, &file, "the version before I fixed the citations");
    let id = search.search_id.clone();
    let mut model = FakeModel::answering(vec![
        Ok(understood(
            "change",
            Some("before"),
            Some("any"),
            &["yes", "no", "yes"],
        )),
        Ok(citation_question()),
    ]);
    let mut second = None;
    // While the first run waits for the model, a second run starts: it sees
    // the step out, sends nothing and returns at once.
    let view = run_to_end_with(&f, &id, &mut model, &mut || {
        if second.is_none() {
            let mut never = |_: &OutgoingRequest, _: &mut dyn FnMut(&Recipient) -> bool| {
                panic!("a step another run has out is never sent again")
            };
            second = Some(
                f.service()
                    .run_memory_search_to_end_with(&id, &mut never)
                    .unwrap(),
            );
        }
    });
    assert_eq!(second.unwrap().state, SearchState::Working);
    assert!(
        matches!(view.state, SearchState::Candidates { .. }),
        "{:?}",
        view.state
    );
    assert_eq!(model.sent.len(), 2);
}

/// A pair whose slides only changed order counts as described, and a fixed
/// line in Traeva's words (no text of the document) lets a search for
/// "before I reordered the slides" find it (contract v23 §4.4).
#[test]
fn a_new_slide_order_is_described_and_can_be_found() {
    let mut moved = described_pair(1, 0);
    moved.state = PairState::StructureOnly {
        change: StructureChange::SlideOrder,
    };
    let pairs = vec![described_pair(0, 1), moved];
    let (coverage, sent) = coverage_of(&pairs, false);
    assert_eq!(coverage.described, 2);
    assert!(coverage.complete);
    let order = sent
        .iter()
        .find(|(_, index, _)| *index == 1)
        .map(|(_, _, change)| change)
        .unwrap();
    assert_eq!(order.position, 0);
    assert_eq!(order.topic, "slide order");
    assert_eq!(
        order.description,
        "Only the order of the slides changed here."
    );
    assert!(order.change_id.starts_with("structure-"));
}

/// A workbook whose cells' labels only changed: the writer moved values or
/// stored them as another type, which the fixed line says (not "the lines
/// Traeva adds"); a deck's stay as they were.
#[test]
fn a_workbooks_moved_cells_are_named_as_cell_places() {
    let mut cells = described_pair(1, 0);
    cells.state = PairState::StructureOnly {
        change: StructureChange::Labels,
    };
    cells.after.relative_path = "Budget.XLSX".to_string();
    let mut slides = cells.clone();
    slides.after.relative_path = "talk.pptx".to_string();
    for (pair, topic, description) in [
        (
            cells,
            "cell places",
            "Only cell addresses or value types changed here; the values are the same.",
        ),
        (
            slides,
            "structure lines",
            "Only the lines Traeva adds (such as slide numbers) changed here; the text is the same.",
        ),
    ] {
        let line = structure_edit(&pair, StructureChange::Labels);
        assert_eq!(
            (line.topic.as_str(), line.description.as_str()),
            (topic, description)
        );
    }
}

/// A question keeps what a description is unsure of: a sheet that "may have
/// been hidden" is never offered as "removed" (R1-M28-O2).
#[test]
fn a_question_keeps_a_hidden_sheet_hidden() {
    assert_eq!(QUESTION_PROMPT_VERSION, "memory-question-2");
    assert!(QUESTION_INSTRUCTIONS.contains(
        "when it says a slide or sheet is no longer included or may have been hidden, say that, never that it was removed or deleted"
    ));
}
