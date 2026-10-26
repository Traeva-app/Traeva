//! Plans through the public service API (organize-spec.md §3, §6; O05, O06,
//! O35): exact previews, superseding, cancelling, no-op, conflicts, refused
//! names, stale choices and restart. Synthetic temporary files only.

mod common;

use std::fs;
use std::path::PathBuf;

use common::setup::Setup;
use common::*;
use traeva_core::{
    BuildOutcome, ErrorCode, PlanKind, PlanLifecycle, PlanRequest, PlanStatus, PlanTarget,
};

#[test]
fn a_plan_previews_exactly_what_will_happen_and_writes_nothing() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let before = snapshot::snapshot(s.root_dir.path());

    let plan = s.plan(&file, "Notes", &["2026 taxes", "Q1"]);

    assert_eq!(
        snapshot::snapshot(s.root_dir.path()),
        before,
        "a preview writes nothing"
    );
    assert!(!s.path("Notes/2026 taxes").exists());
    assert_eq!(plan.kind, PlanKind::Organize);
    assert_eq!(plan.source.relative_path, "todo.txt");
    assert_eq!(plan.target.relative_path, "Notes/2026 taxes/Q1/todo.txt");
    assert_eq!(
        PathBuf::from(&plan.target.display_path),
        fs::canonicalize(s.root_dir.path())
            .unwrap()
            .join("Notes/2026 taxes/Q1/todo.txt")
    );
    assert_eq!(plan.basename, "todo.txt");
    assert_eq!(
        plan.folders_to_create,
        ["Notes/2026 taxes", "Notes/2026 taxes/Q1"]
    );
    assert_eq!(plan.file_revision, file.file_revision);
    assert_eq!(plan.expires_at_ms - plan.created_at_ms, 15 * 60 * 1000);
    assert_eq!(
        (plan.status, plan.can_confirm, plan.reason.clone()),
        (PlanStatus::Confirmable, true, None)
    );
    assert_eq!(s.service().get_plan(&plan.plan_id).unwrap(), plan);

    let (segments, to_create, sha256): (String, String, String) = s
        .db()
        .query_row(
            "SELECT target_segments_json, folders_to_create_json,
                    json_extract(expected_source_json, '$.sha256')
             FROM plans WHERE plan_id = ?1",
            [&plan.plan_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(segments, r#"["Notes","2026 taxes","Q1"]"#);
    assert_eq!(to_create, r#"["2026 taxes","Q1"]"#);
    assert_eq!(sha256, file.sha256);
    assert_eq!(s.count("actions"), 0);
}

#[test]
fn choosing_again_supersedes_the_old_plan_and_cancelling_is_final() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let first = s.plan(&file, "Taxes", &[]);
    let second = s.plan(&file, "Notes", &[]);

    assert_ne!(first.plan_id, second.plan_id);
    let old = s.service().get_plan(&first.plan_id).unwrap();
    assert_eq!(
        (old.status, old.lifecycle),
        (PlanStatus::Superseded, PlanLifecycle::Superseded)
    );
    assert!(old.reason.unwrap().contains(&second.plan_id));
    assert_eq!(
        old.target, first.target,
        "the old plan's content is unchanged"
    );
    assert!(s.service().get_plan(&second.plan_id).unwrap().can_confirm);

    let cancelled = s.service().cancel_plan(&second.plan_id).unwrap();
    assert_eq!(
        (cancelled.status, cancelled.can_confirm),
        (PlanStatus::Cancelled, false)
    );
    assert_eq!(s.service().cancel_plan(&second.plan_id).unwrap(), cancelled);
    assert_eq!(
        s.service().cancel_plan(&first.plan_id).unwrap().status,
        PlanStatus::Superseded,
        "a closed plan stays as it is"
    );

    // Confirming either one anyway creates no action and moves nothing.
    let before = snapshot::snapshot(s.root_dir.path());
    for plan_id in [&first.plan_id, &second.plan_id] {
        let error = s.service().apply_plan(plan_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::StalePlan);
    }
    assert_eq!(s.count("actions"), 0);
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
}

#[test]
fn a_file_already_in_the_chosen_folder_needs_no_plan() {
    let s = Setup::new();
    let file = s.register("Notes/2026/plan.md");
    match s.build(&file, "Notes/2026", &[]).unwrap() {
        BuildOutcome::AlreadyInPlace { file_id } => assert_eq!(file_id, file.file_id),
        other => panic!("{other:?}"),
    }
    assert_eq!((s.count("plans"), s.count("actions")), (0, 0));
}

#[test]
fn taken_names_are_conflicts_never_overwrites() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    fs::write(s.path("Taxes/todo.txt"), "already here").unwrap();

    let error = s.build(&file, "Taxes", &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::TargetConflict);
    assert_eq!(
        s.build(&file, "Notes", &["2026"]).unwrap_err().code,
        ErrorCode::TargetConflict
    );
    // Another spelling of an existing folder name counts when the volume says so.
    let result = s.build(&file, "", &["notes"]);
    if fs::symlink_metadata(s.path("notes")).is_ok() {
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetConflict);
    } else {
        assert!(result.is_ok());
    }
    assert_eq!(fs::read(s.path("Taxes/todo.txt")).unwrap(), b"already here");
}

#[test]
fn bad_folder_names_are_refused_not_renamed() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    for bad in [
        &["a/b"][..],
        &[".hidden"],
        &["x:y"],
        &[""],
        &["Tools.app"],
        &["a", "b", "c", "d"],
    ] {
        let error = s.build(&file, "Taxes", bad).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidFolderName, "{bad:?}");
    }
    assert_eq!(s.count("plans"), 0);
}

#[test]
fn names_with_edge_spaces_or_a_gap_are_refused_as_typed() {
    // #93: the window sends the names untrimmed; the core names the exact
    // name it refused and writes nothing.
    let s = Setup::new();
    let file = s.register("todo.txt");
    let before = snapshot::snapshot(s.root_dir.path());
    for (names, quoted, why) in [
        (&["  Amber  ", " Winter"][..], "“  Amber  ”", "space"),
        (&["2026", " Winter"], "“ Winter”", "space"),
        (&["Winter "], "“Winter ”", "space"),
        (&["   "], "“   ”", "space"),
        (&["", "Winter"], "“”", "empty"),
    ] {
        let error = s.build(&file, "Taxes", names).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidFolderName, "{names:?}");
        assert!(error.message.contains(quoted), "{}", error.message);
        assert!(error.message.contains(why), "{}", error.message);
    }
    assert_eq!(s.count("plans"), 0);
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
}

#[test]
fn a_project_marker_name_is_refused_as_a_new_folder() {
    // A folder named "Cargo.toml" would make the folder around it a project
    // (#58): Traeva would then refuse to undo or organize anything there.
    let s = Setup::new();
    let file = s.register("todo.txt");
    let before = snapshot::snapshot(s.root_dir.path());
    for (folder, names) in [
        ("", &["package.json"][..]),
        ("Notes", &["Cargo.toml"]),
        ("Taxes", &["2026", "CMAKELISTS.TXT"]),
    ] {
        let error = s.build(&file, folder, names).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidFolderName, "{names:?}");
        assert!(error.message.contains("project"), "{}", error.message);
    }
    assert_eq!(s.count("plans"), 0);
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
}

#[test]
fn stale_or_unknown_choices_are_refused() {
    let s = Setup::new();
    let file = s.register("todo.txt");
    let old_catalog = s.catalog();
    let catalog = s.catalog(); // a newer list makes the old one unusable
    let build = |request: &PlanRequest| s.service().build_plan(request).unwrap_err().code;

    let mut request = s.request(&file, &old_catalog, "Taxes", &[]);
    assert_eq!(build(&request), ErrorCode::FolderListOutdated);
    request = s.request(&file, &catalog, "Taxes", &[]);
    if let PlanTarget::Folder { folder_id, .. } = &mut request.target {
        *folder_id = "F999".into();
    }
    assert_eq!(build(&request), ErrorCode::FolderListOutdated);
    assert_eq!(
        build(&s.request(&file, &catalog, "web", &[])),
        ErrorCode::ProtectedLocation
    );

    request = s.request(&file, &catalog, "Taxes", &[]);
    request.file_revision -= 1;
    assert_eq!(build(&request), ErrorCode::FileChanged);
    request = s.request(&file, &catalog, "Taxes", &[]);
    request.file_id = "8a1f4d0e-6c3b-4f9a-9d7e-2b5c1a0e9f33".into();
    assert_eq!(build(&request), ErrorCode::UnknownFile);
    request.file_id = "not-an-id".into();
    assert_eq!(build(&request), ErrorCode::InvalidInput);

    // The file was edited after it was chosen.
    fs::write(s.path("todo.txt"), "buy milk and eggs\n").unwrap();
    assert_eq!(
        build(&s.request(&file, &catalog, "Taxes", &[])),
        ErrorCode::FileChanged
    );
    assert_eq!(s.count("plans"), 0);
}

#[test]
fn a_restart_invalidates_open_plans_and_old_folder_lists() {
    let mut s = Setup::new();
    let file = s.register("todo.txt");
    let catalog = s.catalog();
    let plan = s.plan(&file, "Taxes", &[]);

    s.restart();

    let view = s.service().get_plan(&plan.plan_id).unwrap();
    assert_eq!(
        (view.status, view.can_confirm),
        (PlanStatus::Invalidated, false)
    );
    assert_eq!(view.reason.as_deref(), Some("Traeva restarted"));
    let error = s
        .service()
        .build_plan(&s.request(&file, &catalog, "Taxes", &[]))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FolderListOutdated);

    // Confirming the old plan after the restart does nothing (O33).
    let before = snapshot::snapshot(s.root_dir.path());
    let error = s.service().apply_plan(&plan.plan_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::StalePlan);
    assert_eq!(s.count("actions"), 0);
    assert_eq!(snapshot::snapshot(s.root_dir.path()), before);
}

#[test]
fn a_plan_can_target_another_root_on_the_same_volume() {
    let s = Setup::new();
    let other = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    fs::create_dir(other.path().join("Archive")).unwrap();
    let other_root = s.service().authorize_root(other.path()).unwrap().root_id;
    let file = s.register("todo.txt");
    let catalog = s.service().list_folders(&other_root).unwrap();

    match s
        .service()
        .build_plan(&s.request(&file, &catalog, "Archive", &[]))
        .unwrap()
    {
        BuildOutcome::Planned { plan } => {
            assert_eq!(plan.source.root_id, s.root_id);
            assert_eq!(plan.target.root_id, other_root);
            assert_eq!(plan.target.relative_path, "Archive/todo.txt");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn plan_ids_and_requests_are_checked() {
    let s = Setup::new();
    assert_eq!(
        s.service().get_plan("not-an-id").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let unknown = "0f4b6c1e-3a2d-4e5f-8a9b-7c6d5e4f3a2b";
    assert_eq!(
        s.service().get_plan(unknown).unwrap_err().code,
        ErrorCode::UnknownPlan
    );
    assert_eq!(
        s.service().cancel_plan(unknown).unwrap_err().code,
        ErrorCode::UnknownPlan
    );

    let json = r#"{"file_id":"x","file_revision":1,"target_root_id":"y","catalog_id":"z","folder_id":"F1","sneaky":true}"#;
    assert!(
        serde_json::from_str::<PlanRequest>(json).is_err(),
        "unknown fields are refused"
    );
}

/// A folder whose contents cannot be listed cannot be checked for project
/// markers, so it is never a target: neither in a new plan nor in a plan
/// made before (organize-spec.md §4.2, O12).
#[test]
fn a_folder_that_cannot_be_listed_is_never_a_target() {
    use std::os::unix::fs::PermissionsExt;
    let s = Setup::new();
    let file = s.register("todo.txt");
    let earlier = s.plan(&file, "Taxes", &[]);
    let taxes = s.path("Taxes");
    fs::set_permissions(&taxes, fs::Permissions::from_mode(0o300)).unwrap();

    let applied = s.service().apply_plan(&earlier.plan_id);
    let catalog = s.catalog();
    fs::set_permissions(&taxes, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(applied.unwrap_err().code, ErrorCode::StalePlan);
    assert!(
        !catalog.folders.iter().any(|f| f.relative_path == "Taxes"),
        "not offered in a new folder list"
    );
    assert_eq!(s.count("actions"), 0);
    assert!(s.path("todo.txt").exists());
    assert_eq!(fs::read_dir(&taxes).unwrap().count(), 0);
}

/// The target root is on another volume: no plan is made, because moving
/// there would not be one atomic rename, and nothing is written on either
/// volume (organize-spec.md §1, §2.1, O11).
#[test]
#[ignore = "mounts a temporary disk image; run with: cargo test -p traeva-core -- --ignored"]
fn a_plan_to_another_volume_is_refused_and_nothing_is_written() {
    let s = Setup::new();
    let holder = tempfile::Builder::new()
        .prefix("traeva-vol-")
        .tempdir()
        .unwrap();
    let apfs = common::disk_image::MountedImage::mount_in(holder.path(), "apfs", "APFS", "16m");
    let other_dir = apfs.mount_point.join("Inbox");
    fs::create_dir_all(other_dir.join("Archive")).unwrap();
    let other = s.service().authorize_root(&other_dir).unwrap().root_id;
    let file = s.register("todo.txt");
    let catalog = s.service().list_folders(&other).unwrap();
    let before = (
        snapshot::snapshot(s.root_dir.path()),
        snapshot::snapshot(&other_dir),
    );

    let error = s
        .service()
        .build_plan(&s.request(&file, &catalog, "Archive", &[]))
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedLocation, "{error:?}");
    assert!(
        error.message.contains("different volumes"),
        "{}",
        error.message
    );
    assert_eq!(s.count("plans"), 0);
    assert_eq!(
        (
            snapshot::snapshot(s.root_dir.path()),
            snapshot::snapshot(&other_dir)
        ),
        before
    );
}
