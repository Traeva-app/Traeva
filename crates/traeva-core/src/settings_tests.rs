//! Tests for settings.rs. Synthetic data only; API keys stay in memory
//! (the crate's own tests never use the real Keychain).

use std::fs;

use super::*;
use crate::test_fixture::Fixture;

/// Looks like a key, but is not one.
pub(crate) const TEST_KEY: &str = "sk-test-KEYCANARY-0123456789abcdef";

pub(crate) fn openai(model: &str, key: Option<&str>) -> ModelSettingsInput {
    ModelSettingsInput {
        provider: PROVIDER_OPENAI.to_string(),
        model: model.to_string(),
        api_key: key.map(str::to_string),
    }
}

fn settings_file(f: &Fixture) -> std::path::PathBuf {
    f.app.path().join("Traeva").join(SETTINGS_FILE_NAME)
}

fn generation(f: &Fixture) -> Option<u64> {
    f.service().settings_generation().unwrap()
}

#[test]
fn without_settings_nothing_is_configured_and_the_notice_says_what_is_sent() {
    let f = Fixture::new();
    let view = f.service().model_settings().unwrap();
    assert_eq!(view.status, ModelStatus::NotConfigured);
    assert_eq!(view.provider, None);
    assert!(!view.key_saved);
    assert!(!view.diagnostics);
    for part in [
        "OpenAI",
        "gpt-6-luna",
        "the file's name",
        "a limited excerpt of its text",
        "the text layer of a PDF",
        "text recognized on this Mac from a scanned PDF's pages",
        "from a JPEG or PNG image",
        "the image itself only if you turn on sending images below",
        "without its location or camera data",
        "For short files, the excerpt can be the whole text.",
        "the folder it is in now",
        "never the path of your home folder",
        "each question and your answer",
    ] {
        assert!(view.notice.contains(part), "the notice should say: {part}");
    }
    assert_eq!(
        f.service().get_app_state().unwrap().model,
        ModelStatus::NotConfigured
    );
    assert!(!settings_file(&f).exists());
}

#[test]
fn settings_a_key_and_consent_together_allow_sending() {
    let f = Fixture::new();
    let service = f.service();
    let view = service
        .save_model_settings(&openai(DEFAULT_MODEL, None))
        .unwrap();
    assert_eq!(view.status, ModelStatus::NotConfigured, "no key yet");
    assert_eq!(view.model.as_deref(), Some("gpt-6-luna"));

    let view = service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    assert_eq!(view.status, ModelStatus::ConsentRequired);
    assert!(view.key_saved);
    assert_eq!(view.consent_agreed_at_ms, None);

    let view = service.set_analysis_consent(true).unwrap();
    assert_eq!(view.status, ModelStatus::Configured);
    assert_eq!(
        view.consent_agreed_at_ms,
        Some(crate::test_fixture::START_MS)
    );
    assert_eq!(
        service.get_app_state().unwrap().model,
        ModelStatus::Configured
    );

    let setup = service.sending_setup(None).unwrap();
    assert_eq!(
        (setup.provider.as_str(), setup.model.as_str()),
        ("openai", "gpt-6-luna")
    );
    assert_eq!(setup.api_key, TEST_KEY);
}

#[test]
fn a_service_change_clears_the_consent() {
    let f = Fixture::new();
    let service = f.service();
    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    service.set_analysis_consent(true).unwrap();
    let before = generation(&f).unwrap();

    // Saving the same provider and model again changes nothing.
    let same = service
        .save_model_settings(&openai(DEFAULT_MODEL, None))
        .unwrap();
    assert_eq!(same.status, ModelStatus::Configured);
    assert_eq!(generation(&f), Some(before));

    let other = service
        .save_model_settings(&openai("gpt-5.6-luna", None))
        .unwrap();
    assert_eq!(other.status, ModelStatus::ConsentRequired);
    assert_eq!(other.consent_agreed_at_ms, None);
    assert!(generation(&f).unwrap() > before);
    assert_eq!(
        service.sending_setup(None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );

    // Switching back is a service change too: the old agreement is gone.
    let back = service
        .save_model_settings(&openai(DEFAULT_MODEL, None))
        .unwrap();
    assert_eq!(back.status, ModelStatus::ConsentRequired);
    assert_eq!(
        service.sending_setup(None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
}

#[test]
fn a_broken_key_saved_outside_traeva_is_never_sent() {
    let f = Fixture::new();
    let service = f.service();
    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    service.set_analysis_consent(true).unwrap();
    // As if `security add-generic-password` had saved a bad paste.
    for broken in [
        "",
        "short",
        " sk-test-KEYCANARY-0123456789abcdef",
        "sk-test-KEYCANARY 0123456789abcdef",
        "sk-test-KEYCANARY-0123456789abcdef\n",
    ] {
        service.model.keys.save(PROVIDER_OPENAI, broken).unwrap();
        let err = service.sending_setup(None).unwrap_err();
        assert_eq!(err.code, ErrorCode::KeyUnavailable, "{broken:?}");
        assert!(!err.message.contains("KEYCANARY"));
    }
}

#[test]
fn withdrawing_consent_or_removing_the_key_stops_sending() {
    let f = Fixture::new();
    let service = f.service();
    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    service.set_analysis_consent(true).unwrap();

    let g = generation(&f).unwrap();
    let view = service.set_analysis_consent(false).unwrap();
    assert_eq!(view.status, ModelStatus::ConsentRequired);
    assert!(generation(&f).unwrap() > g);
    assert_eq!(
        service.sending_setup(None).unwrap_err().code,
        ErrorCode::ConsentRequired
    );

    service.set_analysis_consent(true).unwrap();
    let g = generation(&f).unwrap();
    let view = service.remove_api_key().unwrap();
    assert_eq!(view.status, ModelStatus::NotConfigured);
    assert!(!view.key_saved);
    assert!(generation(&f).unwrap() > g);
    assert_eq!(
        service.sending_setup(None).unwrap_err().code,
        ErrorCode::ModelNotConfigured
    );
    // Removing it again is not an error.
    service.remove_api_key().unwrap();
}

#[test]
fn consent_counts_only_for_the_same_provider_model_and_notice() {
    let f = Fixture::new();
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    // A file whose agreement was given for something else, for example
    // by an older Traeva with another notice.
    // Version 3 is the notice before on-device text recognition (§14.8),
    // version 4 the one before images (§14.9), version 5 the one before
    // sending images (§14.10).
    for (model, notice) in [
        ("gpt-5.6-luna", NOTICE_VERSION),
        (DEFAULT_MODEL, 0),
        (DEFAULT_MODEL, 3),
        (DEFAULT_MODEL, 4),
        (DEFAULT_MODEL, 5),
    ] {
        let settings = ModelSettings {
            provider: PROVIDER_OPENAI.to_string(),
            model: DEFAULT_MODEL.to_string(),
            consent: Some(Consent {
                provider: PROVIDER_OPENAI.to_string(),
                model: model.to_string(),
                notice_version: notice,
                agreed_at_ms: 1,
            }),
            find_consent: None,
            diagnostics: false,
            generation: 7,
            image_consent: None,
        };
        fs::write(settings_file(&f), serde_json::to_vec(&settings).unwrap()).unwrap();
        let view = f.service().model_settings().unwrap();
        assert_eq!(
            view.status,
            ModelStatus::ConsentRequired,
            "{model} {notice}"
        );
        assert_eq!(view.consent_agreed_at_ms, None);
        assert_eq!(
            f.service().sending_setup(None).unwrap_err().code,
            ErrorCode::ConsentRequired
        );
    }
}

#[test]
fn consent_needs_saved_settings() {
    let f = Fixture::new();
    let err = f.service().set_analysis_consent(true).unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelNotConfigured);
    assert!(!settings_file(&f).exists());
}

#[test]
fn only_openai_valid_model_names_and_one_line_keys_are_accepted() {
    let f = Fixture::new();
    let service = f.service();
    let refused = |input: &ModelSettingsInput| {
        let err = service.save_model_settings(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{input:?}");
        assert!(!err.message.contains("KEYCANARY"), "never repeat the key");
    };
    refused(&ModelSettingsInput {
        provider: "anthropic".to_string(),
        ..openai(DEFAULT_MODEL, None)
    });
    for model in ["", "GPT-6", "gpt 6", "-gpt", "gpt/6", &"g".repeat(65)] {
        refused(&openai(model, None));
    }
    for key in [
        "short",
        "sk-test KEYCANARY with spaces",
        "sk-test-KEYCANARY\n0123456789",
        &format!("sk-{}", "K".repeat(600)),
    ] {
        refused(&openai(DEFAULT_MODEL, Some(key)));
    }
    assert!(!settings_file(&f).exists(), "nothing was saved");
    assert!(!service.model.keys.exists(PROVIDER_OPENAI).unwrap());
}

#[test]
fn the_key_is_kept_out_of_the_settings_file_and_debug_output() {
    let f = Fixture::new();
    let input = openai(DEFAULT_MODEL, Some(TEST_KEY));
    assert!(!format!("{input:?}").contains("KEYCANARY"));
    f.service().save_model_settings(&input).unwrap();
    let text = fs::read_to_string(settings_file(&f)).unwrap();
    assert!(!text.contains("KEYCANARY"));
    assert!(text.contains("gpt-6-luna"));
}

#[test]
fn an_unreadable_settings_file_is_reported_not_replaced() {
    let f = Fixture::new();
    fs::write(settings_file(&f), b"{ not json").unwrap();
    let err = f.service().model_settings().unwrap_err();
    assert_eq!(err.code, ErrorCode::ModelNotConfigured);
    assert_eq!(
        f.service().get_app_state().unwrap().model,
        ModelStatus::NotConfigured
    );
    assert_eq!(fs::read(settings_file(&f)).unwrap(), b"{ not json");

    // Unknown fields are refused too (a file from a newer Traeva).
    fs::write(
        settings_file(&f),
        br#"{"provider":"openai","model":"gpt-6-luna","consent":null,"diagnostics":false,"generation":1,"extra":1}"#,
    )
    .unwrap();
    assert!(f.service().model_settings().is_err());

    // Saving the settings again writes a new, readable file.
    f.service()
        .save_model_settings(&openai(DEFAULT_MODEL, None))
        .unwrap();
    assert!(f.service().model_settings().is_ok());
}

#[test]
fn diagnostics_are_off_until_turned_on_and_can_be_cleared() {
    let f = Fixture::new();
    let service = f.service();
    assert_eq!(
        service.set_model_diagnostics(true).unwrap_err().code,
        ErrorCode::ModelNotConfigured
    );
    service
        .save_model_settings(&openai(DEFAULT_MODEL, None))
        .unwrap();
    assert!(!service.model_settings().unwrap().diagnostics);
    let g = generation(&f);
    assert!(service.set_model_diagnostics(true).unwrap().diagnostics);
    assert_eq!(generation(&f), g, "diagnostics are not a service change");

    let dir = diagnostics_dir(&f.app.path().join("Traeva"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.json"), b"{}").unwrap();
    fs::write(dir.join("b.json"), b"{}").unwrap();
    assert_eq!(service.clear_model_diagnostics().unwrap(), 2);
    assert_eq!(service.clear_model_diagnostics().unwrap(), 0);
}

#[test]
fn a_blocked_storage_refuses_every_settings_call() {
    let f = Fixture::new();
    // A second service on the same app data cannot take the process lock.
    let second = crate::OrganizeService::start(f.app.path().join("Traeva"));
    assert!(second.model_settings().is_err());
    assert!(
        second
            .save_model_settings(&openai(DEFAULT_MODEL, None))
            .is_err()
    );
    assert_eq!(
        second.get_app_state().unwrap().model,
        ModelStatus::NotConfigured
    );
}

#[test]
fn the_notice_text_belongs_to_its_version() {
    // A changed notice must raise NOTICE_VERSION, so agreements to the old
    // text stop counting. Then update the pair below.
    let text = consent_notice(PROVIDER_OPENAI, "{model}");
    assert_eq!(
        (NOTICE_VERSION, crate::observe::sha256_hex(text.as_bytes())),
        (
            7,
            String::from("50ad88d7352a74e0e40370d8d91ce4e339bd67a5c0551d94f630f230bb4b1fd3")
        )
    );
}

/// Sending images (§14.10): off by default; on only with saved settings and
/// while Organize's own agreement counts; off again after the service
/// changes or when the user turns it off.
#[test]
fn sending_images_is_off_unless_turned_on_and_follows_the_agreement() {
    let f = Fixture::new();
    let service = f.service();
    assert_eq!(
        service.model_settings().unwrap().image_sending,
        ImageSending::Off
    );
    assert!(
        service
            .set_image_sending(ImageSending::AskEachTime)
            .is_err()
    );

    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    // Turned on before agreeing to send: it does not count yet.
    let view = service
        .set_image_sending(ImageSending::AskEachTime)
        .unwrap();
    assert_eq!(view.image_sending, ImageSending::Off);
    assert!(view.image_notice.contains("smaller copy"));
    assert!(
        view.image_notice
            .contains("without the file's location, camera or other data")
    );

    let view = service.set_analysis_consent(true).unwrap();
    assert_eq!(view.image_sending, ImageSending::AskEachTime);
    assert_eq!(service.image_sending_now(), ImageSending::AskEachTime);
    let view = service
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();
    assert_eq!(view.image_sending, ImageSending::WithoutAsking);

    // Withdrawing Organize's agreement stops it too.
    service.set_analysis_consent(false).unwrap();
    assert_eq!(service.image_sending_now(), ImageSending::Off);
    service.set_analysis_consent(true).unwrap();
    assert_eq!(service.image_sending_now(), ImageSending::WithoutAsking);

    // Off takes effect at once.
    service.set_image_sending(ImageSending::Off).unwrap();
    assert_eq!(service.image_sending_now(), ImageSending::Off);

    // Another model ends it with the agreement.
    service
        .set_image_sending(ImageSending::AskEachTime)
        .unwrap();
    service
        .save_model_settings(&openai("gpt-5.6-luna", None))
        .unwrap();
    assert_eq!(service.image_sending_now(), ImageSending::Off);
}

#[test]
fn an_image_agreement_for_an_older_notice_does_not_count() {
    let f = Fixture::new();
    let service = f.service();
    service
        .save_model_settings(&openai(DEFAULT_MODEL, Some(TEST_KEY)))
        .unwrap();
    service.set_analysis_consent(true).unwrap();
    service
        .set_image_sending(ImageSending::WithoutAsking)
        .unwrap();
    let path = settings_file(&f);
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        file["image_consent"]["notice_version"],
        IMAGE_NOTICE_VERSION
    );
    file["image_consent"]["notice_version"] = serde_json::json!(0);
    fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    assert_eq!(service.image_sending_now(), ImageSending::Off);
}
