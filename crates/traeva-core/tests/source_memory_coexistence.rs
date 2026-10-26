//! Source / Memory / Organize combinations through public production APIs.
//! Browser events here are synthetic; this does not replace the real Chrome run.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use traeva_core::{
    ActionStatus, BuildOutcome, CANDIDATE_NOTICE_VERSION, CandidateChoice, CandidateDecision,
    CandidateDecisionRequest, CandidateSwitchRequest, CheckOutcome, ErrorCode, FileView,
    OrganizeService, PlanRequest, PlanTarget, RelocationRequest, SourceDownloadEvent,
    SourceDownloadState, SourceRecordView, SourceRelation, SourceStatus, UndoPreview,
};

struct Setup {
    app: tempfile::TempDir,
    root: tempfile::TempDir,
    service: Option<OrganizeService>,
    root_id: String,
}

impl Setup {
    fn new() -> Self {
        let app = tempfile::Builder::new()
            .prefix("traeva-source-coexist-app-")
            .tempdir()
            .unwrap();
        let root = tempfile::Builder::new()
            .prefix("traeva-source-coexist-root-")
            .tempdir()
            .unwrap();
        fs::create_dir(root.path().join("Archive")).unwrap();
        fs::write(
            root.path().join("download.txt"),
            b"Synthetic download version one.\n",
        )
        .unwrap();
        let service = OrganizeService::start(app.path().join("Traeva"));
        let root_id = service.authorize_root(root.path()).unwrap().root_id;
        service.set_source_enabled(true).unwrap();
        Self {
            app,
            root,
            service: Some(service),
            root_id,
        }
    }

    fn service(&self) -> &OrganizeService {
        self.service.as_ref().unwrap()
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().canonicalize().unwrap().join(relative)
    }

    fn register(&self, relative: &str) -> FileView {
        self.service()
            .register_file(&self.root_id, &self.path(relative))
            .unwrap()
            .file
    }

    fn source(&self, id: &str) -> SourceRecordView {
        self.service()
            .source_state()
            .unwrap()
            .records
            .into_iter()
            .find(|record| record.source_id == id)
            .unwrap()
    }

    fn ingest(&self, relative: &str, download_id: &str) -> SourceRecordView {
        let outcome = self
            .service()
            .ingest_source_event(SourceDownloadEvent {
                profile_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
                download_id: download_id.to_string(),
                started_at_ms: Some(1_700_000_000_000),
                completed_at_ms: Some(1_700_000_000_100),
                state: SourceDownloadState::Complete,
                filename: self.path(relative).display().to_string(),
                initial_url: Some("https://example.test/start.txt".to_string()),
                final_url: Some("https://example.test/download.txt".to_string()),
                referrer: None,
                file_size: fs::metadata(self.path(relative)).ok().map(|m| m.len()),
                incognito: false,
                explicit_history: true,
            })
            .unwrap();
        assert_eq!(outcome.outcome, "saved");
        self.source(&outcome.source_id.unwrap())
    }

    fn confirmed(&self) -> SourceRecordView {
        let source = self.ingest("download.txt", "1");
        assert_eq!(source.status, SourceStatus::Likely);
        assert_eq!(source.candidates.len(), 1);
        self.service()
            .confirm_source(
                &source.source_id,
                &source.candidates[0].file_id,
                SourceRelation::Download,
                "Synthetic explicit user confirmation.",
            )
            .unwrap()
    }

    fn restart(&mut self) {
        self.service = None;
        self.service = Some(OrganizeService::start(self.app.path().join("Traeva")));
    }
}

fn assert_source(
    source: &SourceRecordView,
    original: &SourceRecordView,
    file_id: &str,
    location_confirmed: bool,
) {
    assert_eq!(source.status, SourceStatus::UserConfirmed);
    assert_eq!(source.source_id, original.source_id);
    assert_eq!(source.imported_at_ms, original.imported_at_ms);
    assert_eq!(source.started_at_ms, original.started_at_ms);
    assert_eq!(source.completed_at_ms, original.completed_at_ms);
    assert_eq!(source.initial_url, original.initial_url);
    assert_eq!(source.final_url, original.final_url);
    assert_eq!(source.title, None);
    let association = source.association.as_ref().unwrap();
    assert_eq!(association.file_id, file_id);
    assert_eq!(association.basis, "user_confirmed");
    assert_eq!(association.relation, SourceRelation::Download);
    assert_eq!(association.current_location_confirmed, location_confirmed);
    assert_eq!(
        association.confirmed_at_ms,
        original.association.as_ref().unwrap().confirmed_at_ms
    );
}

fn only_check(s: &Setup) -> CheckOutcome {
    let checks = s.service().check_memory().unwrap();
    assert_eq!(checks.len(), 1, "{checks:?}");
    checks.into_iter().next().unwrap().outcome
}

#[test]
fn confirmed_source_survives_memory_edit_organize_undo_and_reopen() {
    let mut s = Setup::new();
    let original = s.confirmed();
    let file_id = original.association.as_ref().unwrap().file_id.clone();
    let record = s.service().start_memory_for_file(&file_id).unwrap();
    let initial = s.service().list_snapshots(&file_id).unwrap();
    assert_eq!(initial.len(), 1);
    let inode = fs::metadata(s.path("download.txt")).unwrap().ino();
    let edited = b"Synthetic download version one, edited in place.\n";
    fs::write(s.path("download.txt"), edited).unwrap();
    assert_eq!(fs::metadata(s.path("download.txt")).unwrap().ino(), inode);
    let CheckOutcome::Saved { snapshot } = only_check(&s) else {
        panic!("Memory did not capture the same-object edit");
    };
    assert_eq!(snapshot.file_id, file_id);
    assert_eq!(
        s.service().read_snapshot(&snapshot.snapshot_id).unwrap().1,
        edited
    );
    assert_source(&s.source(&original.source_id), &original, &file_id, true);

    let file = s.register("download.txt");
    let folders = s.service().list_folders(&s.root_id).unwrap();
    let archive = folders
        .folders
        .iter()
        .find(|folder| folder.relative_path == "Archive")
        .unwrap();
    let BuildOutcome::Planned { plan } = s
        .service()
        .build_plan(&PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: s.root_id.clone(),
            operation_id: None,
            target: PlanTarget::Folder {
                catalog_id: folders.catalog_id,
                folder_id: archive.folder_id.clone(),
                new_folders: vec![],
            },
        })
        .unwrap()
    else {
        panic!("Expected an Organize plan");
    };
    let action = s.service().apply_plan(&plan.plan_id).unwrap();
    assert_eq!(action.status, ActionStatus::Committed);
    assert_eq!(
        fs::metadata(s.path("Archive/download.txt")).unwrap().ino(),
        inode
    );
    let moved = s.source(&original.source_id);
    assert_source(&moved, &original, &file_id, true);
    assert!(
        moved
            .association
            .unwrap()
            .display_path
            .ends_with("Archive/download.txt")
    );
    assert_eq!(only_check(&s), CheckOutcome::Unchanged);
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .place
            .relative_path,
        "Archive/download.txt"
    );
    let UndoPreview::Planned { plan } = s.service().preview_undo(&action.action_id).unwrap() else {
        panic!("Expected an Undo plan");
    };
    assert_eq!(
        s.service().apply_plan(&plan.plan_id).unwrap().status,
        ActionStatus::Committed
    );
    assert_eq!(fs::metadata(s.path("download.txt")).unwrap().ino(), inode);
    assert_source(&s.source(&original.source_id), &original, &file_id, true);
    assert_eq!(only_check(&s), CheckOutcome::Unchanged);
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .place
            .relative_path,
        "download.txt"
    );

    let snapshots = s.service().list_snapshots(&file_id).unwrap();
    assert_eq!(snapshots.len(), 2);
    s.restart();
    assert_source(&s.source(&original.source_id), &original, &file_id, true);
    assert_eq!(s.service().list_snapshots(&file_id).unwrap(), snapshots);
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .line_versions,
        2
    );
    assert_eq!(only_check(&s), CheckOutcome::Unchanged);
    assert_eq!(s.service().source_state().unwrap().records.len(), 1);
}

#[test]
fn memory_automatic_link_does_not_transfer_source_to_same_content_replacement() {
    let mut s = Setup::new();
    let original = s.confirmed();
    let old_file_id = original.association.as_ref().unwrap().file_id.clone();
    let record = s.service().start_memory_for_file(&old_file_id).unwrap();
    // Turned on from the notice as shown: its place (v18). The automatic
    // link comes first all the same (the student's 10/14 decision).
    let notice = s.service().candidate_capture(&record.chain_id).unwrap();
    s.service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: true,
            notice_version: Some(CANDIDATE_NOTICE_VERSION),
            occupant_token: None,
            place_token: Some(notice.place_token),
        })
        .unwrap();
    let old_inode = fs::metadata(s.path("download.txt")).unwrap().ino();
    fs::copy(s.path("download.txt"), s.path("replacement.txt")).unwrap();
    let replacement_inode = fs::metadata(s.path("replacement.txt")).unwrap().ino();
    assert_ne!(replacement_inode, old_inode);
    fs::rename(s.path("replacement.txt"), s.path("download.txt")).unwrap();
    // Memory goes on at the new file by itself: the next version, a new
    // file record. Source stays with the file it was confirmed for.
    let CheckOutcome::LinkedAutomatically { snapshot } = only_check(&s) else {
        panic!("Expected Memory to link the replacement as the next version");
    };
    assert_ne!(snapshot.file_id, old_file_id);
    assert_eq!(
        snapshot.sha256,
        s.service().list_snapshots(&old_file_id).unwrap()[0].sha256
    );
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .watch
            .file_id,
        snapshot.file_id
    );
    assert_eq!(s.register("download.txt").file_id, snapshot.file_id);
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    let pending = s.service().refresh_source(&original.source_id).unwrap();
    assert!(
        pending
            .candidates
            .iter()
            .any(|c| c.file_id == snapshot.file_id && c.can_confirm)
    );
    assert_source(&pending, &original, &old_file_id, false);
    s.restart();
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .watch
            .file_id,
        snapshot.file_id
    );
    assert_eq!(only_check(&s), CheckOutcome::Unchanged);
}

#[test]
fn memory_user_continuation_does_not_transfer_source_to_same_content_replacement() {
    let s = Setup::new();
    let original = s.confirmed();
    let old_file_id = original.association.as_ref().unwrap().file_id.clone();
    let record = s.service().start_memory_for_file(&old_file_id).unwrap();
    // Replaced before any pass looks: turning candidate capture on saves
    // the file at the place as a candidate, and the user's own "continue"
    // goes on at it (the automatic link never saw it).
    fs::copy(s.path("download.txt"), s.path("replacement.txt")).unwrap();
    fs::rename(s.path("replacement.txt"), s.path("download.txt")).unwrap();
    let notice = s.service().candidate_capture(&record.chain_id).unwrap();
    assert!(notice.saves_occupant_now);
    s.service()
        .set_candidate_capture(&CandidateSwitchRequest {
            chain_id: record.chain_id.clone(),
            on: true,
            notice_version: Some(CANDIDATE_NOTICE_VERSION),
            occupant_token: notice.unread_occupant.map(|o| o.occupant_token),
            place_token: Some(notice.place_token),
        })
        .unwrap();
    let page = s.service().pending_candidates(&record.chain_id).unwrap();
    assert_eq!(page.items.len(), 1);
    let candidate = page.items[0].clone();
    assert_ne!(candidate.file_id, old_file_id);
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    let continued = s
        .service()
        .decide_pending_candidates(&CandidateDecisionRequest {
            chain_id: record.chain_id.clone(),
            page_token: page.page_token,
            items: vec![CandidateDecision {
                candidate_id: candidate.candidate_id,
                choice: CandidateChoice::Continue {
                    parent_snapshot_id: candidate.continue_from.unwrap().snapshot_id,
                },
            }],
            current_object: None,
        })
        .unwrap();
    assert_eq!(
        continued.continued_watch.unwrap().file_id,
        candidate.file_id
    );
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    assert_eq!(s.register("download.txt").file_id, candidate.file_id);
    let pending = s.service().refresh_source(&original.source_id).unwrap();
    assert!(
        pending
            .candidates
            .iter()
            .any(|c| c.file_id == candidate.file_id && c.can_confirm)
    );
    assert_source(&pending, &original, &old_file_id, false);
    assert_eq!(only_check(&s), CheckOutcome::Unchanged);
}

#[test]
fn memory_relocation_preserves_source_boundary_until_explicit_registration() {
    let mut s = Setup::new();
    let original = s.confirmed();
    let old_file_id = original.association.as_ref().unwrap().file_id.clone();
    let record = s.service().start_memory_for_file(&old_file_id).unwrap();
    fs::rename(s.path("download.txt"), s.path("Archive/renamed.txt")).unwrap();
    assert!(
        matches!(only_check(&s), CheckOutcome::Problem { error } if error.code == ErrorCode::FileNotFound)
    );
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    let preview = s
        .service()
        .relocation_preview(&record.chain_id, &s.root_id, &s.path("Archive/renamed.txt"))
        .unwrap();
    let relocated = s
        .service()
        .relocate_record(&RelocationRequest {
            chain_id: preview.chain_id,
            root_id: preview.new_place.root_id,
            relative_path: preview.new_place.relative_path,
            page_token: preview.page_token,
        })
        .unwrap();
    assert_ne!(relocated.snapshot.file_id, old_file_id);
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    assert_eq!(relocated.record.chain_id, record.chain_id);
    assert_eq!(relocated.record.line_versions, 2);

    // Contract v16 §4.1: relocation itself does not run the Source registration hook.
    let registered = s.register("Archive/renamed.txt");
    assert_eq!(registered.file_id, relocated.snapshot.file_id);
    let source = s.source(&original.source_id);
    assert_source(&source, &original, &registered.file_id, true);
    assert!(
        source
            .association
            .unwrap()
            .display_path
            .ends_with("Archive/renamed.txt")
    );
    s.restart();
    assert_source(
        &s.source(&original.source_id),
        &original,
        &registered.file_id,
        true,
    );
    assert_eq!(
        s.service()
            .memory_record(&record.chain_id)
            .unwrap()
            .watch
            .file_id,
        registered.file_id
    );
    fs::write(
        s.path("Archive/renamed.txt"),
        b"Changed at the relocated place.\n",
    )
    .unwrap();
    assert!(
        matches!(only_check(&s), CheckOutcome::Saved { snapshot } if snapshot.file_id == registered.file_id)
    );
    assert_source(
        &s.source(&original.source_id),
        &original,
        &registered.file_id,
        true,
    );
}

#[test]
fn copy_delete_and_an_independent_memory_record_do_not_inherit_source() {
    let mut s = Setup::new();
    let original = s.confirmed();
    let old_file_id = original.association.as_ref().unwrap().file_id.clone();
    let record = s.service().start_memory_for_file(&old_file_id).unwrap();
    let old_inode = fs::metadata(s.path("download.txt")).unwrap().ino();
    fs::copy(s.path("download.txt"), s.path("Archive/copy.txt")).unwrap();
    assert_ne!(
        fs::metadata(s.path("Archive/copy.txt")).unwrap().ino(),
        old_inode
    );
    fs::remove_file(s.path("download.txt")).unwrap();
    assert!(
        matches!(only_check(&s), CheckOutcome::Problem { error } if error.code == ErrorCode::FileNotFound)
    );
    assert_eq!(
        s.service()
            .relocation_preview(&record.chain_id, &s.root_id, &s.path("Archive/copy.txt"))
            .unwrap_err()
            .code,
        ErrorCode::NotSameObject
    );
    let copy = s.register("Archive/copy.txt");
    assert_ne!(copy.file_id, old_file_id);
    let independent = s.service().start_memory_for_file(&copy.file_id).unwrap();
    assert_ne!(independent.family_id, record.family_id);
    assert!(!independent.same_content.is_empty());
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    let unknown = s.ingest("missing.txt", "2");
    assert_eq!(unknown.status, SourceStatus::Unknown);
    assert!(unknown.association.is_none());
    assert!(unknown.candidates.is_empty());
    s.service().set_source_enabled(false).unwrap();
    fs::write(
        s.path("Archive/copy.txt"),
        b"Independent copy edited while Source is off.\n",
    )
    .unwrap();
    let checks = s.service().check_memory().unwrap();
    assert!(checks.iter().any(|c| matches!(&c.outcome, CheckOutcome::Saved { snapshot } if snapshot.file_id == copy.file_id)));
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    s.restart();
    assert!(!s.service().source_state().unwrap().enabled);
    assert_source(
        &s.source(&original.source_id),
        &original,
        &old_file_id,
        false,
    );
    assert_eq!(s.source(&unknown.source_id).status, SourceStatus::Unknown);
    assert_eq!(s.service().list_snapshots(&copy.file_id).unwrap().len(), 2);
    assert_eq!(s.service().memory_records().unwrap().len(), 2);
}
