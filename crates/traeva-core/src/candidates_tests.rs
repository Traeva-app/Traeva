use std::fs;

use super::*;
use crate::capture::{CheckOutcome, WatchCheck};
use crate::family::MemoryRecordView;
use crate::memory_contracts::{PairCheck, PairRefusal};
use crate::test_fixture::{Fixture, quick_read};
use crate::test_snapshot as tree;

pub(crate) fn start_record(fixture: &Fixture, relative: &str) -> MemoryRecordView {
    let file = fixture.register(relative);
    fixture
        .service()
        .start_memory_for_file_with(&file.file_id, &mut |root, parent, name| {
            quick_read(parent, name, root.id().dev)
        })
        .unwrap()
}

/// How many apps save: write a new file, then rename it over the old name.
pub(crate) fn replace(fixture: &Fixture, relative: &str, text: &str) {
    let swap = format!("{relative}.swap");
    fs::write(fixture.path(&swap), text).unwrap();
    fs::rename(fixture.path(&swap), fixture.path(relative)).unwrap();
}

/// Turns candidate capture on exactly as the notice showed it.
pub(crate) fn enable(fixture: &Fixture, chain_id: &str) -> Result<CandidateCaptureInfo, AppError> {
    let info = fixture.service().candidate_capture(chain_id).unwrap();
    fixture.service().set_candidate_capture_with(
        &CandidateSwitchRequest {
            chain_id: chain_id.to_string(),
            on: true,
            notice_version: Some(info.notice_version),
            occupant_token: info.unread_occupant.map(|o| o.occupant_token),
            place_token: Some(info.place_token.clone()),
        },
        &mut |root, parent, name| quick_read(parent, name, root.id().dev),
    )
}

/// One pass, counting how many times a file was read.
pub(crate) fn check_counting_reads(fixture: &Fixture) -> (Vec<WatchCheck>, usize) {
    let mut reads = 0;
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            reads += 1;
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    (checks, reads)
}

pub(crate) fn only(checks: Vec<WatchCheck>) -> CheckOutcome {
    assert_eq!(checks.len(), 1, "{checks:?}");
    checks.into_iter().next().unwrap().outcome
}

fn candidate_saved(outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::CandidateSaved { candidate_id, .. } => candidate_id.clone(),
        other => panic!("expected a candidate, got {other:?}"),
    }
}

/// (seq, state, reason, after, first, file) of the chain's candidates.
fn candidates(
    fixture: &Fixture,
    chain_id: &str,
) -> Vec<(i64, String, String, Option<String>, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT seq, state, reason, after_snapshot_id, first_snapshot_id, file_id
             FROM memory_candidates WHERE chain_id = ?1 ORDER BY seq",
        )
        .unwrap();
    stmt.query_map([chain_id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn latest_snapshot(fixture: &Fixture, file_id: &str) -> SnapshotRef {
    fixture
        .service()
        .list_snapshots(file_id)
        .unwrap()
        .pop()
        .unwrap()
}

#[test]
fn replacements_with_capture_on_become_one_waiting_list() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    let info = enable(&fixture, &record.chain_id).unwrap();
    assert_eq!(info.candidate_capture, CandidateCapture::On);
    assert_eq!(info.place_holds, PlaceHolds::RecordedFile);
    assert!(!info.saves_occupant_now);
    for n in 1..=3 {
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", &format!("save {n}\n"));
        let (checks, reads) = check_counting_reads(&fixture);
        candidate_saved(&only(checks));
        assert_eq!(reads, 1);
    }
    let rows = candidates(&fixture, &record.chain_id);
    assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(rows.iter().all(|r| r.1 == "pending" && r.2 == "replaced"));
    let original_last = latest_snapshot(&fixture, &record.origin_file_id).snapshot_id;
    assert_eq!(rows[0].3.as_deref(), Some(original_last.as_str()));
    assert_eq!(
        rows[1].3.as_deref(),
        Some(rows[0].4.as_str()),
        "after the previous object"
    );
    assert_eq!(rows[2].3.as_deref(), Some(rows[1].4.as_str()));
    assert_eq!(fixture.count("files"), 4, "each object has its own record");
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.pending.objects, 3);
    assert_eq!(view.pending.snapshots, 3);
    // Saving a candidate confirms nothing: it is never a usable version.
    assert!(
        fixture
            .service()
            .all_usable_pairs()
            .unwrap()
            .iter()
            .all(|pair| pair.after.file_id == record.origin_file_id)
    );
    assert_eq!(
        fixture
            .service()
            .check_usable_pair(&original_last, &rows[0].4)
            .unwrap(),
        PairCheck::NotUsable {
            reason: PairRefusal::CandidateVersion
        }
    );
    // Nothing new: nothing is read or saved again.
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Unchanged));
    assert_eq!(reads, 0);
    assert_eq!(candidates(&fixture, &record.chain_id).len(), 3);
}

#[test]
fn a_waiting_candidate_edited_in_place_adds_counted_versions() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "new object\n");
    let first = candidate_saved(&only(fixture.check_memory()));
    fs::write(fixture.path("todo.txt"), "edited in place\n").unwrap();
    let second = candidate_saved(&only(fixture.check_memory()));
    assert_eq!(first, second, "the same candidate, a new version");
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!((view.pending.objects, view.pending.snapshots), (1, 2));
}

#[test]
fn enabling_after_a_replacement_saves_the_object_shown_first() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "saved before the permission\n");
    fixture.check_memory();
    let info = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    assert_eq!(info.place_holds, PlaceHolds::UnreadObject);
    assert!(
        info.saves_occupant_now,
        "the notice must say it is saved now"
    );
    let token = info
        .unread_occupant
        .as_ref()
        .unwrap()
        .occupant_token
        .clone();
    let mut reads = 0;
    let on = fixture
        .service()
        .set_candidate_capture_with(
            &CandidateSwitchRequest {
                chain_id: record.chain_id.clone(),
                on: true,
                notice_version: Some(CANDIDATE_NOTICE_VERSION),
                occupant_token: Some(token),
                place_token: Some(info.place_token.clone()),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert_eq!(reads, 1);
    assert_eq!(on.candidate_capture, CandidateCapture::On);
    assert_eq!(on.place_holds, PlaceHolds::PendingCandidate);
    let rows = candidates(&fixture, &record.chain_id);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_str()),
        ("pending", "present_when_enabled")
    );
    let included: Option<String> = fixture
        .db()
        .query_row(
            "SELECT included_file_id FROM memory_candidate_switches",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(included.as_deref(), Some(rows[0].5.as_str()));
    // The unread stretch before it ends with this capture.
    let gaps = fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].kind, GapKind::ReplacedUncaptured);
    assert_eq!(gaps[0].end_reason, Some(GapEnd::Captured));
}

#[test]
fn enabling_is_refused_when_the_object_changed_since_it_was_shown() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "first unread object\n");
    fixture.check_memory();
    let info = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    replace(&fixture, "todo.txt", "another one since\n");
    let mut reads = 0;
    let refused = fixture
        .service()
        .set_candidate_capture_with(
            &CandidateSwitchRequest {
                chain_id: record.chain_id.clone(),
                on: true,
                notice_version: Some(info.notice_version),
                occupant_token: info.unread_occupant.map(|o| o.occupant_token),
                place_token: Some(info.place_token.clone()),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::PageOutdated);
    assert!(refused.retryable);
    assert_eq!(reads, 0);
    assert_eq!(fixture.count("memory_candidate_switches"), 0);
    assert_eq!(fixture.count("memory_candidates"), 0);
}

#[test]
fn an_old_notice_or_a_stopped_record_cannot_turn_it_on() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let old = fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: true,
            notice_version: Some(CANDIDATE_NOTICE_VERSION + 1),
            occupant_token: None,
            place_token: None,
        });
    assert_eq!(old.unwrap_err().code, ErrorCode::PageOutdated);
    let missing = fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: true,
            notice_version: None,
            occupant_token: None,
            place_token: None,
        });
    assert_eq!(missing.unwrap_err().code, ErrorCode::PageOutdated);
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    assert_eq!(
        enable(&fixture, &record.chain_id).unwrap_err().code,
        ErrorCode::RecordStopped
    );
    assert_eq!(fixture.count("memory_candidate_switches"), 0);
}

#[test]
fn a_read_that_fails_while_enabling_writes_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "unread\n");
    fixture.check_memory();
    let info = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    let mut writes = 0;
    let failed = fixture
        .service()
        .set_candidate_capture_with(
            &CandidateSwitchRequest {
                chain_id: record.chain_id.clone(),
                on: true,
                notice_version: Some(info.notice_version),
                occupant_token: info.unread_occupant.map(|o| o.occupant_token),
                place_token: Some(info.place_token.clone()),
            },
            &mut |root, parent, name| {
                crate::observe::read_stable_with(
                    parent,
                    name,
                    root.id().dev,
                    crate::observe::ReadLimits::SPEC,
                    &mut |_| {
                        writes += 1;
                        fs::write(fixture.path("todo.txt"), format!("still saving {writes}\n"))
                            .unwrap();
                    },
                )
            },
        )
        .unwrap_err();
    // The switch is not on, so nothing tries again: open the notice again.
    assert_eq!(failed.code, ErrorCode::PageOutdated);
    assert!(failed.retryable);
    assert_eq!(
        fixture.count("memory_candidate_switches"),
        0,
        "the switch stays off"
    );
    assert_eq!(fixture.count("memory_candidates"), 0);
}

#[test]
fn twenty_waiting_objects_pause_capture_until_some_are_decided() {
    let mut fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    for n in 1..=20 {
        replace(&fixture, "todo.txt", &format!("save {n}\n"));
        candidate_saved(&only(fixture.check_memory()));
    }
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.candidate_capture, CandidateCapture::Paused);
    assert_eq!(
        view.open_gaps.iter().map(|g| g.kind).collect::<Vec<_>>(),
        vec![GapKind::Paused]
    );
    replace(&fixture, "todo.txt", "save 21\n");
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::CandidatePaused));
    assert_eq!(reads, 0, "nothing is read while paused");
    // A restart does not reset it.
    fixture.restart();
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::CandidatePaused));
    assert_eq!(reads, 0);
    // Deciding one frees room: the pause ends and the object there now is
    // saved at the next pass.
    let page = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    fixture
        .service()
        .decide_pending_candidates(&crate::CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: page.page_token,
            items: vec![crate::CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: crate::CandidateChoice::Independent {},
            }],
            current_object: None,
        })
        .unwrap();
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.candidate_capture, CandidateCapture::On);
    assert!(view.open_gaps.is_empty());
    candidate_saved(&only(fixture.check_memory()));
    let paused = fixture
        .service()
        .store()
        .unwrap()
        .gaps_of_chain(&record.chain_id)
        .unwrap()
        .into_iter()
        .find(|gap| gap.kind == GapKind::Paused)
        .unwrap();
    assert_eq!(paused.end_reason, Some(GapEnd::Released));
}

#[test]
fn twenty_versions_of_one_waiting_object_pause_capture() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "candidate\n");
    candidate_saved(&only(fixture.check_memory()));
    for n in 2..=20 {
        fs::write(fixture.path("todo.txt"), format!("in place {n}\n")).unwrap();
        candidate_saved(&only(fixture.check_memory()));
    }
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!((view.pending.objects, view.pending.snapshots), (1, 20));
    assert_eq!(view.candidate_capture, CandidateCapture::Paused);
    fs::write(fixture.path("todo.txt"), "one more\n").unwrap();
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::CandidatePaused));
    assert_eq!(reads, 0);
}

#[test]
fn turning_it_off_is_final_and_stopping_turns_it_off() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    let off = fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: false,
            notice_version: None,
            occupant_token: None,
            place_token: None,
        })
        .unwrap();
    assert_eq!(off.candidate_capture, CandidateCapture::Off);
    replace(&fixture, "todo.txt", "after it was turned off\n");
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Problem { .. }));
    assert_eq!(reads, 0);
    enable(&fixture, &record.chain_id).unwrap();
    assert_eq!(
        fixture.count("memory_candidate_switches"),
        2,
        "on again is a new period"
    );
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    let reasons: Vec<String> = {
        let db = fixture.db();
        let mut stmt = db
            .prepare("SELECT disabled_reason FROM memory_candidate_switches ORDER BY rowid")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(reasons, vec!["user", "stopped"]);
}

#[test]
fn objects_that_are_not_new_are_never_saved_here() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    // A hard-linked object at the place: not a file Traeva reads.
    fs::write(fixture.path("Taxes/linked.txt"), "linked\n").unwrap();
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fs::hard_link(fixture.path("Taxes/linked.txt"), fixture.path("todo.txt")).unwrap();
    let outcome = only(fixture.check_memory());
    assert!(
        matches!(&outcome, CheckOutcome::Problem { error } if error.code == ErrorCode::UnsupportedFile),
        "{outcome:?}"
    );
    assert_eq!(fixture.count("memory_candidates"), 0);
    // A copy put in place by copy, delete and rename: only a candidate.
    fs::remove_file(fixture.path("todo.txt")).unwrap();
    fs::copy(fixture.path("Notes/2026/plan.md"), fixture.path("todo.tmp")).unwrap();
    fs::rename(fixture.path("todo.tmp"), fixture.path("todo.txt")).unwrap();
    candidate_saved(&only(fixture.check_memory()));
    assert_eq!(candidates(&fixture, &record.chain_id)[0].1, "pending");
}

#[test]
fn only_the_exact_place_is_captured() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    // Save As next to it, and an editor's temporary name: never looked at.
    fs::copy(fixture.path("todo.txt"), fixture.path("todo copy.txt")).unwrap();
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Unchanged));
    assert_eq!(reads, 0);
    fs::rename(fixture.path("todo.txt"), fixture.path("todo.txt.sb-1")).unwrap();
    fs::write(fixture.path("todo.txt"), "written fresh\n").unwrap();
    let mut names = Vec::new();
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            names.push(name.to_string());
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    candidate_saved(&only(checks));
    assert_eq!(names, vec!["todo.txt"]);
    let registered: i64 = fixture
        .db()
        .query_row(
            "SELECT count(*) FROM files WHERE relative_path IN ('todo copy.txt', 'todo.txt.sb-1')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(registered, 0);
}

#[test]
fn a_b_a_by_replacement_keeps_three_objects() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    for text in ["A\n", "B\n", "A\n"] {
        replace(&fixture, "todo.txt", text);
        candidate_saved(&only(fixture.check_memory()));
    }
    let rows = candidates(&fixture, &record.chain_id);
    let files: std::collections::HashSet<&str> = rows.iter().map(|r| r.5.as_str()).collect();
    assert_eq!(
        files.len(),
        3,
        "the same bytes in another object are another object"
    );
}

#[test]
fn a_revoked_folder_stops_candidate_capture() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    fixture.service().revoke_root(&fixture.root_id).unwrap();
    let view = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(view.candidate_capture, CandidateCapture::Off);
    replace(&fixture, "todo.txt", "after the revoke\n");
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(
        matches!(&only(checks), CheckOutcome::Problem { error } if error.code == ErrorCode::RootRevoked)
    );
    assert_eq!(reads, 0);
}

#[test]
fn candidate_capture_writes_nothing_inside_the_root() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "unread\n");
    let before = tree::snapshot(fixture.root.path());
    fixture.check_memory();
    enable(&fixture, &record.chain_id).unwrap();
    fixture.check_memory();
    fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    assert_eq!(tree::snapshot(fixture.root.path()), before);
}

#[test]
fn candidate_json_names_are_fixed() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    replace(&fixture, "todo.txt", "unread\n");
    fixture.check_memory();
    let info = serde_json::to_value(
        fixture
            .service()
            .candidate_capture(&record.chain_id)
            .unwrap(),
    )
    .unwrap();
    let keys: Vec<&str> = info
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "candidate_capture",
            "chain_id",
            "notice_version",
            "pending",
            "place",
            "place_holds",
            "place_problem",
            "place_token",
            "root_active",
            "saves_occupant_now",
            "state",
            "unread_occupant"
        ]
    );
    assert_eq!(info["place_holds"], "unread_object");
    let occupant: Vec<&str> = info["unread_occupant"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        occupant,
        vec!["byte_length", "modified_at_ms", "occupant_token"]
    );
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "another\n");
    let outcome = serde_json::to_value(only(fixture.check_memory())).unwrap();
    assert_eq!(outcome["kind"], "candidate_saved");
    assert!(outcome["candidate_id"].is_string() && outcome["snapshot"].is_object());
    let request: CandidateSwitchRequest = serde_json::from_value(serde_json::json!({
        "chain_id": record.chain_id, "on": false
    }))
    .unwrap();
    assert!(!request.on);
    assert!(
        serde_json::from_value::<CandidateSwitchRequest>(serde_json::json!({
            "chain_id": record.chain_id, "on": false, "path": "/etc"
        }))
        .is_err()
    );
}

#[test]
fn a_waiting_candidate_changed_with_capture_off_is_not_read() {
    let fixture = Fixture::new().without_auto_link();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "candidate\n");
    candidate_saved(&only(fixture.check_memory()));
    fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: false,
            notice_version: None,
            occupant_token: None,
            place_token: None,
        })
        .unwrap();
    fs::write(
        fixture.path("todo.txt"),
        "edited in place after it was turned off\n",
    )
    .unwrap();
    for _ in 0..2 {
        let (checks, reads) = check_counting_reads(&fixture);
        assert!(
            matches!(&only(checks), CheckOutcome::Problem { error } if error.code == ErrorCode::FileChanged)
        );
        assert_eq!(reads, 0);
    }
    let open: Vec<GapKind> = fixture
        .service()
        .memory_record(&record.chain_id)
        .unwrap()
        .open_gaps
        .iter()
        .map(|gap| gap.kind)
        .collect();
    assert_eq!(open, vec![GapKind::ReplacedUncaptured]);
}

/// A record with 20 waiting candidates, candidate capture then turned off
/// and the place replaced once more (unread).
fn full_record_with_an_unread_object(fixture: &Fixture) -> MemoryRecordView {
    let record = start_record(fixture, "todo.txt");
    enable(fixture, &record.chain_id).unwrap();
    for n in 1..=20 {
        replace(fixture, "todo.txt", &format!("save {n}\n"));
        candidate_saved(&only(fixture.check_memory()));
    }
    fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: false,
            notice_version: None,
            occupant_token: None,
            place_token: None,
        })
        .unwrap();
    replace(fixture, "todo.txt", "unread while full\n");
    fixture.check_memory();
    record
}

#[test]
fn enabling_with_a_full_quota_pauses_and_reads_nothing() {
    let fixture = Fixture::new().without_auto_link();
    let record = full_record_with_an_unread_object(&fixture);
    let info = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    assert_eq!(info.place_holds, PlaceHolds::UnreadObject);
    assert!(
        !info.saves_occupant_now,
        "the quota is full: nothing is saved now"
    );
    let mut reads = 0;
    let on = fixture
        .service()
        .set_candidate_capture_with(
            &CandidateSwitchRequest {
                chain_id: record.chain_id.clone(),
                on: true,
                notice_version: Some(info.notice_version),
                occupant_token: info.unread_occupant.map(|o| o.occupant_token),
                place_token: Some(info.place_token.clone()),
            },
            &mut |root, parent, name| {
                reads += 1;
                quick_read(parent, name, root.id().dev)
            },
        )
        .unwrap();
    assert_eq!(reads, 0);
    assert_eq!(on.candidate_capture, CandidateCapture::Paused);
}

#[test]
fn a_notice_shown_while_full_cannot_save_the_object_after_room_was_made() {
    let fixture = Fixture::new().without_auto_link();
    let record = full_record_with_an_unread_object(&fixture);
    let shown = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    assert!(!shown.saves_occupant_now);
    // Room is made elsewhere: turning it on would now save the object, which
    // the notice the user saw did not say.
    let page = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    fixture
        .service()
        .decide_pending_candidates(&crate::CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: page.page_token,
            items: vec![crate::CandidateDecision {
                candidate_id: page.items[0].candidate_id.clone(),
                choice: crate::CandidateChoice::Independent {},
            }],
            current_object: None,
        })
        .unwrap();
    let refused = fixture
        .service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: true,
            notice_version: Some(shown.notice_version),
            occupant_token: shown.unread_occupant.map(|o| o.occupant_token),
            place_token: Some(shown.place_token.clone()),
        })
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::PageOutdated);
    // With the notice shown now, it is saved.
    enable(&fixture, &record.chain_id).unwrap();
    assert_eq!(
        fixture.count("memory_candidates"),
        21,
        "20 before, one decided, one new"
    );
}

#[test]
fn a_file_recorded_elsewhere_moved_to_the_place_is_not_read() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    let other = start_record(&fixture, "Notes/2026/plan.md");
    fs::rename(fixture.path("Notes/2026/plan.md"), fixture.path("todo.txt")).unwrap();
    let mut names = Vec::new();
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            names.push(name.to_string());
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert!(names.is_empty(), "read {names:?}");
    let mine = checks
        .iter()
        .find(|c| c.watch_id == record.watch.watch_id)
        .unwrap();
    assert!(
        matches!(&mine.outcome, CheckOutcome::Problem { error } if error.code == ErrorCode::FileChanged)
    );
    assert_eq!(fixture.count("memory_candidates"), 0);
    assert_ne!(other.chain_id, record.chain_id);
}

#[test]
fn what_changes_during_a_candidate_read_saves_nothing() {
    // Each of these happens while the object is read; the capture's own
    // transaction sees it and writes nothing.
    type During = fn(&Fixture, &str);
    let cases: [(&str, During); 4] = [
        ("folder revoked", |f, _| {
            f.service().revoke_root(&f.root_id).unwrap();
        }),
        ("chosen again in Organize", |f, _| {
            f.register("todo.txt");
        }),
        ("switch turned off", |f, chain| {
            f.db()
                .execute(
                    "UPDATE memory_candidate_switches SET disabled_at_ms = 1, disabled_reason = 'user'
                     WHERE chain_id = ?1",
                    [chain],
                )
                .unwrap();
        }),
        ("watch stopped", |f, _| {
            f.db()
                .execute(
                    "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                    [],
                )
                .unwrap();
        }),
    ];
    for (name, during) in cases {
        let fixture = Fixture::new().without_auto_link();
        let record = start_record(&fixture, "todo.txt");
        enable(&fixture, &record.chain_id).unwrap();
        replace(&fixture, "todo.txt", "new object\n");
        let before = (
            fixture.count("files"),
            fixture.count("snapshots"),
            fixture.count("memory_candidates"),
        );
        let checks = fixture
            .service()
            .check_memory_with(&mut |root, parent, folder_name| {
                let read = quick_read(parent, folder_name, root.id().dev);
                during(&fixture, &record.chain_id);
                read
            })
            .unwrap();
        let outcome = only(checks);
        assert!(
            !matches!(outcome, CheckOutcome::CandidateSaved { .. }),
            "{name}: {outcome:?}"
        );
        let after = (
            fixture.count("files"),
            fixture.count("snapshots"),
            fixture.count("memory_candidates"),
        );
        let registered_by_organize = usize::from(name == "chosen again in Organize") as i64;
        assert_eq!(
            after,
            (before.0 + registered_by_organize, before.1, before.2),
            "{name}"
        );
    }
}

#[test]
fn a_registered_record_with_its_own_versions_is_never_taken_as_new() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "registered by Organize\n");
    let other = fixture.register("todo.txt");
    fixture.capture(&other.file_id).unwrap();
    let (file, _) = fixture
        .service()
        .store()
        .unwrap()
        .get_file(&other.file_id)
        .unwrap()
        .unwrap();
    let switch = fixture
        .service()
        .store()
        .unwrap()
        .open_switch(&record.chain_id)
        .unwrap()
        .unwrap();
    let seen = SeenFile {
        root_id: file.root_id.clone(),
        relative_path: file.relative_path.clone(),
        basename: file.basename.clone(),
        sha256: file.sha256.clone(),
        byte_length: file.byte_length,
        evidence: file.evidence.clone(),
        now_ms: 1,
    };
    let recorded = fixture
        .service()
        .store()
        .unwrap()
        .record_candidate(
            &record.chain_id,
            &record.watch.watch_id,
            &SwitchMode::Open(switch),
            &Target::New {
                evidence: file.evidence.clone(),
                expected: Some((file.file_id.clone(), file.record_revision)),
            },
            &seen,
            "replaced",
        )
        .unwrap();
    assert!(matches!(recorded, CandidateRecorded::OtherRecord));
    assert_eq!(fixture.count("memory_candidates"), 0);
}

#[test]
fn nothing_is_read_while_an_action_in_the_folder_is_not_settled() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    // A move Organize left unsettled (a crash, say), in the same folder.
    let other = fixture.register("Notes/2026/plan.md");
    let plan = fixture.plan(&other, "Taxes", &[]);
    fixture.insert_action(&plan.plan_id, "needs_user_decision");
    replace(
        &fixture,
        "todo.txt",
        "an object Organize may have put here\n",
    );
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(
        matches!(&only(checks), CheckOutcome::Problem { error } if error.code == ErrorCode::UnresolvedAction)
    );
    assert_eq!(reads, 0);
    assert_eq!(fixture.count("memory_candidates"), 0);
}

#[test]
fn a_record_stopped_while_a_capture_waits_for_the_lock_reads_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    enable(&fixture, &record.chain_id).unwrap();
    replace(&fixture, "todo.txt", "new object\n");
    let reads = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        // Hold the lock a capture needs, so the pass waits after its look.
        let held = fixture.service().lock_execution();
        let pass = scope.spawn(|| {
            fixture
                .service()
                .check_memory_with(&mut |root, parent, name| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    quick_read(parent, name, root.id().dev)
                })
                .unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Stopped meanwhile (as stop_memory_record would, once it has the lock).
        fixture
            .db()
            .execute(
                "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                [],
            )
            .unwrap();
        drop(held);
        let outcome = only(pass.join().unwrap());
        assert!(matches!(outcome, CheckOutcome::Stopped), "{outcome:?}");
    });
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing was read"
    );
    assert_eq!(fixture.count("memory_candidates"), 0);
}

/// C's #37 finding (#65 issuecomment-6102864317): the notice showed one
/// place, Organize moved the file, and the old notice still turned capture
/// on for the new place. The permission is for the place shown.
#[test]
fn a_notice_shown_before_the_file_moved_does_not_turn_capture_on_for_its_new_place() {
    let fixture = Fixture::new();
    let file = fixture.register("todo.txt");
    let record = start_record(&fixture, "todo.txt");
    let shown = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    assert_eq!(shown.place.relative_path, "todo.txt");
    let plan = fixture.plan(&file, "Taxes", &[]);
    fixture.apply(&plan.plan_id).unwrap();
    let old_notice = CandidateSwitchRequest {
        chain_id: record.chain_id.clone(),
        on: true,
        notice_version: Some(shown.notice_version),
        occupant_token: None,
        place_token: Some(shown.place_token.clone()),
    };
    let before = fixture.count("memory_candidate_switches");
    assert_eq!(
        fixture
            .service()
            .set_candidate_capture(&old_notice)
            .unwrap_err()
            .code,
        ErrorCode::PageOutdated
    );
    // A request that names no place is refused the same way.
    let unnamed = CandidateSwitchRequest {
        place_token: None,
        ..old_notice.clone()
    };
    assert_eq!(
        fixture
            .service()
            .set_candidate_capture(&unnamed)
            .unwrap_err()
            .code,
        ErrorCode::PageOutdated
    );
    assert_eq!(fixture.count("memory_candidate_switches"), before);
    // The notice read again names the new place, and turns it on there.
    let again = fixture
        .service()
        .candidate_capture(&record.chain_id)
        .unwrap();
    assert_eq!(again.place.relative_path, "Taxes/todo.txt");
    assert_ne!(again.place_token, shown.place_token);
    let on = enable(&fixture, &record.chain_id).unwrap();
    assert_eq!(on.candidate_capture, CandidateCapture::On);
    assert_eq!(on.place.relative_path, "Taxes/todo.txt");
}
