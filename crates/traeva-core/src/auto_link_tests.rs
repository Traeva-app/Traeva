use std::fs;
use std::os::unix::fs::MetadataExt;

use crate::candidates::tests::{check_counting_reads, enable, only, replace, start_record};
use crate::candidates::{CandidateSwitchRequest, PlaceHolds};
use crate::capture::CheckOutcome;
use crate::contracts::ErrorCode;
use crate::family::{CandidateCapture, HoldReason, RecordState, RelationStatus};
use crate::family_correct::{RelationAction, RelationRequest};
use crate::memory_contracts::{GapKind, PairBasis, PairEvidence, SnapshotRef};
use crate::observe::ReadError;
use crate::test_fixture::{Fixture, quick_read};

fn linked(outcome: CheckOutcome) -> SnapshotRef {
    match outcome {
        CheckOutcome::LinkedAutomatically { snapshot } => snapshot,
        other => panic!("expected the next version, got {other:?}"),
    }
}

fn problem_code(outcome: &CheckOutcome) -> Option<ErrorCode> {
    match outcome {
        CheckOutcome::Problem { error } => Some(error.code),
        _ => None,
    }
}

/// (edge, member, chain, decision) of every automatic link, oldest first.
fn auto_links(fixture: &Fixture) -> Vec<(String, String, String, String)> {
    let db = fixture.db();
    let mut stmt = db
        .prepare(
            "SELECT edge_id, member_id, chain_id, decision_id FROM memory_auto_links
             ORDER BY rowid",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Every Memory row a link writes: none of them changes when it is refused.
fn memory_rows(fixture: &Fixture) -> [i64; 8] {
    [
        "files",
        "snapshots",
        "memory_watches",
        "memory_members",
        "memory_edges",
        "memory_decisions",
        "memory_gaps",
        "memory_auto_links",
    ]
    .map(|table| fixture.count(table))
}

#[test]
fn a_replacement_at_the_place_is_the_next_version_of_the_same_record() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    let first = fixture
        .service()
        .list_snapshots(&record.origin_file_id)
        .unwrap()
        .pop()
        .unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "buy milk\nand eggs\n");
    let (checks, reads) = check_counting_reads(&fixture);
    let old_watch = checks[0].watch_id.clone();
    let snapshot = linked(only(checks));
    assert_eq!(reads, 1, "read once, the usual stable way");
    assert_ne!(snapshot.file_id, record.origin_file_id, "a new file record");
    assert_eq!(snapshot.relative_path, "todo.txt");

    // The same record goes on at the new file.
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.state, RecordState::Active);
    assert_eq!(now.watch.file_id, snapshot.file_id);
    assert_ne!(now.watch.watch_id, old_watch);
    assert_eq!(now.line_versions, 2);
    assert_eq!(now.permission_file_id, record.origin_file_id);
    assert!(now.open_gaps.is_empty(), "{:?}", now.open_gaps);
    assert_eq!(now.pending.objects, 0);
    assert_eq!(
        fixture.count("memory_gaps"),
        0,
        "no gap: Traeva saw the save"
    );
    assert_eq!(fixture.count("memory_candidates"), 0, "nothing waited");

    // The rows of a "continue", confirmed by Traeva, marked as its own.
    let links = auto_links(&fixture);
    assert_eq!(links.len(), 1);
    let (edge_id, member_id, chain_id, decision_id) = links[0].clone();
    assert_eq!(chain_id, record.chain_id);
    let db = fixture.db();
    let edge: (String, Option<String>, String, String, i64, String) = db
        .query_row(
            "SELECT kind, parent_snapshot_id, child_snapshot_id, status, skipped_items,
                    decision_id
             FROM memory_edges WHERE edge_id = ?1",
            [&edge_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        edge,
        (
            "continuation".to_string(),
            Some(first.snapshot_id.clone()),
            snapshot.snapshot_id.clone(),
            "confirmed_system".to_string(),
            0,
            decision_id.clone()
        )
    );
    let member: (String, String, String) = db
        .query_row(
            "SELECT file_id, status, basis FROM memory_members WHERE member_id = ?1",
            [&member_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        member,
        (
            snapshot.file_id.clone(),
            "confirmed_system".to_string(),
            "user_decision".to_string()
        )
    );
    let decision: (String, Option<String>, Option<String>, String) = db
        .query_row(
            "SELECT choice, candidate_id, parent_snapshot_id, page_token
             FROM memory_decisions WHERE decision_id = ?1",
            [&decision_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(decision.0, "continue");
    assert_eq!(decision.1, None);
    assert_eq!(decision.2.as_deref(), Some(first.snapshot_id.as_str()));
    assert_eq!(decision.3.len(), 64);
    let link: (String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT reason, previous_watch_id, decision_id FROM memory_chain_watches
             WHERE watch_id = ?1",
            [&now.watch.watch_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        link,
        ("continued".to_string(), Some(old_watch), Some(decision_id))
    );
    drop(db);

    // Nothing waits for the user.
    let page = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.unread_occupant.is_none());
    assert_eq!(page.place_holds, PlaceHolds::RecordedFile);

    // One history: the pair is a usable continuation with no gap, under the
    // first file's agreement, and the views say Traeva linked it.
    let pairs = fixture
        .service()
        .usable_pairs_of_line(&snapshot.file_id)
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].before.snapshot_id, first.snapshot_id);
    assert_eq!(pairs[0].after.snapshot_id, snapshot.snapshot_id);
    assert_eq!(pairs[0].basis, PairBasis::Continuation);
    assert_eq!(pairs[0].evidence, PairEvidence::System);
    assert!(!pairs[0].gap, "{:?}", pairs[0].gap_kinds);
    assert!(!pairs[0].unplaced);
    assert_eq!(pairs[0].edge_id.as_deref(), Some(edge_id.as_str()));
    assert_eq!(
        pairs[0].permission_file_ids,
        vec![record.origin_file_id.clone()]
    );
    let history = fixture.service().memory_changes(&snapshot.file_id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].edge_id.as_deref(), Some(edge_id.as_str()));
    assert!(history[0].linked_automatically);
    let work = fixture.service().memory_family(&now.family_id).unwrap();
    let relation = work
        .relations
        .iter()
        .find(|r| r.edge_id == edge_id)
        .unwrap();
    assert_eq!(relation.status, RelationStatus::ConfirmedSystem);
    assert!(relation.linked_automatically);

    // The next pass finds the new file as recorded: nothing is read.
    let (checks, reads) = check_counting_reads(&fixture);
    assert!(matches!(only(checks), CheckOutcome::Unchanged));
    assert_eq!(reads, 0);
}

#[test]
fn auto_link_json_names_are_fixed() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved again\n");
    let checks = fixture.check_memory();
    let json = serde_json::to_value(&checks[0].outcome).unwrap();
    assert_eq!(json["kind"], "linked_automatically");
    assert!(json["snapshot"]["snapshot_id"].is_string());
    let file_id = fixture
        .service()
        .memory_record(&record.chain_id)
        .unwrap()
        .watch
        .file_id;
    let history = fixture.service().memory_changes(&file_id).unwrap();
    let pair = serde_json::to_value(&history[0]).unwrap();
    assert!(pair["edge_id"].is_string());
    assert_eq!(pair["linked_automatically"], true);
    let work = fixture.service().memory_family(&record.family_id).unwrap();
    let relation = serde_json::to_value(&work.relations[0]).unwrap();
    assert_eq!(relation["linked_automatically"], true);
    // Two versions of one file rest on no relation.
    fixture.advance(1_000);
    fs::write(fixture.path("todo.txt"), "edited in place\n").unwrap();
    assert!(matches!(
        only(fixture.check_memory()),
        CheckOutcome::Saved { .. }
    ));
    let history = fixture.service().memory_changes(&file_id).unwrap();
    let same_file = serde_json::to_value(&history[1]).unwrap();
    assert_eq!(same_file["edge_id"], serde_json::Value::Null);
    assert_eq!(same_file["linked_automatically"], false);
}

#[test]
fn saves_in_a_row_are_consecutive_versions_and_use_no_candidate_quota() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    // Candidate capture on: linking still comes first.
    enable(&fixture, &record.chain_id).unwrap();
    let saves = 25; // more than the 20 candidates a record may keep
    for n in 1..=saves {
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", &format!("save {n}\n"));
        linked(only(fixture.check_memory()));
    }
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.state, RecordState::Active);
    assert_eq!(now.line_versions, saves + 1);
    assert_eq!(now.pending.objects, 0);
    assert_eq!(now.pending.snapshots, 0);
    assert_eq!(now.candidate_capture, CandidateCapture::On, "never paused");
    assert_eq!(fixture.count("memory_candidates"), 0);
    assert_eq!(fixture.count("memory_gaps"), 0);
    assert_eq!(auto_links(&fixture).len(), saves as usize);
    let pairs = fixture
        .service()
        .usable_pairs_of_line(&now.watch.file_id)
        .unwrap();
    assert_eq!(pairs.len(), saves as usize);
    for (n, pair) in pairs.iter().enumerate() {
        assert_eq!(pair.basis, PairBasis::Continuation);
        assert!(!pair.gap, "{n}: {:?}", pair.gap_kinds);
        if n > 0 {
            assert_eq!(pair.before, pairs[n - 1].after, "consecutive");
        }
    }
    let history = fixture
        .service()
        .memory_changes(&record.origin_file_id)
        .unwrap();
    assert_eq!(
        history.len(),
        saves as usize,
        "any file gives the same line"
    );
    assert!(history.iter().all(|pair| pair.linked_automatically));
}

#[test]
fn a_file_still_being_written_writes_nothing_and_the_next_pass_links_it() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "half written\n");
    let before = memory_rows(&fixture);
    // The read sees the file still changing.
    let checks = fixture
        .service()
        .check_memory_with(&mut |_, _, _| Err(ReadError::StillChanging { attempts: 1 }))
        .unwrap();
    let outcome = only(checks);
    assert_eq!(problem_code(&outcome), Some(ErrorCode::FileChanging));
    assert_eq!(memory_rows(&fixture), before, "nothing written, no gap");
    // Replaced again while it was read: the object read is not the one seen.
    let checks = fixture
        .service()
        .check_memory_with(&mut |root, parent, name| {
            replace(&fixture, "todo.txt", "saved once more\n");
            quick_read(parent, name, root.id().dev)
        })
        .unwrap();
    assert_eq!(problem_code(&only(checks)), Some(ErrorCode::FileChanging));
    assert_eq!(memory_rows(&fixture), before);
    // The next pass links what is there now.
    let snapshot = linked(only(fixture.check_memory()));
    assert_eq!(
        fs::read(fixture.path("todo.txt")).unwrap(),
        b"saved once more\n"
    );
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.watch.file_id, snapshot.file_id);
    assert_eq!(fixture.count("memory_gaps"), 0);
}

#[test]
fn a_replacement_traeva_cannot_read_is_reported_and_waits_for_the_user() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    // A hard link: the new file has a second name.
    fs::write(fixture.path("todo.swap"), "linked twice\n").unwrap();
    fs::hard_link(fixture.path("todo.swap"), fixture.path("other-name.txt")).unwrap();
    fs::rename(fixture.path("todo.swap"), fixture.path("todo.txt")).unwrap();
    assert_eq!(fs::metadata(fixture.path("todo.txt")).unwrap().nlink(), 2);
    let outcome = only(fixture.check_memory());
    assert_eq!(problem_code(&outcome), Some(ErrorCode::FileChanged));
    assert!(auto_links(&fixture).is_empty());
    // As before: the replacement is a gap, and the page asks the user.
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.watch.file_id, record.origin_file_id);
    assert_eq!(
        now.open_gaps.iter().map(|g| g.kind).collect::<Vec<_>>(),
        vec![GapKind::ReplacedUncaptured]
    );
    let page = fixture
        .service()
        .pending_candidates(&record.chain_id)
        .unwrap();
    assert_eq!(page.place_holds, PlaceHolds::UnreadObject);
    assert!(page.unread_occupant.is_some());

    // Too large: the same.
    fixture.advance(1_000);
    let big = fs::File::create(fixture.path("todo.swap")).unwrap();
    big.set_len(crate::observe::MAX_FILE_BYTES + 1).unwrap();
    drop(big);
    fs::rename(fixture.path("todo.swap"), fixture.path("todo.txt")).unwrap();
    let outcome = only(fixture.check_memory());
    assert_eq!(problem_code(&outcome), Some(ErrorCode::FileChanged));
    assert!(auto_links(&fixture).is_empty());

    // A file the app saves normally afterwards is linked; the pair keeps
    // the gap: Traeva did not record what stood there in between.
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "a normal save\n");
    let snapshot = linked(only(fixture.check_memory()));
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert!(now.open_gaps.is_empty());
    let pairs = fixture
        .service()
        .usable_pairs_of_line(&snapshot.file_id)
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].gap_kinds, vec![GapKind::ReplacedUncaptured]);
}

#[test]
fn an_earlier_waiting_candidate_keeps_later_saves_waiting_too() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    // Replaced before a pass, then candidate capture turned on: the object
    // already there is saved as a candidate at once (D2).
    replace(&fixture, "todo.txt", "first save\n");
    enable(&fixture, &record.chain_id).unwrap();
    assert_eq!(fixture.count("memory_candidates"), 1);
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "second save\n");
    // It must not be linked across the object that waits for the user, and
    // is read once: by candidate capture, not by the link.
    let (checks, reads) = check_counting_reads(&fixture);
    let outcome = only(checks);
    assert!(
        matches!(outcome, CheckOutcome::CandidateSaved { .. }),
        "{outcome:?}"
    );
    assert_eq!(reads, 1);
    assert!(auto_links(&fixture).is_empty());
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.watch.file_id, record.origin_file_id);
    assert_eq!(now.pending.objects, 2);
    // With capture off, the next save is only reported, at every pass, and
    // never read.
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
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "third save\n");
    for _ in 0..2 {
        let (checks, reads) = check_counting_reads(&fixture);
        assert_eq!(problem_code(&only(checks)), Some(ErrorCode::FileChanged));
        assert_eq!(reads, 0);
    }
    assert!(auto_links(&fixture).is_empty());
}

#[test]
fn what_changes_during_the_read_links_nothing() {
    type During = fn(&Fixture);
    let cases: [(&str, During); 2] = [
        ("chosen again in Organize", |f| {
            f.register("todo.txt");
        }),
        ("watch stopped", |f| {
            f.db()
                .execute(
                    "UPDATE memory_watches SET status = 'stopped', stopped_at_ms = 1",
                    [],
                )
                .unwrap();
        }),
    ];
    for (name, during) in cases {
        let fixture = Fixture::new();
        let record = start_record(&fixture, "todo.txt");
        fixture.advance(1_000);
        replace(&fixture, "todo.txt", "new object\n");
        let checks = fixture
            .service()
            .check_memory_with(&mut |root, parent, file_name| {
                let read = quick_read(parent, file_name, root.id().dev);
                during(&fixture);
                read
            })
            .unwrap();
        let outcome = only(checks);
        assert!(
            !matches!(outcome, CheckOutcome::LinkedAutomatically { .. }),
            "{name}: {outcome:?}"
        );
        assert!(auto_links(&fixture).is_empty(), "{name}");
        assert_eq!(fixture.count("memory_decisions"), 0, "{name}");
        if name == "chosen again in Organize" {
            // The next pass links it, with the record Organize made.
            let organized = fixture.register("todo.txt").file_id;
            let snapshot = linked(only(fixture.check_memory()));
            assert_eq!(snapshot.file_id, organized);
            let now = fixture.service().memory_record(&record.chain_id).unwrap();
            assert_eq!(now.watch.file_id, organized);
        }
    }
}

#[test]
fn undoing_an_automatic_link_revokes_it_and_pauses_the_record() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved by an app\n");
    let snapshot = linked(only(fixture.check_memory()));
    let (edge_id, ..) = auto_links(&fixture)[0].clone();

    let shown = fixture.service().relation_preview(&edge_id).unwrap();
    assert_eq!(shown.relation.status, RelationStatus::ConfirmedSystem);
    assert!(shown.relation.linked_automatically);
    assert!(shown.can_revoke);
    assert_eq!(shown.leaves_unplaced.snapshot_id, snapshot.snapshot_id);
    assert_eq!(shown.records_held.len(), 1);
    let outcome = fixture
        .service()
        .correct_relation(&RelationRequest {
            edge_id: edge_id.clone(),
            action: RelationAction::Revoke,
            page_token: shown.page_token.clone(),
        })
        .unwrap();
    assert_eq!(outcome.held_records, vec![record.chain_id.clone()]);
    let status: String = fixture
        .db()
        .query_row(
            "SELECT status FROM memory_edges WHERE edge_id = ?1",
            [&edge_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "superseded");
    // The new file stays in the work, its place unknown; the record is
    // paused until the user records it again or stops it.
    let now = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(now.state, RecordState::Stopped);
    assert_eq!(now.hold.as_ref().unwrap().why, HoldReason::RelationRevoked);
    assert_eq!(
        now.hold.as_ref().unwrap().edge_id.as_deref(),
        Some(edge_id.as_str())
    );
    let history = fixture.service().memory_changes(&snapshot.file_id).unwrap();
    assert!(history.is_empty(), "the pair no longer counts: {history:?}");
    let work = fixture.service().memory_family(&now.family_id).unwrap();
    let relation = work
        .relations
        .iter()
        .find(|r| r.edge_id == edge_id)
        .unwrap();
    assert_eq!(relation.status, RelationStatus::Superseded);
    assert!(
        relation.linked_automatically,
        "the history keeps who made it"
    );
    // Twice is refused: the page was used.
    assert!(
        fixture
            .service()
            .correct_relation(&RelationRequest {
                edge_id,
                action: RelationAction::Revoke,
                page_token: shown.page_token,
            })
            .is_err()
    );

    // Recording again goes on at the new file, and the next save is linked
    // from there.
    fixture
        .service()
        .start_memory_for_file(&snapshot.file_id)
        .unwrap();
    let again = fixture.service().memory_record(&record.chain_id).unwrap();
    assert_eq!(again.state, RecordState::Active);
    assert_eq!(again.permission_file_id, snapshot.file_id);
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved again\n");
    let next = linked(only(fixture.check_memory()));
    let pairs = fixture
        .service()
        .usable_pairs_of_line(&next.file_id)
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].before.snapshot_id, snapshot.snapshot_id);
    assert_eq!(pairs[0].permission_file_ids, vec![snapshot.file_id.clone()]);
}

#[test]
fn a_link_the_user_confirmed_again_is_no_longer_shown_as_automatic() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved by an app\n");
    let snapshot = linked(only(fixture.check_memory()));
    let (edge_id, ..) = auto_links(&fixture)[0].clone();
    // Moving members (M2b-3b) disputes relations; here the automatic link
    // is disputed with its recorded reason, as 0008 asks.
    let db = fixture.db();
    db.execute(
        "INSERT INTO memory_work_decisions (work_decision_id, kind, family_id, edge_id,
             page_token, decided_at_ms)
         VALUES ('11111111-1111-4111-8111-111111111111', 'revoke', ?1, ?2, ?3, 1)",
        rusqlite::params![record.family_id, edge_id, "0".repeat(64)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memory_work_changes (change_id, work_decision_id, edge_id, from_status,
             to_status, changed_at_ms)
         VALUES ('c', '11111111-1111-4111-8111-111111111111', ?1, 'confirmed_system',
                 'disputed', 1)",
        [&edge_id],
    )
    .unwrap();
    db.execute(
        "UPDATE memory_edges SET status = 'disputed' WHERE edge_id = ?1",
        [&edge_id],
    )
    .unwrap();
    drop(db);
    let shown = fixture.service().relation_preview(&edge_id).unwrap();
    assert!(shown.can_confirm_again);
    fixture
        .service()
        .correct_relation(&RelationRequest {
            edge_id: edge_id.clone(),
            action: RelationAction::ConfirmAgain,
            page_token: shown.page_token,
        })
        .unwrap();
    // The pair is the user's now: no "Linked automatically", no undo of it.
    let history = fixture.service().memory_changes(&snapshot.file_id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].edge_id.as_deref(), Some(edge_id.as_str()));
    assert!(!history[0].linked_automatically);
    // The work's history still says who made the relation first.
    let work = fixture.service().memory_family(&record.family_id).unwrap();
    let relation = work
        .relations
        .iter()
        .find(|r| r.edge_id == edge_id)
        .unwrap();
    assert_eq!(relation.status, RelationStatus::ConfirmedUser);
    assert!(relation.linked_automatically);
    assert_eq!(auto_links(&fixture).len(), 1);
}

#[test]
fn a_stopped_record_links_nothing() {
    let fixture = Fixture::new();
    let record = start_record(&fixture, "todo.txt");
    fixture
        .service()
        .stop_memory_record(&record.chain_id)
        .unwrap();
    fixture.advance(1_000);
    replace(&fixture, "todo.txt", "saved while off\n");
    assert!(fixture.check_memory().is_empty());
    assert!(auto_links(&fixture).is_empty());
    assert_eq!(fixture.count("snapshots"), 1);
}
