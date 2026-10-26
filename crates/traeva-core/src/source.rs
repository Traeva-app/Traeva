//! Source records, candidate evidence and explicit user decisions (#27).
//! Chrome supplies download facts, never a claim about a file's identity.

use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::contracts::{AppError, ErrorCode, ErrorStage};
use crate::platform::macos;
use crate::store::{FileEvidence, FileRow, Store, sqlite_error, to_json};
use crate::{OrganizeService, evidence_of, is_canonical_uuid};

const MAX_FIELD: usize = 8192;
const MAX_NOTE: usize = 1000;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDownloadEvent {
    pub profile_id: String,
    pub download_id: String,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub state: SourceDownloadState,
    pub filename: String,
    pub initial_url: Option<String>,
    pub final_url: Option<String>,
    pub referrer: Option<String>,
    pub file_size: Option<u64>,
    pub incognito: bool,
    #[serde(default)]
    pub explicit_history: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDownloadState {
    InProgress,
    Complete,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Unknown,
    Likely,
    UserConfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRelation {
    Download,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLinkKind {
    Initial,
    Final,
    Referrer,
}

#[derive(Clone, Serialize)]
pub struct SourceState {
    pub enabled: bool,
    pub records: Vec<SourceRecordView>,
}

#[derive(Clone, Serialize)]
pub struct SourceRecordView {
    pub source_id: String,
    pub profile_id: String,
    pub download_id: String,
    pub download_state: SourceDownloadState,
    pub status: SourceStatus,
    pub filename: String,
    pub initial_url: Option<String>,
    pub final_url: Option<String>,
    pub referrer: Option<String>,
    pub title: Option<String>,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub imported_at_ms: i64,
    pub updated_at_ms: i64,
    pub reason: Option<String>,
    pub candidates: Vec<SourceCandidateView>,
    pub association: Option<SourceAssociationView>,
}

#[derive(Clone, Serialize)]
pub struct SourceCandidateView {
    pub file_id: String,
    pub display_path: String,
    pub reason: String,
    pub can_confirm: bool,
}

#[derive(Clone, Serialize)]
pub struct SourceAssociationView {
    pub file_id: String,
    pub display_path: String,
    pub relation: SourceRelation,
    pub basis: &'static str,
    pub note: String,
    pub confirmed_at_ms: i64,
    pub current_location_confirmed: bool,
}

#[derive(Clone, Serialize)]
pub struct SourceCaptureState {
    pub enabled: bool,
    pub enabled_since_ms: Option<i64>,
    pub roots: Vec<SourceRoot>,
}

#[derive(Clone, Serialize)]
pub struct SourceRoot {
    pub root_id: String,
    pub path: String,
}

#[derive(Clone, Serialize)]
pub struct SourceIngestOutcome {
    pub outcome: &'static str,
    pub source_id: Option<String>,
    pub reason: Option<String>,
}

struct SourceRow {
    id: String,
    event: SourceDownloadEvent,
    imported: i64,
    updated: i64,
    reason: Option<String>,
}

struct CandidateRow {
    file_id: String,
    evidence: FileEvidence,
    reason: String,
}

struct AssociationRow {
    file_id: String,
    evidence: FileEvidence,
    relation: SourceRelation,
    note: String,
    confirmed: i64,
}

fn source_error(code: ErrorCode, message: &str) -> AppError {
    AppError {
        code,
        message: message.to_string(),
        stage: ErrorStage::Memory,
        retryable: false,
        file_id: None,
        plan_id: None,
        action_id: None,
        allowed_next: Vec::new(),
    }
}

fn invalid(message: &str) -> AppError {
    source_error(ErrorCode::InvalidInput, message)
}

fn db(error: rusqlite::Error) -> AppError {
    sqlite_error("Cannot save or read source records", &error)
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_str(value).map_err(|_| {
        source_error(
            ErrorCode::DatabaseCorrupt,
            "A source record could not be read. It was kept unchanged.",
        )
    })
}

fn ignored(reason: &str) -> SourceIngestOutcome {
    SourceIngestOutcome {
        outcome: "ignored",
        source_id: None,
        reason: Some(reason.to_string()),
    }
}

impl OrganizeService {
    pub fn source_state(&self) -> Result<SourceState, AppError> {
        let store = self.store()?;
        let ids = store.source_ids()?;
        let records = ids
            .iter()
            .map(|id| self.source_view(id))
            .collect::<Result<_, _>>()?;
        Ok(SourceState {
            enabled: store.source_enabled()?,
            records,
        })
    }

    pub fn set_source_enabled(&self, enabled: bool) -> Result<SourceState, AppError> {
        {
            let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
            self.store()?
                .connection()?
                .execute(
                    "UPDATE source_settings
                     SET enabled_since_ms = CASE
                         WHEN ? AND (enabled = 0 OR enabled_since_ms IS NULL) THEN ?
                         ELSE enabled_since_ms END,
                         enabled = ? WHERE singleton = 1",
                    params![enabled, self.now(), enabled],
                )
                .map_err(db)?;
        }
        self.source_state()
    }

    pub fn source_capture_state(&self) -> Result<SourceCaptureState, AppError> {
        let (enabled, enabled_since_ms) = self.store()?.source_capture_window()?;
        let roots = if enabled {
            self.list_roots()?
                .into_iter()
                .filter(|r| r.problem.is_none())
                .map(|r| SourceRoot {
                    root_id: r.root_id,
                    path: r.display_path,
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(SourceCaptureState {
            enabled,
            enabled_since_ms,
            roots,
        })
    }

    pub fn source_pairing_token(&self) -> Result<String, AppError> {
        let store = self.store()?;
        let mut conn = store.connection()?;
        let tx = conn.transaction().map_err(db)?;
        let token: Option<String> = tx
            .query_row(
                "SELECT pairing_token FROM source_settings WHERE singleton = 1",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        let token = match token {
            Some(token) => token,
            None => {
                let token = format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                );
                tx.execute(
                    "UPDATE source_settings SET pairing_token = ? WHERE singleton = 1",
                    [&token],
                )
                .map_err(db)?;
                token
            }
        };
        tx.commit().map_err(db)?;
        Ok(token)
    }

    pub fn source_receiver_port(&self) -> Result<Option<u16>, AppError> {
        self.store()?
            .connection()?
            .query_row(
                "SELECT receiver_port FROM source_settings WHERE singleton = 1",
                [],
                |r| r.get(0),
            )
            .map_err(db)
    }

    pub fn set_source_receiver_port(&self, port: u16) -> Result<(), AppError> {
        if port == 0 {
            return Err(invalid("The receiver port is not valid."));
        }
        self.store()?
            .connection()?
            .execute(
                "UPDATE source_settings SET receiver_port = ? WHERE singleton = 1",
                [port],
            )
            .map_err(db)?;
        Ok(())
    }

    pub fn ingest_source_event(
        &self,
        mut event: SourceDownloadEvent,
    ) -> Result<SourceIngestOutcome, AppError> {
        if event.incognito {
            return Ok(ignored("Incognito downloads are not collected."));
        }
        validate_event(&event)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        let store = self.store()?;
        let (enabled, enabled_since_ms) = store.source_capture_window()?;
        if !enabled {
            return Ok(ignored("Source collection is off."));
        }
        if !event.explicit_history
            && !matches!((event.started_at_ms, enabled_since_ms), (Some(start), Some(since)) if start >= since && start <= self.now())
        {
            return Ok(ignored(
                "This download did not start during the current collection interval. Import earlier downloads explicitly if wanted.",
            ));
        }
        let Some(root_id) = self.source_scope(Path::new(&event.filename))? else {
            return Ok(ignored(
                "The download is outside a currently accessible authorized folder.",
            ));
        };
        if event.state != SourceDownloadState::Complete {
            event.completed_at_ms = None;
        }
        let source_id = store.save_source_event(&root_id, &event, self.now())?;
        self.refresh_source_inner(&source_id)?;
        Ok(SourceIngestOutcome {
            outcome: "saved",
            source_id: Some(source_id),
            reason: None,
        })
    }

    pub fn refresh_source(&self, source_id: &str) -> Result<SourceRecordView, AppError> {
        check_id(source_id)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh_source_inner(source_id)?;
        self.source_view(source_id)
    }

    fn refresh_source_inner(&self, source_id: &str) -> Result<(), AppError> {
        let store = self.store()?;
        let row = store.source_row(source_id)?;
        if row.event.state != SourceDownloadState::Complete {
            return store.source_reason(
                source_id,
                Some("The browser has not reported a successful completed download."),
            );
        }
        let picked = Path::new(&row.event.filename);
        match self.source_register(picked) {
            Ok(file) => {
                store.source_candidate(source_id, &file, "This file is currently at the browser's recorded download path. The path is a clue, not proof of origin.", false)?;
                self.continue_source(source_id, &file)?;
                store.source_reason(source_id, None)
            }
            Err(error) if error.stage != ErrorStage::Storage => {
                store.source_reason(source_id, Some(&error.message))
            }
            Err(error) => Err(error),
        }
    }

    pub fn suggest_source_file(
        &self,
        source_id: &str,
        picked: &Path,
    ) -> Result<SourceRecordView, AppError> {
        check_id(source_id)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        let store = self.store()?;
        store.source_row(source_id)?;
        let file = self.source_register(picked)?;
        store.source_candidate(
            source_id,
            &file,
            "You selected this file as a possible match. Confirm only the relationship you know.",
            true,
        )?;
        self.continue_source(source_id, &file)?;
        store.source_reason(source_id, None)?;
        self.source_view(source_id)
    }

    fn source_register(&self, picked: &Path) -> Result<FileRow, AppError> {
        let root_id = self.source_scope(picked)?.ok_or_else(|| {
            source_error(
                ErrorCode::OutsideRoot,
                "The location is not inside a currently accessible authorized folder.",
            )
        })?;
        let observed = self.register_file(&root_id, picked)?;
        self.store()?
            .get_file(&observed.file.file_id)?
            .map(|(row, _)| row)
            .ok_or_else(|| invalid("The registered file is unavailable."))
    }

    // The lexical prefix only chooses a root; handles prove the directory chain.
    fn source_scope(&self, picked: &Path) -> Result<Option<String>, AppError> {
        if !picked.is_absolute()
            || picked
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Ok(None);
        }
        for row in self.store()?.active_roots()? {
            let Ok(relative) = picked.strip_prefix(&row.display_path) else {
                continue;
            };
            let Some(parent) = relative.parent() else {
                continue;
            };
            let Ok((_, root)) = self.verified_root(&row.root_id) else {
                continue;
            };
            let Some(names) = parent
                .components()
                .map(|c| match c {
                    Component::Normal(x) => x.to_str(),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Ok(chain) = macos::open_chain(&root, &names) else {
                continue;
            };
            if macos::verify_chain(&root, &chain).is_ok() {
                return Ok(Some(row.root_id));
            }
        }
        Ok(None)
    }

    pub fn confirm_source(
        &self,
        source_id: &str,
        file_id: &str,
        relation: SourceRelation,
        note: &str,
    ) -> Result<SourceRecordView, AppError> {
        check_id(source_id)?;
        check_id(file_id)?;
        check_note(note)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        let store = self.store()?;
        let candidate = store
            .source_candidates(source_id)?
            .into_iter()
            .find(|c| c.file_id == file_id)
            .ok_or_else(|| invalid("Choose an active candidate before confirming."))?;
        let (row, _) = store
            .get_file(file_id)?
            .ok_or_else(|| invalid("The candidate file is unavailable."))?;
        let root = store
            .get_root(&row.root_id)?
            .ok_or_else(|| invalid("The folder is unavailable."))?;
        let current =
            self.source_register(&Path::new(&root.display_path).join(&row.relative_path))?;
        if current.file_id != file_id
            || !candidate.evidence.same_object(&current.evidence)
            || current.evidence.link_count != 1
        {
            return Err(source_error(
                ErrorCode::FileChanged,
                "The candidate was replaced. Refresh the suggested files before confirming.",
            ));
        }
        store.confirm_source(source_id, &current, relation, note, self.now())?;
        self.source_view(source_id)
    }

    pub fn reject_source(
        &self,
        source_id: &str,
        file_id: &str,
    ) -> Result<SourceRecordView, AppError> {
        check_id(source_id)?;
        check_id(file_id)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        self.store()?
            .reject_source(source_id, file_id, self.now())?;
        self.source_view(source_id)
    }

    pub fn unlink_source(&self, source_id: &str, note: &str) -> Result<SourceRecordView, AppError> {
        check_id(source_id)?;
        check_note(note)?;
        let _execution = self.execution.lock().unwrap_or_else(|e| e.into_inner());
        self.store()?.unlink_source(source_id, note, self.now())?;
        self.source_view(source_id)
    }

    pub fn source_link(&self, source_id: &str, kind: SourceLinkKind) -> Result<String, AppError> {
        check_id(source_id)?;
        let event = self.store()?.source_row(source_id)?.event;
        let value = match kind {
            SourceLinkKind::Initial => event.initial_url,
            SourceLinkKind::Final => event.final_url,
            SourceLinkKind::Referrer => event.referrer,
        };
        let value = value.ok_or_else(|| invalid("This source link is unknown."))?;
        let uri: ureq::http::Uri = value
            .parse()
            .map_err(|_| invalid("This source link cannot be opened."))?;
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.host().is_none()
            || uri
                .authority()
                .is_some_and(|authority| authority.as_str().contains('@'))
            || value.chars().any(char::is_control)
        {
            return Err(invalid("Only HTTP and HTTPS source links can be opened."));
        }
        Ok(value)
    }

    fn source_view(&self, source_id: &str) -> Result<SourceRecordView, AppError> {
        let store = self.store()?;
        let row = store.source_row(source_id)?;
        let saved_association = store.source_association(source_id)?;
        let candidates = store
            .source_candidates(source_id)?
            .into_iter()
            .filter(|c| {
                saved_association
                    .as_ref()
                    .is_none_or(|a| a.file_id != c.file_id)
            })
            .map(|c| {
                let (path, current, problem) = self.source_location(&c.file_id, &c.evidence)?;
                Ok(SourceCandidateView {
                    file_id: c.file_id,
                    display_path: path,
                    reason: match problem {
                        Some(problem) => format!("{} {problem}", c.reason),
                        None => c.reason,
                    },
                    can_confirm: current,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let mut location_problem = None;
        let association = saved_association
            .map(|a| {
                let (path, current, problem) = self.source_location(&a.file_id, &a.evidence)?;
                location_problem = problem;
                Ok(SourceAssociationView {
                    file_id: a.file_id,
                    display_path: path,
                    relation: a.relation,
                    basis: "user_confirmed",
                    note: a.note,
                    confirmed_at_ms: a.confirmed,
                    current_location_confirmed: current,
                })
            })
            .transpose()?;
        let status = if association.is_some() {
            SourceStatus::UserConfirmed
        } else if candidates.iter().any(|c| c.can_confirm) {
            SourceStatus::Likely
        } else {
            SourceStatus::Unknown
        };
        let event = row.event;
        Ok(SourceRecordView {
            source_id: row.id,
            profile_id: event.profile_id,
            download_id: event.download_id,
            download_state: event.state,
            status,
            filename: event.filename,
            initial_url: event.initial_url,
            final_url: event.final_url,
            referrer: event.referrer,
            title: None,
            started_at_ms: event.started_at_ms,
            completed_at_ms: event.completed_at_ms,
            imported_at_ms: row.imported,
            updated_at_ms: row.updated,
            reason: location_problem.or(row.reason),
            candidates,
            association,
        })
    }

    fn source_location(
        &self,
        file_id: &str,
        evidence: &FileEvidence,
    ) -> Result<(String, bool, Option<String>), AppError> {
        let store = self.store()?;
        let Some((file, present)) = store.get_file(file_id)? else {
            return Ok((
                "Location unknown".to_string(),
                false,
                Some("The saved file or folder record is unavailable.".to_string()),
            ));
        };
        let Some(root) = store.get_root(&file.root_id)? else {
            return Ok((
                "Location unknown".to_string(),
                false,
                Some("The saved file or folder record is unavailable.".to_string()),
            ));
        };
        let path = PathBuf::from(&root.display_path)
            .join(&file.relative_path)
            .display()
            .to_string();
        if !present {
            return Ok((
                path,
                false,
                Some(
                    "The saved file record is no longer current. Check or choose the file again."
                        .to_string(),
                ),
            ));
        }
        let root_handle = match self.verified_root(&file.root_id) {
            Ok((_, handle)) => handle,
            Err(error) => return Ok((path, false, Some(error.message))),
        };
        let mut parts: Vec<&str> = file.relative_path.split('/').collect();
        let Some(name) = parts.pop() else {
            return Ok((
                path,
                false,
                Some("The saved location is invalid.".to_string()),
            ));
        };
        let current = (|| {
            let chain = macos::open_chain(&root_handle, &parts)?;
            let parent = chain.last().unwrap_or(&root_handle);
            let observation = macos::observe(parent, name)?;
            macos::check_movable_file(name, &observation, root_handle.id().dev)?;
            macos::verify_chain(&root_handle, &chain)?;
            Ok::<_, macos::FsError>(evidence.same_object(&evidence_of(&observation)))
        })();
        match current {
            Ok(true) => Ok((path, true, None)),
            Ok(false) => Ok((path, false, Some("A different file object occupies the saved location. The historical source is retained.".to_string()))),
            Err(error) => {
                let message = crate::file_error(&path, error).message;
                Ok((path, false, Some(message)))
            }
        }
    }

    // Called by core immediately after a successful stable file registration.
    // The row must come from that observation, never a stale database lookup.
    pub(crate) fn continue_registered_sources(&self, file: &FileRow) -> Result<(), AppError> {
        if file.evidence.link_count != 1 {
            return Ok(());
        }
        for id in self.store()?.associated_source_ids()? {
            self.continue_source(&id, file)?;
        }
        Ok(())
    }

    fn continue_source(&self, source_id: &str, file: &FileRow) -> Result<(), AppError> {
        let store = self.store()?;
        if let Some(old) = store.source_association(source_id)?
            && old.file_id != file.file_id
            && file.evidence.shows_object_of(&old.evidence)
        {
            store.continue_source(source_id, &old, file, self.now())?;
        }
        Ok(())
    }
}

fn check_id(id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(id) {
        Ok(())
    } else {
        Err(invalid("The source or file ID is not valid."))
    }
}

fn check_note(note: &str) -> Result<(), AppError> {
    if note.len() > MAX_NOTE || note.contains('\0') {
        Err(invalid(
            "The correction note is too long or contains an invalid character.",
        ))
    } else {
        Ok(())
    }
}

fn validate_event(event: &SourceDownloadEvent) -> Result<(), AppError> {
    if !is_canonical_uuid(&event.profile_id)
        || event.download_id.is_empty()
        || event.download_id.len() > 32
        || !event.download_id.bytes().all(|b| b.is_ascii_digit())
        || event.filename.len() > MAX_FIELD
        || event.filename.contains('\0')
    {
        return Err(invalid("The browser download record is not valid."));
    }
    for value in [&event.initial_url, &event.final_url, &event.referrer]
        .into_iter()
        .flatten()
    {
        if value.is_empty() || value.len() > MAX_FIELD || value.chars().any(char::is_control) {
            return Err(invalid("A browser source field is not valid."));
        }
    }
    if event.started_at_ms.is_some_and(|x| x < 0)
        || event.completed_at_ms.is_some_and(|x| x < 0)
        || matches!((event.started_at_ms,event.completed_at_ms),(Some(a),Some(b)) if b<a)
        || event.file_size.is_some_and(|n| n > i64::MAX as u64)
    {
        return Err(invalid("The browser download time or size is not valid."));
    }
    Ok(())
}

impl Store {
    fn source_capture_window(&self) -> Result<(bool, Option<i64>), AppError> {
        self.connection()?
            .query_row(
                "SELECT enabled, enabled_since_ms FROM source_settings WHERE singleton = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db)
    }

    fn source_enabled(&self) -> Result<bool, AppError> {
        self.connection()?
            .query_row(
                "SELECT enabled FROM source_settings WHERE singleton = 1",
                [],
                |r| r.get(0),
            )
            .map_err(db)
    }

    fn associated_source_ids(&self) -> Result<Vec<String>, AppError> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare("SELECT source_id FROM source_associations")
            .map_err(db)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    fn source_ids(&self) -> Result<Vec<String>, AppError> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare("SELECT source_id FROM source_records ORDER BY imported_at_ms DESC, source_id")
            .map_err(db)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    fn source_row(&self, id: &str) -> Result<SourceRow, AppError> {
        let row = self.connection()?.query_row("SELECT event_json, imported_at_ms, updated_at_ms, reason FROM source_records WHERE source_id = ?", [id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(db)?;
        let (event, imported, updated, reason) =
            row.ok_or_else(|| invalid("There is no source record with this ID."))?;
        Ok(SourceRow {
            id: id.to_string(),
            event: decode(&event)?,
            imported,
            updated,
            reason,
        })
    }

    fn save_source_event(
        &self,
        root_id: &str,
        event: &SourceDownloadEvent,
        now: i64,
    ) -> Result<String, AppError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        ensure_active(&tx, root_id, true)?;
        let existing:Option<(String,String)> = tx.query_row("SELECT source_id,event_json FROM source_records WHERE profile_id = ? AND download_id = ?",params![event.profile_id,event.download_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?;
        let id = if let Some((id, old)) = existing {
            let previous: SourceDownloadEvent = decode(&old)?;
            if matches!((previous.started_at_ms,event.started_at_ms),(Some(a),Some(b)) if a!=b)
                || matches!((&previous.initial_url,&event.initial_url),(Some(a),Some(b)) if a!=b)
            {
                return Err(invalid(
                    "This browser record conflicts with saved evidence. The saved record was kept.",
                ));
            }
            if previous.state != SourceDownloadState::Complete
                || event.state == SourceDownloadState::Complete
            {
                let mut updated = event.clone();
                updated.explicit_history = previous.explicit_history;
                updated.initial_url = previous.initial_url.or(updated.initial_url);
                updated.started_at_ms = previous.started_at_ms.or(updated.started_at_ms);
                updated.final_url = updated.final_url.or(previous.final_url);
                updated.referrer = updated.referrer.or(previous.referrer);
                updated.completed_at_ms = updated.completed_at_ms.or(previous.completed_at_ms);
                let json = to_json(&updated)?;
                if old != json {
                    tx.execute("UPDATE source_records SET event_json = ?, updated_at_ms = ? WHERE source_id = ?",params![json,now,id]).map_err(db)?;
                }
            }
            id
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO source_records (source_id,profile_id,download_id,root_id,event_json,imported_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?)",params![id,event.profile_id,event.download_id,root_id,to_json(event)?,now,now]).map_err(db)?;
            id
        };
        tx.commit().map_err(db)?;
        Ok(id)
    }

    fn source_reason(&self, id: &str, reason: Option<&str>) -> Result<(), AppError> {
        self.connection()?
            .execute(
                "UPDATE source_records SET reason = ? WHERE source_id = ?",
                params![reason, id],
            )
            .map_err(db)?;
        Ok(())
    }

    fn source_candidate(
        &self,
        id: &str,
        file: &FileRow,
        reason: &str,
        manual: bool,
    ) -> Result<(), AppError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        ensure_active(&tx, &file.root_id, false)?;
        tx.execute("INSERT INTO source_candidates (source_id,file_id,evidence_json,reason,status) VALUES (?,?,?,?,'active') ON CONFLICT (source_id,file_id) DO UPDATE SET evidence_json=excluded.evidence_json,reason=excluded.reason,status=CASE WHEN ? THEN 'active' ELSE source_candidates.status END",params![id,file.file_id,to_json(&file.evidence)?,reason,manual]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(())
    }

    fn source_candidates(&self, id: &str) -> Result<Vec<CandidateRow>, AppError> {
        let conn = self.connection()?;
        let mut stmt=conn.prepare("SELECT file_id,evidence_json,reason FROM source_candidates WHERE source_id = ? AND status = 'active' ORDER BY file_id").map_err(db)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        rows.into_iter()
            .map(|(file_id, evidence, reason)| {
                Ok(CandidateRow {
                    file_id,
                    evidence: decode(&evidence)?,
                    reason,
                })
            })
            .collect()
    }

    fn source_association(&self, id: &str) -> Result<Option<AssociationRow>, AppError> {
        let row=self.connection()?.query_row("SELECT file_id,evidence_json,relation,note,confirmed_at_ms FROM source_associations WHERE source_id = ?",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))).optional().map_err(db)?;
        row.map(|(file_id, evidence, relation, note, confirmed)| {
            Ok(AssociationRow {
                file_id,
                evidence: decode(&evidence)?,
                relation: parse_relation(&relation)?,
                note,
                confirmed,
            })
        })
        .transpose()
    }

    fn confirm_source(
        &self,
        id: &str,
        file: &FileRow,
        relation: SourceRelation,
        note: &str,
        now: i64,
    ) -> Result<(), AppError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        ensure_active(&tx, &file.root_id, false)?;
        let relation = match relation {
            SourceRelation::Download => "download",
            SourceRelation::Copy => "copy",
        };
        let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM source_associations WHERE source_id=? AND file_id=? AND relation=? AND note=?)",params![id,file.file_id,relation,note],|r|r.get(0)).map_err(db)?;
        if !same {
            tx.execute("INSERT INTO source_associations (source_id,file_id,original_file_id,evidence_json,relation,note,confirmed_at_ms) VALUES (?,?,?,?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET file_id=excluded.file_id,original_file_id=excluded.original_file_id,evidence_json=excluded.evidence_json,relation=excluded.relation,note=excluded.note,confirmed_at_ms=excluded.confirmed_at_ms",params![id,file.file_id,file.file_id,to_json(&file.evidence)?,relation,note,now]).map_err(db)?;
            history(
                &tx,
                id,
                Some(&file.file_id),
                "confirmed",
                &to_json(&file.evidence)?,
                &format!("{relation}: {note}"),
                now,
            )?;
        }
        tx.commit().map_err(db)?;
        Ok(())
    }

    fn reject_source(&self, id: &str, file_id: &str, now: i64) -> Result<(), AppError> {
        self.source_row(id)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        let evidence:Option<String>=tx.query_row("SELECT evidence_json FROM source_candidates WHERE source_id=? AND file_id=? AND status='active'",params![id,file_id],|r|r.get(0)).optional().map_err(db)?;
        if let Some(evidence) = evidence {
            tx.execute(
                "UPDATE source_candidates SET status='rejected' WHERE source_id=? AND file_id=?",
                params![id, file_id],
            )
            .map_err(db)?;
            history(
                &tx,
                id,
                Some(file_id),
                "rejected",
                &evidence,
                "User rejected this candidate.",
                now,
            )?;
        }
        tx.commit().map_err(db)?;
        Ok(())
    }

    fn unlink_source(&self, id: &str, note: &str, now: i64) -> Result<(), AppError> {
        self.source_row(id)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT file_id,evidence_json FROM source_associations WHERE source_id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        if let Some((file, evidence)) = old {
            history(&tx, id, Some(&file), "unlinked", &evidence, note, now)?;
            tx.execute("DELETE FROM source_associations WHERE source_id=?", [id])
                .map_err(db)?;
            tx.execute(
                "UPDATE source_candidates SET status='rejected' WHERE source_id=? AND file_id=?",
                params![id, file],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(())
    }

    fn continue_source(
        &self,
        id: &str,
        old: &AssociationRow,
        file: &FileRow,
        now: i64,
    ) -> Result<(), AppError> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(db)?;
        ensure_active(&tx, &file.root_id, false)?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE file_id=? AND presence='present' AND record_revision=? AND observation_json=?)",
            params![file.file_id,file.record_revision,to_json(&file.evidence)?], |r| r.get(0)
        ).map_err(db)?;
        if !current {
            return Err(source_error(
                ErrorCode::FileChanged,
                "The file record changed during source continuity checks.",
            ));
        }
        let changed = tx.execute(
            "UPDATE source_associations SET file_id=?,evidence_json=? WHERE source_id=? AND file_id=? AND evidence_json=?",
            params![file.file_id,to_json(&file.evidence)?,id,old.file_id,to_json(&old.evidence)?],
        ).map_err(db)?;
        if changed == 1 {
            history(
                &tx,
                id,
                Some(&file.file_id),
                "continued",
                &to_json(&file.evidence)?,
                "Same object observed during core registration; User Confirmed basis retained.",
                now,
            )?;
        }
        tx.commit().map_err(db)?;
        Ok(())
    }
}

fn parse_relation(value: &str) -> Result<SourceRelation, AppError> {
    match value {
        "download" => Ok(SourceRelation::Download),
        "copy" => Ok(SourceRelation::Copy),
        _ => Err(source_error(
            ErrorCode::DatabaseCorrupt,
            "The saved source relation is not valid.",
        )),
    }
}

fn ensure_active(conn: &Connection, root_id: &str, require_capture: bool) -> Result<(), AppError> {
    let active: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM authorized_roots WHERE root_id=? AND status='active')",
            [root_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    if !active {
        return Err(source_error(
            ErrorCode::RootRevoked,
            "The folder authorization was revoked; nothing was linked.",
        ));
    }
    if require_capture {
        let enabled: bool = conn
            .query_row(
                "SELECT enabled FROM source_settings WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !enabled {
            return Err(invalid(
                "Source collection was turned off; nothing was saved.",
            ));
        }
    }
    Ok(())
}

fn history(
    conn: &Connection,
    id: &str,
    file: Option<&str>,
    kind: &str,
    evidence: &str,
    note: &str,
    now: i64,
) -> Result<(), AppError> {
    conn.execute("INSERT INTO source_history (history_id,source_id,file_id,kind,evidence_json,note,recorded_at_ms) VALUES (?,?,?,?,?,?,?)",params![uuid::Uuid::new_v4().to_string(),id,file,kind,evidence,note,now]).map_err(db)?;
    Ok(())
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
