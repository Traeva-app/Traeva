//! "Keep versions of files Traeva organizes" (#201; the student's decisions
//! of 2026-10-12 are on the issue): one switch, for people who should not
//! have to turn Memory on file by file.
//!
//! While the switch's agreement counts, each time Traeva moves a file it
//! organizes (the normal flow or a dropped file, never an Undo), Memory
//! starts recording that file, and Traeva gives the file Memory's own
//! agreement to send (memory_consent.rs, notice version 4), the same row a
//! user's click would make. So everything after that is the existing code:
//! the background pass describes each new version, the last check before a
//! request, outdated agreements, withdrawing one file.
//!
//! The switch never touches a file Memory records or recorded before (a
//! record the user stopped stays stopped), never records a file Memory
//! cannot read (PDF, or over 20 MiB), and stops starting new records once
//! 50 are on. Turning it off withdraws every per-file agreement it gave (so
//! nothing more is sent about those files) and starts nothing more; the
//! records it started keep saving versions on this Mac until the user stops
//! them on the Memory page. Choosing another provider or model turns it off
//! the same way (settings.rs), so switching back never revives it.
//!
//! Automatic starts run one at a time, from the limit check to the record,
//! so two moves finishing together cannot both pass the check (#227).
//!
//! Two narrow windows are accepted (G4's review of #204): a switch turned
//! off while a start is under way can still leave that one record on, on
//! this Mac only (no agreement is given then); and a user turning Memory on
//! for the same file at that very moment gets the switch's agreement for
//! it, which the switch covers anyway.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::OrganizeService;
use crate::capture::Read;
use crate::contracts::{ActionStatus, AppError, ErrorCode, PlanKind};
use crate::memory_consent::{MEMORY_NOTICE_VERSION, memory_consent_notice, memory_error};
use crate::observe::{self, MAX_FILE_BYTES};
use crate::reader;
use crate::settings::{self, PROVIDER_OPENAI};
use crate::store::{Store, sqlite_error};

/// Raise this when the notice text changes: earlier agreements stop counting.
pub const AUTO_MEMORY_NOTICE_VERSION: u32 = 1;
/// The switch starts no record while this many Memory records are on.
pub const AUTO_MEMORY_LIMIT: usize = 50;

/// What the user agrees to when turning the switch on.
pub fn auto_memory_notice(provider: &str, model: &str) -> String {
    format!(
        "When this is on, Traeva keeps versions of the files it organizes for you. After it \
         moves a text, Markdown, Word, PowerPoint or Excel file (up to 20 MiB), Memory starts \
         recording that file, as if you had turned Memory on for it, until {AUTO_MEMORY_LIMIT} \
         files are being recorded in all. Each new save is kept on this Mac. PDF files, and files \
         Memory records or recorded before (also ones you stopped), are left as they are.\n\n\
         Memory also describes what changed in \
         each of these files. For each one, turning this on gives Memory's own agreement:\n\n\
         {}\n\nTurning this off stops it for the files Traeva organizes from then on, and \
         withdraws the agreements it gave, so nothing more is sent about those files. The \
         versions already kept stay; you can stop recording a file on the Memory page. Choosing \
         another provider or model turns this off too.",
        memory_consent_notice(provider, model)
    )
}

/// The switch, as the Memory page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoMemoryView {
    /// The switch's agreement counts now (the saved provider and model, this
    /// notice version).
    pub enabled: bool,
    /// An agreement is open but was given for another provider, model or
    /// notice: turning it on again replaces it.
    pub outdated: bool,
    pub agreed_at_ms: Option<i64>,
    /// The saved settings; null until a provider and model are chosen.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub notice: String,
    /// The version to send back when turning it on.
    pub notice_version: u32,
    pub limit: usize,
    /// Memory records that are on now, all of them (the limit counts these).
    pub recording: usize,
    /// Records the switch started, for a label.
    pub started_chain_ids: Vec<String>,
}

/// Turns the switch on (with the notice version the page showed) or off.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoMemoryRequest {
    pub enable: bool,
    pub notice_version: u32,
}

/// What happened after one action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutoMemoryOutcome {
    /// The switch is off, or its agreement does not count now.
    Off,
    /// Not a finished organize move (an Undo, or the move did not finish).
    NotAMove,
    /// This action was handled before.
    AlreadyHandled,
    /// Memory has no reader for this file, or it is larger than 20 MiB.
    NotReadable,
    /// Memory records this file, or recorded it before (the user may have
    /// stopped it); nothing changed.
    AlreadyRecorded,
    /// `AUTO_MEMORY_LIMIT` records are on.
    LimitReached,
    /// Memory records the file now. `agreement_given`: the switch gave it
    /// Memory's agreement to send (false when it had its own already, or
    /// the switch was turned off meanwhile). As when the user turns Memory
    /// on, a first version that cannot be read yet is read by the next pass.
    Started {
        chain_id: Option<String>,
        agreement_given: bool,
    },
    /// Memory refused to start (for example the folder was removed from
    /// Traeva). Nothing is tried again; the move itself is not affected.
    Failed { code: ErrorCode },
}

/// The open agreement of the switch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgreementRow {
    agreement_id: String,
    provider: String,
    model: String,
    notice_version: u32,
    agreed_at_ms: i64,
}

impl AgreementRow {
    fn counts_for(&self, provider: &str, model: &str) -> bool {
        self.provider == provider
            && self.model == model
            && self.notice_version == AUTO_MEMORY_NOTICE_VERSION
    }
}

impl OrganizeService {
    /// The switch.
    pub fn auto_memory(&self) -> Result<AutoMemoryView, AppError> {
        let store = self.store()?;
        let open = open_agreement(store)?;
        let settings = self.model_settings()?;
        let counts = match (&open, &settings.provider, &settings.model) {
            (Some(open), Some(provider), Some(model)) => open.counts_for(provider, model),
            _ => false,
        };
        let started_files = started_files(store)?;
        let records = self.memory_records()?;
        let recording = active_watches(store)?;
        let (provider, model) = match (&settings.provider, &settings.model) {
            (Some(provider), Some(model)) => (provider.clone(), model.clone()),
            _ => (
                PROVIDER_OPENAI.to_string(),
                settings::DEFAULT_MODEL.to_string(),
            ),
        };
        Ok(AutoMemoryView {
            enabled: counts,
            outdated: open.is_some() && !counts,
            agreed_at_ms: open.as_ref().filter(|_| counts).map(|o| o.agreed_at_ms),
            notice: auto_memory_notice(&provider, &model),
            provider: settings.provider,
            model: settings.model,
            notice_version: AUTO_MEMORY_NOTICE_VERSION,
            limit: AUTO_MEMORY_LIMIT,
            recording,
            started_chain_ids: records
                .into_iter()
                .filter(|record| started_files.contains(&record.origin_file_id))
                .map(|record| record.chain_id)
                .collect(),
        })
    }

    /// Turns the switch on or off. On: the page's notice version must be
    /// this one, and a provider and model must be saved. Off: the agreement
    /// and every per-file agreement it gave are withdrawn together; the
    /// records it started keep recording.
    pub fn set_auto_memory(&self, request: &AutoMemoryRequest) -> Result<AutoMemoryView, AppError> {
        let store = self.store()?;
        {
            // The same lock as every settings change and as the last check
            // before a Memory request goes out (changes.rs).
            let _one_at_a_time = settings::lock(&self.model.settings_lock);
            let open = open_agreement(store)?;
            if request.enable {
                if request.notice_version != AUTO_MEMORY_NOTICE_VERSION {
                    return Err(memory_error(
                        ErrorCode::PageOutdated,
                        "The notice changed. Read it again before turning this on.",
                        true,
                        None,
                    ));
                }
                let settings = self.model_settings()?;
                let (Some(provider), Some(model)) = (settings.provider, settings.model) else {
                    return Err(memory_error(
                        ErrorCode::ModelNotConfigured,
                        "Choose the provider and model before turning this on.",
                        false,
                        None,
                    ));
                };
                if !open
                    .as_ref()
                    .is_some_and(|o| o.counts_for(&provider, &model))
                {
                    let row = AgreementRow {
                        agreement_id: uuid::Uuid::new_v4().to_string(),
                        provider,
                        model,
                        notice_version: AUTO_MEMORY_NOTICE_VERSION,
                        agreed_at_ms: self.now(),
                    };
                    replace_agreement(store, open.as_ref(), &row, self.now())?;
                }
            } else {
                turn_off(store, open.as_ref(), self.now())?;
            }
        }
        self.auto_memory()
    }

    /// Runs the switch for one finished action (the window calls it in the
    /// background after a move; the drop queue after its own moves).
    pub fn keep_versions_after(&self, action_id: &str) -> Result<AutoMemoryOutcome, AppError> {
        self.keep_versions_after_with(action_id, AUTO_MEMORY_LIMIT, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    pub(crate) fn keep_versions_after_with(
        &self,
        action_id: &str,
        limit: usize,
        read: Read,
    ) -> Result<AutoMemoryOutcome, AppError> {
        // One automatic start at a time, from the checks to the record: the
        // count of records on is read and taken in one step (#227). A user
        // turning Memory on by hand is not held up by this.
        let _one_at_a_time = self
            .auto_memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = self.store()?;
        let Some(agreement) = self.counting_agreement(store)? else {
            return Ok(AutoMemoryOutcome::Off);
        };
        let action = self.get_action(action_id)?;
        if action.kind != PlanKind::Organize || action.status != ActionStatus::Committed {
            return Ok(AutoMemoryOutcome::NotAMove);
        }
        if was_handled(store, action_id)? {
            return Ok(AutoMemoryOutcome::AlreadyHandled);
        }
        let Some((file, present)) = store.get_file(&action.file_id)? else {
            return Ok(AutoMemoryOutcome::NotReadable);
        };
        if !present || !reader::has_reader(&file.basename) || file.byte_length > MAX_FILE_BYTES {
            return Ok(AutoMemoryOutcome::NotReadable);
        }
        if was_ever_recorded(store, &file.file_id)? {
            return Ok(AutoMemoryOutcome::AlreadyRecorded);
        }
        if active_watches(store)? >= limit {
            return Ok(AutoMemoryOutcome::LimitReached);
        }
        let watch = match self.start_memory_with(&file.file_id, read) {
            Ok(watch) => watch,
            Err(error) => return Ok(AutoMemoryOutcome::Failed { code: error.code }),
        };
        // The agreement is given only if the switch still counts now, under
        // the settings lock, and the file has no agreement of its own.
        let agreement_given = {
            let _one_at_a_time = settings::lock(&self.model.settings_lock);
            let still = self
                .counting_agreement(store)?
                .filter(|now| now.agreement_id == agreement.agreement_id);
            let own = store.open_consent(&file.file_id)?;
            let consent = match (&still, own) {
                (Some(still), None) => Some((uuid::Uuid::new_v4().to_string(), still)),
                _ => None,
            };
            record_start(
                store,
                action_id,
                &agreement.agreement_id,
                &file.file_id,
                &watch.watch_id,
                consent.as_ref().map(|(id, still)| (id.as_str(), *still)),
                self.now(),
            )?;
            consent.is_some()
        };
        let chain_id = self
            .memory_records()?
            .into_iter()
            .find(|record| record.watch.watch_id == watch.watch_id)
            .map(|record| record.chain_id);
        Ok(AutoMemoryOutcome::Started {
            chain_id,
            agreement_given,
        })
    }

    /// The switch's open agreement if it counts for the saved provider and
    /// model now.
    fn counting_agreement(&self, store: &Store) -> Result<Option<AgreementRow>, AppError> {
        let Some(open) = open_agreement(store)? else {
            return Ok(None);
        };
        let Ok(settings) = self.model_settings() else {
            return Ok(None);
        };
        Ok(match (settings.provider, settings.model) {
            (Some(provider), Some(model)) if open.counts_for(&provider, &model) => Some(open),
            _ => None,
        })
    }
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

fn failed(err: rusqlite::Error) -> AppError {
    sqlite_error("Cannot read or save the switch for keeping versions", &err)
}

fn open_agreement(store: &Store) -> Result<Option<AgreementRow>, AppError> {
    let conn = store.connection()?;
    conn.query_row(
        "SELECT agreement_id, provider, model, notice_version, agreed_at_ms
         FROM memory_auto_agreements WHERE withdrawn_at_ms IS NULL",
        [],
        |row| {
            Ok(AgreementRow {
                agreement_id: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                notice_version: row.get(3)?,
                agreed_at_ms: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(failed)
}

fn replace_agreement(
    store: &Store,
    old: Option<&AgreementRow>,
    new: &AgreementRow,
    now_ms: i64,
) -> Result<(), AppError> {
    let mut conn = store.connection()?;
    let tx = conn.transaction().map_err(failed)?;
    if let Some(old) = old {
        tx.execute(
            "UPDATE memory_auto_agreements SET withdrawn_at_ms = ?2
             WHERE agreement_id = ?1 AND withdrawn_at_ms IS NULL",
            params![old.agreement_id, now_ms],
        )
        .map_err(failed)?;
    }
    tx.execute(
        "INSERT INTO memory_auto_agreements
             (agreement_id, provider, model, notice_version, agreed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            new.agreement_id,
            new.provider,
            new.model,
            new.notice_version,
            new.agreed_at_ms
        ],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}

/// Ends the switch when the provider or model changes (settings.rs, under
/// the settings lock): its agreement and the per-file agreements it gave
/// are withdrawn, as when the user turns it off.
pub(crate) fn end_with_the_service(store: &Store, now_ms: i64) -> Result<(), AppError> {
    let open = open_agreement(store)?;
    turn_off(store, open.as_ref(), now_ms)
}

/// Withdraws the switch's agreement and every per-file agreement it gave
/// that is still open, in one transaction.
fn turn_off(store: &Store, open: Option<&AgreementRow>, now_ms: i64) -> Result<(), AppError> {
    let mut conn = store.connection()?;
    let tx = conn.transaction().map_err(failed)?;
    if let Some(open) = open {
        tx.execute(
            "UPDATE memory_auto_agreements SET withdrawn_at_ms = ?2
             WHERE agreement_id = ?1 AND withdrawn_at_ms IS NULL",
            params![open.agreement_id, now_ms],
        )
        .map_err(failed)?;
    }
    tx.execute(
        "UPDATE memory_consents SET withdrawn_at_ms = ?1
         WHERE withdrawn_at_ms IS NULL
           AND consent_id IN (SELECT consent_id FROM memory_auto_starts
                              WHERE consent_id IS NOT NULL)",
        [now_ms],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}

/// Saves the start, and the per-file agreement it gives, in one transaction.
fn record_start(
    store: &Store,
    action_id: &str,
    agreement_id: &str,
    file_id: &str,
    watch_id: &str,
    consent: Option<(&str, &AgreementRow)>,
    now_ms: i64,
) -> Result<(), AppError> {
    let mut conn = store.connection()?;
    let tx = conn.transaction().map_err(failed)?;
    if let Some((consent_id, agreement)) = consent {
        tx.execute(
            "INSERT INTO memory_consents (consent_id, file_id, provider, model, notice_version,
                                          agreed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                consent_id,
                file_id,
                agreement.provider,
                agreement.model,
                MEMORY_NOTICE_VERSION,
                now_ms
            ],
        )
        .map_err(failed)?;
    }
    tx.execute(
        "INSERT INTO memory_auto_starts
             (action_id, agreement_id, file_id, watch_id, consent_id, started_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            action_id,
            agreement_id,
            file_id,
            watch_id,
            consent.map(|(id, _)| id),
            now_ms
        ],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}

fn was_handled(store: &Store, action_id: &str) -> Result<bool, AppError> {
    let conn = store.connection()?;
    conn.query_row(
        "SELECT 1 FROM memory_auto_starts WHERE action_id = ?1",
        [action_id],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(failed)
}

/// Memory records the file now or did before, in any state: a record the
/// user stopped is never started again by the switch.
fn was_ever_recorded(store: &Store, file_id: &str) -> Result<bool, AppError> {
    let conn = store.connection()?;
    conn.query_row(
        "SELECT 1 FROM memory_watches WHERE file_id = ?1 LIMIT 1",
        [file_id],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(failed)
}

fn active_watches(store: &Store) -> Result<usize, AppError> {
    let conn = store.connection()?;
    conn.query_row(
        "SELECT count(*) FROM memory_watches WHERE status = 'active'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| usize::try_from(count).unwrap_or(0))
    .map_err(failed)
}

/// The files the switch started Memory for.
fn started_files(store: &Store) -> Result<Vec<String>, AppError> {
    let conn = store.connection()?;
    let mut statement = conn
        .prepare("SELECT DISTINCT file_id FROM memory_auto_starts")
        .map_err(failed)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(failed)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(failed)
}

#[cfg(test)]
#[path = "auto_memory_tests.rs"]
mod tests;
