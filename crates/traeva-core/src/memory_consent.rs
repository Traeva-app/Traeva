//! Memory's agreement to send (#33): before Memory describes the changes in
//! a file with the model, the user agrees, for that one file, to the
//! notice below. The agreement covers the file's versions now and the ones
//! Memory saves later, until the user withdraws it.
//!
//! It is separate from Organize's agreement (settings.rs): agreeing to one
//! never turns on the other. Both use the same provider, model and API key,
//! so an agreement counts only while the saved settings still name the
//! provider and model it was given for, under this notice version. Like
//! Organize's, it does not come back when the settings return to that model:
//! the next background pass withdraws an agreement that no longer counts
//! (`withdraw_outdated_consents`).
//!
//! Withdrawing is final for that agreement, and it takes the settings lock,
//! the same lock the last check before a request goes out holds (changes.rs).
//! So once `set_memory_consent(file, false)` returns, no new request about
//! that file is decided. One that was decided just before (it passed the
//! last check) can still be on its way out; its answer is dropped. What was
//! already sent cannot be called back.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::OrganizeService;
use crate::contracts::{AppError, ErrorCode, ErrorStage};
use crate::settings::{self, PROVIDER_OPENAI};
use crate::snapshot::{check_file_id, unknown_file};
use crate::store::{Store, sqlite_error};

/// Raise this when the notice text changes: earlier agreements stop counting.
/// 2: Word documents (.docx, #66) and what of them is read and sent (the
/// student's decision 3 of 2026-10-10, #100). 3: only citation fields are
/// left out; a citation that is ordinary text is part of the body and can
/// be sent (D103-02, contract v22 §4.4). 4: PowerPoint decks and Excel
/// workbooks: speaker notes, formulas and saved values are sent, hidden
/// slides and sheets are not (the student's decision of 2026-10-12,
/// contract v23 §4.4).
pub const MEMORY_NOTICE_VERSION: u32 = 4;

/// What the user agrees to before Memory sends anything about a file.
pub fn memory_consent_notice(provider: &str, model: &str) -> String {
    let name = if provider == PROVIDER_OPENAI {
        "OpenAI"
    } else {
        provider
    };
    format!(
        "When Memory describes what changed in this file, Traeva sends to {name} (model {model}) \
         the lines that changed between two saved versions, with up to two unchanged lines \
         around each change. When you search this file's history, it sends your question and \
         the short descriptions of the changes Memory found. It does not send the file's name \
         or folder. If a change rewrites most of the file, that can be most of its text. For a \
         Word document (.docx), Traeva reads only the text in the document's body: tables, \
         headers and footers, footnotes, comments, text boxes, pictures and embedded objects, \
         fields (for example citation fields a reference manager such as Zotero or EndNote \
         inserted, and page numbers) and tracked changes are not read or sent, so changes there \
         are not described. A citation that is ordinary text in the document (for example after \
         its field was unlinked) is part of the body and can be sent, and so can text that Word \
         hides. For a PowerPoint deck (.pptx), Traeva reads the text on the visible slides, \
         speaker notes included, and adds lines that say which slide and part each line is on; \
         hidden slides and their notes are not read or sent, and pictures, charts and \
         animations are not read. For an Excel workbook (.xlsx), Traeva reads every cell of the \
         visible sheets: the values, the formulas as written and the value each formula last \
         saved, with the sheet's name and the cell's address; hidden sheets are not read or \
         sent, and nothing is recalculated. While \
         this stays on, Traeva describes every new version it saves of this file by itself. \
         You can turn it off at any time: nothing more is sent after that, but what was \
         already sent cannot be called back. {name} handles the lines under its own data \
         policy; Traeva asks it not to store the answer. This agreement is only for Memory and \
         this file; Organize asks separately."
    )
}

/// Memory's agreement for one file, as a settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryConsentView {
    pub file_id: String,
    /// Whether Memory may send now: an agreement is open and was given for
    /// the provider and model the settings name now, under this notice.
    pub agreed: bool,
    /// An open agreement that no longer counts (the provider, model or
    /// notice changed since). Agreeing again replaces it.
    pub outdated: bool,
    pub agreed_at_ms: Option<i64>,
    /// The saved settings; null until a provider and model are chosen.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// What will be sent, shown before the user agrees.
    pub notice: String,
}

/// One row of `memory_consents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentRow {
    pub consent_id: String,
    pub file_id: String,
    pub provider: String,
    pub model: String,
    pub notice_version: u32,
    pub agreed_at_ms: i64,
}

impl ConsentRow {
    /// Whether this agreement covers sending to `provider` / `model` now.
    pub(crate) fn covers(&self, provider: &str, model: &str) -> bool {
        self.provider == provider
            && self.model == model
            && self.notice_version == MEMORY_NOTICE_VERSION
    }
}

impl OrganizeService {
    /// Memory's agreement for file `file_id`.
    pub fn memory_consent(&self, file_id: &str) -> Result<MemoryConsentView, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        let open = store.open_consent(file_id)?;
        self.consent_view(file_id, open.as_ref())
    }

    /// Agrees (or stops agreeing) that Memory may send the changed lines of
    /// file `file_id` to the saved provider and model. Agreeing while an
    /// agreement already counts changes nothing; agreeing while an outdated
    /// one is open replaces it. Withdrawing when none is open changes
    /// nothing.
    pub fn set_memory_consent(
        &self,
        file_id: &str,
        agree: bool,
    ) -> Result<MemoryConsentView, AppError> {
        let store = self.store()?;
        check_file_id(file_id)?;
        if store.get_file(file_id)?.is_none() {
            return Err(unknown_file());
        }
        // The same lock as every settings change and as the last check
        // before a Memory request goes out (changes.rs).
        let _one_at_a_time = settings::lock(&self.model.settings_lock);
        let open = store.open_consent(file_id)?;
        if !agree {
            if let Some(open) = &open {
                store.withdraw_consent(&open.consent_id, self.now())?;
            }
            return self.consent_view(file_id, None);
        }
        let settings = self.model_settings()?;
        let (Some(provider), Some(model)) = (settings.provider, settings.model) else {
            return Err(memory_error(
                ErrorCode::ModelNotConfigured,
                "Choose the provider and model before agreeing to send.",
                false,
                Some(file_id),
            ));
        };
        if let Some(open) = &open
            && open.covers(&provider, &model)
        {
            return self.consent_view(file_id, Some(open));
        }
        let row = ConsentRow {
            consent_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            provider,
            model,
            notice_version: MEMORY_NOTICE_VERSION,
            agreed_at_ms: self.now(),
        };
        store.replace_consent(open.as_ref(), &row, self.now())?;
        self.consent_view(file_id, Some(&row))
    }

    /// The open agreement of file `file_id` if it counts for the provider
    /// and model the settings name now; otherwise none. When the settings
    /// cannot be read (a damaged settings file, a Keychain error), nothing
    /// counts: Memory then sends nothing, and its local history still shows.
    pub(crate) fn counting_consent(&self, file_id: &str) -> Result<Option<ConsentRow>, AppError> {
        let store = self.store()?;
        let Some(open) = store.open_consent(file_id)? else {
            return Ok(None);
        };
        let Ok(settings) = self.model_settings() else {
            return Ok(None);
        };
        match (settings.provider, settings.model) {
            (Some(provider), Some(model)) if open.covers(&provider, &model) => Ok(Some(open)),
            _ => Ok(None),
        }
    }

    /// Withdraws every open agreement given for another provider, model or
    /// notice than the settings name now, the way Organize's agreement ends
    /// when the provider or model changes. Run by each background pass,
    /// under the settings lock. Without readable settings nothing changes.
    pub(crate) fn withdraw_outdated_consents(&self) -> Result<(), AppError> {
        let store = self.store()?;
        let _one_at_a_time = settings::lock(&self.model.settings_lock);
        let Ok(settings) = self.model_settings() else {
            return Ok(());
        };
        let (Some(provider), Some(model)) = (settings.provider, settings.model) else {
            return Ok(());
        };
        let conn = store.connection()?;
        conn.execute(
            "UPDATE memory_consents SET withdrawn_at_ms = ?4
             WHERE withdrawn_at_ms IS NULL
               AND (provider <> ?1 OR model <> ?2 OR notice_version <> ?3)",
            params![provider, model, MEMORY_NOTICE_VERSION, self.now()],
        )
        .map_err(|err| sqlite_error("Cannot withdraw Memory's agreement", &err))?;
        Ok(())
    }

    fn consent_view(
        &self,
        file_id: &str,
        open: Option<&ConsentRow>,
    ) -> Result<MemoryConsentView, AppError> {
        let settings = self.model_settings()?;
        let counts = match (open, &settings.provider, &settings.model) {
            (Some(open), Some(provider), Some(model)) => open.covers(provider, model),
            _ => false,
        };
        let notice = memory_consent_notice(
            settings.provider.as_deref().unwrap_or(PROVIDER_OPENAI),
            settings.model.as_deref().unwrap_or(settings::DEFAULT_MODEL),
        );
        Ok(MemoryConsentView {
            file_id: file_id.to_string(),
            agreed: counts,
            outdated: open.is_some() && !counts,
            agreed_at_ms: open.filter(|_| counts).map(|open| open.agreed_at_ms),
            provider: settings.provider,
            model: settings.model,
            notice,
        })
    }
}

/// An error of Memory's own work (#33 and later), with its own `retryable`
/// (shared-contract.md §5). Snapshot reads use `AppError::snapshot`.
pub(crate) fn memory_error(
    code: ErrorCode,
    message: impl Into<String>,
    retryable: bool,
    file_id: Option<&str>,
) -> AppError {
    AppError {
        code,
        message: message.into(),
        stage: ErrorStage::Memory,
        retryable,
        file_id: file_id.map(str::to_string),
        plan_id: None,
        action_id: None,
        allowed_next: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// The `memory_consents` table
// ---------------------------------------------------------------------------

impl Store {
    /// The file's agreement that is not withdrawn, if any.
    pub(crate) fn open_consent(&self, file_id: &str) -> Result<Option<ConsentRow>, AppError> {
        let conn = self.connection()?;
        open_consent_in(&conn, file_id)
    }

    fn withdraw_consent(&self, consent_id: &str, now_ms: i64) -> Result<(), AppError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE memory_consents SET withdrawn_at_ms = ?2
             WHERE consent_id = ?1 AND withdrawn_at_ms IS NULL",
            params![consent_id, now_ms],
        )
        .map_err(|err| sqlite_error("Cannot withdraw Memory's agreement", &err))?;
        Ok(())
    }

    /// Withdraws `old` (if given) and saves `new`, in one transaction.
    fn replace_consent(
        &self,
        old: Option<&ConsentRow>,
        new: &ConsentRow,
        now_ms: i64,
    ) -> Result<(), AppError> {
        let fail = |err: rusqlite::Error| sqlite_error("Cannot save Memory's agreement", &err);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(fail)?;
        if let Some(old) = old {
            tx.execute(
                "UPDATE memory_consents SET withdrawn_at_ms = ?2
                 WHERE consent_id = ?1 AND withdrawn_at_ms IS NULL",
                params![old.consent_id, now_ms],
            )
            .map_err(fail)?;
        }
        tx.execute(
            "INSERT INTO memory_consents (consent_id, file_id, provider, model, notice_version,
                                          agreed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new.consent_id,
                new.file_id,
                new.provider,
                new.model,
                new.notice_version,
                new.agreed_at_ms
            ],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)
    }
}

const CONSENT_COLUMNS: &str = "SELECT consent_id, file_id, provider, model, notice_version,
                                      agreed_at_ms
                               FROM memory_consents";

pub(crate) fn open_consent_in(
    conn: &Connection,
    file_id: &str,
) -> Result<Option<ConsentRow>, AppError> {
    conn.query_row(
        &format!("{CONSENT_COLUMNS} WHERE file_id = ?1 AND withdrawn_at_ms IS NULL"),
        [file_id],
        consent_row,
    )
    .optional()
    .map_err(|err| sqlite_error("Cannot read Memory's agreement", &err))
}

fn consent_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConsentRow> {
    Ok(ConsentRow {
        consent_id: row.get(0)?,
        file_id: row.get(1)?,
        provider: row.get(2)?,
        model: row.get(3)?,
        notice_version: row.get(4)?,
        agreed_at_ms: row.get(5)?,
    })
}
