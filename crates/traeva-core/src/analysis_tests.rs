//! Tests for analysis.rs (#16), all offline: the local test server from the
//! model tests plays OpenAI, answering each request in turn. Synthetic files
//! and folders only. The acceptance rows of the #16 material §7 are named in
//! the comments.

use std::collections::HashMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::classify::{Candidate, Clarification, Disposition};
use crate::contracts::{BuildOutcome, ClarificationOption, PlanView, UncertainField};
use crate::model::tests::{Planned, TestServer, completed};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::{Fixture, edit_keeping_size_and_mtime, quick_read};
use crate::test_snapshot as snapshot;

// ---------------------------------------------------------------------------
// The routing table, on its own
// ---------------------------------------------------------------------------

fn result(disposition: Disposition, confidence: Option<f64>) -> ClassificationResult {
    ClassificationResult {
        request_id: uuid::Uuid::new_v4().to_string(),
        file_id: uuid::Uuid::new_v4().to_string(),
        file_revision: 1,
        target_root_id: uuid::Uuid::new_v4().to_string(),
        catalog_id: uuid::Uuid::new_v4().to_string(),
        disposition,
        summary: "A trip.".to_string(),
        candidates: vec![Candidate {
            candidate_id: "C1".to_string(),
            folder_id: "F1".to_string(),
            new_segments: Vec::new(),
            reason: "Trip notes.".to_string(),
            evidence_ids: vec!["E1".to_string()],
        }],
        preferred_id: (disposition == Disposition::Recommend).then(|| "C1".to_string()),
        confidence,
        uncertain_field: None,
        clarification: None,
    }
}

fn with_question(mut result: ClassificationResult) -> ClassificationResult {
    result.uncertain_field = Some(UncertainField::Purpose);
    result.clarification = Some(Clarification {
        question_id: uuid::Uuid::new_v4().to_string(),
        question: "Work or holiday?".to_string(),
        options: Vec::new(),
    });
    result
}

#[test]
fn the_routing_table_holds_row_by_row() {
    let clear = result(Disposition::Recommend, Some(0.70));
    // Row 2: 0.70 is enough (v0.12; it was 0.80); unique, supported,
    // nothing unsure.
    assert_eq!(route(&clear, 0), Route::Recommend);
    assert_eq!(route(&clear, 3), Route::Recommend);
    let was_too_low = result(Disposition::Recommend, Some(0.79));
    assert_eq!(route(&was_too_low, 0), Route::Recommend);

    // Not recommendable, and no question: manual, never a made-up question.
    let low = result(Disposition::Recommend, Some(0.69));
    assert_eq!(route(&low, 0), Route::Manual);
    let unknown = result(Disposition::Recommend, None);
    assert_eq!(route(&unknown, 0), Route::Manual);
    let mut unsupported = result(Disposition::Recommend, Some(0.99));
    unsupported.candidates[0].evidence_ids.clear();
    assert_eq!(route(&unsupported, 0), Route::Manual);
    let mut two = result(Disposition::Recommend, Some(0.99));
    two.candidates.push(Candidate {
        candidate_id: "C2".to_string(),
        ..two.candidates[0].clone()
    });
    assert_eq!(route(&two, 0), Route::Manual);
    assert_eq!(route(&result(Disposition::Abstain, None), 0), Route::Manual);

    // Row 3: up to three questions, one at a time; a question after the
    // third answer means manual (organize-spec §15).
    let spare = with_question(result(Disposition::Recommend, Some(0.99)));
    assert_eq!(route(&spare, 0), Route::Ask);
    assert_eq!(route(&spare, 2), Route::Ask);
    assert_eq!(route(&spare, 3), Route::Manual);
    let mut clarify = with_question(result(Disposition::Clarify, None));
    clarify.preferred_id = None;
    assert_eq!(route(&clarify, 0), Route::Ask);
    assert_eq!(route(&clarify, 1), Route::Ask);
    assert_eq!(route(&clarify, 3), Route::Manual);
    let mut two_and_asks = with_question(two);
    two_and_asks.uncertain_field = Some(UncertainField::Topic);
    assert_eq!(route(&two_and_asks, 0), Route::Ask);

    // The table on its own: a checked answer never looks like these (#15
    // keeps uncertain_field and the question together, and only a
    // recommendation names a preferred candidate), but the rows still hold.
    let mut unsure = result(Disposition::Recommend, Some(0.99));
    unsure.uncertain_field = Some(UncertainField::Destination);
    assert_eq!(route(&unsure, 0), Route::Manual);
    let mut not_recommended = result(Disposition::Abstain, Some(0.99));
    not_recommended.preferred_id = Some("C1".to_string());
    assert_eq!(route(&not_recommended, 0), Route::Manual);
}

// ---------------------------------------------------------------------------
// Whole operations through the local test server
// ---------------------------------------------------------------------------

/// Three paragraphs, so three text pieces: E1, E2, E3.
const TEXT: &str = "Trip to Kyoto in May: train tickets and a hotel near the station.\n\n\
                    Budget about 400 USD for the week.\n\n\
                    Packing list: rain jacket and walking shoes.\n";

/// Folder IDs by path, as every listing of the unchanged tree gives them.
type Ids = HashMap<String, String>;

struct Run {
    f: Fixture,
    server: TestServer,
    operation_id: String,
}

fn ok(answer: Value) -> Planned {
    Planned::answer(200, completed(&answer))
}

/// A root with the sample tree plus Travel/, the note trip.md read through an
/// operation (so it is `ready_for_analysis`), and a server that gives the
/// answers `answers(ids)` in turn.
fn run(answers: impl FnOnce(&Ids) -> Vec<Planned>) -> Run {
    let mut f = Fixture::new();
    fs::create_dir(f.path("Travel")).unwrap();
    fs::write(f.path("trip.md"), TEXT).unwrap();
    let ids: Ids = f
        .service()
        .list_folders(&f.root_id)
        .unwrap()
        .folders
        .into_iter()
        .map(|folder| (folder.relative_path, folder.folder_id))
        .collect();
    let server = TestServer::start_sequence(answers(&ids));
    f.service_mut().model.endpoint = server.url.clone();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    f.service().set_analysis_consent(true).unwrap();
    let started = f.service().start_operation(&f.root_id, &f.root_id).unwrap();
    f.service().run_operation_with(
        &started.operation_id,
        &f.path("trip.md"),
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    );
    let run = Run {
        f,
        server,
        operation_id: started.operation_id,
    };
    assert_eq!(run.view().stage, OperationStage::ReadyForAnalysis);
    run
}

impl Run {
    fn view(&self) -> OperationView {
        self.f.service().get_operation(&self.operation_id).unwrap()
    }

    fn start(&self) -> Result<OperationView, AppError> {
        self.f.service().start_analysis(&self.operation_id)
    }

    /// Runs the waiting request, as the background task would.
    fn work(&self) -> OperationView {
        self.f
            .service()
            .run_analysis_with(&self.operation_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            });
        self.view()
    }

    fn analyze(&self) -> OperationView {
        self.start().unwrap();
        self.work()
    }

    fn answer(
        &self,
        option_id: Option<&str>,
        text: Option<&str>,
    ) -> Result<OperationView, AppError> {
        let question_id = self.view().question.unwrap().question_id;
        self.answer_to(&question_id, option_id, text)
    }

    fn answer_to(
        &self,
        question_id: &str,
        option_id: Option<&str>,
        text: Option<&str>,
    ) -> Result<OperationView, AppError> {
        self.f.service().answer_clarification(
            &self.operation_id,
            &ClarificationAnswer {
                question_id: question_id.to_string(),
                option_id: option_id.map(str::to_string),
                text: text.map(str::to_string),
            },
        )
    }

    fn file(&self) -> FileView {
        self.view().file.unwrap()
    }

    fn request(&self, target: PlanTarget, with_operation: bool) -> PlanRequest {
        let file = self.file();
        PlanRequest {
            file_id: file.file_id,
            file_revision: file.file_revision,
            target_root_id: self.f.root_id.clone(),
            operation_id: with_operation.then(|| self.operation_id.clone()),
            target,
        }
    }

    fn plan_candidate(&self, candidate_id: &str) -> Result<PlanView, AppError> {
        let target = PlanTarget::Candidate {
            candidate_id: candidate_id.to_string(),
        };
        planned(self.f.service().build_plan(&self.request(target, true)))
    }

    fn plan_folder(&self, path: &str) -> Result<PlanView, AppError> {
        let listing = self.f.service().list_folders(&self.f.root_id).unwrap();
        let folder_id = listing
            .folders
            .iter()
            .find(|f| f.relative_path == path)
            .unwrap()
            .folder_id
            .clone();
        let target = PlanTarget::Folder {
            catalog_id: listing.catalog_id,
            folder_id,
            new_folders: Vec::new(),
        };
        planned(self.f.service().build_plan(&self.request(target, true)))
    }

    /// The data part of request number `index` (0 = first), parsed.
    fn sent_data(&self, index: usize) -> Value {
        let received = self.server.received.lock().unwrap();
        let input = received[index].json()["input"]
            .as_str()
            .unwrap()
            .to_string();
        let (_, data) = input.split_once('\n').unwrap();
        serde_json::from_str(data).unwrap()
    }

    /// The metadata context of every analyses row, oldest first.
    fn contexts(&self) -> Vec<(String, Value)> {
        let db = self.f.db();
        let mut statement = db
            .prepare(
                "SELECT status, request_meta_json FROM analyses ORDER BY requested_at_ms, rowid",
            )
            .unwrap();
        statement
            .query_map([], |r| {
                let meta: String = r.get(1)?;
                Ok((
                    r.get(0)?,
                    serde_json::from_str::<Value>(&meta).unwrap()["context"].clone(),
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn stored_choice(&self, plan_id: &str) -> (String, Option<String>) {
        self.f
            .db()
            .query_row(
                "SELECT user_choice, analysis_request_id FROM plans WHERE plan_id = ?1",
                [plan_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn wait_for_requests(&self, count: usize) {
        let start = Instant::now();
        while self.server.count() < count {
            assert!(start.elapsed() < Duration::from_secs(5), "waited too long");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn planned(outcome: Result<BuildOutcome, AppError>) -> Result<PlanView, AppError> {
    match outcome? {
        BuildOutcome::Planned { plan } => Ok(plan),
        other => panic!("expected a plan, got {other:?}"),
    }
}

fn candidate(id: &str, folder_id: &str, evidence: &[&str]) -> Value {
    json!({
        "id": id,
        "folder_id": folder_id,
        "new_segments": [],
        "reason": "Fits the trip.",
        "evidence_ids": evidence
    })
}

/// A recommendation of one folder, citing E1.
fn recommend(folder_id: &str, confidence: f64) -> Value {
    json!({
        "disposition": "recommend",
        "summary": "Notes for a trip.",
        "candidates": [candidate("C1", folder_id, &["E1"])],
        "preferred_id": "C1",
        "confidence": confidence,
        "uncertain_field": null,
        "clarification": null
    })
}

/// `answer` with a question added.
fn asking(mut answer: Value, field: &str, options: &[&str]) -> Value {
    answer["uncertain_field"] = json!(field);
    answer["clarification"] =
        json!({ "question": "Is this for work or for a holiday?", "options": options });
    answer
}

/// A "clarify" answer asking `question`, with `options`.
fn clarify_with(question: &str, options: &[&str]) -> Value {
    let mut answer = clarify(json!([]));
    answer["clarification"] = json!({ "question": question, "options": options });
    answer
}

fn clarify(candidates: Value) -> Value {
    asking(
        json!({
            "disposition": "clarify",
            "summary": "Notes about a trip.",
            "candidates": candidates,
            "preferred_id": null,
            "confidence": null,
            "uncertain_field": null,
            "clarification": null
        }),
        "purpose",
        &["Work", "Holiday"],
    )
}

#[test]
fn a_clear_file_is_recommended_once_and_a_double_start_sends_nothing_more() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.80))]);
    let before = snapshot::snapshot(r.f.root.path());

    let started = r.start().unwrap();
    assert_eq!(started.stage, OperationStage::Analyzing);
    // A double click while analyzing returns the same operation.
    assert_eq!(r.start().unwrap(), started);
    let view = r.work();

    assert_eq!(view.stage, OperationStage::RecommendationReady);
    let suggestion = view.suggestion.unwrap();
    assert_eq!(suggestion.recommended_id.as_deref(), Some("C1"));
    assert_eq!(suggestion.candidates[0].folder_path, "Travel");
    // The checked reason and the text pieces it cites reach the window (O02).
    assert_eq!(suggestion.summary, "Notes for a trip.");
    assert_eq!(suggestion.candidates[0].reason, "Fits the trip.");
    assert_eq!(suggestion.candidates[0].evidence_ids, ["E1"]);
    assert_eq!(suggestion.confidence, Some(0.80));
    assert_eq!((view.model_requests, r.server.count()), (1, 1));
    // A recommendation is not the end of an analysis that can start again.
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);
    assert_eq!(r.server.count(), 1);
    assert_eq!(snapshot::snapshot(r.f.root.path()), before);
}

#[test]
fn a_question_is_answered_locked_and_sent_with_the_next_request() {
    let r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    let asked = r.analyze();
    assert_eq!(asked.stage, OperationStage::NeedsClarification);
    let question = asked.question.clone().unwrap();
    assert_eq!(question.uncertain_field, UncertainField::Purpose);
    assert_eq!((question.number, question.max_questions), (1, 3));
    assert!(asked.answered.is_empty());
    assert_eq!(
        question.options,
        [
            ClarificationOption {
                option_id: "O1".to_string(),
                text: "Work".to_string()
            },
            ClarificationOption {
                option_id: "O2".to_string(),
                text: "Holiday".to_string()
            }
        ]
    );
    assert!(asked.suggestion.is_none());

    let answering = r.answer(Some("O2"), None).unwrap();
    assert_eq!(answering.stage, OperationStage::Analyzing);
    assert_eq!(answering.answer.as_deref(), Some("Holiday"));
    assert_eq!(
        answering.answered,
        [AnsweredView {
            question_id: question.question_id.clone(),
            number: 1,
            question: "Is this for work or for a holiday?".to_string(),
            answer: "Holiday".to_string(),
        }]
    );
    let done = r.work();

    // "回答后明确 → 推荐": two requests in all, the second one carrying the
    // locked answer as data.
    assert_eq!(done.stage, OperationStage::RecommendationReady);
    assert_eq!((done.model_requests, r.server.count()), (2, 2));
    assert_eq!(
        r.sent_data(1)["answered_questions"],
        json!([{
            "uncertain_field": "purpose",
            "question": "Is this for work or for a holiday?",
            "options": ["Work", "Holiday"],
            "answer": "Holiday"
        }])
    );
    assert_eq!(r.sent_data(0)["answered_questions"], json!([]));
    let contexts = r.contexts();
    assert_eq!(contexts[0].1["round"], json!("first"));
    assert_eq!(contexts[1].1["round"], json!("answered"));
    assert_eq!(contexts[1].1["retry"], json!(false));
    assert!(
        contexts[1].1["operation_generation"].as_u64()
            > contexts[0].1["operation_generation"].as_u64()
    );
}

#[test]
fn up_to_three_questions_one_at_a_time_then_a_manual_choice_never_a_fourth() {
    // organize-spec §15: narrowing level by level, each request carries every answer
    // so far; a question after the third answer means a manual choice.
    let r = run(|ids| {
        vec![
            ok(clarify_with("Which folder?", &["Notes", "Travel"])),
            ok(clarify_with(
                "Which folder inside Travel?",
                &["2025", "2026"],
            )),
            ok(clarify_with("Work or holiday?", &[])),
            ok(clarify(json!([
                candidate("C1", &ids["Travel"], &["E1"]),
                candidate("C2", &ids["Notes"], &[])
            ]))),
        ]
    });
    let mut questions = Vec::new();
    let mut view = r.analyze();
    for (number, answer) in [(1, Some("O2")), (2, Some("O2")), (3, None)] {
        assert_eq!(view.stage, OperationStage::NeedsClarification);
        let question = view.question.clone().unwrap();
        assert_eq!((question.number, question.max_questions), (number, 3));
        // The new question has no answer yet; the earlier ones are listed.
        assert_eq!(view.answer, None);
        assert_eq!(view.answered.len(), number as usize - 1);
        questions.push(question.question_id.clone());
        let given = match answer {
            Some(option) => r.answer(Some(option), None).unwrap(),
            None => r.answer(None, Some("A holiday")).unwrap(),
        };
        assert_eq!(given.answered.len(), number as usize);
        view = r.work();
    }

    assert_eq!(view.stage, OperationStage::NeedsManualLocation);
    assert_eq!(
        view.question.unwrap().question_id,
        questions[2],
        "no fourth question"
    );
    let answers: Vec<&str> = view.answered.iter().map(|a| a.answer.as_str()).collect();
    assert_eq!(answers, ["Travel", "2026", "A holiday"]);
    let numbers: Vec<u32> = view.answered.iter().map(|a| a.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
    // Every request carried every earlier answer, oldest first.
    for index in 0..4 {
        let sent = r.sent_data(index)["answered_questions"].clone();
        assert_eq!(sent.as_array().unwrap().len(), index);
    }
    assert_eq!(
        r.sent_data(3)["answered_questions"][1],
        json!({
            "uncertain_field": "purpose",
            "question": "Which folder inside Travel?",
            "options": ["2025", "2026"],
            "answer": "2026"
        })
    );
    // The candidates are offered, never preselected or recommended.
    let suggestion = view.suggestion.unwrap();
    assert_eq!(suggestion.recommended_id, None);
    assert_eq!(suggestion.candidates.len(), 2);
    assert_eq!((view.model_requests, r.server.count()), (4, 4));
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);
}

#[test]
fn an_earlier_question_cannot_be_answered_again_once_the_next_is_asked() {
    let r = run(|ids| {
        vec![
            ok(clarify_with("Which folder?", &["Notes", "Travel"])),
            ok(clarify_with("Work or holiday?", &[])),
            ok(recommend(&ids["Travel"], 0.9)),
        ]
    });
    let first = r.analyze().question.unwrap().question_id;
    r.answer(Some("O2"), None).unwrap();
    let second = r.work();
    assert_eq!(second.question.as_ref().unwrap().number, 2);
    assert_eq!(
        r.answer_to(&first, Some("O1"), None).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(r.server.count(), 2);
    r.answer(None, Some("A holiday")).unwrap();
    let done = r.work();
    // Clear after two answers: recommended, with both answers listed.
    assert_eq!(done.stage, OperationStage::RecommendationReady);
    assert_eq!(done.answered.len(), 2);
    assert_eq!(done.answer.as_deref(), Some("A holiday"));
    assert_eq!(r.server.count(), 3);
}

#[test]
fn a_spare_question_is_asked_and_after_the_third_answer_it_means_manual() {
    // "recommend 携带合法备用问题 → 追问；第三次回答后再出现则手选"
    let spare = |ids: &Ids| asking(recommend(&ids["Travel"], 0.95), "destination", &[]);
    let r = run(|ids| (0..4).map(|_| ok(spare(ids))).collect());
    assert_eq!(r.analyze().stage, OperationStage::NeedsClarification);
    for _ in 0..2 {
        r.answer(None, Some("By year")).unwrap();
        assert_eq!(r.work().stage, OperationStage::NeedsClarification);
    }
    r.answer(None, Some("By year")).unwrap();
    let done = r.work();
    assert_eq!(done.stage, OperationStage::NeedsManualLocation);
    assert_eq!(done.suggestion.unwrap().recommended_id, None);
    assert_eq!(r.server.count(), 4);
}

#[test]
fn without_a_question_unclear_results_go_to_manual_with_no_extra_request() {
    // "需要追问但没有合法问题 → 手选；不额外调用模型编问题", and "高分但无证据、
    // 多个候选 → 不能直接推荐".
    for answer in [
        |ids: &Ids| recommend(&ids["Travel"], 0.5),
        |ids: &Ids| {
            let mut answer = recommend(&ids["Travel"], 0.99);
            answer["candidates"][0]["evidence_ids"] = json!([]);
            answer
        },
        |ids: &Ids| {
            let mut answer = recommend(&ids["Travel"], 0.99);
            answer["candidates"] = json!([
                candidate("C1", &ids["Travel"], &["E1"]),
                candidate("C2", &ids["Notes"], &["E2"])
            ]);
            answer
        },
    ] {
        let r = run(|ids| vec![ok(answer(ids))]);
        let done = r.analyze();
        assert_eq!(done.stage, OperationStage::NeedsManualLocation);
        assert!(done.question.is_none());
        assert_eq!(done.suggestion.unwrap().recommended_id, None);
        assert_eq!(r.server.count(), 1);
    }
}

#[test]
fn answers_are_checked_locked_and_never_resent_or_changed() {
    let r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    let question_id = r.analyze().question.unwrap().question_id;
    let generation = r.view().generation;

    let refused = |result: Result<OperationView, AppError>| result.unwrap_err().code;
    let other_question = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        refused(r.answer_to(&other_question, Some("O1"), None)),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(r.answer(Some("O1"), Some("Work"))),
        ErrorCode::InvalidInput
    );
    assert_eq!(refused(r.answer(None, None)), ErrorCode::InvalidInput);
    assert_eq!(refused(r.answer(Some("O9"), None)), ErrorCode::InvalidInput);
    assert_eq!(
        refused(r.answer(None, Some("   "))),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        refused(r.answer(None, Some(&"a".repeat(501)))),
        ErrorCode::InvalidInput
    );
    // Nothing changed and nothing was sent.
    assert_eq!(r.view().stage, OperationStage::NeedsClarification);
    assert_eq!(r.view().generation, generation);
    assert_eq!(r.server.count(), 1);

    // "双击回答": the same answer twice locks once and sends once.
    let first = r.answer_to(&question_id, None, Some("A holiday")).unwrap();
    assert_eq!(
        r.answer_to(&question_id, None, Some("A holiday")).unwrap(),
        first
    );
    // "修改已锁答案": refused.
    assert_eq!(
        refused(r.answer_to(&question_id, Some("O1"), None)),
        ErrorCode::WrongStage
    );
    r.work();
    r.work(); // a second background run finds nothing to do
    assert_eq!(r.server.count(), 2);
    // After the result, too: the same answer is a no-op, another is refused.
    let after = r.view();
    assert_eq!(
        r.answer_to(&question_id, None, Some("A holiday")).unwrap(),
        after
    );
    assert_eq!(
        refused(r.answer_to(&question_id, None, Some("Work"))),
        ErrorCode::WrongStage
    );
    assert_eq!(r.view().answer.as_deref(), Some("A holiday"));
    assert_eq!(r.server.count(), 2);
    // A cancel ends the questions and the answers with them.
    let cancelled = r.f.service().cancel_operation(&r.operation_id).unwrap();
    assert_eq!(
        (cancelled.question, cancelled.answer, cancelled.error),
        (None, None, None)
    );
    assert!(cancelled.answered.is_empty());
}

#[test]
fn after_the_answer_the_first_results_candidates_are_never_offered_again() {
    // The answer ends the first result's candidates, even when the request
    // after it fails and the user then picks by hand.
    let r = run(|ids| {
        vec![
            ok(clarify(json!([candidate("C1", &ids["Travel"], &["E1"])]))),
            Planned::answer(503, b"{}".to_vec()),
        ]
    });
    r.analyze();
    r.answer(Some("O1"), None).unwrap();
    assert_eq!(r.work().stage, OperationStage::AnalysisFailed);
    let manual =
        r.f.service()
            .choose_manual_location(&r.operation_id)
            .unwrap();
    assert_eq!(manual.stage, OperationStage::NeedsManualLocation);
    assert!(manual.suggestion.is_none());
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
}

#[test]
fn an_answer_without_consent_is_refused_and_not_locked() {
    let r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let before = r.view();
    r.f.service().set_analysis_consent(false).unwrap();
    assert_eq!(
        r.answer(Some("O2"), None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
    assert_eq!(r.view(), before, "nothing locked, nothing sent");
    // Once sending is allowed again, the same question can be answered.
    r.f.service().set_analysis_consent(true).unwrap();
    assert_eq!(
        r.answer(Some("O2"), None).unwrap().answer.as_deref(),
        Some("Holiday")
    );
    assert_eq!(r.work().stage, OperationStage::RecommendationReady);
    assert_eq!(r.server.count(), 2);
}

#[test]
fn a_cancel_or_a_manual_choice_drops_the_late_result() {
    // "取消或手选后响应才返回 → 不恢复候选、问题、目录清单或预览"
    for leave in ["cancel", "manual"] {
        let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
        let listing = r.f.service().list_folders(&r.f.root_id).unwrap();
        r.start().unwrap();
        thread::scope(|scope| {
            let worker = scope.spawn(|| r.work());
            r.wait_for_requests(1);
            let left = match leave {
                "cancel" => r.f.service().cancel_operation(&r.operation_id).unwrap(),
                _ => {
                    r.f.service()
                        .choose_manual_location(&r.operation_id)
                        .unwrap()
                }
            };
            r.server.release();
            worker.join().unwrap();
            let after = r.view();
            assert_eq!(after, left, "{leave}: the late result changed nothing");
            assert!(after.suggestion.is_none());
        });
        let (status, _) = r.contexts().remove(0);
        assert_eq!(status, "cancelled", "{leave}");
        // The root's folder list was not replaced by the late result.
        let folder = listing
            .folders
            .iter()
            .find(|f| f.relative_path == "Notes")
            .unwrap();
        let target = PlanTarget::Folder {
            catalog_id: listing.catalog_id.clone(),
            folder_id: folder.folder_id.clone(),
            new_folders: Vec::new(),
        };
        let manual = r.f.service().build_plan(&r.request(target, false));
        assert!(manual.is_ok(), "{leave}: {manual:?}");
    }
}

#[test]
fn a_request_not_sent_yet_is_never_sent_once_the_user_moves_on() {
    // #16 confirmation 2: cancel or manual choice while the file is read.
    for leave in ["cancel", "manual"] {
        let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
        r.start().unwrap();
        r.f.service()
            .run_analysis_with(&r.operation_id, &mut |root, parent, name| {
                match leave {
                    "cancel" => r.f.service().cancel_operation(&r.operation_id).unwrap(),
                    _ => {
                        r.f.service()
                            .choose_manual_location(&r.operation_id)
                            .unwrap()
                    }
                };
                quick_read(parent, name, root.id().dev)
            });
        let view = r.view();
        assert_eq!((view.model_requests, r.server.count()), (0, 0), "{leave}");
        // The request was stopped at the last moment and recorded as such.
        let statuses: Vec<String> = r.contexts().into_iter().map(|(status, _)| status).collect();
        assert_eq!(statuses, ["cancelled"], "{leave}");
        assert!(view.suggestion.is_none());
    }
}

#[test]
fn two_runs_of_one_request_send_it_once() {
    // "双击…不重复请求": the window may start the background run twice.
    let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
    r.start().unwrap();
    thread::scope(|scope| {
        let first = scope.spawn(|| r.work());
        let second = scope.spawn(|| r.work());
        r.wait_for_requests(1);
        r.server.release();
        first.join().unwrap();
        second.join().unwrap();
    });
    let view = r.view();
    assert_eq!(view.stage, OperationStage::RecommendationReady);
    assert_eq!((view.model_requests, r.server.count()), (1, 1));
}

#[test]
fn a_late_result_of_one_operation_never_touches_the_next_one() {
    // "旧请求在新请求之后返回 → 新状态不被覆盖": the user cancels, chooses the
    // same file again, and the new answer arrives before the old one.
    let r = run(|ids| {
        vec![
            Planned::gated(completed(&recommend(&ids["Travel"], 0.9))),
            ok(recommend(&ids["Notes"], 0.9)),
        ]
    });
    r.start().unwrap();
    thread::scope(|scope| {
        let old = scope.spawn(|| r.work());
        r.wait_for_requests(1);
        r.f.service().cancel_operation(&r.operation_id).unwrap();

        let again =
            r.f.service()
                .start_operation(&r.f.root_id, &r.f.root_id)
                .unwrap();
        r.f.service().run_operation_with(
            &again.operation_id,
            &r.f.path("trip.md"),
            &mut |root, parent, name| quick_read(parent, name, root.id().dev),
        );
        r.f.service().start_analysis(&again.operation_id).unwrap();
        r.f.service()
            .run_analysis_with(&again.operation_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            });
        let newer = r.f.service().get_operation(&again.operation_id).unwrap();
        assert_eq!(newer.stage, OperationStage::RecommendationReady);

        r.server.release();
        old.join().unwrap();
        // The old answer changed neither operation.
        assert_eq!(
            r.f.service().get_operation(&again.operation_id).unwrap(),
            newer
        );
        assert_eq!(r.view().stage, OperationStage::Cancelled);
    });
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
}

#[test]
fn a_confirmed_plan_is_never_cancelled_as_if_nothing_happened() {
    // spec §6, §9: after confirmation the action decides; cancel is refused.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let plan = r.plan_candidate("C1").unwrap();
    assert_eq!(r.view().action_id, None);
    let action = r.f.apply(&plan.plan_id).unwrap();
    let before = r.view();
    // Reading the operation shows the action its plan became.
    assert_eq!(before.action_id.as_deref(), Some(action.action_id.as_str()));
    let refused = r.f.service().cancel_operation(&r.operation_id).unwrap_err();
    assert_eq!(refused.code, ErrorCode::WrongStage);
    assert_eq!(
        refused.action_id.as_deref(),
        Some(action.action_id.as_str())
    );
    assert_eq!(r.view(), before);
    assert!(r.f.path("Travel/trip.md").exists());

    // A plan confirmed while a newer one was being made (the two raced):
    // the newer one is saved last, yet the confirmed one still decides.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let confirmed = r.plan_candidate("C1").unwrap();
    let request = r.request(
        PlanTarget::Candidate {
            candidate_id: "C1".to_string(),
        },
        true,
    );
    let choice = r.f.service().plan_choice(&request).unwrap();
    let action = r.f.apply(&confirmed.plan_id).unwrap();
    let store = r.f.service().store().unwrap();
    let mut newer = crate::plan::load_plan(store, &confirmed.plan_id).unwrap();
    newer.plan_id = uuid::Uuid::new_v4().to_string();
    newer.action_id = None;
    r.f.service()
        .save_operation_plan(store, &newer, &r.operation_id, choice.epoch.unwrap())
        .unwrap();
    assert_eq!(r.view().plan_id.as_deref(), Some(newer.plan_id.as_str()));
    let refused = r.f.service().cancel_operation(&r.operation_id).unwrap_err();
    assert_eq!(
        refused.action_id.as_deref(),
        Some(action.action_id.as_str())
    );
    assert_ne!(r.view().stage, OperationStage::Cancelled);
    // The newer plan cannot run either: no second move.
    assert_eq!(
        r.f.service().get_plan(&newer.plan_id).unwrap().status,
        crate::contracts::PlanStatus::Cancelled
    );

    // Cancelled before confirmation, the plan can no longer run.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let plan = r.plan_candidate("C1").unwrap();
    assert_eq!(
        r.f.service()
            .cancel_operation(&r.operation_id)
            .unwrap()
            .stage,
        OperationStage::Cancelled
    );
    assert_eq!(
        r.f.apply(&plan.plan_id).unwrap_err().code,
        ErrorCode::StalePlan
    );
    assert!(r.f.path("trip.md").exists());
}

#[test]
fn a_failed_step_can_be_retried_with_the_locked_answer_and_no_new_question() {
    // "首次或回答后请求超时，再显式重试 → 新请求有效；回答锁定与追问次数保留"
    let r = run(|ids| {
        vec![
            ok(clarify(json!([]))),
            Planned::answer(503, b"{}".to_vec()),
            ok(recommend(&ids["Travel"], 0.9)),
        ]
    });
    let question = r.analyze().question.unwrap();
    r.answer(Some("O2"), None).unwrap();
    let failed = r.work();
    assert_eq!(failed.stage, OperationStage::AnalysisFailed);
    assert!(failed.error.unwrap().retryable);

    let retrying = r.start().unwrap();
    assert_eq!(retrying.stage, OperationStage::Analyzing);
    let done = r.work();
    assert_eq!(done.stage, OperationStage::RecommendationReady);
    assert_eq!(done.question.unwrap(), question, "never asked again");
    assert_eq!(done.answer.as_deref(), Some("Holiday"));
    assert_eq!((done.model_requests, r.server.count()), (3, 3));
    assert_eq!(
        r.sent_data(2)["answered_questions"][0]["answer"],
        json!("Holiday")
    );
    let contexts = r.contexts();
    assert_eq!(contexts[2].1["retry"], json!(true));
    assert_eq!(contexts[2].1["round"], json!("answered"));
}

#[test]
fn an_invalid_answer_can_be_retried_but_a_rejected_key_cannot() {
    let r = run(|ids| {
        vec![
            ok(json!({ "disposition": "move" })),
            ok(recommend(&ids["Travel"], 0.9)),
        ]
    });
    assert_eq!(r.analyze().stage, OperationStage::InvalidModelOutput);
    assert_eq!(r.analyze().stage, OperationStage::RecommendationReady);

    let r = run(|_| vec![Planned::answer(401, b"{}".to_vec())]);
    let failed = r.analyze();
    assert_eq!(failed.stage, OperationStage::AnalysisFailed);
    assert_eq!(failed.error.unwrap().code, ErrorCode::ModelKeyRejected);
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);
    assert_eq!(
        r.f.service()
            .choose_manual_location(&r.operation_id)
            .unwrap()
            .stage,
        OperationStage::NeedsManualLocation
    );
    assert_eq!(r.server.count(), 1);
}

#[test]
fn a_file_changed_or_a_folder_revoked_while_waiting_drops_the_result() {
    // "文件变化、授权撤销 → 旧结果不能应用"
    for what in ["edit", "revoke"] {
        let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
        r.start().unwrap();
        thread::scope(|scope| {
            let worker = scope.spawn(|| r.work());
            r.wait_for_requests(1);
            match what {
                "edit" => fs::write(r.f.path("trip.md"), "Something else entirely.\n").unwrap(),
                _ => {
                    r.f.service().revoke_root(&r.f.root_id).unwrap();
                }
            }
            r.server.release();
            worker.join().unwrap();
        });
        let view = r.view();
        assert!(view.suggestion.is_none(), "{what}");
        match what {
            "edit" => {
                assert_eq!(view.stage, OperationStage::Stale);
                let error = view.error.unwrap();
                assert_eq!(error.code, ErrorCode::FileChanged);
                assert_eq!(error.allowed_next, [crate::contracts::NextStep::ChooseFile]);
            }
            _ => {
                assert_eq!(view.stage, OperationStage::Blocked);
                assert_eq!(view.error.unwrap().code, ErrorCode::RootRevoked);
            }
        }
        // "手选不能绕过安全失败": no manual choice within the operation, and
        // a plain manual plan meets the same failure.
        assert_eq!(
            r.f.service()
                .choose_manual_location(&r.operation_id)
                .unwrap_err()
                .code,
            ErrorCode::WrongStage
        );
        assert_eq!(
            r.plan_candidate("C1").unwrap_err().code,
            ErrorCode::SuggestionOutdated
        );
        if what == "edit" {
            let listing = r.f.service().list_folders(&r.f.root_id).unwrap();
            let target = PlanTarget::Folder {
                catalog_id: listing.catalog_id,
                folder_id: listing.folders[0].folder_id.clone(),
                new_folders: vec!["Trips".to_string()],
            };
            let error =
                r.f.service()
                    .build_plan(&r.request(target, true))
                    .unwrap_err();
            assert_eq!(error.code, ErrorCode::WrongStage);
            let listing = r.f.service().list_folders(&r.f.root_id).unwrap();
            let target = PlanTarget::Folder {
                catalog_id: listing.catalog_id,
                folder_id: listing.folders[0].folder_id.clone(),
                new_folders: vec!["Trips".to_string()],
            };
            let error =
                r.f.service()
                    .build_plan(&r.request(target, false))
                    .unwrap_err();
            assert_eq!(error.code, ErrorCode::FileChanged);
        }
        assert_eq!(r.f.count("plans"), 0);
    }
}

#[test]
fn an_attribute_added_while_waiting_keeps_the_result_but_a_hidden_edit_does_not() {
    // macOS or another app adds an attribute while the model thinks (10/14:
    // a PDF dropped from the Desktop): only ctime moves, the bytes are the
    // registered ones, and the result stands. An edit that keeps size and
    // mtime moves only ctime too, and the result is dropped.
    for what in ["attribute", "edit"] {
        let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
        r.start().unwrap();
        thread::scope(|scope| {
            let worker = scope.spawn(|| r.work());
            r.wait_for_requests(1);
            match what {
                "attribute" => {
                    xattr::set(r.f.path("trip.md"), "com.traeva.test", b"1").unwrap();
                }
                _ => edit_keeping_size_and_mtime(
                    &r.f.path("trip.md"),
                    &TEXT.replace("Kyoto", "Osaka"),
                ),
            }
            r.server.release();
            worker.join().unwrap();
        });
        let view = r.view();
        match what {
            "attribute" => {
                assert_eq!(view.stage, OperationStage::RecommendationReady);
                assert!(view.error.is_none());
                assert!(r.plan_candidate("C1").unwrap().can_confirm);
            }
            _ => {
                assert_eq!(view.stage, OperationStage::Stale);
                assert_eq!(view.error.unwrap().code, ErrorCode::FileChanged);
            }
        }
    }
}

#[test]
fn a_recommendation_becomes_a_plan_only_through_build_plan_with_its_choice_recorded() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    let before = snapshot::snapshot(r.f.root.path());
    let request_id = r.analyze().suggestion.unwrap().request_id;

    // Accepting the recommended candidate.
    let accepted = r.plan_candidate("C1").unwrap();
    assert_eq!(accepted.target.relative_path, "Travel/trip.md");
    assert_eq!(accepted.user_choice, Some(UserChoice::Accept));
    assert_eq!(
        accepted.analysis_request_id.as_deref(),
        Some(request_id.as_str())
    );
    assert_eq!(
        r.stored_choice(&accepted.plan_id),
        ("accept".to_string(), Some(request_id.clone()))
    );
    assert_eq!(r.view().plan_id.as_deref(), Some(accepted.plan_id.as_str()));

    // Another target while the recommendation is shown: a correction.
    let corrected = r.plan_folder("Notes").unwrap();
    assert_eq!(corrected.user_choice, Some(UserChoice::Correct));
    assert_eq!(
        corrected.analysis_request_id.as_deref(),
        Some(request_id.as_str())
    );
    assert_eq!(r.view().stage, OperationStage::RecommendationReady);

    // Nothing was written and nothing ran: preview and confirmation remain.
    assert_eq!(r.f.count("actions"), 0);
    assert_eq!(snapshot::snapshot(r.f.root.path()), before);

    // Cancelling ends the candidates and the open plan of the operation.
    r.f.service().cancel_operation(&r.operation_id).unwrap();
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
    assert_eq!(
        r.f.service().get_plan(&corrected.plan_id).unwrap().status,
        crate::contracts::PlanStatus::Cancelled
    );
    assert_eq!(r.f.count("plans"), 2);
}

#[test]
fn offered_candidates_in_a_manual_choice_are_recorded_as_manual() {
    let r = run(|ids| {
        let mut answer = recommend(&ids["Travel"], 0.99);
        answer["candidates"] = json!([
            candidate("C1", &ids["Travel"], &["E1"]),
            candidate("C2", &ids["Notes"], &["E2"])
        ]);
        vec![ok(answer)]
    });
    let request_id = r.analyze().suggestion.unwrap().request_id;
    let plan = r.plan_candidate("C2").unwrap();
    assert_eq!(plan.target.relative_path, "Notes/trip.md");
    assert_eq!(plan.user_choice, Some(UserChoice::Manual));
    assert_eq!(
        plan.analysis_request_id.as_deref(),
        Some(request_id.as_str())
    );
    assert_eq!(
        r.plan_candidate("C9").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
}

#[test]
fn candidates_of_an_older_result_are_refused() {
    // "取消、提交回答或开始重试之后，旧结果里的候选一律拒绝"
    let r = run(|ids| {
        vec![
            ok(clarify(json!([candidate("C1", &ids["Travel"], &["E1"])]))),
            Planned::answer(503, b"{}".to_vec()),
            ok(recommend(&ids["Notes"], 0.9)),
        ]
    });
    r.analyze();
    // While the question is shown, its candidates are not choosable.
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
    r.answer(Some("O1"), None).unwrap();
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
    r.work(); // fails
    r.start().unwrap(); // retry
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::SuggestionOutdated
    );
    r.work();
    // Only the new result's candidate works now, with its own folder.
    assert_eq!(
        r.plan_candidate("C1").unwrap().target.relative_path,
        "Notes/trip.md"
    );
    assert_eq!(r.f.count("plans"), 1);
}

#[test]
fn a_manual_pick_ends_the_analysis_for_good() {
    // From ready_for_analysis, by building a plan: recorded as manual.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    let plan = r.plan_folder("Notes").unwrap();
    assert_eq!(plan.user_choice, Some(UserChoice::Manual));
    assert_eq!(plan.analysis_request_id, None);
    assert_eq!(r.view().stage, OperationStage::NeedsManualLocation);
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);

    // From ready_for_analysis, by choosing manual.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.f.service()
        .choose_manual_location(&r.operation_id)
        .unwrap();
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);
    assert_eq!(r.server.count(), 0);
}

#[test]
fn a_candidate_folder_that_changed_since_the_suggestion_is_refused() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    fs::rename(r.f.path("Travel"), r.f.path("Trips")).unwrap();
    fs::create_dir(r.f.path("Travel")).unwrap(); // another folder, same name
    assert_eq!(
        r.plan_candidate("C1").unwrap_err().code,
        ErrorCode::FolderListOutdated
    );
    assert_eq!(r.f.count("plans"), 0);
}

#[test]
fn a_plan_is_saved_only_if_the_operation_did_not_move_on() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let first = r.plan_candidate("C1").unwrap();
    // The same plan once more, as if a second click raced a cancel: the
    // choice was made before the cancel, the save comes after it.
    let request = r.request(
        PlanTarget::Candidate {
            candidate_id: "C1".to_string(),
        },
        true,
    );
    let choice = r.f.service().plan_choice(&request).unwrap();
    r.f.service().cancel_operation(&r.operation_id).unwrap();
    let store = r.f.service().store().unwrap();
    let mut late = crate::plan::load_plan(store, &first.plan_id).unwrap();
    late.plan_id = uuid::Uuid::new_v4().to_string();
    let error =
        r.f.service()
            .save_operation_plan(store, &late, &r.operation_id, choice.epoch.unwrap())
            .unwrap_err();
    assert_eq!(error.code, ErrorCode::SuggestionOutdated);
    assert_eq!(error.allowed_next, [crate::contracts::NextStep::ChooseFile]);
    // Nothing was saved, and the first plan was cancelled with the operation.
    assert_eq!(r.f.count("plans"), 1);
    assert_eq!(
        r.f.service().get_plan(&first.plan_id).unwrap().status,
        crate::contracts::PlanStatus::Cancelled
    );
}

#[test]
fn the_operation_always_knows_its_latest_plan() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    let first = r.plan_candidate("C1").unwrap();
    let second = r.plan_folder("Notes").unwrap();
    assert_eq!(r.view().plan_id.as_deref(), Some(second.plan_id.as_str()));
    assert_eq!(
        r.f.service().get_plan(&first.plan_id).unwrap().status,
        crate::contracts::PlanStatus::Superseded
    );
    r.f.service().cancel_operation(&r.operation_id).unwrap();
    assert_eq!(
        r.f.service().get_plan(&second.plan_id).unwrap().status,
        crate::contracts::PlanStatus::Cancelled
    );
}

#[test]
fn a_bad_new_folder_name_changes_nothing_in_the_operation() {
    // Invalid input is refused before the operation is touched: no switch
    // to a manual choice, no request cancelled.
    let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
    let ready = r.view();
    let listing = r.f.service().list_folders(&r.f.root_id).unwrap();
    let bad = |names: &[&str]| PlanTarget::Folder {
        catalog_id: listing.catalog_id.clone(),
        folder_id: listing.folders[0].folder_id.clone(),
        new_folders: names.iter().map(|n| n.to_string()).collect(),
    };
    let refused =
        r.f.service()
            .build_plan(&r.request(bad(&["a/b"]), true))
            .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidFolderName);
    assert_eq!(r.view(), ready);

    r.start().unwrap();
    thread::scope(|scope| {
        let worker = scope.spawn(|| r.work());
        r.wait_for_requests(1);
        let analyzing = r.view();
        let refused =
            r.f.service()
                .build_plan(&r.request(bad(&["", ".."]), true))
                .unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidFolderName);
        assert_eq!(r.view(), analyzing);
        r.server.release();
        worker.join().unwrap();
    });
    assert_eq!(r.view().stage, OperationStage::RecommendationReady);
    assert_eq!(r.f.count("plans"), 0);
}

#[test]
fn picking_a_folder_during_an_analysis_is_the_manual_choice_even_if_refused() {
    // A folder pick ends the analysis (the material's "用户选择手选"): if the
    // plan is then refused, the user picks again by hand.
    let r = run(|ids| vec![Planned::gated(completed(&recommend(&ids["Travel"], 0.9)))]);
    fs::write(r.f.path("Notes/trip.md"), "someone else's file\n").unwrap();
    r.start().unwrap();
    thread::scope(|scope| {
        let worker = scope.spawn(|| r.work());
        r.wait_for_requests(1);
        let taken = r.plan_folder("Notes").unwrap_err();
        assert_eq!(taken.code, ErrorCode::TargetConflict);
        assert_eq!(r.view().stage, OperationStage::NeedsManualLocation);
        r.server.release();
        worker.join().unwrap();
    });
    // The late answer was dropped, and another folder can be picked.
    assert!(r.view().suggestion.is_none());
    let plan = r.plan_folder("Taxes").unwrap();
    assert_eq!(plan.user_choice, Some(UserChoice::Manual));
    assert_eq!(r.start().unwrap_err().code, ErrorCode::WrongStage);
}

#[test]
fn a_setup_refused_during_the_run_can_be_retried_once_fixed() {
    // Consent is withdrawn after the start but before the run: nothing is
    // sent; once consent is back, the same step can be retried.
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.start().unwrap();
    r.f.service().set_analysis_consent(false).unwrap();
    let failed = r.work();
    assert_eq!(failed.stage, OperationStage::AnalysisFailed);
    let error = failed.error.unwrap();
    assert_eq!(error.code, ErrorCode::ConsentRequired);
    // The window offers the retry because the error says so.
    assert!(error.retryable);
    assert_eq!(
        error.allowed_next,
        [
            crate::contracts::NextStep::TryAgain,
            crate::contracts::NextStep::ChooseFolder
        ]
    );
    assert_eq!(r.server.count(), 0);
    // Still refused while consent is off, and nothing changes.
    assert_eq!(r.start().unwrap_err().code, ErrorCode::ConsentRequired);
    r.f.service().set_analysis_consent(true).unwrap();
    assert_eq!(r.analyze().stage, OperationStage::RecommendationReady);
    assert_eq!(r.server.count(), 1);
}

#[test]
fn without_settings_or_consent_nothing_starts_and_nothing_changes() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    r.f.service().set_analysis_consent(false).unwrap();
    let before = r.view();
    assert_eq!(r.start().unwrap_err().code, ErrorCode::ConsentRequired);
    assert_eq!(r.view(), before);
    assert_eq!(r.server.count(), 0);
}

#[test]
fn a_file_without_text_cannot_be_analyzed() {
    let r = run(|ids| vec![ok(recommend(&ids["Travel"], 0.9))]);
    fs::write(r.f.path("scan.pdf"), b"%PDF-1.7 synthetic").unwrap();
    let started =
        r.f.service()
            .start_operation(&r.f.root_id, &r.f.root_id)
            .unwrap();
    r.f.service().run_operation_with(
        &started.operation_id,
        &r.f.path("scan.pdf"),
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    );
    assert_eq!(
        r.f.service()
            .get_operation(&started.operation_id)
            .unwrap()
            .stage,
        OperationStage::NeedsManualLocation
    );
    assert_eq!(
        r.f.service()
            .start_analysis(&started.operation_id)
            .unwrap_err()
            .code,
        ErrorCode::WrongStage
    );
    assert_eq!(r.server.count(), 0);
}

#[test]
fn a_restart_ends_the_operation_and_sends_nothing_again() {
    // "窗口刷新、应用重启 → 不自动重发，不重复应用"
    let mut r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    r.analyze();
    // A refresh only reads the operation again.
    assert_eq!(r.view(), r.view());
    r.f.restart();
    assert_eq!(
        r.f.service()
            .get_operation(&r.operation_id)
            .unwrap_err()
            .code,
        ErrorCode::UnknownOperation
    );
    assert_eq!(r.server.count(), 1);
}

#[test]
fn the_whole_flow_writes_nothing_before_confirmation() {
    let r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    let before = snapshot::snapshot(r.f.root.path());
    r.analyze();
    r.answer(Some("O2"), None).unwrap();
    r.work();
    r.plan_candidate("C1").unwrap();
    assert_eq!(snapshot::snapshot(r.f.root.path()), before);
    assert_eq!(r.f.count("actions"), 0);
}

#[test]
fn the_page_flow_runs_through_the_same_plan_preview_and_executor() {
    // #17: question → answer → recommendation → preview → another folder
    // (a new plan) → confirm → Undo, in the order the page calls the core.
    let r = run(|ids| vec![ok(clarify(json!([]))), ok(recommend(&ids["Travel"], 0.9))]);
    let before = snapshot::snapshot(r.f.root.path());
    assert_eq!(
        r.analyze().stage,
        OperationStage::NeedsClarification,
        "the model's question is shown"
    );
    let answered = r.answer(None, Some("ANSWERCANARY a client visit")).unwrap();
    assert_eq!(answered.stage, OperationStage::Analyzing);
    let shown = r.work();
    assert_eq!(shown.stage, OperationStage::RecommendationReady);
    // A model success is no action: nothing was planned or run.
    assert_eq!((shown.plan_id, shown.action_id), (None, None));

    // The preview of the recommendation, then "Choose another folder": the
    // page closes the shown plan and the user picks Notes instead.
    let accepted = r.plan_candidate("C1").unwrap();
    assert!(accepted.can_confirm);
    let closed = r.f.service().cancel_plan(&accepted.plan_id).unwrap();
    assert_eq!(closed.status, crate::contracts::PlanStatus::Cancelled);
    assert_eq!(r.view().stage, OperationStage::RecommendationReady);
    let corrected = r.plan_folder("Notes").unwrap();
    assert_ne!(corrected.plan_id, accepted.plan_id);
    assert_eq!(corrected.user_choice, Some(UserChoice::Correct));
    assert_eq!(corrected.target.relative_path, "Notes/trip.md");
    assert_eq!(snapshot::snapshot(r.f.root.path()), before);
    assert_eq!(r.f.count("actions"), 0);

    // Only the plan on screen runs, through the Gate 1 executor.
    assert_eq!(
        r.f.apply(&accepted.plan_id).unwrap_err().code,
        ErrorCode::StalePlan
    );
    let action = r.f.apply(&corrected.plan_id).unwrap();
    assert_eq!(action.status, crate::contracts::ActionStatus::Committed);
    assert_eq!(
        r.view().action_id.as_deref(),
        Some(action.action_id.as_str())
    );
    assert!(r.f.path("Notes/trip.md").exists());
    assert_eq!(r.f.count("actions"), 1);

    // Neither the file's text nor the user's answer is kept in app-data;
    // the checked result is (spec §5), which shows the search sees the DB.
    let app_data = r.f.app.path().join("Traeva");
    assert!(!crate::model::tests::files_containing(&app_data, "Notes for a trip.").is_empty());
    for needle in ["train tickets", "rain jacket", "ANSWERCANARY", "KEYCANARY"] {
        assert_eq!(
            crate::model::tests::files_containing(&app_data, needle),
            Vec::<String>::new(),
            "{needle}"
        );
    }

    // Undo is its own preview and confirmation.
    let undo = match r.f.service().preview_undo(&action.action_id).unwrap() {
        crate::contracts::UndoPreview::Planned { plan } => plan,
        other => panic!("expected an undo plan, got {other:?}"),
    };
    assert_eq!(undo.target.relative_path, "trip.md");
    let undone = r.f.apply(&undo.plan_id).unwrap();
    assert_eq!(undone.status, crate::contracts::ActionStatus::Committed);
    assert_eq!(fs::read_to_string(r.f.path("trip.md")).unwrap(), TEXT);
    assert!(!r.f.path("Notes/trip.md").exists());
}

// ---------------------------------------------------------------------------
// #20: new folders from the model, and failed requests at operation level
// ---------------------------------------------------------------------------

/// A recommendation with two new folders becomes a preview that lists both,
/// and nothing is created before the confirmation (O05).
#[test]
fn a_recommended_new_folder_path_is_previewed_in_full_and_not_created() {
    let r = run(|ids| {
        let mut answer = recommend(&ids["Travel"], 0.9);
        answer["candidates"][0]["new_segments"] = json!(["Kyoto", "2026"]);
        vec![ok(answer)]
    });
    let before = snapshot::snapshot(r.f.root.path());
    let view = r.analyze();
    assert_eq!(view.stage, OperationStage::RecommendationReady);
    assert_eq!(
        view.suggestion.unwrap().candidates[0].new_segments,
        ["Kyoto", "2026"]
    );

    let plan = r.plan_candidate("C1").unwrap();

    assert_eq!(
        plan.folders_to_create,
        ["Travel/Kyoto", "Travel/Kyoto/2026"]
    );
    assert_eq!(plan.target.relative_path, "Travel/Kyoto/2026/trip.md");
    assert!(plan.can_confirm);
    assert!(!r.f.path("Travel/Kyoto").exists());
    assert_eq!(snapshot::snapshot(r.f.root.path()), before);
    assert_eq!(r.f.count("actions"), 0);
}

/// The provider fails, cannot be reached or is too slow: the operation shows
/// the same error the record keeps, offers trying again or choosing by hand,
/// and nothing is planned, done or written (O14).
#[test]
fn a_failed_request_offers_retry_or_a_manual_choice_and_changes_nothing() {
    for case in ["503", "429", "unreachable", "timeout"] {
        let mut r = run(|_| {
            vec![match case {
                "503" => Planned::answer(503, b"{}".to_vec()),
                "429" => Planned::answer(429, b"{}".to_vec()),
                "timeout" => Planned::gated(completed(&json!({}))),
                _ => Planned::answer(200, completed(&json!({}))),
            }]
        });
        if case == "unreachable" {
            let closed = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap();
            r.f.service_mut().model.endpoint = format!("http://{closed}/v1/responses");
        }
        if case == "timeout" {
            r.f.service_mut().model.timeout = Duration::from_millis(300);
        }
        let before = snapshot::snapshot(r.f.root.path());

        let view = r.analyze();
        r.server.release();

        assert_eq!(view.stage, OperationStage::AnalysisFailed, "{case}");
        let error = view.error.clone().unwrap();
        assert!(error.retryable, "{case}");
        assert!(error.allowed_next.contains(&NextStep::TryAgain), "{case}");
        assert!(
            error.allowed_next.contains(&NextStep::ChooseFolder),
            "{case}"
        );
        assert!(
            view.suggestion.is_none() && view.plan_id.is_none(),
            "{case}"
        );
        let stored: (String, Option<String>) =
            r.f.db()
                .query_row("SELECT status, error_code FROM analyses", [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap();
        assert_eq!(stored.0, "failed", "{case}");
        assert_eq!(
            stored.1.map(Value::String),
            Some(serde_json::to_value(error.code).unwrap()),
            "{case}: the window shows what the record says"
        );
        assert_eq!((r.f.count("plans"), r.f.count("actions")), (0, 0), "{case}");
        assert_eq!(snapshot::snapshot(r.f.root.path()), before, "{case}");

        // Choosing by hand still works.
        r.f.service()
            .choose_manual_location(&r.operation_id)
            .unwrap();
        assert!(r.plan_folder("Travel").unwrap().can_confirm, "{case}");
    }
}
