//! Authorized roots through the public service API (organize-spec.md §2.1, §2.2, §4.2):
//! authorize and revoke, app-data isolation in both directions, protected and
//! unsupported locations, and re-checking a root every time it is used.
//! Every folder here is a synthetic temporary folder.

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use common::*;
use traeva_core::{AppError, ErrorCode, ErrorStage, NextStep, OrganizeService, RootStatus};

fn temp(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(prefix).tempdir().unwrap()
}

/// A ready service with its own temporary app-data folder.
fn service() -> (tempfile::TempDir, OrganizeService) {
    let app = temp("traeva-appdata-");
    let service = OrganizeService::start(app.path().join("Traeva"));
    expect_ready(&service);
    (app, service)
}

fn refused(service: &OrganizeService, path: &Path) -> AppError {
    service
        .authorize_root(path)
        .expect_err(&format!("{} should be refused", path.display()))
}

#[test]
fn authorizing_reads_only_and_saves_an_active_root() {
    let (app, service) = service();
    let root = temp("traeva-root-");
    snapshot::fill_sample_tree(root.path());
    let before = snapshot::snapshot(root.path());

    let view = service.authorize_root(root.path()).unwrap();
    let listed = service.list_roots().unwrap();

    assert_eq!(view.status, RootStatus::Active);
    assert_eq!(
        PathBuf::from(&view.display_path),
        fs::canonicalize(root.path()).unwrap()
    );
    assert_eq!(view.revoked_at_ms, None);
    assert_eq!(view.problem, None);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].root_id, view.root_id);
    assert_eq!(listed[0].problem, None);
    assert_eq!(
        snapshot::snapshot(root.path()),
        before,
        "authorizing and checking a root must not write anything inside it"
    );

    let conn = open_for_inspection(&app.path().join("Traeva"));
    let (status, evidence): (String, String) = conn
        .query_row(
            "SELECT status, evidence_json FROM authorized_roots WHERE root_id = ?1",
            [&view.root_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
    assert_eq!(status, "active");
    assert_eq!(evidence["fs_type"], "apfs");
    assert!(
        evidence["dev"].is_i64() && evidence["ino"].is_u64() && evidence["birthtime_ns"].is_i64()
    );
}

#[test]
fn authorizing_the_same_folder_again_returns_the_same_root() {
    let (_app, service) = service();
    let root = temp("traeva-root-");
    let first = service.authorize_root(root.path()).unwrap();
    // The same folder through another spelling of its path (/var vs /private/var).
    let second = service
        .authorize_root(&fs::canonicalize(root.path()).unwrap())
        .unwrap();
    assert_eq!(first.root_id, second.root_id);
    assert_eq!(service.list_roots().unwrap().len(), 1);
}

#[test]
fn revocation_is_permanent_and_authorizing_again_gives_a_new_root() {
    let (_app, service) = service();
    let root = temp("traeva-root-");
    let first = service.authorize_root(root.path()).unwrap();

    let revoked = service.revoke_root(&first.root_id).unwrap();
    assert_eq!(revoked.status, RootStatus::Revoked);
    assert!(revoked.revoked_at_ms.is_some());
    assert_eq!(
        service.revoke_root(&first.root_id).unwrap(),
        revoked,
        "revoking twice changes nothing"
    );
    assert!(service.list_roots().unwrap().is_empty());

    let again = service.authorize_root(root.path()).unwrap();
    assert_ne!(again.root_id, first.root_id);
    assert_eq!(
        service.revoke_root(&first.root_id).unwrap().status,
        RootStatus::Revoked
    );
    assert_eq!(service.list_roots().unwrap()[0].root_id, again.root_id);
}

#[test]
fn a_root_may_not_contain_or_be_inside_the_app_data_folder() {
    // The root contains Traeva's app data.
    let outer = temp("traeva-root-");
    let inner_app_data = OrganizeService::start(outer.path().join("Traeva"));
    expect_ready(&inner_app_data);
    let error = refused(&inner_app_data, outer.path());
    assert_eq!(error.code, ErrorCode::OverlapsAppData);
    assert!(
        error.message.contains("contains Traeva's own data folder"),
        "{}",
        error.message
    );
    drop(inner_app_data);

    // The root is Traeva's app-data folder, or inside it.
    let (app, service) = service();
    let app_data = app.path().join("Traeva");
    fs::create_dir(app_data.join("sub")).unwrap();
    for root in [app_data.join("sub"), app_data.clone()] {
        let error = refused(&service, &root);
        assert_eq!(error.code, ErrorCode::OverlapsAppData);
        assert!(error.message.contains("or inside it"), "{}", error.message);
    }

    // Traeva cannot tell, because the folder's parents cannot be looked up from it.
    let root = temp("traeva-root-");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let error = refused(&service, root.path());
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.code, ErrorCode::OverlapsAppData);
    assert!(
        error.message.contains("cannot rule it out"),
        "{}",
        error.message
    );
    assert_eq!(error.stage, ErrorStage::Authorization);
    assert!(!error.retryable);
    assert_eq!(error.allowed_next, vec![NextStep::ChooseFolder]);
    assert!(service.list_roots().unwrap().is_empty());
}

#[test]
fn protected_and_unsupported_folders_are_refused_without_writing() {
    let (_app, service) = service();
    let area = temp("traeva-area-");
    let base = area.path();
    fs::create_dir_all(base.join("web")).unwrap();
    fs::write(base.join("web/package.json"), "{}").unwrap();
    fs::create_dir_all(base.join("repo/.git")).unwrap();
    fs::create_dir_all(base.join("repo/docs")).unwrap();
    fs::create_dir(base.join(".stash")).unwrap();
    fs::create_dir(base.join("Album.photoslibrary")).unwrap();
    fs::create_dir_all(base.join("Tool.app/Contents")).unwrap();
    fs::create_dir(base.join("real")).unwrap();
    symlink(base.join("real"), base.join("link")).unwrap();
    fs::write(base.join("file.txt"), "not a folder").unwrap();
    let before = snapshot::snapshot(base);

    let protected = [
        "web",                 // a project folder
        "repo/docs",           // inside a project folder
        ".stash",              // hidden
        "Album.photoslibrary", // a package
        "Tool.app/Contents",   // inside a package
    ];
    for name in protected {
        assert_eq!(
            refused(&service, &base.join(name)).code,
            ErrorCode::ProtectedLocation,
            "{name}"
        );
    }
    assert_eq!(
        refused(&service, &base.join("link")).code,
        ErrorCode::UnsupportedLocation
    );
    assert_eq!(
        refused(&service, &base.join("file.txt")).code,
        ErrorCode::UnsupportedLocation
    );
    let missing = refused(&service, &base.join("missing"));
    assert_eq!(missing.code, ErrorCode::RootUnavailable);
    assert!(missing.retryable);
    assert_eq!(
        refused(&service, Path::new("relative/folder")).code,
        ErrorCode::InvalidInput
    );

    assert!(service.list_roots().unwrap().is_empty());
    assert_eq!(
        snapshot::snapshot(base),
        before,
        "refusing a folder must not write anything"
    );
}

#[test]
fn a_moved_or_replaced_root_is_noticed_after_a_restart() {
    let app = temp("traeva-appdata-");
    let area = temp("traeva-area-");
    let root = area.path().join("Inbox");
    fs::create_dir(&root).unwrap();
    let first = OrganizeService::start(app.path().join("Traeva"));
    let view = first.authorize_root(&root).unwrap();
    drop(first);

    // Another program moves the folder away and creates a look-alike at its path.
    fs::rename(&root, area.path().join("Inbox-moved")).unwrap();
    fs::create_dir(&root).unwrap();
    let service = OrganizeService::start(app.path().join("Traeva"));
    let problem = |service: &OrganizeService| service.list_roots().unwrap()[0].problem.clone();
    assert_eq!(problem(&service).unwrap().code, ErrorCode::RootChanged);

    // Nothing at the path at all.
    fs::remove_dir(&root).unwrap();
    assert_eq!(problem(&service).unwrap().code, ErrorCode::RootUnavailable);

    // The original folder comes back: it is the authorized root again.
    fs::rename(area.path().join("Inbox-moved"), &root).unwrap();
    assert_eq!(problem(&service), None);
    assert_eq!(service.list_roots().unwrap()[0].root_id, view.root_id);
}

#[test]
fn every_use_checks_the_rules_again() {
    let (_app, service) = service();
    let root = temp("traeva-root-");
    service.authorize_root(root.path()).unwrap();
    // The folder later becomes a project.
    fs::write(root.path().join("Cargo.toml"), "[package]").unwrap();
    let problem = service.list_roots().unwrap()[0].problem.clone().unwrap();
    assert_eq!(problem.code, ErrorCode::ProtectedLocation);
}

#[test]
fn root_ids_must_be_exact_known_ids() {
    let (_app, service) = service();
    let root = temp("traeva-root-");
    let view = service.authorize_root(root.path()).unwrap();

    assert_eq!(
        service.revoke_root("not-a-uuid").unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        service
            .revoke_root(&view.root_id.to_uppercase())
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        service
            .revoke_root("6f9619ff-8b86-4d01-b42d-00cf4fc964ff")
            .unwrap_err()
            .code,
        ErrorCode::UnknownRoot
    );
    assert_eq!(service.list_roots().unwrap()[0].status, RootStatus::Active);
}

#[test]
fn root_operations_wait_for_storage() {
    let (app, first) = service();
    let second = OrganizeService::start(app.path().join("Traeva"));
    let root = temp("traeva-root-");
    let error = second.authorize_root(root.path()).unwrap_err();
    assert_eq!(error.code, ErrorCode::AnotherInstanceRunning);
    assert_eq!(error.stage, ErrorStage::Storage);
    assert!(first.list_roots().unwrap().is_empty());
}

#[test]
#[ignore = "mounts temporary disk images; run with: cargo test -p traeva-core -- --ignored"]
fn only_local_apfs_volumes_can_hold_a_root() {
    let (_app, service) = service();
    let holder = temp("traeva-vol-");

    let hfs = disk_image::MountedImage::mount_in(holder.path(), "hfs", "HFS+", "8m");
    fs::create_dir(hfs.mount_point.join("Inbox")).unwrap();
    let error = refused(&service, &hfs.mount_point.join("Inbox"));
    assert_eq!(error.code, ErrorCode::UnsupportedLocation);
    assert!(error.message.contains("hfs"), "{}", error.message);

    let apfs = disk_image::MountedImage::mount_in(holder.path(), "apfs", "APFS", "16m");
    fs::create_dir(apfs.mount_point.join("Inbox")).unwrap();
    let view = service
        .authorize_root(&apfs.mount_point.join("Inbox"))
        .unwrap();
    assert_eq!(view.status, RootStatus::Active);
}

/// The app-data folder is moved into an authorized root while Traeva runs,
/// with a symbolic link left at its old place: every use of that root checks
/// the overlap again and is refused, and nothing in the root changes
/// (organize-spec.md §2.2, O11).
#[test]
fn a_data_folder_moved_into_a_root_while_traeva_runs_stops_every_use_of_it() {
    let base = temp("traeva-appdata-");
    let dir = base.path().join("Traeva");
    let service = OrganizeService::start(dir.clone());
    expect_ready(&service);
    let root = temp("traeva-root-");
    fs::write(root.path().join("todo.txt"), "buy milk\n").unwrap();
    let root_id = service.authorize_root(root.path()).unwrap().root_id;
    let inside = root.path().join("Traeva");
    fs::rename(&dir, &inside).unwrap();
    symlink(&inside, &dir).unwrap();
    let before = snapshot::snapshot(root.path());

    let registered = service
        .register_file(&root_id, &root.path().join("todo.txt"))
        .unwrap_err();
    let listed = service.list_folders(&root_id).unwrap_err();
    let problem = service.list_roots().unwrap()[0].problem.clone().unwrap();

    for error in [&registered, &listed, &problem] {
        assert_eq!(error.code, ErrorCode::OverlapsAppData, "{error:?}");
    }
    assert_eq!(snapshot::snapshot(root.path()), before, "nothing written");
}
