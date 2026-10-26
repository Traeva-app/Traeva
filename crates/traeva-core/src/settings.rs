//! Model settings, the API key and the consent to send (organize-spec.md §5,
//! §9 `save_model_settings` / `set_analysis_consent`; #14).
//!
//! Three things decide whether Traeva may send a request:
//!
//! 1. Settings: the provider and the model. There is one provider, OpenAI.
//!    They are kept, with nothing secret, in `model-settings.json` in app
//!    data (#14 adds no database table).
//! 2. The API key: kept only in the macOS Keychain, as a generic password
//!    (service `io.github.p30g53.traeva`, account = the provider; a test
//!    app-data folder adds `@test-…` to the account, #65). It never goes
//!    into the settings file, the database, an error message or a log.
//! 3. Consent: the user agreed to the notice for exactly this provider and
//!    model. Choosing another provider or model is a service change: it
//!    clears the consent, and nothing is sent until the user agrees again.
//!
//! Every change to the provider, model, key, Organize's consent or image
//! sending raises `generation`. A request that comes back after such a
//! change is dropped (model.rs). Find's own agreement does not raise it.
//!
//! Memory's analysis (#33) uses only 1 and 2 (`send_only_setup`): it asks
//! for its own agreement instead of 3, at the last moment, and checks its
//! late answers itself (`OrganizeService::send_only`, shared-contract.md
//! §4.6). Find (#198) does the same with its own agreement, kept here as
//! `find_consent` (find.rs).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode, ModelStatus};

/// The one provider Traeva talks to (organize-spec.md §5).
pub const PROVIDER_OPENAI: &str = "openai";
/// The model used unless the settings name another one.
pub const DEFAULT_MODEL: &str = "gpt-6-luna";
/// The Keychain service of the API key; the account is the provider (plus
/// `@test-…` for a test app-data folder, #65).
pub const KEYCHAIN_SERVICE: &str = "io.github.p30g53.traeva";
/// Raise this when the notice text changes: earlier agreements stop counting.
/// 3: the excerpt may come from a PDF's text layer (organize-spec.md §14.4).
/// 4: or from text recognized on this Mac from a scanned PDF's pages (§14.8).
/// 5: or from a JPEG or PNG image; never the image itself (§14.9).
/// 6: the image itself only when sending images is turned on (§14.10), and
/// up to 3 questions with their answers (§15).
/// 7: or from a Word, PowerPoint or Excel file (organize-spec v0.16, the
/// student's decision 10/17).
pub const NOTICE_VERSION: u32 = 7;
/// Raise this when the image notice text changes (§14.10).
pub const IMAGE_NOTICE_VERSION: u32 = 1;

const SETTINGS_FILE_NAME: &str = "model-settings.json";
const DIAGNOSTICS_DIR_NAME: &str = "diagnostics";
/// The Keychain's "no such item" (errSecItemNotFound).
const KEYCHAIN_NOT_FOUND: i32 = -25300;

/// What `model-settings.json` holds. Unknown fields are refused, so a file
/// written by a newer Traeva is not half understood.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSettings {
    pub provider: String,
    pub model: String,
    /// The user's agreement; it counts only for the same provider, model
    /// and notice version.
    pub consent: Option<Consent>,
    /// Find's agreement to send the question (#198, find.rs): separate from
    /// Organize's, and counted the same way. Absent in files written before
    /// Find.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find_consent: Option<Consent>,
    /// Extra diagnostic records, off unless the user turns them on.
    pub diagnostics: bool,
    /// Raised by every change to the provider, model, key, Organize's
    /// consent or image sending (not by Find's agreement or diagnostics).
    pub generation: u64,
    /// Sending a smaller copy of an image to the model (§14.10): absent
    /// means off. Counted like `consent`, and only while that counts too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_consent: Option<ImageConsent>,
}

/// Whether Organize may send the model a smaller copy of a JPEG or PNG
/// image, or of a scanned PDF's first pages (organize-spec.md §14.10, the
/// student's decision of 10/14). Off unless the user turns it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSending {
    Off,
    /// The user confirms each image before it is sent.
    AskEachTime,
    /// The user chose "Don't ask again".
    WithoutAsking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageConsent {
    pub provider: String,
    pub model: String,
    pub notice_version: u32,
    pub mode: ImageSending,
    pub agreed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Consent {
    pub provider: String,
    pub model: String,
    pub notice_version: u32,
    pub agreed_at_ms: i64,
}

impl ModelSettings {
    fn consent_counts(&self) -> bool {
        self.consent.as_ref().is_some_and(|consent| {
            consent.provider == self.provider
                && consent.model == self.model
                && consent.notice_version == NOTICE_VERSION
        })
    }

    /// How images may be sent now: off unless the image agreement is for
    /// this provider, model and image notice, and Organize's own agreement
    /// counts too.
    pub(crate) fn image_sending(&self) -> ImageSending {
        match &self.image_consent {
            Some(image)
                if self.consent_counts()
                    && image.provider == self.provider
                    && image.model == self.model
                    && image.notice_version == IMAGE_NOTICE_VERSION =>
            {
                image.mode
            }
            _ => ImageSending::Off,
        }
    }

    /// Find's agreement counts for this provider, model and notice version.
    pub(crate) fn find_consent_counts(&self, notice_version: u32) -> bool {
        self.find_consent.as_ref().is_some_and(|consent| {
            consent.provider == self.provider
                && consent.model == self.model
                && consent.notice_version == notice_version
        })
    }
}

/// What a settings screen shows. Never contains the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelSettingsView {
    pub status: ModelStatus,
    /// Null until settings were saved.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Whether the Keychain holds a key for the provider (checked without
    /// reading the key).
    pub key_saved: bool,
    /// The Keychain account the key is kept under (service
    /// `io.github.p30g53.traeva`): the provider, or `openai@test-…` when a
    /// test app-data folder is in use (#65).
    pub key_account: String,
    /// When the user agreed to send, if that agreement still counts.
    pub consent_agreed_at_ms: Option<i64>,
    pub diagnostics: bool,
    /// What will be sent, shown before the user agrees.
    pub notice: String,
    /// Whether images may be sent now (§14.10), and what that sends.
    pub image_sending: ImageSending,
    pub image_notice: String,
}

/// The provider and model to use, and optionally a new API key. Unknown
/// fields are refused. The key goes to the Keychain and nowhere else.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettingsInput {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Written by hand so the key can never end up in a log or a test failure.
impl std::fmt::Debug for ModelSettingsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSettingsInput")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

/// Everything a request needs, once the setup allows sending.
pub(crate) struct SendingSetup {
    pub provider: String,
    pub model: String,
    pub generation: u64,
    pub diagnostics: bool,
    /// How images may be sent (§14.10), read together with `generation`, so
    /// the last check before sending also covers the choice of images.
    pub image_sending: ImageSending,
    pub api_key: String,
}

impl std::fmt::Debug for SendingSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendingSetup")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("generation", &self.generation)
            .field("image_sending", &self.image_sending)
            .field("api_key", &"<hidden>")
            .finish_non_exhaustive()
    }
}

/// What turning on sending images means (§14.10), shown next to the choice.
pub(crate) fn image_notice(provider: &str, model: &str) -> String {
    let name = if provider == PROVIDER_OPENAI {
        "OpenAI"
    } else {
        provider
    };
    format!(
        "When this is on, Traeva may also send {name} (model {model}) a smaller copy of a JPEG \
         or PNG image, or of a scanned PDF's first 3 pages, so the model can look at it. \
         Traeva makes the copy on this Mac, at most 1568 pixels on its longest side, without \
         the file's location, camera or other data. An image can show faces, documents, screens \
         or places: send only what you are fine with {name} seeing. With \"Ask each time\", \
         Traeva asks before every image; a dropped file that would need asking goes with its \
         recognized text only. You can turn this off any time; no image is sent after that."
    )
}

/// What the user agrees to before anything is sent (organize-spec.md §5).
pub fn consent_notice(provider: &str, model: &str) -> String {
    let name = if provider == PROVIDER_OPENAI {
        "OpenAI"
    } else {
        provider
    };
    format!(
        "When you ask Traeva to suggest a folder, it sends this to {name} (model {model}): the \
         file's name, a limited excerpt of its text (from a .txt, Markdown, Word, PowerPoint \
         or Excel file, the text layer of a PDF, or text recognized on this Mac from a scanned PDF's pages or from a \
         JPEG or PNG image; the image itself only if you turn on sending images below, and \
         then as a smaller copy without its location or camera data), the \
         folder it is in now and the folders it \
         could go into (paths inside the folders you authorized, never the path of your home \
         folder), and, if Traeva asks you questions (up to 3), each question and your answer. For short files, the excerpt can be the whole \
         text. For a Word document, the text is the document's body, including text Word \
         hides (not its tables, headers and footers, footnotes, comments or text boxes); for a \
         PowerPoint deck, the visible slides with their speaker notes, and lines saying which \
         slide and part each line is on; for an Excel workbook, the cells of the visible sheets \
         with their addresses: the values, the formulas as written and the value each formula \
         last saved, but not the sheets' names. Hidden slides and sheets are not read. Traeva \
         does not keep the text after the request, unless you turn on diagnostics. \
         {name} receives the text and handles it under its own data policy; Traeva asks it not \
         to store the answer. Your API key is used only to sign in to {name}."
    )
}

impl OrganizeService {
    /// The current model setup, for a settings screen.
    pub fn model_settings(&self) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        let settings = read_settings(&dir)?;
        self.settings_view(settings.as_ref())
    }

    /// `ModelStatus` for `get_app_state`. Without saved settings the
    /// Keychain is not even looked at.
    pub(crate) fn model_status(&self) -> ModelStatus {
        self.model_settings()
            .map(|view| view.status)
            .unwrap_or(ModelStatus::NotConfigured)
    }

    /// Saves the provider and model, and the API key if one is given.
    /// Choosing another provider or model clears the consent.
    pub fn save_model_settings(
        &self,
        input: &ModelSettingsInput,
    ) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        check_provider(&input.provider)?;
        check_model(&input.model)?;
        if let Some(key) = &input.api_key {
            check_key(key)?;
        }
        let _one_at_a_time = lock(&self.model.settings_lock);
        let old = read_settings(&dir).unwrap_or(None);
        let mut settings = old.clone().unwrap_or(ModelSettings {
            provider: input.provider.clone(),
            model: input.model.clone(),
            consent: None,
            find_consent: None,
            diagnostics: false,
            generation: 0,
            image_consent: None,
        });
        let service_changed = old
            .as_ref()
            .is_none_or(|old| old.provider != input.provider || old.model != input.model);
        if service_changed {
            settings.provider = input.provider.clone();
            settings.model = input.model.clone();
            settings.consent = None;
            settings.find_consent = None;
            settings.image_consent = None;
            settings.generation += 1;
        }
        if let Some(key) = &input.api_key {
            self.model
                .keys
                .save(&self.model.key_account(&input.provider), key)?;
            settings.generation += 1;
        }
        if service_changed {
            // #201: the switch for keeping versions ends with the service,
            // and so do the per-file agreements it gave. The database first:
            // if the settings cannot be written, the switch is off, never on.
            crate::auto_memory::end_with_the_service(self.store()?, self.now())?;
        }
        write_settings(&dir, &settings)?;
        self.settings_view(Some(&settings))
    }

    /// Agrees to (or withdraws from) sending to the saved provider and model.
    pub fn set_analysis_consent(&self, agree: bool) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        let _one_at_a_time = lock(&self.model.settings_lock);
        let mut settings = read_settings(&dir)?.ok_or_else(|| {
            AppError::analysis(
                ErrorCode::ModelNotConfigured,
                "Choose the provider and model before agreeing to send.",
                false,
                None,
            )
        })?;
        settings.consent = agree.then(|| Consent {
            provider: settings.provider.clone(),
            model: settings.model.clone(),
            notice_version: NOTICE_VERSION,
            agreed_at_ms: self.now(),
        });
        settings.generation += 1;
        write_settings(&dir, &settings)?;
        self.settings_view(Some(&settings))
    }

    /// Turns sending images on (asking each time, or without asking) or off
    /// (§14.10). Turning it on needs saved settings; it counts only while
    /// Organize's own agreement counts. Off takes effect at once: no image
    /// is sent after this returns.
    pub fn set_image_sending(&self, mode: ImageSending) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        let _one_at_a_time = lock(&self.model.settings_lock);
        let mut settings = read_settings(&dir)?.ok_or_else(|| {
            AppError::analysis(
                ErrorCode::ModelNotConfigured,
                "Choose the provider and model first.",
                false,
                None,
            )
        })?;
        settings.image_consent = (mode != ImageSending::Off).then(|| ImageConsent {
            provider: settings.provider.clone(),
            model: settings.model.clone(),
            notice_version: IMAGE_NOTICE_VERSION,
            mode,
            agreed_at_ms: self.now(),
        });
        settings.generation += 1;
        write_settings(&dir, &settings)?;
        self.settings_view(Some(&settings))
    }

    /// How images may be sent right now (§14.10); off when anything is
    /// missing or cannot be read.
    pub(crate) fn image_sending_now(&self) -> ImageSending {
        self.app_data_dir()
            .ok()
            .and_then(|dir| read_settings(&dir).ok().flatten())
            .map_or(ImageSending::Off, |settings| settings.image_sending())
    }

    /// Removes the saved API key from the Keychain.
    pub fn remove_api_key(&self) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        let _one_at_a_time = lock(&self.model.settings_lock);
        let mut settings = read_settings(&dir)?;
        let provider = settings
            .as_ref()
            .map_or(PROVIDER_OPENAI.to_string(), |s| s.provider.clone());
        self.model.keys.remove(&self.model.key_account(&provider))?;
        if let Some(settings) = settings.as_mut() {
            settings.generation += 1;
            write_settings(&dir, settings)?;
        }
        self.settings_view(settings.as_ref())
    }

    /// Turns the diagnostic records on or off. They are the only place
    /// where request and answer bodies may be written, and only while on.
    pub fn set_model_diagnostics(&self, on: bool) -> Result<ModelSettingsView, AppError> {
        let dir = self.app_data_dir()?;
        let _one_at_a_time = lock(&self.model.settings_lock);
        let mut settings = read_settings(&dir)?.ok_or_else(|| {
            AppError::analysis(
                ErrorCode::ModelNotConfigured,
                "Choose the provider and model first.",
                false,
                None,
            )
        })?;
        settings.diagnostics = on;
        write_settings(&dir, &settings)?;
        self.settings_view(Some(&settings))
    }

    /// Deletes every diagnostic record. Returns how many were deleted.
    pub fn clear_model_diagnostics(&self) -> Result<usize, AppError> {
        let dir = diagnostics_dir(&self.app_data_dir()?);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err(app_data_error("read the diagnostic records")),
        };
        let mut removed = 0;
        for entry in entries {
            let path = entry
                .map_err(|_| app_data_error("read the diagnostic records"))?
                .path();
            if path.extension().is_some_and(|ext| ext == "json") {
                fs::remove_file(&path).map_err(|_| app_data_error("delete a diagnostic record"))?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Settings, consent and key, checked in that order. A refused request
    /// sends nothing and saves nothing.
    pub(crate) fn sending_setup(&self, file_id: Option<&str>) -> Result<SendingSetup, AppError> {
        self.read_sending_setup(file_id, true)
    }

    /// Settings and key for Memory's send path (`send_only`, #33). Organize's
    /// agreement to send does not count there: Memory checks its own
    /// agreement at the last moment instead.
    pub(crate) fn send_only_setup(&self) -> Result<SendingSetup, AppError> {
        self.read_sending_setup(None, false)
    }

    fn read_sending_setup(
        &self,
        file_id: Option<&str>,
        organize_consent: bool,
    ) -> Result<SendingSetup, AppError> {
        let refuse = |code, message: &str| AppError::analysis(code, message, false, file_id);
        let settings = read_settings(&self.app_data_dir()?)?.ok_or_else(|| {
            refuse(
                ErrorCode::ModelNotConfigured,
                "No model is set up. Choose the provider and model and save an API key, or pick \
                 the folder by hand.",
            )
        })?;
        if organize_consent && !settings.consent_counts() {
            return Err(refuse(
                ErrorCode::ConsentRequired,
                "Sending to the model is not turned on for this provider and model. Nothing was \
                 sent.",
            ));
        }
        let account = self.model.key_account(&settings.provider);
        let api_key = self.model.keys.read(&account)?.ok_or_else(|| {
            refuse(
                ErrorCode::ModelNotConfigured,
                "No API key is saved for this provider. Nothing was sent.",
            )
        })?;
        // The Keychain item can also be written outside Traeva (`security`),
        // so the key is checked here too: an empty or broken key is never sent.
        if check_key(&api_key).is_err() {
            return Err(AppError::analysis(
                ErrorCode::KeyUnavailable,
                "The API key saved in the Keychain is not a valid key (empty, too short, or with \
                 spaces or line breaks). Save it again. Nothing was sent.",
                false,
                file_id,
            ));
        }
        Ok(SendingSetup {
            image_sending: settings.image_sending(),
            provider: settings.provider,
            model: settings.model,
            generation: settings.generation,
            diagnostics: settings.diagnostics,
            api_key,
        })
    }

    /// The current `generation`, to tell whether the setup changed while a
    /// request was waiting.
    pub(crate) fn settings_generation(&self) -> Result<Option<u64>, AppError> {
        Ok(read_settings(&self.app_data_dir()?)?.map(|s| s.generation))
    }

    fn settings_view(
        &self,
        settings: Option<&ModelSettings>,
    ) -> Result<ModelSettingsView, AppError> {
        let Some(settings) = settings else {
            return Ok(ModelSettingsView {
                status: ModelStatus::NotConfigured,
                provider: None,
                model: None,
                key_saved: false,
                key_account: self.model.key_account(PROVIDER_OPENAI),
                consent_agreed_at_ms: None,
                diagnostics: false,
                notice: consent_notice(PROVIDER_OPENAI, DEFAULT_MODEL),
                image_sending: ImageSending::Off,
                image_notice: image_notice(PROVIDER_OPENAI, DEFAULT_MODEL),
            });
        };
        let key_account = self.model.key_account(&settings.provider);
        let key_saved = self.model.keys.exists(&key_account)?;
        let consent_counts = settings.consent_counts();
        let status = match (key_saved, consent_counts) {
            (false, _) => ModelStatus::NotConfigured,
            (true, false) => ModelStatus::ConsentRequired,
            (true, true) => ModelStatus::Configured,
        };
        Ok(ModelSettingsView {
            status,
            provider: Some(settings.provider.clone()),
            model: Some(settings.model.clone()),
            key_saved,
            key_account,
            consent_agreed_at_ms: settings
                .consent
                .as_ref()
                .filter(|_| consent_counts)
                .map(|consent| consent.agreed_at_ms),
            diagnostics: settings.diagnostics,
            notice: consent_notice(&settings.provider, &settings.model),
            image_sending: settings.image_sending(),
            image_notice: image_notice(&settings.provider, &settings.model),
        })
    }

    pub(crate) fn app_data_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.store()?.app_data_dir().to_path_buf())
    }
}

pub(crate) fn diagnostics_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DIAGNOSTICS_DIR_NAME)
}

/// None when no settings were saved yet. A file that cannot be read or
/// understood is reported, not replaced: saving settings again writes a
/// new one.
pub(crate) fn read_settings(app_data_dir: &Path) -> Result<Option<ModelSettings>, AppError> {
    let unreadable = || {
        AppError::analysis(
            ErrorCode::ModelNotConfigured,
            format!(
                "The model settings in {} cannot be read. Save the settings again.",
                app_data_dir.join(SETTINGS_FILE_NAME).display()
            ),
            false,
            None,
        )
    };
    let bytes = match fs::read(app_data_dir.join(SETTINGS_FILE_NAME)) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unreadable()),
    };
    let settings: ModelSettings = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    if check_provider(&settings.provider).is_err() || check_model(&settings.model).is_err() {
        return Err(unreadable());
    }
    Ok(Some(settings))
}

/// Writes a new file next to the old one, then renames it over the old one,
/// so a crash leaves either the old or the new settings, never half a file.
pub(crate) fn write_settings(
    app_data_dir: &Path,
    settings: &ModelSettings,
) -> Result<(), AppError> {
    let fail = || app_data_error("save the model settings");
    let bytes = serde_json::to_vec_pretty(settings).map_err(|_| fail())?;
    let temporary = app_data_dir.join(format!("{SETTINGS_FILE_NAME}.new"));
    let mut file = File::create(&temporary).map_err(|_| fail())?;
    file.write_all(&bytes).map_err(|_| fail())?;
    file.sync_all().map_err(|_| fail())?;
    fs::rename(&temporary, app_data_dir.join(SETTINGS_FILE_NAME)).map_err(|_| fail())?;
    File::open(app_data_dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| fail())
}

fn app_data_error(what: &str) -> AppError {
    AppError::analysis(
        ErrorCode::AppDataUnavailable,
        format!("Traeva could not {what} in its app-data folder."),
        true,
        None,
    )
}

fn check_provider(provider: &str) -> Result<(), AppError> {
    if provider == PROVIDER_OPENAI {
        Ok(())
    } else {
        Err(invalid("Traeva supports one provider: openai."))
    }
}

/// Model names look like `gpt-6-luna`: lower-case letters, digits, `.`,
/// `-` and `_`, at most 64 characters.
fn check_model(model: &str) -> Result<(), AppError> {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c);
    if !model.is_empty()
        && model.len() <= 64
        && model.starts_with(|c: char| c.is_ascii_alphanumeric())
        && model.chars().all(allowed)
    {
        Ok(())
    } else {
        Err(invalid(
            "The model name must be like gpt-6-luna: lower-case letters, digits, '.', '-' and \
             '_', at most 64 characters.",
        ))
    }
}

/// One line of visible ASCII characters, no spaces. The message never
/// repeats the key.
fn check_key(key: &str) -> Result<(), AppError> {
    if (20..=512).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic()) {
        Ok(())
    } else {
        Err(invalid(
            "The API key must be one line of 20 to 512 letters, digits and symbols, without \
             spaces.",
        ))
    }
}

fn invalid(message: &str) -> AppError {
    AppError::analysis(ErrorCode::InvalidInput, message, false, None)
}

pub(crate) fn lock(mutex: &std::sync::Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where API keys are kept: the login Keychain in the app. The crate's own
/// tests keep them in memory, so no test reads or writes the real Keychain.
pub(crate) enum KeyStore {
    Keychain,
    #[cfg(test)]
    Memory(Mutex<HashMap<String, String>>),
}

impl KeyStore {
    #[cfg(test)]
    pub(crate) fn memory() -> KeyStore {
        KeyStore::Memory(Mutex::new(HashMap::new()))
    }

    /// The key, or None when none is saved. Reading the key itself can make
    /// macOS ask the user to allow it.
    pub(crate) fn read(&self, account: &str) -> Result<Option<String>, AppError> {
        match self {
            KeyStore::Keychain => {
                match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account)
                {
                    Ok(bytes) => String::from_utf8(bytes)
                        .map(Some)
                        .map_err(|_| keychain_error("read", None)),
                    Err(err) if err.code() == KEYCHAIN_NOT_FOUND => Ok(None),
                    Err(err) => Err(keychain_error("read", Some(err.code()))),
                }
            }
            #[cfg(test)]
            KeyStore::Memory(keys) => Ok(lock_keys(keys).get(account).cloned()),
        }
    }

    /// Whether a key is saved, looking only at the item's attributes, so
    /// macOS does not ask to allow reading the key.
    pub(crate) fn exists(&self, account: &str) -> Result<bool, AppError> {
        match self {
            KeyStore::Keychain => {
                use security_framework::item::{ItemClass, ItemSearchOptions};
                let found = ItemSearchOptions::new()
                    .class(ItemClass::generic_password())
                    .service(KEYCHAIN_SERVICE)
                    .account(account)
                    .load_attributes(true)
                    .search();
                match found {
                    Ok(items) => Ok(!items.is_empty()),
                    Err(err) if err.code() == KEYCHAIN_NOT_FOUND => Ok(false),
                    Err(err) => Err(keychain_error("check", Some(err.code()))),
                }
            }
            #[cfg(test)]
            KeyStore::Memory(keys) => Ok(lock_keys(keys).contains_key(account)),
        }
    }

    pub(crate) fn save(&self, account: &str, key: &str) -> Result<(), AppError> {
        match self {
            KeyStore::Keychain => security_framework::passwords::set_generic_password(
                KEYCHAIN_SERVICE,
                account,
                key.as_bytes(),
            )
            .map_err(|err| keychain_error("save", Some(err.code()))),
            #[cfg(test)]
            KeyStore::Memory(keys) => {
                lock_keys(keys).insert(account.to_string(), key.to_string());
                Ok(())
            }
        }
    }

    /// Removing a key that is not there is not an error.
    pub(crate) fn remove(&self, account: &str) -> Result<(), AppError> {
        match self {
            KeyStore::Keychain => {
                match security_framework::passwords::delete_generic_password(
                    KEYCHAIN_SERVICE,
                    account,
                ) {
                    Ok(()) => Ok(()),
                    Err(err) if err.code() == KEYCHAIN_NOT_FOUND => Ok(()),
                    Err(err) => Err(keychain_error("remove", Some(err.code()))),
                }
            }
            #[cfg(test)]
            KeyStore::Memory(keys) => {
                lock_keys(keys).remove(account);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
fn lock_keys(
    keys: &Mutex<HashMap<String, String>>,
) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
    keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The macOS status code helps when looking it up; the key is never shown.
fn keychain_error(what: &str, code: Option<i32>) -> AppError {
    let code = code.map_or(String::new(), |code| format!(" (macOS code {code})"));
    AppError::analysis(
        ErrorCode::KeyUnavailable,
        format!(
            "Traeva could not {what} the API key in the Keychain{code}. If macOS asked for \
             permission, allow it and try again."
        ),
        true,
        None,
    )
}

#[cfg(test)]
#[path = "settings_tests.rs"]
pub(crate) mod tests;
