//! Tests for auto_drop.rs (#168, organize-spec.md §13), all offline: the
//! local test server from the model tests plays OpenAI. Synthetic files and
//! folders only. The acceptance rows A01–A12 of §13.9 are named in the
//! comments.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::contracts::{BuildOutcome, ClarificationAnswer, PlanRequest, PlanTarget, UndoStatus};
use crate::model::tests::{Planned, TestServer, completed};
use crate::settings::DEFAULT_MODEL;
use crate::settings::tests::{TEST_KEY, openai};
use crate::test_fixture::{Fixture, quick_read};
use crate::test_snapshot as snapshot;

const TEXT: &str = "Trip to Kyoto in May: train tickets and a hotel near the station.\n\n\
                    Budget about 400 USD for the week.\n\n\
                    Packing list: rain jacket and walking shoes.\n";

fn ok(answer: Value) -> Planned {
    Planned::answer(200, completed(&answer))
}

fn candidate(folder_id: &str, new_segments: &[&str]) -> Value {
    json!({
        "id": "C1",
        "folder_id": folder_id,
        "new_segments": new_segments,
        "reason": "Fits the trip.",
        "evidence_ids": ["E1"]
    })
}

fn recommend_into(folder_id: &str, new_segments: &[&str]) -> Value {
    json!({
        "disposition": "recommend",
        "summary": "Notes for a trip.",
        "candidates": [candidate(folder_id, new_segments)],
        "preferred_id": "C1",
        "confidence": 0.9,
        "uncertain_field": null,
        "clarification": null
    })
}

/// For a test that sends nothing: the server wants at least one answer.
fn never_sent() -> Planned {
    ok(json!({ "never": "sent" }))
}

fn recommend(folder_id: &str) -> Value {
    recommend_into(folder_id, &[])
}

fn clarify() -> Value {
    json!({
        "disposition": "clarify",
        "summary": "Notes about a trip.",
        "candidates": [],
        "preferred_id": null,
        "confidence": null,
        "uncertain_field": "purpose",
        "clarification": { "question": "Is this for work or for a holiday?",
                           "options": ["Work", "Holiday"] }
    })
}

struct Setup {
    f: Fixture,
    server: TestServer,
}

/// A root with the sample tree plus Travel/ and Work/, trip.md at the top,
/// model settings and Organize's sending agreement, and a server giving the
/// answers `answers(ids)` in turn (folder IDs by path).
fn setup(answers: impl FnOnce(&HashMap<String, String>) -> Vec<Planned>) -> Setup {
    let mut f = Fixture::new();
    fs::create_dir(f.path("Travel")).unwrap();
    fs::create_dir(f.path("Work")).unwrap();
    fs::write(f.path("trip.md"), TEXT).unwrap();
    let ids: HashMap<String, String> = f
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
    Setup { f, server }
}

impl Setup {
    fn service(&self) -> &OrganizeService {
        self.f.service()
    }

    fn turn_on(&self, scopes: &[&str]) -> AutoDropSettingsView {
        self.service()
            .set_auto_drop(&AutoDropRequest {
                enable: true,
                notice_version: Some(AUTO_DROP_NOTICE_VERSION),
                target_root_id: Some(self.f.root_id.clone()),
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
            })
            .unwrap()
    }

    fn drop_files(&self, names: &[&str]) -> DropQueueView {
        let paths: Vec<PathBuf> = names.iter().map(|name| self.f.path(name)).collect();
        self.service().receive_drops(&paths).unwrap()
    }

    fn run(&self) -> DropQueueView {
        self.service()
            .run_drops_with(&mut |root, parent, name| quick_read(parent, name, root.id().dev));
        self.service().drop_queue()
    }

    fn event(&self, drop_id: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        self.f
            .db()
            .query_row(
                "SELECT outcome, consent_id, plan_id, action_id FROM organize_drop_events
                 WHERE drop_id = ?1",
                [drop_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    fn answer(&self, operation_id: &str, option: &str) {
        let question = self
            .service()
            .get_operation(operation_id)
            .unwrap()
            .question
            .unwrap();
        self.service()
            .answer_clarification(
                operation_id,
                &ClarificationAnswer {
                    question_id: question.question_id,
                    option_id: Some(option.to_string()),
                    text: None,
                },
            )
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Turning the mode on and off (A01)
// ---------------------------------------------------------------------------

#[test]
fn the_mode_is_off_until_the_user_agrees_and_a_new_range_needs_a_new_agreement() {
    let s = setup(|_| vec![never_sent()]);
    let off = s.service().auto_drop_settings().unwrap();
    assert!(!off.enabled);
    assert_eq!(off.consent_id, None);
    assert_eq!(off.notice_version, AUTO_DROP_NOTICE_VERSION);
    assert!(off.notice.contains("never creates folders"));

    // An old or missing notice version, no or unknown folders: refused,
    // nothing saved.
    let request = |version: Option<u32>, scopes: &[&str]| AutoDropRequest {
        enable: true,
        notice_version: version,
        target_root_id: Some(s.f.root_id.clone()),
        scopes: scopes.iter().map(|x| x.to_string()).collect(),
    };
    for refused in [
        request(None, &["Travel"]),
        request(Some(AUTO_DROP_NOTICE_VERSION + 1), &["Travel"]),
        request(Some(AUTO_DROP_NOTICE_VERSION), &[]),
        request(Some(AUTO_DROP_NOTICE_VERSION), &["Nowhere"]),
        request(Some(AUTO_DROP_NOTICE_VERSION), &["../Travel"]),
        request(Some(AUTO_DROP_NOTICE_VERSION), &["/Travel"]),
    ] {
        assert!(s.service().set_auto_drop(&refused).is_err(), "{refused:?}");
    }
    assert_eq!(s.f.count("organize_auto_consents"), 0);

    let on = s.turn_on(&["Travel"]);
    assert!(on.enabled, "{on:?}");
    assert_eq!(on.scopes.len(), 1);
    assert_eq!(on.scopes[0].relative_path, "Travel");
    assert!(on.scopes[0].valid);
    let first = on.consent_id.clone().unwrap();

    // Another range is a new agreement; the old one is withdrawn.
    let again = s.turn_on(&["Travel", "Work"]);
    assert_ne!(again.consent_id.as_deref(), Some(first.as_str()));
    let open: i64 =
        s.f.db()
            .query_row(
                "SELECT count(*) FROM organize_auto_consents WHERE withdrawn_at_ms IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(open, 1);

    // Off withdraws it.
    let off = s
        .service()
        .set_auto_drop(&AutoDropRequest {
            enable: false,
            notice_version: None,
            target_root_id: None,
            scopes: Vec::new(),
        })
        .unwrap();
    assert!(!off.enabled);
    assert_eq!(off.consent_id, None);
}

#[test]
fn a_range_whose_folders_are_gone_no_longer_counts() {
    let s = setup(|_| vec![never_sent()]);
    s.turn_on(&["Travel"]);
    fs::remove_dir(s.f.path("Travel")).unwrap();
    let view = s.service().auto_drop_settings().unwrap();
    assert!(!view.enabled);
    assert!(!view.scopes[0].valid);
    assert!(view.problem.is_some());
}

// ---------------------------------------------------------------------------
// Moving without a confirmation page (A05, A08, A10)
// ---------------------------------------------------------------------------

#[test]
fn a_clear_file_is_moved_into_the_range_recorded_and_can_be_undone() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    let on = s.turn_on(&["Travel"]);
    let before = snapshot::snapshot(s.f.root.path());

    let queued = s.drop_files(&["trip.md"]);
    assert_eq!(queued.items[0].state, DropState::Waiting);
    // Receiving the drop writes nothing in the folder (A08).
    assert_eq!(snapshot::snapshot(s.f.root.path()), before);

    let queue = s.run();
    let item = &queue.items[0];
    assert_eq!(item.state, DropState::Moved, "{item:?}");
    assert!(item.automatic);
    assert!(
        item.moved_to
            .as_deref()
            .unwrap()
            .ends_with("Travel/trip.md")
    );
    assert!(s.f.path("Travel/trip.md").exists());
    assert!(!s.f.path("trip.md").exists());
    assert_eq!(s.server.count(), 1);

    // The record names the agreement, the plan and the action (A10).
    let (outcome, consent, plan, action) = s.event(&item.drop_id);
    assert_eq!(outcome, "moved");
    assert_eq!(consent, on.consent_id);
    assert_eq!(plan, item.plan_id);
    assert_eq!(action.as_deref(), item.action_id.as_deref());

    // The normal executor ran: one action, committed, with Undo offered.
    let action = s
        .service()
        .get_action(item.action_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(action.undo_status, UndoStatus::Available);
    assert_eq!(s.f.count("actions"), 1);
    // The queue says how it ended, for the desktop pet (#208).
    let item = s.service().drop_queue().items[0].clone();
    assert!(!item.needs_user);
    assert_eq!(item.action_status, Some(ActionStatus::Committed));
}

/// 10/14 (RC2): a file dropped from the Desktop got an attribute from macOS
/// or another app while Traeva worked on it, and stopped as "changed". Only
/// its ctime moved and its bytes are the registered ones, so it still moves,
/// attribute and all.
#[test]
fn a_file_that_gets_an_attribute_after_it_was_read_is_still_moved() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let mut reads = 0;
    s.service().run_drops_with(&mut |root, parent, name| {
        reads += 1;
        let read = quick_read(parent, name, root.id().dev);
        if reads == 2 {
            // The read for the request: the attribute comes after it.
            xattr::set(s.f.path("trip.md"), "com.traeva.test", b"1").unwrap();
        }
        read
    });

    let item = s.service().drop_queue().items[0].clone();
    assert_eq!(item.state, DropState::Moved, "{item:?}");
    assert_eq!(
        xattr::get(s.f.path("Travel/trip.md"), "com.traeva.test")
            .unwrap()
            .as_deref(),
        Some(&b"1"[..])
    );
}

/// #201: with the switch for keeping versions on, a file moved from the
/// drop queue starts being recorded too, with Memory's agreement.
#[test]
fn a_moved_drop_is_recorded_when_versions_are_kept() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.service()
        .set_auto_memory(&crate::AutoMemoryRequest {
            enable: true,
            notice_version: crate::AUTO_MEMORY_NOTICE_VERSION,
        })
        .unwrap();

    s.drop_files(&["trip.md"]);
    let queue = s.run();

    assert_eq!(queue.items[0].state, DropState::Moved);
    let watches = s.service().memory_watches().unwrap();
    assert_eq!(watches.len(), 1);
    assert_eq!(watches[0].relative_path, "Travel/trip.md");
    assert!(
        s.service()
            .counting_consent(&watches[0].file_id)
            .unwrap()
            .is_some()
    );
}

/// #186 (organize-spec §14.5): a dropped PDF takes the same way, its text
/// layer read in the child (a stand-in reader here).
#[test]
fn a_dropped_pdf_is_moved_like_a_text_file() {
    fn pdf_text(_bytes: &[u8]) -> crate::memory_contracts::TextExtraction {
        crate::memory_contracts::TextExtraction {
            summary: crate::memory_contracts::TextSummary {
                reader: "pdf-1".to_string(),
                status: crate::memory_contracts::TextStatus::Complete,
                chars: TEXT.chars().count(),
                covered: Vec::new(),
                left_out: Vec::new(),
                note: None,
            },
            text: TEXT.to_string(),
        }
    }
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    fs::write(s.f.path("trip.pdf"), b"%PDF-1.7 stand-in\n").unwrap();
    s.service()
        .set_pdf_helper(Some(crate::PdfHelper::in_process(pdf_text)));
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.pdf"]);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert!(s.f.path("Travel/trip.pdf").exists());
}

/// §14.8 (v0.7): text from on-device recognition may contain mistakes, so
/// a scanned PDF is analyzed but never moved automatically: it waits at its
/// recommendation for the normal preview.
/// v0.14 (the student, 10/15): a scanned PDF whose text was recognized on
/// this Mac moves by itself once every condition holds, like any text.
#[test]
fn a_dropped_scanned_pdf_whose_text_was_recognized_moves_automatically() {
    fn scanned(_bytes: &[u8]) -> crate::memory_contracts::TextExtraction {
        crate::memory_contracts::TextExtraction {
            summary: crate::memory_contracts::TextSummary {
                reader: "pdf-1".to_string(),
                status: crate::memory_contracts::TextStatus::Unsupported,
                chars: 0,
                covered: Vec::new(),
                left_out: Vec::new(),
                note: Some(
                    "No page has a text layer that this reader can use. 1 of 1 pages are \
                     mostly images (likely scans); reading them needs OCR, which this reader \
                     does not do."
                        .to_string(),
                ),
            },
            text: String::new(),
        }
    }
    fn small(_pid: u32) -> Option<crate::ProcessMemory> {
        Some(crate::ProcessMemory {
            footprint_bytes: 1 << 20,
            lifetime_peak_bytes: 1 << 20,
            kernel_lifetime_peak_bytes: 1 << 20,
        })
    }
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    fs::write(s.f.path("scan.pdf"), b"%PDF-1.7 scanned stand-in\n").unwrap();
    s.service()
        .set_pdf_helper(Some(crate::PdfHelper::in_process(scanned)));
    let sh = |script: &str| (PathBuf::from("/bin/sh"), vec!["-c".into(), script.into()]);
    s.service().set_ocr_helper(Some(crate::OcrHelper::commands(
        sh("/bin/cat > /dev/null; printf '%s' '{\"status\":\"partial\",\"text\":\"[Page 1]\\nTrip to Kyoto in May: train tickets.\",\"note\":null}'"),
        sh("printf '%s' '{\"status\":\"ready\"}'"),
        small,
    )));
    assert_eq!(s.service().start_ocr_preparation(), crate::OcrState::Ready);
    s.turn_on(&["Travel"]);
    let before = snapshot::snapshot(s.f.root.path());
    s.drop_files(&["scan.pdf"]);
    let queue = s.run();

    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert_eq!(s.server.count(), 1);
    assert_ne!(snapshot::snapshot(s.f.root.path()), before);
    assert_eq!(s.f.count("actions"), 1);
    assert!(s.f.root.path().join("Travel/scan.pdf").exists());
}

/// A scanned PDF dropped while text recognition gets ready (often the drop
/// that started Traeva) goes to the manual pick, and the queue says why: it
/// is not that Traeva cannot tell where the file goes.
#[test]
fn a_scanned_pdf_dropped_while_recognition_gets_ready_says_so() {
    fn scanned(_bytes: &[u8]) -> crate::memory_contracts::TextExtraction {
        crate::memory_contracts::TextExtraction {
            summary: crate::memory_contracts::TextSummary {
                reader: "pdf-1".to_string(),
                status: crate::memory_contracts::TextStatus::Unsupported,
                chars: 0,
                covered: Vec::new(),
                left_out: Vec::new(),
                note: Some(
                    "No page has a text layer that this reader can use. 1 of 1 pages are \
                     mostly images (likely scans); reading them needs OCR, which this reader \
                     does not do."
                        .to_string(),
                ),
            },
            text: String::new(),
        }
    }
    fn small(_pid: u32) -> Option<crate::ProcessMemory> {
        Some(crate::ProcessMemory {
            footprint_bytes: 1 << 20,
            lifetime_peak_bytes: 1 << 20,
            kernel_lifetime_peak_bytes: 1 << 20,
        })
    }
    // An answer is planned, but nothing is ever sent.
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    fs::write(s.f.path("scan.pdf"), b"%PDF-1.7 scanned stand-in\n").unwrap();
    s.service()
        .set_pdf_helper(Some(crate::PdfHelper::in_process(scanned)));
    let sh = |script: &str| (PathBuf::from("/bin/sh"), vec!["-c".into(), script.into()]);
    s.service().set_ocr_helper(Some(crate::OcrHelper::commands(
        sh("/bin/cat > /dev/null; printf '%s' '{\"status\":\"partial\",\"text\":\"[Page 1]\\nTrip\",\"note\":null}'"),
        // Never ready by itself: the test stops it after the drop.
        sh("exec /bin/sleep 30"),
        small,
    )));
    s.turn_on(&["Travel"]);
    let before = snapshot::snapshot(s.f.root.path());
    std::thread::scope(|scope| {
        let preparing = scope.spawn(|| s.service().start_ocr_preparation());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while s.service().ocr_state() != crate::OcrState::Preparing
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(s.service().ocr_state(), crate::OcrState::Preparing);
        s.drop_files(&["scan.pdf"]);
        let queue = s.run();
        assert_eq!(queue.items[0].state, DropState::HandedOver, "{queue:?}");
        let reason = queue.items[0].reason.clone().unwrap();
        assert!(reason.contains("still getting ready"), "{reason}");
        let operation = s
            .service()
            .get_operation(queue.items[0].operation_id.as_ref().unwrap())
            .unwrap();
        assert_eq!(operation.stage, OperationStage::NeedsManualLocation);
        // Still preparing after the drop; stopped as when the app quits.
        assert_eq!(s.service().ocr_state(), crate::OcrState::Preparing);
        s.service().stop_ocr();
        assert_eq!(preparing.join().unwrap(), crate::OcrState::Cancelled);
    });
    assert_eq!(s.server.count(), 0);
    assert_eq!(snapshot::snapshot(s.f.root.path()), before);
    assert_eq!(s.f.count("actions"), 0);
}

/// An encrypted PDF (one that opens without a password too) is never read
/// (§14); the queue says so instead of "cannot read text".
#[test]
fn a_protected_pdf_is_handed_over_saying_why() {
    fn encrypted(_bytes: &[u8]) -> crate::memory_contracts::TextExtraction {
        crate::memory_contracts::TextExtraction {
            summary: crate::memory_contracts::TextSummary {
                reader: "pdf-1".to_string(),
                status: crate::memory_contracts::TextStatus::Unsupported,
                chars: 0,
                covered: Vec::new(),
                left_out: Vec::new(),
                note: Some(crate::pdf::ENCRYPTED_NOTE.to_string()),
            },
            text: String::new(),
        }
    }
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    fs::write(s.f.path("handbook.pdf"), b"%PDF-1.7 encrypted stand-in\n").unwrap();
    s.service()
        .set_pdf_helper(Some(crate::PdfHelper::in_process(encrypted)));
    s.turn_on(&["Travel"]);
    let before = snapshot::snapshot(s.f.root.path());
    s.drop_files(&["handbook.pdf"]);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::HandedOver, "{queue:?}");
    let reason = queue.items[0].reason.clone().unwrap();
    assert!(reason.contains("protected by its author"), "{reason}");
    let operation = s
        .service()
        .get_operation(queue.items[0].operation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(operation.stage, OperationStage::NeedsManualLocation);
    assert_eq!(s.server.count(), 0, "nothing is sent");
    assert_eq!(snapshot::snapshot(s.f.root.path()), before);
    assert_eq!(s.f.count("actions"), 0);
}

#[test]
fn several_files_go_one_after_another() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"])), ok(recommend(&ids["Travel"]))]);
    fs::write(s.f.path("trip-2.md"), TEXT.replace("Kyoto", "Osaka")).unwrap();
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md", "trip-2.md"]);
    let queue = s.run();
    let states: Vec<DropState> = queue.items.iter().map(|item| item.state).collect();
    assert_eq!(states, [DropState::Moved, DropState::Moved], "{queue:?}");
    assert!(s.f.path("Travel/trip.md").exists() && s.f.path("Travel/trip-2.md").exists());
}

// ---------------------------------------------------------------------------
// Stopping instead of moving (A04, A06, A07, A09, A11)
// ---------------------------------------------------------------------------

#[test]
fn a_folder_that_must_be_created_goes_to_the_normal_preview_and_the_rest_is_not_processed() {
    let s = setup(|ids| vec![ok(recommend_into(&ids["Travel"], &["Japan"]))]);
    fs::write(s.f.path("trip-2.md"), TEXT).unwrap();
    s.turn_on(&["Travel"]);
    let before = snapshot::snapshot(s.f.root.path());
    s.drop_files(&["trip.md", "trip-2.md"]);
    let queue = s.run();

    assert_eq!(queue.items[0].state, DropState::HandedOver, "{queue:?}");
    assert!(
        queue.items[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("does not exist")
    );
    // The operation stays at its recommendation, for the normal preview.
    let operation = s
        .service()
        .get_operation(queue.items[0].operation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(operation.stage, OperationStage::RecommendationReady);
    // The next file is not processed (A09); nothing moved or was created.
    assert_eq!(queue.items[1].state, DropState::NotProcessed);
    assert_eq!(s.event(&queue.items[1].drop_id).0, "not_processed");
    assert_eq!(snapshot::snapshot(s.f.root.path()), before);
    assert_eq!(s.f.count("actions"), 0);
}

/// A stop reaches only the rest of its own drop (§13.1: "一次拖入多个文件
/// 时"); a file dropped separately is still organized, and each row tells
/// its drop and when it came (the UI session's check on the real app).
#[test]
fn a_stop_only_skips_the_rest_of_its_own_drop() {
    let s = setup(|ids| {
        vec![
            ok(recommend_into(&ids["Travel"], &["Japan"])),
            ok(recommend(&ids["Travel"])),
        ]
    });
    fs::write(s.f.path("trip-2.md"), TEXT.replace("Kyoto", "Osaka")).unwrap();
    fs::write(s.f.path("trip-3.md"), TEXT.replace("Kyoto", "Nara")).unwrap();
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md", "trip-3.md"]);
    s.drop_files(&["trip-2.md"]);
    let queue = s.run();
    let states: Vec<DropState> = queue.items.iter().map(|item| item.state).collect();
    assert_eq!(
        states,
        [
            DropState::HandedOver,
            DropState::NotProcessed,
            DropState::Moved
        ],
        "{queue:?}"
    );
    assert!(s.f.path("Travel/trip-2.md").exists());
    assert!(s.f.path("trip-3.md").exists());
    assert_eq!(queue.items[0].batch_id, queue.items[1].batch_id);
    assert_ne!(queue.items[0].batch_id, queue.items[2].batch_id);
    assert!(queue.items.iter().all(|item| item.received_at_ms > 0));
}

#[test]
fn a_folder_outside_the_range_goes_to_the_normal_preview() {
    let s = setup(|ids| vec![ok(recommend(&ids["Work"]))]);
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::HandedOver);
    assert!(
        queue.items[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("outside the target range")
    );
    assert!(s.f.path("trip.md").exists());
}

#[test]
fn a_question_pauses_the_queue_and_the_answer_lets_it_move() {
    let s = setup(|ids| vec![ok(clarify()), ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::NeedsAnswer, "{item:?}");
    s.answer(item.operation_id.as_ref().unwrap(), "O2");
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert_eq!(s.server.count(), 2);
}

/// The range changes while the question waits (same target folder): the
/// file moves under the new agreement, and its record names that one, not
/// the withdrawn one it was received under (§13.6, A10).
#[test]
fn a_move_after_the_agreement_changed_names_the_agreement_it_ran_under() {
    let s = setup(|ids| vec![ok(clarify()), ok(recommend(&ids["Travel"]))]);
    let first = s.turn_on(&["Travel"]).consent_id.unwrap();
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::NeedsAnswer, "{item:?}");
    let second = s.turn_on(&["Travel", "Work"]).consent_id.unwrap();
    assert_ne!(first, second);
    s.answer(item.operation_id.as_ref().unwrap(), "O2");
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    let (outcome, consent, _, _) = s.event(&item.drop_id);
    assert_eq!(outcome, "moved");
    assert_eq!(consent.as_deref(), Some(second.as_str()));
}

/// The window answers with the command, which runs the request after the
/// answer on its own thread, and then asks the queue to go on: the pass
/// waits for that request instead of stopping, and the file keeps its name
/// in the queue while it works.
#[test]
fn an_answer_whose_request_runs_on_another_thread_still_lets_the_file_move() {
    let s = setup(|ids| {
        vec![
            ok(clarify()),
            Planned::gated(completed(&recommend(&ids["Travel"]))),
        ]
    });
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::NeedsAnswer, "{item:?}");
    let operation_id = item.operation_id.clone().unwrap();
    s.answer(&operation_id, "O2");

    let service = s.service();
    let queue = thread::scope(|scope| {
        // What the answer_clarification command starts in the background.
        scope.spawn(|| {
            service.run_analysis_with(&operation_id, &mut |root, parent, name| {
                quick_read(parent, name, root.id().dev)
            })
        });
        wait_until(|| s.server.count() == 2);
        // What continue_drops starts in the background.
        let pass = scope.spawn(|| {
            service
                .run_drops_with(&mut |root, parent, name| quick_read(parent, name, root.id().dev))
        });
        wait_until(|| service.drop_queue().items[0].state == DropState::Working);
        let working = service.drop_queue().items[0].clone();
        assert_eq!(working.drop_id, item.drop_id);
        assert_eq!(working.file_name, "trip.md");
        s.server.release();
        pass.join().unwrap();
        service.drop_queue()
    });
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert!(s.f.path("Travel/trip.md").exists());
    assert_eq!(s.server.count(), 2);
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "waited too long");
        thread::sleep(Duration::from_millis(10));
    }
}

/// continue_drops comes in just as a pass ends (it last looked before the
/// answer): it is turned away, and the running pass looks once more, so the
/// answered file is not left alone (the UI session's check of #172). The
/// moment cannot be hit reliably with threads, so the test plays the
/// running pass by hand.
#[test]
fn a_call_turned_away_by_a_running_pass_makes_it_look_once_more() {
    let s = setup(|ids| vec![ok(clarify()), ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::NeedsAnswer, "{item:?}");

    // A pass is running and has just found nothing to do...
    s.service().drops().running = true;
    // ...when the answer comes in, and continue_drops is turned away.
    s.answer(item.operation_id.as_ref().unwrap(), "O2");
    let turned_away = s.run();
    assert_eq!(turned_away.items[0].state, DropState::NeedsAnswer);
    assert!(turned_away.running);

    // The running pass does not end: it looks once more and moves the file.
    assert!(!s.service().end_round());
    let mut next = s.service().next_item().expect("the answered file goes on");
    let read =
        &mut |root: &Folder, parent: &Folder, name: &str| quick_read(parent, name, root.id().dev);
    let go_on = s.service().after_answer(&mut next, read);
    s.service().put_back(next, go_on);
    assert!(s.service().end_round());
    let queue = s.service().drop_queue();
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert!(!queue.running);
}

#[test]
fn each_question_pauses_the_queue_and_still_unsure_after_three_goes_to_a_manual_choice() {
    // organize-spec §15: up to three questions, one at a time; each one waits for the
    // user. A question after the third answer means a manual choice.
    let s = setup(|_| (0..4).map(|_| ok(clarify())).collect());
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let mut queue = s.run();
    for _ in 0..3 {
        let item = queue.items[0].clone();
        assert_eq!(item.state, DropState::NeedsAnswer, "{item:?}");
        s.answer(item.operation_id.as_ref().unwrap(), "O1");
        queue = s.run();
    }
    assert_eq!(queue.items[0].state, DropState::HandedOver, "{queue:?}");
    assert!(s.f.path("trip.md").exists());
    // No fourth question: four requests in all.
    assert_eq!(s.server.count(), 4);
}

#[test]
fn without_organizes_sending_agreement_nothing_is_sent_and_nothing_moves() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.service().set_analysis_consent(false).unwrap();
    s.turn_on(&["Travel"]);
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::HandedOver, "{queue:?}");
    assert_eq!(s.server.count(), 0);
    assert!(s.f.path("trip.md").exists());
}

#[test]
fn a_name_already_taken_in_the_target_is_never_overwritten() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    fs::write(s.f.path("Travel/trip.md"), "someone else's file\n").unwrap();
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    assert_ne!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert_eq!(
        fs::read_to_string(s.f.path("Travel/trip.md")).unwrap(),
        "someone else's file\n"
    );
    assert!(s.f.path("trip.md").exists());
}

#[test]
fn a_move_not_recorded_as_finished_keeps_its_action() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    // No result can be saved once the file has moved: neither the move nor
    // the need for a decision, so `apply_plan` returns an error.
    s.f.db()
        .execute_batch(
            "CREATE TRIGGER test_no_result BEFORE UPDATE OF status ON actions
             WHEN NEW.status IN ('committed', 'needs_user_decision')
             BEGIN SELECT RAISE(ABORT, 'test: no result'); END;",
        )
        .unwrap();
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    let item = &queue.items[0];
    assert_eq!(item.state, DropState::Stopped, "{item:?}");
    // The window can still find the action and its record check.
    let action_id = item.action_id.clone().expect("the action is kept");
    assert_eq!(
        s.event(&item.drop_id).3.as_deref(),
        Some(action_id.as_str())
    );
    assert_eq!(
        s.service().get_action(&action_id).unwrap().status,
        ActionStatus::Running
    );
    assert!(s.f.path("Travel/trip.md").exists());
}

/// #182 (R1-A168-01): the row names the agreement and the plan before
/// anything moves. When that cannot be saved, nothing moves.
#[test]
fn a_plan_that_cannot_be_recorded_first_is_never_run() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.f.db()
        .execute_batch(
            "CREATE TRIGGER test_no_binding BEFORE UPDATE ON organize_drop_events
             WHEN NEW.outcome = 'received' AND NEW.plan_id IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'test: no binding'); END;",
        )
        .unwrap();
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::Stopped, "{queue:?}");
    assert!(s.f.path("trip.md").exists());
    assert!(!s.f.path("Travel/trip.md").exists());
    assert_eq!(s.f.count("actions"), 0);
}

fn fail_moved_rows(s: &Setup) {
    s.f.db()
        .execute_batch(
            "CREATE TRIGGER test_no_moved_row BEFORE UPDATE ON organize_drop_events
             WHEN NEW.outcome = 'moved'
             BEGIN SELECT RAISE(ABORT, 'test: no moved row'); END;",
        )
        .unwrap();
}

fn allow_rows(s: &Setup, trigger: &str) {
    s.f.db()
        .execute_batch(&format!("DROP TRIGGER {trigger};"))
        .unwrap();
}

/// #182: the move happened but its final row could not be saved. The row
/// already names the agreement and the plan, and the next look saves it.
#[test]
fn a_move_whose_row_cannot_be_saved_is_recorded_at_the_next_look() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    let on = s.turn_on(&["Travel"]);
    fail_moved_rows(&s);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::Moved, "{item:?}");
    assert!(item.reason.as_deref().unwrap().contains("Not recorded"));
    let (outcome, consent, plan, action) = s.event(&item.drop_id);
    assert_eq!(outcome, "received");
    assert_eq!(consent, on.consent_id);
    assert_eq!(plan, item.plan_id);
    assert_eq!(action, None);

    allow_rows(&s, "test_no_moved_row");
    let again = s.service().drop_queue().items[0].clone();
    assert_eq!(again.reason.as_deref(), Some("Moved."));
    let (outcome, consent, _, action) = s.event(&item.drop_id);
    assert_eq!(outcome, "moved");
    assert_eq!(consent, on.consent_id);
    assert_eq!(action, item.action_id);
}

/// #182: the same, then Traeva restarts: the row is found again through
/// its plan and becomes `moved` with the action that ran.
#[test]
fn a_move_whose_row_cannot_be_saved_is_recorded_after_a_restart() {
    let mut s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    let on = s.turn_on(&["Travel"]);
    fail_moved_rows(&s);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::Moved, "{item:?}");
    allow_rows(&s, "test_no_moved_row");
    s.f.restart();
    let (outcome, consent, plan, action) = s.event(&item.drop_id);
    assert_eq!(outcome, "moved");
    assert_eq!(consent, on.consent_id);
    assert_eq!(plan, item.plan_id);
    assert_eq!(action, item.action_id);
}

/// #182: a restart while the move is not settled (its action still
/// running) leaves the row `interrupted` with that action, never `moved`;
/// the action's record check decides.
#[test]
fn an_unsettled_move_is_interrupted_with_its_action_after_a_restart() {
    let mut s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    s.f.db()
        .execute_batch(
            "CREATE TRIGGER test_no_result BEFORE UPDATE OF status ON actions
             WHEN NEW.status IN ('committed', 'needs_user_decision')
             BEGIN SELECT RAISE(ABORT, 'test: no result'); END;
             CREATE TRIGGER test_no_final_row BEFORE UPDATE ON organize_drop_events
             WHEN NEW.outcome <> 'received'
             BEGIN SELECT RAISE(ABORT, 'test: no final row'); END;",
        )
        .unwrap();
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    let action_id = item.action_id.clone().expect("the move started");
    allow_rows(&s, "test_no_result");
    allow_rows(&s, "test_no_final_row");
    s.f.restart();
    let (outcome, _, plan, action) = s.event(&item.drop_id);
    assert_eq!(outcome, "interrupted");
    assert_eq!(plan, item.plan_id);
    assert_eq!(action.as_deref(), Some(action_id.as_str()));
}

/// An earlier unsettled action that holds the file or a location is named
/// in the error, but it is not this file's move (the UI session's check of
/// #172). It only happens in a race between building and running the plan,
/// so the rule is checked on the errors themselves.
#[test]
fn only_the_plans_own_action_is_kept_from_an_apply_error() {
    let blocked = crate::execute::unresolved("file", Some("plan"), "other-action");
    assert_eq!(own_action(&blocked), None);
    let unrecorded = AppError {
        action_id: Some("own-action".to_string()),
        ..AppError::storage(ErrorCode::DatabaseCorrupt, "Not recorded.")
    };
    assert_eq!(own_action(&unrecorded).as_deref(), Some("own-action"));
}

// ---------------------------------------------------------------------------
// Only dropped files; their folder authorized first (A02, A03)
// ---------------------------------------------------------------------------

#[test]
fn a_dropped_folder_is_refused() {
    let s = setup(|_| vec![never_sent()]);
    s.turn_on(&["Travel"]);
    let queue = s.drop_files(&["Work"]);
    assert_eq!(queue.items[0].state, DropState::Stopped);
    assert_eq!(s.event(&queue.items[0].drop_id).0, "stopped");
}

#[test]
fn a_file_outside_the_authorized_folders_waits_for_its_folder_to_be_authorized() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.turn_on(&["Travel"]);
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    let file = outside.path().join("notes.md");
    fs::write(&file, TEXT).unwrap();
    let before = snapshot::snapshot(outside.path());

    s.service()
        .receive_drops(std::slice::from_ref(&file))
        .unwrap();
    let queue = s.run();
    let item = queue.items[0].clone();
    assert_eq!(item.state, DropState::NeedsSourceAuthorization, "{item:?}");
    assert!(item.folder_to_authorize.is_some());
    assert!(item.needs_user, "the pet shows it waits (#208)");
    assert_eq!(snapshot::snapshot(outside.path()), before);
    assert_eq!(s.server.count(), 0);

    // Agreeing authorizes the folder for good and the file goes on.
    let roots_before = s.service().list_roots().unwrap().len();
    s.service().authorize_drop_source(&item.drop_id).unwrap();
    assert_eq!(s.service().list_roots().unwrap().len(), roots_before + 1);
    let queue = s.run();
    assert_eq!(queue.items[0].state, DropState::Moved, "{queue:?}");
    assert!(s.f.path("Travel/notes.md").exists());
}

#[test]
fn cancelling_the_authorization_ends_the_drop_and_writes_nothing() {
    let s = setup(|_| vec![never_sent()]);
    s.turn_on(&["Travel"]);
    let outside = tempfile::Builder::new()
        .prefix("traeva-outside-")
        .tempdir()
        .unwrap();
    let file = outside.path().join("notes.md");
    fs::write(&file, TEXT).unwrap();
    let before = snapshot::snapshot(outside.path());
    s.service()
        .receive_drops(&[file.clone(), s.f.path("trip.md")])
        .unwrap();
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::NeedsSourceAuthorization);
    let queue = s.service().cancel_drop(&item.drop_id).unwrap();
    assert_eq!(queue.items[0].state, DropState::Cancelled);
    assert_eq!(queue.items[1].state, DropState::NotProcessed);
    assert_eq!(snapshot::snapshot(outside.path()), before);
    assert_eq!(s.service().list_roots().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// The mode off; a restart (A12)
// ---------------------------------------------------------------------------

#[test]
fn with_the_mode_off_a_dropped_file_goes_to_the_normal_flow() {
    let s = setup(|_| vec![never_sent()]);
    s.drop_files(&["trip.md"]);
    let queue = s.run();
    let item = queue.items[0].clone();
    assert_eq!(item.state, DropState::HandedOver, "{item:?}");
    assert!(!item.automatic);
    assert_eq!(s.event(&item.drop_id).1, None);

    // The window starts the normal operation for it.
    let (operation, path) = s
        .service()
        .start_drop_operation(&item.drop_id, &s.f.root_id)
        .unwrap();
    assert_eq!(operation.stage, OperationStage::Observing);
    assert_eq!(path, s.f.path("trip.md"));
    assert_eq!(
        s.service().drop_queue().items[0].operation_id.as_deref(),
        Some(operation.operation_id.as_str())
    );
    assert_eq!(s.server.count(), 0);
}

#[test]
fn cancelling_a_handed_over_file_cancels_its_operation_and_the_rest() {
    let s = setup(|_| vec![never_sent()]);
    fs::write(s.f.path("trip-2.md"), TEXT).unwrap();
    s.drop_files(&["trip.md", "trip-2.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::HandedOver, "{item:?}");
    let (operation, _) = s
        .service()
        .start_drop_operation(&item.drop_id, &s.f.root_id)
        .unwrap();

    let queue = s.service().cancel_drop(&item.drop_id).unwrap();
    assert_eq!(queue.items[0].state, DropState::Cancelled);
    assert_eq!(queue.items[1].state, DropState::NotProcessed);
    assert_eq!(s.event(&item.drop_id).0, "cancelled");
    assert_eq!(
        s.service()
            .get_operation(&operation.operation_id)
            .unwrap()
            .stage,
        OperationStage::Cancelled
    );
}

#[test]
fn cancelling_only_skips_the_rest_of_its_own_drop() {
    let s = setup(|_| vec![never_sent()]);
    fs::write(s.f.path("trip-2.md"), TEXT).unwrap();
    s.drop_files(&["trip.md"]);
    s.drop_files(&["trip-2.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::HandedOver, "{item:?}");
    let queue = s.service().cancel_drop(&item.drop_id).unwrap();
    assert_eq!(queue.items[0].state, DropState::Cancelled);
    assert_eq!(queue.items[1].state, DropState::Waiting);
    // The next drop goes on as usual (the mode is off: the normal flow).
    assert_eq!(s.run().items[1].state, DropState::HandedOver);
}

/// With the mode off, the file goes through the normal flow in the window;
/// the plan and action it confirms are recorded for the drop, and the drop
/// can no longer be cancelled.
#[test]
fn the_normal_flow_of_a_handed_over_file_is_recorded() {
    let s = setup(|ids| vec![ok(recommend(&ids["Travel"]))]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    let (operation, path) = s
        .service()
        .start_drop_operation(&item.drop_id, &s.f.root_id)
        .unwrap();
    let id = operation.operation_id;
    let read =
        &mut |root: &Folder, parent: &Folder, name: &str| quick_read(parent, name, root.id().dev);
    s.service().run_operation_with(&id, &path, read);
    s.service().start_analysis(&id).unwrap();
    s.service().run_analysis_with(&id, read);
    let view = s.service().get_operation(&id).unwrap();
    assert_eq!(view.stage, OperationStage::RecommendationReady, "{view:?}");
    let file = view.file.unwrap();
    let suggestion = view.suggestion.unwrap();
    let BuildOutcome::Planned { plan } = s
        .service()
        .build_plan(&PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: s.f.root_id.clone(),
            operation_id: Some(id.clone()),
            target: PlanTarget::Candidate {
                candidate_id: suggestion.recommended_id.unwrap(),
            },
        })
        .unwrap()
    else {
        panic!("not planned");
    };
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);

    let queue = s.service().drop_queue();
    assert_eq!(queue.items[0].state, DropState::HandedOver);
    assert_eq!(
        queue.items[0].plan_id.as_deref(),
        Some(plan.plan_id.as_str())
    );
    assert_eq!(
        queue.items[0].action_id.as_deref(),
        Some(action.action_id.as_str())
    );
    let (outcome, consent, saved_plan, saved_action) = s.event(&item.drop_id);
    assert_eq!(outcome, "handed_over");
    assert_eq!(consent, None);
    assert_eq!(saved_plan.as_deref(), Some(plan.plan_id.as_str()));
    assert_eq!(saved_action.as_deref(), Some(action.action_id.as_str()));

    assert!(s.service().cancel_drop(&item.drop_id).is_err());
    assert_eq!(
        s.service().drop_queue().items[0].state,
        DropState::HandedOver
    );
}

#[test]
fn a_restart_marks_files_still_in_the_queue_interrupted() {
    let mut s = setup(|_| vec![never_sent()]);
    s.turn_on(&["Travel"]);
    let queued = s.drop_files(&["trip.md"]);
    s.f.restart();
    assert!(s.service().drop_queue().items.is_empty());
    assert_eq!(s.event(&queued.items[0].drop_id).0, "interrupted");
}

// ---------------------------------------------------------------------------
// What the desktop pet reads (#208)
// ---------------------------------------------------------------------------

#[test]
fn the_queue_says_when_a_handed_over_file_waits_and_how_its_confirmed_move_ended() {
    let s = setup(|_| vec![never_sent()]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    assert_eq!(item.state, DropState::HandedOver);
    assert!(item.needs_user, "its normal flow has not started yet");
    assert_eq!(item.action_status, None);

    let (operation, path) = s
        .service()
        .start_drop_operation(&item.drop_id, &s.f.root_id)
        .unwrap();
    assert!(s.service().drop_queue().items[0].needs_user);
    let id = operation.operation_id.clone();
    s.service()
        .run_operation_with(&id, &path, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        });
    s.service().choose_manual_location(&id).unwrap();
    assert!(s.service().drop_queue().items[0].needs_user, "a pick waits");
    let file = s.service().get_operation(&id).unwrap().file.unwrap();
    let catalog = s.service().list_folders(&s.f.root_id).unwrap();
    let travel = catalog
        .folders
        .iter()
        .find(|folder| folder.relative_path == "Travel")
        .unwrap()
        .folder_id
        .clone();
    let outcome = s
        .service()
        .build_plan(&PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: s.f.root_id.clone(),
            operation_id: Some(id.clone()),
            target: PlanTarget::Folder {
                catalog_id: catalog.catalog_id,
                folder_id: travel,
                new_folders: Vec::new(),
            },
        })
        .unwrap();
    let BuildOutcome::Planned { plan } = outcome else {
        panic!("{outcome:?}");
    };
    let action = s.f.apply(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);

    let item = s.service().drop_queue().items[0].clone();
    assert_eq!(item.state, DropState::HandedOver);
    assert!(!item.needs_user, "confirmed: nothing waits");
    assert_eq!(item.action_id.as_deref(), Some(action.action_id.as_str()));
    assert_eq!(item.action_status, Some(ActionStatus::Committed));
}

#[test]
fn a_normal_flow_left_without_confirming_stops_waiting_once_its_operation_is_cancelled() {
    let s = setup(|_| vec![never_sent()]);
    s.drop_files(&["trip.md"]);
    let item = s.run().items[0].clone();
    let (operation, _) = s
        .service()
        .start_drop_operation(&item.drop_id, &s.f.root_id)
        .unwrap();
    assert!(s.service().drop_queue().items[0].needs_user);

    s.service()
        .cancel_operation(&operation.operation_id)
        .unwrap();

    let item = s.service().drop_queue().items[0].clone();
    assert_eq!(item.state, DropState::HandedOver);
    assert!(!item.needs_user, "the window ended the flow");
    assert_eq!(item.action_status, None);
}
