use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;

use super::*;
use crate::OrganizeService;
use crate::contracts::{AppDataSource, StorageStatus};
use crate::settings::{DEFAULT_MODEL, ModelSettingsInput, PROVIDER_OPENAI};

/// A fake home folder; the normal app-data folder would be inside it.
fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("traeva-home-")
        .tempdir()
        .unwrap()
}

fn normal_dir(home: &tempfile::TempDir) -> PathBuf {
    home.path().join("Library/Application Support/Traeva")
}

/// The two refusals about the normal folder, told apart (every refusal
/// also says it "does not fall back to its normal data folder").
const INSIDE: &str = "is, or is inside, Traeva's normal data folder";
const CONTAINS: &str = "it contains Traeva's normal data folder";

fn os(path: &Path) -> Option<OsString> {
    Some(path.as_os_str().to_os_string())
}

fn refusal(override_dir: Option<OsString>, home: &tempfile::TempDir) -> String {
    match choose(override_dir, os(home.path())) {
        Ok(chosen) => panic!("expected a refusal, got {chosen:?}"),
        Err(error) => {
            assert_eq!(error.code, ErrorCode::AppDataUnavailable);
            assert!(
                error.message.contains("does not fall back"),
                "{}",
                error.message
            );
            error.message
        }
    }
}

#[test]
fn without_the_variable_the_normal_folder_is_used() {
    let home = home();
    assert_eq!(
        choose(None, os(home.path())).unwrap(),
        Chosen::Default(normal_dir(&home))
    );
    assert_eq!(
        choose(None, None).unwrap_err().code,
        ErrorCode::AppDataUnavailable
    );
}

#[test]
fn a_new_empty_or_earlier_test_folder_is_used_by_its_real_path() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let real_tests = fs::canonicalize(tests.path()).unwrap();

    let new = tests.path().join("r-gate2/appdata");
    assert_eq!(
        choose(os(&new), os(home.path())).unwrap(),
        Chosen::Test(real_tests.join("r-gate2/appdata"))
    );
    assert!(!new.exists(), "choosing creates nothing");

    let empty = tests.path().join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(matches!(
        choose(os(&empty), os(home.path())).unwrap(),
        Chosen::Test(_)
    ));

    let earlier = tests.path().join("earlier");
    fs::create_dir(&earlier).unwrap();
    fs::write(earlier.join(DATABASE_FILE_NAME), b"").unwrap();
    fs::write(earlier.join(LOCK_FILE_NAME), b"").unwrap();
    assert!(matches!(
        choose(os(&earlier), os(home.path())).unwrap(),
        Chosen::Test(_)
    ));
}

#[test]
fn malformed_values_are_refused() {
    let home = home();
    assert!(refusal(Some(OsString::new()), &home).contains("empty"));
    assert!(refusal(Some("relative/appdata".into()), &home).contains("absolute"));
    assert!(refusal(Some("/tmp/a/../b".into()), &home).contains("“..”"));
    assert!(refusal(Some("/tmp/./b".into()), &home).contains("“.”"));
    assert!(refusal(Some("/tmp/b/.".into()), &home).contains("“.”"));
    let not_utf8 = OsString::from_vec(b"/tmp/\xFF".to_vec());
    assert!(refusal(Some(not_utf8), &home).contains("UTF-8"));
}

#[test]
fn the_normal_folder_itself_or_around_it_is_refused() {
    let home = home();
    let normal = normal_dir(&home);
    fs::create_dir_all(&normal).unwrap();
    for inside in [normal.clone(), normal.join("inside")] {
        let message = refusal(os(&inside), &home);
        assert!(message.contains(INSIDE), "{message}");
    }
    for around in [
        home.path().join("Library/Application Support"),
        home.path().to_path_buf(),
    ] {
        let message = refusal(os(&around), &home);
        assert!(message.contains(CONTAINS), "{message}");
    }
    // Also before the normal folder exists.
    let fresh = self::home();
    let message = refusal(os(&normal_dir(&fresh)), &fresh);
    assert!(message.contains(INSIDE), "{message}");
}

#[test]
fn a_way_round_through_a_symbolic_link_is_refused() {
    let home = home();
    let normal = normal_dir(&home);
    fs::create_dir_all(&normal).unwrap();
    let tests = tempfile::tempdir().unwrap();
    // A link to "Application Support": the normal folder under another name.
    let link = tests.path().join("support");
    std::os::unix::fs::symlink(normal.parent().unwrap(), &link).unwrap();
    let message = refusal(os(&link.join("Traeva")), &home);
    assert!(message.contains(INSIDE), "{message}");

    // The folder itself a link (even to somewhere harmless).
    let elsewhere = tests.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let alias = tests.path().join("alias");
    std::os::unix::fs::symlink(&elsewhere, &alias).unwrap();
    assert!(refusal(os(&alias), &home).contains("symbolic link"));
}

#[test]
fn a_file_or_a_folder_with_other_files_is_refused() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let file = tests.path().join("a-file");
    fs::write(&file, b"x").unwrap();
    assert!(refusal(os(&file), &home).contains("not a folder"));

    let personal = tests.path().join("Documents");
    fs::create_dir(&personal).unwrap();
    fs::write(personal.join("essay.txt"), b"mine").unwrap();
    assert!(refusal(os(&personal), &home).contains("other files"));
    assert_eq!(fs::read_dir(&personal).unwrap().count(), 1, "nothing added");
}

#[test]
fn a_refused_folder_blocks_storage_and_never_opens_the_normal_one() {
    let home = home();
    let service = OrganizeService::start_from(Some("relative".into()), os(home.path()));
    let state = service.get_app_state().unwrap();
    let StorageStatus::Blocked { error } = state.storage else {
        panic!("expected blocked storage");
    };
    assert_eq!(error.code, ErrorCode::AppDataUnavailable);
    assert!(
        !normal_dir(&home).exists(),
        "the normal folder was not created"
    );
    assert!(!home.path().join("Library").exists());
}

#[test]
fn a_test_folder_is_opened_marked_and_kept_apart_from_the_normal_one() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let folder = tests.path().join("appdata");
    let service = OrganizeService::start_from(os(&folder), os(home.path()));
    let state = service.get_app_state().unwrap();
    let StorageStatus::Ready(info) = state.storage else {
        panic!("expected ready storage");
    };
    let real = fs::canonicalize(&folder).unwrap();
    assert_eq!(info.app_data_dir, real.display().to_string());
    assert_eq!(info.app_data_source, AppDataSource::Test);
    assert!(real.join(DATABASE_FILE_NAME).exists());
    assert!(
        !normal_dir(&home).exists(),
        "the normal folder was not touched"
    );

    // Its own Keychain account, derived from the folder.
    let view = service.model_settings().unwrap();
    assert_eq!(
        view.key_account,
        format!("{PROVIDER_OPENAI}{}", test_key_suffix(&real))
    );
    assert!(view.key_account.starts_with("openai@test-"));
    assert_eq!(view.key_account.len(), "openai@test-".len() + 12);
}

#[test]
fn a_key_saved_in_a_test_folder_goes_to_its_own_account() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let service = OrganizeService::start_from(os(&tests.path().join("appdata")), os(home.path()));
    service
        .save_model_settings(&ModelSettingsInput {
            provider: PROVIDER_OPENAI.to_string(),
            model: DEFAULT_MODEL.to_string(),
            api_key: Some(format!("sk-test-{}", "k".repeat(40))),
        })
        .unwrap();
    let account = service.model_settings().unwrap().key_account;
    assert!(service.model.keys.exists(&account).unwrap());
    assert!(
        !service.model.keys.exists(PROVIDER_OPENAI).unwrap(),
        "the normal account was not written"
    );
    service.remove_api_key().unwrap();
    assert!(!service.model.keys.exists(&account).unwrap());
}

#[test]
fn the_normal_folder_keeps_the_plain_account() {
    let home = home();
    let service = OrganizeService::start_from(None, os(home.path()));
    let StorageStatus::Ready(info) = service.get_app_state().unwrap().storage else {
        panic!("expected ready storage");
    };
    assert_eq!(info.app_data_source, AppDataSource::Default);
    assert_eq!(info.app_data_dir, normal_dir(&home).display().to_string());
    assert_eq!(
        service.model_settings().unwrap().key_account,
        PROVIDER_OPENAI
    );
}

#[test]
fn two_folders_get_two_accounts_and_one_folder_admits_one_instance() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let a = tests.path().join("a");
    let b = tests.path().join("b");
    let first = OrganizeService::start_from(os(&a), os(home.path()));
    let other = OrganizeService::start_from(os(&b), os(home.path()));
    assert_ne!(
        first.model_settings().unwrap().key_account,
        other.model_settings().unwrap().key_account
    );
    let again = OrganizeService::start_from(os(&a), os(home.path()));
    let StorageStatus::Blocked { error } = again.get_app_state().unwrap().storage else {
        panic!("a second instance on the same folder must be blocked");
    };
    assert_eq!(error.code, ErrorCode::AnotherInstanceRunning);
}

#[test]
fn a_test_folder_keeps_its_records_across_a_restart() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let folder = tests.path().join("appdata");
    let root = tempfile::Builder::new()
        .prefix("traeva-root-")
        .tempdir()
        .unwrap();
    let first = OrganizeService::start_from(os(&folder), os(home.path()));
    let root_id = first.authorize_root(root.path()).unwrap().root_id;
    drop(first);
    let again = OrganizeService::start_from(os(&folder), os(home.path()));
    let roots = again.list_roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].root_id, root_id);
    assert!(!normal_dir(&home).exists());
}

// ---- Regressions found in review (#65) ----

#[test]
fn the_normal_folder_written_through_a_firmlink_is_refused() {
    // /System/Volumes/Data/... is the same folder as /private/... or
    // /Users/..., but the text differs and canonicalize keeps it.
    let firmlink_root = Path::new("/System/Volumes/Data");
    if !firmlink_root.exists() {
        return;
    }
    let home = home();
    let normal = normal_dir(&home);
    fs::create_dir_all(&normal).unwrap();
    fs::write(normal.join(DATABASE_FILE_NAME), b"the real database").unwrap();
    let real = fs::canonicalize(&normal).unwrap();
    let alias = firmlink_root.join(real.strip_prefix("/").unwrap());
    assert!(
        alias.join(DATABASE_FILE_NAME).exists(),
        "{}",
        alias.display()
    );
    let message = refusal(os(&alias), &home);
    assert!(message.contains(INSIDE), "{message}");

    let service = OrganizeService::start_from(os(&alias), os(home.path()));
    assert!(matches!(
        service.get_app_state().unwrap().storage,
        StorageStatus::Blocked { .. }
    ));
    assert_eq!(
        fs::read(normal.join(DATABASE_FILE_NAME)).unwrap(),
        b"the real database"
    );
    assert_eq!(
        fs::read_dir(&normal).unwrap().count(),
        1,
        "nothing was added"
    );
}

#[test]
fn another_spelling_of_the_missing_normal_folder_is_refused_and_not_created() {
    let home = home();
    let support = home.path().join("Library/Application Support");
    fs::create_dir_all(&support).unwrap();
    for spelling in ["traeva", "TRAEVA/inner", "Traeva"] {
        let path = support.join(spelling);
        let message = refusal(os(&path), &home);
        assert!(message.contains(INSIDE), "{spelling}: {message}");
        let service = OrganizeService::start_from(os(&path), os(home.path()));
        assert!(matches!(
            service.get_app_state().unwrap().storage,
            StorageStatus::Blocked { .. }
        ));
    }
    assert_eq!(
        fs::read_dir(&support).unwrap().count(),
        0,
        "the normal folder was not created under any spelling"
    );
}

#[test]
fn a_personal_folder_with_a_traeva_name_in_it_is_still_refused() {
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let personal = tests.path().join("Documents");
    fs::create_dir(&personal).unwrap();
    fs::write(personal.join("essay.txt"), b"mine").unwrap();
    fs::write(personal.join(LOCK_FILE_NAME), b"").unwrap();
    assert!(refusal(os(&personal), &home).contains("other files"));

    // Only Traeva's own names (and Finder's .DS_Store): a folder it used.
    let used = tests.path().join("used");
    fs::create_dir_all(used.join("snapshots/ab")).unwrap();
    fs::create_dir(used.join("diagnostics")).unwrap();
    // Copies of old versions made to open them (#35).
    fs::create_dir_all(used.join("opened/0d9c2f7e-4b1a-4c3e-9f50-2a6b8c1d3e4f")).unwrap();
    for name in [
        DATABASE_FILE_NAME,
        LOCK_FILE_NAME,
        "model-settings.json",
        ".DS_Store",
    ] {
        fs::write(used.join(name), b"").unwrap();
    }
    assert!(matches!(
        choose(os(&used), os(home.path())).unwrap(),
        Chosen::Test(_)
    ));
}

#[test]
fn a_test_instance_never_touches_the_normal_key() {
    // The test instance's key store stands for the shared Keychain: the
    // normal key is already there.
    let home = home();
    let tests = tempfile::tempdir().unwrap();
    let service = OrganizeService::start_from(os(&tests.path().join("appdata")), os(home.path()));
    let normal_key = format!("sk-normal-{}", "n".repeat(40));
    service
        .model
        .keys
        .save(PROVIDER_OPENAI, &normal_key)
        .unwrap();
    let test_key = format!("sk-test-{}", "t".repeat(40));
    service
        .save_model_settings(&ModelSettingsInput {
            provider: PROVIDER_OPENAI.to_string(),
            model: DEFAULT_MODEL.to_string(),
            api_key: Some(test_key.clone()),
        })
        .unwrap();
    service.set_analysis_consent(true).unwrap();
    // A request would send the test folder's key.
    assert_eq!(service.sending_setup(None).unwrap().api_key, test_key);
    service.remove_api_key().unwrap();
    assert_eq!(
        service.model.keys.read(PROVIDER_OPENAI).unwrap(),
        Some(normal_key.clone()),
        "the normal key is unchanged"
    );

    // A blocked instance cannot reach any key.
    let blocked = OrganizeService::start_from(Some("relative".into()), os(home.path()));
    blocked
        .model
        .keys
        .save(PROVIDER_OPENAI, &normal_key)
        .unwrap();
    assert!(
        blocked
            .save_model_settings(&ModelSettingsInput {
                provider: PROVIDER_OPENAI.to_string(),
                model: DEFAULT_MODEL.to_string(),
                api_key: Some(test_key),
            })
            .is_err()
    );
    assert!(blocked.remove_api_key().is_err());
    assert_eq!(
        blocked.model.keys.read(PROVIDER_OPENAI).unwrap(),
        Some(normal_key)
    );
}

#[test]
fn a_colliding_folder_blocks_the_start_and_creates_nothing() {
    let home = home();
    let normal = normal_dir(&home);
    fs::create_dir_all(&normal).unwrap();
    let inside = normal.join("tests/appdata");
    let service = OrganizeService::start_from(os(&inside), os(home.path()));
    let StorageStatus::Blocked { error } = service.get_app_state().unwrap().storage else {
        panic!("expected blocked storage");
    };
    assert_eq!(error.code, ErrorCode::AppDataUnavailable);
    assert_eq!(
        fs::read_dir(&normal).unwrap().count(),
        0,
        "nothing was created"
    );
}
