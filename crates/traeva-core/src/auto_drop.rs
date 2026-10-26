//! Organizing files the user drops on Traeva's icon (organize-spec.md §13,
//! v0.4, #168).
//!
//! The user agrees once to the automatic mode and chooses a target range:
//! existing folders of one authorized folder. A dropped file then goes
//! through the normal operation (read, analyze, up to three questions), and
//! when every condition of §13.4 holds, the core builds the plan and runs it
//! through the same executor, without a confirmation page: the agreement is
//! the confirmation. When a condition fails, the automatic handling stops
//! and the file is left to the normal flow; nothing is overwritten, renamed
//! or retried.
//!
//! The queue lives in memory like operations; each dropped file also gets a
//! row in `organize_drop_events` (what became of it, and the agreement it
//! ran under). A restart ends the queue and marks unfinished rows
//! `interrupted`; an action that already started has its own record check.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::contracts::{
    ActionStatus, AppError, BuildOutcome, ErrorCode, OperationStage, OperationView, PlanRequest,
    PlanTarget,
};
use crate::observe::{self, ReadError, StableRead};
use crate::platform::macos::Folder;
use crate::store::{Store, sqlite_error};
use crate::{OrganizeService, is_canonical_uuid};

/// Raise when the notice text changes: an older agreement stops counting.
pub const AUTO_DROP_NOTICE_VERSION: u32 = 1;

/// Most dropped files kept in the queue view; older finished ones go.
const MAX_QUEUE_ITEMS: usize = 50;

/// How often a pass looks again while the request after an answer runs on
/// another thread.
const WAIT_STEP: Duration = Duration::from_millis(100);

/// What the user agrees to before the automatic mode starts.
pub fn auto_drop_notice() -> String {
    "When you drop a file on Traeva's icon, Traeva reads it, picks a folder for it and \
     moves it there without asking again, but only into existing folders of the target \
     range you choose here. It never creates folders, never overwrites or renames a file, \
     and never scans or organizes any file you did not drop. Reading a file sends what \
     Organize sends to the model, only with Organize's own agreement. When Traeva is not \
     sure, or anything changed, it stops and lets you choose. You can undo a move, and \
     you can turn this off at any time."
        .to_string()
}

/// The automatic mode as the settings page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoDropSettingsView {
    /// On, and the agreement counts (current notice, at least one valid
    /// folder in the range).
    pub enabled: bool,
    pub notice: String,
    pub notice_version: u32,
    /// The open agreement, if any (it may no longer count: see `problem`).
    pub consent_id: Option<String>,
    pub agreed_at_ms: Option<i64>,
    pub target_root_id: Option<String>,
    pub scopes: Vec<ScopeView>,
    /// Why an open agreement does not count now, for people.
    pub problem: Option<String>,
}

/// One folder of the target range, with its subfolders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeView {
    /// Inside the target root; "" is the root itself.
    pub relative_path: String,
    pub display_path: String,
    /// Still an existing folder of an authorized folder.
    pub valid: bool,
}

/// Turns the automatic mode on (with the target range agreed to) or off.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoDropRequest {
    pub enable: bool,
    /// The notice version the user saw; must be the current one to turn on.
    #[serde(default)]
    pub notice_version: Option<u32>,
    #[serde(default)]
    pub target_root_id: Option<String>,
    /// Existing folders of the target root ("" is the root), at least one.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Where a dropped file stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropState {
    /// In the queue, not started yet.
    Waiting,
    /// Its folder is not authorized: the user agrees (authorized for good)
    /// or cancels.
    NeedsSourceAuthorization,
    /// Being read, analyzed or moved.
    Working,
    /// Automatic mode only: a question waits for the user
    /// (`answer_clarification`, then `continue_drops`, in either order).
    NeedsAnswer,
    /// Left to the normal flow: preview and confirmation, or a manual pick,
    /// in its operation (`operation_id`); with the automatic mode off, the
    /// window starts that operation (`start_drop_operation`).
    HandedOver,
    /// Moved automatically.
    Moved,
    /// Already in its folder: nothing moved.
    AlreadyInPlace,
    /// Stopped, with the reason. Nothing moved, unless `action_id` is set:
    /// then the move had started, and the action's record says.
    Stopped,
    /// Not handled: an earlier file of the same drop stopped the automatic
    /// handling, or was cancelled.
    NotProcessed,
    /// Cancelled by the user.
    Cancelled,
}

impl DropState {
    fn finished(self) -> bool {
        matches!(
            self,
            DropState::HandedOver
                | DropState::Moved
                | DropState::AlreadyInPlace
                | DropState::Stopped
                | DropState::NotProcessed
                | DropState::Cancelled
        )
    }
}

/// One dropped file as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DropItemView {
    pub drop_id: String,
    pub file_name: String,
    /// The drop it came in: files dropped together share it. A stop or a
    /// cancel only reaches the files after it in the same drop (§13.1).
    pub batch_id: String,
    /// When it was dropped, for telling apart drops of the same file.
    pub received_at_ms: i64,
    pub state: DropState,
    /// Whether the automatic mode handled it (false: the normal flow).
    pub automatic: bool,
    /// The folder to authorize, in `needs_source_authorization`.
    pub folder_to_authorize: Option<String>,
    pub operation_id: Option<String>,
    /// The plan run (handed over: the one the normal flow confirmed).
    pub plan_id: Option<String>,
    /// The action: once moved; once the normal flow confirmed a handed-over
    /// file; or when it stopped after the move started (it may have moved:
    /// the action's record says). Its current state, Undo included, is in
    /// `get_action`.
    pub action_id: Option<String>,
    /// Where it went, once moved.
    pub moved_to: Option<String>,
    /// Why it stopped or was handed over, for people. Never file text.
    pub reason: Option<String>,
    /// It waits for the user now (worked out when the queue is read, for
    /// the desktop pet, #208): its folder to authorize, a question, or
    /// a normal flow still open (not started yet, or its operation neither
    /// confirmed, finished nor cancelled). The window ends a normal flow it
    /// leaves without confirming by cancelling its operation.
    pub needs_user: bool,
    /// The current state of `action_id`'s action, when there is one (worked
    /// out when the queue is read): whether the move finished, also for a
    /// file the normal flow confirmed.
    pub action_status: Option<ActionStatus>,
}

/// What the desktop pet may read of the queue (#208): enough for its face,
/// and nothing else. No path, file name, folder or reason: the pet's window
/// may call only this (capabilities/pet-window.json; R1's P03 review).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PetStateView {
    pub items: Vec<PetItemView>,
    /// A background pass is working on the queue.
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PetItemView {
    pub drop_id: String,
    pub state: DropState,
    pub received_at_ms: i64,
    pub needs_user: bool,
    pub action_status: Option<ActionStatus>,
}

/// The queue of dropped files, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DropQueueView {
    pub items: Vec<DropItemView>,
    /// A background pass is working on it.
    pub running: bool,
}

#[derive(Debug, Clone)]
struct DropItem {
    view: DropItemView,
    path: PathBuf,
    root_id: Option<String>,
    consent_id: Option<String>,
    /// Why its final row could not be saved; tried again at the next look.
    unrecorded: Option<String>,
}

/// The queue, in memory.
#[derive(Debug, Default)]
pub(crate) struct Drops {
    items: Vec<DropItem>,
    running: bool,
    /// A call came in while a pass ran (and was turned away): that pass
    /// looks once more before it ends.
    again: bool,
}

/// The open agreement, as saved.
#[derive(Debug, Clone)]
struct ConsentRow {
    consent_id: String,
    notice_version: u32,
    target_root_id: String,
    scopes: Vec<String>,
    agreed_at_ms: i64,
}

/// The agreement that counts now: its ID, target root and valid scopes.
#[derive(Debug, Clone)]
struct Counting {
    consent_id: String,
    target_root_id: String,
    scopes: Vec<Vec<String>>,
}

impl OrganizeService {
    /// The automatic mode now, checked against the folders on disk.
    pub fn auto_drop_settings(&self) -> Result<AutoDropSettingsView, AppError> {
        let store = self.store()?;
        let open = open_consent(store)?;
        Ok(self.auto_drop_view(open.as_ref()))
    }

    /// Turns the automatic mode on or off. On: the user saw the current
    /// notice, the target root is usable, and every scope is an existing
    /// folder of it (at least one). The open agreement is withdrawn and the
    /// new one saved in one transaction. Off withdraws it.
    pub fn set_auto_drop(
        &self,
        request: &AutoDropRequest,
    ) -> Result<AutoDropSettingsView, AppError> {
        let store = self.store()?;
        let now = self.now();
        if !request.enable {
            withdraw_open(store, now)?;
            return self.auto_drop_settings();
        }
        if request.notice_version != Some(AUTO_DROP_NOTICE_VERSION) {
            return Err(invalid(
                "Read the current notice about the automatic mode, then turn it on again.",
            ));
        }
        let root_id = request
            .target_root_id
            .as_deref()
            .ok_or_else(|| invalid("Choose the folder files may be moved into."))?;
        let (_, root) = self.verified_root(root_id)?;
        if request.scopes.is_empty() || request.scopes.len() > 20 {
            return Err(invalid("Choose between 1 and 20 target folders."));
        }
        let existing = existing_folders(&root)?;
        let mut scopes = Vec::new();
        for scope in &request.scopes {
            let segments = segments_of(scope)
                .ok_or_else(|| invalid(&format!("{scope} is not a folder path.")))?;
            if !existing.contains(&segments) {
                return Err(invalid(&format!(
                    "{scope} is not an existing folder Traeva may use."
                )));
            }
            if !scopes.contains(scope) {
                scopes.push(scope.clone());
            }
        }
        let row = ConsentRow {
            consent_id: uuid::Uuid::new_v4().to_string(),
            notice_version: AUTO_DROP_NOTICE_VERSION,
            target_root_id: root_id.to_string(),
            scopes,
            agreed_at_ms: now,
        };
        replace_consent(store, &row, now)?;
        self.auto_drop_settings()
    }

    /// Adds dropped files to the queue, in the order given; then run
    /// `run_drops` in the background. Only reads the paths' kind: nothing is
    /// read from inside a file here, and nothing is written anywhere but
    /// app data.
    pub fn receive_drops(&self, paths: &[PathBuf]) -> Result<DropQueueView, AppError> {
        let store = self.store()?;
        if paths.is_empty() || paths.len() > 100 {
            return Err(invalid("Drop between 1 and 100 files."));
        }
        let batch_id = uuid::Uuid::new_v4().to_string();
        let now = self.now();
        let mut items = Vec::new();
        for (position, path) in paths.iter().enumerate() {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty() && name.len() <= 255)
                .unwrap_or("(unnamed)")
                .to_string();
            let drop_id = uuid::Uuid::new_v4().to_string();
            insert_event(store, &drop_id, &batch_id, position, &file_name, now)?;
            let mut item = DropItem {
                view: DropItemView {
                    drop_id,
                    file_name,
                    batch_id: batch_id.clone(),
                    received_at_ms: now,
                    state: DropState::Waiting,
                    automatic: false,
                    folder_to_authorize: None,
                    operation_id: None,
                    plan_id: None,
                    action_id: None,
                    moved_to: None,
                    reason: None,
                    needs_user: false,
                    action_status: None,
                },
                path: path.clone(),
                root_id: None,
                consent_id: None,
                unrecorded: None,
            };
            // Only files: a folder is never organized as a whole (§13.1).
            let is_file = std::fs::symlink_metadata(path)
                .map(|meta| meta.file_type().is_file())
                .unwrap_or(false);
            if !path.is_absolute() || !is_file {
                self.finish(
                    &mut item,
                    DropState::Stopped,
                    "Only files can be dropped; folders and links are not organized.",
                );
            }
            items.push(item);
        }
        let mut drops = self.drops();
        drops.items.extend(items);
        let extra = drops.items.len().saturating_sub(MAX_QUEUE_ITEMS);
        let finished_old: Vec<usize> = drops
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.view.state.finished())
            .map(|(index, _)| index)
            .take(extra)
            .collect();
        for index in finished_old.into_iter().rev() {
            drops.items.remove(index);
        }
        Ok(self.queue_view(&drops))
    }

    /// The queue as it is now. Asking never starts anything. A file handed
    /// to the normal flow gets the plan and action that flow confirmed, in
    /// the view and in its record (§13.6); what became of an action later
    /// (Undo) is in the action itself (`get_action`).
    /// The queue as the desktop pet may see it: no paths or names (#208).
    pub fn pet_state(&self) -> PetStateView {
        let queue = self.drop_queue();
        PetStateView {
            items: queue
                .items
                .into_iter()
                .map(|item| PetItemView {
                    drop_id: item.drop_id,
                    state: item.state,
                    received_at_ms: item.received_at_ms,
                    needs_user: item.needs_user,
                    action_status: item.action_status,
                })
                .collect(),
            running: queue.running,
        }
    }

    pub fn drop_queue(&self) -> DropQueueView {
        let mut drops = self.drops();
        for item in drops.items.iter_mut() {
            if item.unrecorded.is_some() {
                self.save_end(item);
            }
            if item.view.state == DropState::HandedOver && item.view.action_id.is_none() {
                self.note_normal_flow(item);
            }
        }
        self.queue_view(&drops)
    }

    /// The user agrees to authorize the folder of a file waiting for it: the
    /// folder becomes an authorized folder for good (like Add folder), and
    /// the file goes back to the queue. Run `run_drops` next.
    pub fn authorize_drop_source(&self, drop_id: &str) -> Result<DropQueueView, AppError> {
        check_drop_id(drop_id)?;
        let folder = {
            let drops = self.drops();
            let item = find(&drops, drop_id)?;
            if item.view.state != DropState::NeedsSourceAuthorization {
                return Ok(self.queue_view(&drops));
            }
            item.path.parent().map(Path::to_path_buf)
        };
        let folder = folder.ok_or_else(|| invalid("This file has no folder to authorize."))?;
        self.authorize_root(&folder)?;
        let mut drops = self.drops();
        if let Some(item) = drops
            .items
            .iter_mut()
            .find(|item| item.view.drop_id == drop_id)
            && item.view.state == DropState::NeedsSourceAuthorization
        {
            item.view.state = DropState::Waiting;
            item.view.folder_to_authorize = None;
        }
        Ok(self.queue_view(&drops))
    }

    /// Cancels a file still in the queue (waiting, needing its folder
    /// authorized or an answer) or handed to the normal flow: its operation
    /// is cancelled, and the files after it in the same drop are not
    /// processed (§13.1); later drops go on. A
    /// handed-over file whose plan the user already confirmed is not
    /// cancelled (the operation refuses, §6). A file being worked on, or
    /// already done, stays as it is.
    pub fn cancel_drop(&self, drop_id: &str) -> Result<DropQueueView, AppError> {
        check_drop_id(drop_id)?;
        let (operation, handed_over) = {
            let drops = self.drops();
            let item = find(&drops, drop_id)?;
            if !cancellable(item.view.state) {
                return Ok(self.queue_view(&drops));
            }
            (
                item.view.operation_id.clone(),
                item.view.state == DropState::HandedOver,
            )
        };
        if let Some(operation_id) = operation {
            let cancelled = self.cancel_operation(&operation_id);
            if handed_over {
                cancelled?;
            }
        }
        let mut drops = self.drops();
        let Some(index) = drops
            .items
            .iter()
            .position(|item| item.view.drop_id == drop_id)
            .filter(|&index| cancellable(drops.items[index].view.state))
        else {
            return Ok(self.queue_view(&drops));
        };
        self.finish(&mut drops.items[index], DropState::Cancelled, "Cancelled.");
        self.skip_rest(&mut drops, index);
        Ok(self.queue_view(&drops))
    }

    /// With the automatic mode off, starts the normal operation for a dropped
    /// file whose folder is authorized: the window then reads it
    /// (`run_operation`) and goes on as with a chosen file. The file is
    /// handed over; the next one waits until the window runs `run_drops`.
    pub fn start_drop_operation(
        &self,
        drop_id: &str,
        target_root_id: &str,
    ) -> Result<(OperationView, PathBuf), AppError> {
        check_drop_id(drop_id)?;
        let (source_root_id, path) = {
            let drops = self.drops();
            let item = find(&drops, drop_id)?;
            if item.view.state != DropState::HandedOver || item.view.operation_id.is_some() {
                return Err(invalid("This file is not waiting for the normal flow."));
            }
            let root = item
                .root_id
                .clone()
                .ok_or_else(|| invalid("This file's folder is not authorized."))?;
            (root, item.path.clone())
        };
        let view = self.start_operation(&source_root_id, target_root_id)?;
        let mut drops = self.drops();
        if let Some(item) = drops
            .items
            .iter_mut()
            .find(|item| item.view.drop_id == drop_id)
        {
            item.view.operation_id = Some(view.operation_id.clone());
        }
        Ok((view, path))
    }

    /// Works through the queue in order until it needs the user (a folder
    /// to authorize, an answer, the normal flow) or is done. Slow (reads and
    /// model requests); run it in the background. One pass at a time: a
    /// second call while one runs returns at once, and the running pass
    /// looks once more before it ends (an answer may have come in after it
    /// last looked).
    pub fn run_drops(&self) {
        self.run_drops_with(&mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        });
    }

    /// `run_drops` with the stable read passed in, for tests.
    pub(crate) fn run_drops_with(
        &self,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) {
        {
            let mut drops = self.drops();
            if drops.running {
                drops.again = true;
                return;
            }
            drops.running = true;
        }
        loop {
            // One file at a time; the round stops when a file waits for the
            // user.
            while let Some(mut item) = self.next_item() {
                let go_on = if item.view.state == DropState::NeedsAnswer {
                    self.after_answer(&mut item, read)
                } else {
                    self.start_item(&mut item, read)
                };
                if !self.put_back(item, go_on) {
                    break;
                }
            }
            if self.end_round() {
                break;
            }
        }
    }

    /// The end of a round: true when the pass ends, false when a call was
    /// turned away meanwhile and the pass looks once more.
    fn end_round(&self) -> bool {
        let mut drops = self.drops();
        if std::mem::take(&mut drops.again) {
            return false;
        }
        drops.running = false;
        true
    }

    /// The first unfinished file, if it can go on now: waiting, or its
    /// question answered. A copy is taken out, so no lock is held while it
    /// works; the queue keeps showing the file, as working. None when it
    /// waits for the user (its folder to authorize, the answer, or the
    /// normal flow in the window) or nothing is left.
    fn next_item(&self) -> Option<DropItem> {
        let mut drops = self.drops();
        let item = drops
            .items
            .iter_mut()
            .find(|item| !item.view.state.finished())?;
        let can_go_on = match item.view.state {
            DropState::Waiting => true,
            DropState::NeedsAnswer => !item
                .view
                .operation_id
                .as_ref()
                .and_then(|id| self.get_operation(id).ok())
                .is_some_and(|view| view.stage == OperationStage::NeedsClarification),
            _ => false,
        };
        if !can_go_on {
            return None;
        }
        let copy = item.clone();
        item.view.state = DropState::Working;
        Some(copy)
    }

    fn start_item(
        &self,
        item: &mut DropItem,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> bool {
        let Some(root_id) = self.root_containing(&item.path) else {
            item.view.state = DropState::NeedsSourceAuthorization;
            item.view.folder_to_authorize = item
                .path
                .parent()
                .map(|folder| folder.display().to_string());
            return false;
        };
        item.root_id = Some(root_id.clone());
        let Some(counting) = self.auto_consent_counting() else {
            // Off (or no longer counting): the normal flow, in the window.
            self.finish(
                item,
                DropState::HandedOver,
                "The automatic mode is off: preview and confirm as usual.",
            );
            return false;
        };
        item.view.automatic = true;
        item.consent_id = Some(counting.consent_id.clone());
        item.view.state = DropState::Working;
        let started = match self.start_operation(&root_id, &counting.target_root_id) {
            Ok(view) => view,
            Err(error) => {
                self.finish(item, DropState::Stopped, &error.message);
                return false;
            }
        };
        item.view.operation_id = Some(started.operation_id.clone());
        self.run_operation_with(&started.operation_id, &item.path, read);
        let view = match self.get_operation(&started.operation_id) {
            Ok(view) => view,
            Err(error) => {
                self.finish(item, DropState::Stopped, &error.message);
                return false;
            }
        };
        match view.stage {
            OperationStage::ReadyForAnalysis => {}
            OperationStage::NeedsManualLocation => {
                self.finish(item, DropState::HandedOver, unread_reason(&view));
                return false;
            }
            _ => {
                self.finish(item, DropState::Stopped, &stage_reason(&view));
                return false;
            }
        }
        if let Err(error) = self.start_analysis(&started.operation_id) {
            // No model settings, agreement or key: Organize's own rule (§5).
            self.finish(
                item,
                DropState::HandedOver,
                &format!("{} Choose the folder as usual.", error.message),
            );
            return false;
        }
        self.run_analysis_with(&started.operation_id, read);
        self.decide_drop(item, &counting)
    }

    /// The answer was given (the window ran `answer_clarification`): wait
    /// for the request after it, then decide. The command already runs that
    /// request on another thread; this pass runs it only if nothing does
    /// (`run_analysis_with` returns at once while it runs elsewhere), and
    /// waits no longer than the model's timeout allows.
    fn after_answer(
        &self,
        item: &mut DropItem,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> bool {
        let Some(operation_id) = item.view.operation_id.clone() else {
            self.finish(item, DropState::Stopped, "The question was lost.");
            return false;
        };
        let deadline = Instant::now() + self.model.timeout * 2 + Duration::from_secs(30);
        loop {
            match self.get_operation(&operation_id).map(|view| view.stage) {
                Ok(OperationStage::NeedsClarification) => {
                    item.view.state = DropState::NeedsAnswer;
                    return false;
                }
                Ok(OperationStage::Analyzing) if Instant::now() < deadline => {
                    self.run_analysis_with(&operation_id, read);
                    std::thread::sleep(WAIT_STEP);
                }
                Ok(OperationStage::Analyzing) => {
                    self.finish(
                        item,
                        DropState::HandedOver,
                        "The analysis after your answer takes too long: check it as usual.",
                    );
                    return false;
                }
                _ => break,
            }
        }
        let Some(counting) = self.auto_consent_counting() else {
            self.finish(
                item,
                DropState::HandedOver,
                "The automatic mode was turned off: preview and confirm as usual.",
            );
            return false;
        };
        self.decide_drop(item, &counting)
    }

    /// After the analysis: move when §13.4 holds, otherwise stop or hand
    /// over. True when the queue may go on with the next file.
    fn decide_drop(&self, item: &mut DropItem, counting: &Counting) -> bool {
        let Some(operation_id) = item.view.operation_id.clone() else {
            return false;
        };
        let view = match self.get_operation(&operation_id) {
            Ok(view) => view,
            Err(error) => {
                self.finish(item, DropState::Stopped, &error.message);
                return false;
            }
        };
        match view.stage {
            OperationStage::NeedsClarification => {
                item.view.state = DropState::NeedsAnswer;
                return false;
            }
            OperationStage::RecommendationReady => {}
            OperationStage::NeedsManualLocation => {
                self.finish(
                    item,
                    DropState::HandedOver,
                    "Traeva is not sure where this file goes: choose its folder by hand.",
                );
                return false;
            }
            _ => {
                self.finish(item, DropState::Stopped, &stage_reason(&view));
                return false;
            }
        }
        let suggestion = view.suggestion.as_ref();
        let candidate = suggestion.and_then(|suggestion| {
            let id = suggestion.recommended_id.as_ref()?;
            suggestion
                .candidates
                .iter()
                .find(|candidate| &candidate.candidate_id == id)
        });
        let (Some(candidate), Some(file)) = (candidate, view.file.as_ref()) else {
            self.finish(item, DropState::Stopped, "The recommendation was lost.");
            return false;
        };
        // Text from on-device recognition and a picture the model looked at
        // move like any other text once every condition holds (v0.14, the
        // student's decision 10/15; until then they always stopped for the
        // user). The move can be undone like any other.
        // §13.4 condition 5: an existing folder inside the target range.
        if !candidate.new_segments.is_empty() {
            self.finish(
                item,
                DropState::HandedOver,
                "The best folder does not exist yet: check the preview and confirm.",
            );
            return false;
        }
        let in_range = segments_of(&candidate.folder_path).is_some_and(|folder| {
            counting
                .scopes
                .iter()
                .any(|scope| folder.starts_with(scope))
        });
        if view.target_root_id != counting.target_root_id || !in_range {
            self.finish(
                item,
                DropState::HandedOver,
                "The best folder is outside the target range: check the preview and confirm.",
            );
            return false;
        }
        let request = PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: view.target_root_id.clone(),
            operation_id: Some(operation_id.clone()),
            target: PlanTarget::Candidate {
                candidate_id: candidate.candidate_id.clone(),
            },
        };
        let plan = match self.build_plan(&request) {
            Ok(BuildOutcome::Planned { plan }) => plan,
            Ok(BuildOutcome::AlreadyInPlace { .. }) => {
                self.finish(item, DropState::AlreadyInPlace, "Already in its folder.");
                return true;
            }
            Err(error) => {
                self.finish(item, DropState::Stopped, &error.message);
                return false;
            }
        };
        item.view.plan_id = Some(plan.plan_id.clone());
        if !plan.folders_to_create.is_empty() || !plan.can_confirm {
            let reason = plan
                .reason
                .clone()
                .unwrap_or_else(|| "The plan cannot run.".to_string());
            self.finish(item, DropState::Stopped, &reason);
            return false;
        }
        // The agreement must still count at the moment of confirmation
        // (§13.5): the user may have turned the mode off meanwhile.
        if self.auto_consent_counting().map(|now| now.consent_id)
            != Some(counting.consent_id.clone())
        {
            self.finish(
                item,
                DropState::HandedOver,
                "The automatic mode changed: check the preview and confirm.",
            );
            return false;
        }
        // The row names the agreement and the plan before anything moves
        // (§13.6): after a crash it still leads to the action. If it cannot
        // be saved, nothing moves. The agreement is the one checked just
        // above, also when it replaced an earlier one while a question
        // waited, so the row's end names the same one.
        item.consent_id = Some(counting.consent_id.clone());
        let bound = self.store().and_then(|store| {
            bind_event(
                store,
                &item.view.drop_id,
                item.root_id.as_deref(),
                &file.file_id,
                &counting.consent_id,
                &plan.plan_id,
            )
        });
        if let Err(error) = bound {
            self.finish(
                item,
                DropState::Stopped,
                &format!("{} Nothing was moved.", error.message),
            );
            return false;
        }
        match self.apply_plan(&plan.plan_id) {
            Ok(action) => {
                item.view.action_id = Some(action.action_id.clone());
                if action.status == ActionStatus::Committed {
                    item.view.moved_to = Some(action.target.display_path.clone());
                    self.finish(item, DropState::Moved, "Moved.");
                    // Keeping versions (#201) never changes the move's result.
                    let _ = self.keep_versions_after(&action.action_id);
                    true
                } else {
                    let reason = action
                        .problem
                        .as_ref()
                        .map(|error| error.message.clone())
                        .unwrap_or_else(|| "The file was not moved.".to_string());
                    self.finish(item, DropState::Stopped, &reason);
                    false
                }
            }
            Err(error) => {
                // It may have moved without being recorded as finished: the
                // action is kept, and its record check decides (§7).
                item.view.action_id = own_action(&error);
                self.finish(item, DropState::Stopped, &error.message);
                false
            }
        }
    }

    /// A handed-over file whose normal flow confirmed a plan: the plan and
    /// action go into the view and the record. A record that cannot be
    /// saved is tried again at the next look.
    fn note_normal_flow(&self, item: &mut DropItem) {
        let Some(view) = item
            .view
            .operation_id
            .as_ref()
            .and_then(|id| self.get_operation(id).ok())
        else {
            return;
        };
        let (Some(plan_id), Some(action_id)) = (view.plan_id, view.action_id) else {
            return;
        };
        let file_id = view.file.map(|file| file.file_id);
        let saved = self.store().and_then(|store| {
            note_event_action(
                store,
                &item.view.drop_id,
                file_id.as_deref(),
                &plan_id,
                &action_id,
            )
        });
        if saved.is_ok() {
            item.view.plan_id = Some(plan_id);
            item.view.action_id = Some(action_id);
        }
    }

    /// Sets a final state and saves it in the drop's row. A row that cannot
    /// be saved is shown in the reason and saved again at the next look;
    /// the state in memory still holds.
    fn finish(&self, item: &mut DropItem, state: DropState, reason: &str) {
        item.view.state = state;
        item.view.reason = Some(reason.chars().take(500).collect());
        self.save_end(item);
    }

    /// Saves a finished item's row; remembers why when it cannot.
    fn save_end(&self, item: &mut DropItem) {
        let state = item.view.state;
        let outcome = match state {
            DropState::Moved => "moved",
            DropState::AlreadyInPlace => "already_in_place",
            DropState::HandedOver => "handed_over",
            DropState::NotProcessed => "not_processed",
            DropState::Cancelled => "cancelled",
            _ => "stopped",
        };
        let consent = (state == DropState::Moved || item.view.automatic)
            .then(|| item.consent_id.clone())
            .flatten();
        let file_id = item
            .view
            .operation_id
            .as_ref()
            .and_then(|id| self.get_operation(id).ok())
            .and_then(|view| view.file.map(|file| file.file_id));
        let saved = self.store().and_then(|store| {
            finish_event(
                store,
                &item.view.drop_id,
                &EventEnd {
                    outcome,
                    reason: item.view.reason.as_deref(),
                    root_id: item.root_id.as_deref(),
                    file_id: file_id.as_deref(),
                    consent_id: consent.as_deref(),
                    plan_id: item.view.plan_id.as_deref(),
                    action_id: item.view.action_id.as_deref(),
                    finished_at_ms: self.now(),
                },
            )
        });
        item.unrecorded = saved.err().map(|error| error.message);
    }

    /// The files after `index` in the same drop are not processed (§13.1);
    /// other drops are not touched. One a pass is working on finishes as it
    /// goes.
    fn skip_rest(&self, drops: &mut Drops, index: usize) {
        let batch = drops.items[index].view.batch_id.clone();
        for item in drops.items.iter_mut().skip(index + 1).filter(|item| {
            item.view.batch_id == batch
                && !item.view.state.finished()
                && item.view.state != DropState::Working
        }) {
            self.finish(
                item,
                DropState::NotProcessed,
                "Not processed: an earlier file of the same drop stopped.",
            );
        }
    }

    /// Puts the worked-on copy back in its place (found by ID: files dropped
    /// meanwhile may have moved it). When the automatic handling of it
    /// stopped (or it was handed over), the files after it in the same drop
    /// are not processed, and the pass goes on with the next drop. With the
    /// automatic mode off, a handed-over file just waits for the window.
    fn put_back(&self, item: DropItem, go_on: bool) -> bool {
        let stopped_auto = item.view.automatic
            && matches!(item.view.state, DropState::Stopped | DropState::HandedOver);
        let mut drops = self.drops();
        let Some(index) = drops
            .items
            .iter()
            .position(|old| old.view.drop_id == item.view.drop_id)
        else {
            return false;
        };
        drops.items[index] = item;
        if stopped_auto {
            self.skip_rest(&mut drops, index);
            return true;
        }
        go_on
    }

    /// The active authorized folder that holds `path`, the deepest one.
    /// Only a lookup: registering the file walks the folder by its handle.
    fn root_containing(&self, path: &Path) -> Option<String> {
        let store = self.store().ok()?;
        store
            .active_roots()
            .ok()?
            .into_iter()
            .filter(|row| crate::relative_location(&row.display_path, path).is_ok())
            .max_by_key(|row| row.display_path.len())
            .map(|row| row.root_id)
    }

    /// The open agreement if it counts now: current notice, usable target
    /// root, at least one scope still an existing folder.
    fn auto_consent_counting(&self) -> Option<Counting> {
        let store = self.store().ok()?;
        let open = open_consent(store).ok()??;
        if open.notice_version != AUTO_DROP_NOTICE_VERSION {
            return None;
        }
        let (_, root) = self.verified_root(&open.target_root_id).ok()?;
        let existing = existing_folders(&root).ok()?;
        let scopes: Vec<Vec<String>> = open
            .scopes
            .iter()
            .filter_map(|scope| segments_of(scope))
            .filter(|segments| existing.contains(segments))
            .collect();
        (!scopes.is_empty()).then_some(Counting {
            consent_id: open.consent_id,
            target_root_id: open.target_root_id,
            scopes,
        })
    }

    fn auto_drop_view(&self, open: Option<&ConsentRow>) -> AutoDropSettingsView {
        let mut view = AutoDropSettingsView {
            enabled: false,
            notice: auto_drop_notice(),
            notice_version: AUTO_DROP_NOTICE_VERSION,
            consent_id: None,
            agreed_at_ms: None,
            target_root_id: None,
            scopes: Vec::new(),
            problem: None,
        };
        let Some(open) = open else {
            return view;
        };
        view.consent_id = Some(open.consent_id.clone());
        view.agreed_at_ms = Some(open.agreed_at_ms);
        view.target_root_id = Some(open.target_root_id.clone());
        let checked = self.verified_root(&open.target_root_id);
        let existing = checked
            .as_ref()
            .ok()
            .and_then(|(_, root)| existing_folders(root).ok());
        let root_path = checked
            .as_ref()
            .map(|(row, _)| row.display_path.clone())
            .unwrap_or_default();
        view.scopes = open
            .scopes
            .iter()
            .map(|scope| ScopeView {
                relative_path: scope.clone(),
                display_path: if scope.is_empty() {
                    root_path.clone()
                } else {
                    format!("{root_path}/{scope}")
                },
                valid: segments_of(scope).is_some_and(|segments| {
                    existing.as_ref().is_some_and(|all| all.contains(&segments))
                }),
            })
            .collect();
        view.problem = if open.notice_version != AUTO_DROP_NOTICE_VERSION {
            Some("The notice changed since you agreed: agree again to go on.".to_string())
        } else if let Err(error) = &checked {
            Some(error.message.clone())
        } else if !view.scopes.iter().any(|scope| scope.valid) {
            Some("No folder of the target range exists any more.".to_string())
        } else {
            None
        };
        view.enabled = view.problem.is_none();
        view
    }

    pub(crate) fn drops(&self) -> std::sync::MutexGuard<'_, Drops> {
        self.drops
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// At start, before the actions' record check: rows of files that were
/// still in the queue when Traeva stopped. A row bound to a plan whose
/// action committed (and may since have been undone) was moved: it becomes
/// `moved` with that action. Every other row is `interrupted`, with its
/// action if one started: that action's record check decides (§7), and the
/// row never claims a move that is not settled.
pub(crate) fn interrupt_unfinished(store: &Store, now_ms: i64) -> Result<(), AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot update the dropped files", &err);
    let mut conn = store.connection()?;
    let tx = conn.transaction().map_err(fail)?;
    tx.execute(
        "UPDATE organize_drop_events
         SET outcome = 'moved',
             action_id = (SELECT a.action_id FROM actions a
                          WHERE a.plan_id = organize_drop_events.plan_id),
             reason = 'Moved. Recorded when Traeva started again.',
             finished_at_ms = ?1
         WHERE outcome = 'received' AND consent_id IS NOT NULL AND plan_id IS NOT NULL
           AND EXISTS (SELECT 1 FROM actions a
                       WHERE a.plan_id = organize_drop_events.plan_id
                         AND a.status IN ('committed', 'undone'))",
        params![now_ms],
    )
    .map_err(fail)?;
    tx.execute(
        "UPDATE organize_drop_events
         SET outcome = 'interrupted',
             action_id = (SELECT a.action_id FROM actions a
                          WHERE a.plan_id = organize_drop_events.plan_id),
             reason = CASE
                 WHEN EXISTS (SELECT 1 FROM actions a
                              WHERE a.plan_id = organize_drop_events.plan_id)
                 THEN 'Traeva stopped while this file was being moved: its action''s record check decides.'
                 ELSE 'Traeva stopped before this file was done.'
             END,
             finished_at_ms = ?1
         WHERE outcome = 'received'",
        params![now_ms],
    )
    .map_err(fail)?;
    tx.commit().map_err(fail)
}

/// The action an `apply_plan` error names, if it is the plan's own: the
/// move started. An `unresolved_action` error names another, earlier action
/// that holds the file or a location, not this file's move.
fn own_action(error: &AppError) -> Option<String> {
    if error.code == ErrorCode::UnresolvedAction {
        None
    } else {
        error.action_id.clone()
    }
}

/// A file the user may still cancel: not being worked on, not done (a
/// handed-over file counts as not done for this).
fn cancellable(state: DropState) -> bool {
    state == DropState::HandedOver || !(state.finished() || state == DropState::Working)
}

impl OrganizeService {
    /// The queue as the windows see it, with what each file waits for and
    /// how its move ended worked out now (#208).
    fn queue_view(&self, drops: &Drops) -> DropQueueView {
        let items = drops
            .items
            .iter()
            .map(|item| {
                let mut view = item.view.clone();
                if let Some(error) = &item.unrecorded {
                    let reason = view.reason.unwrap_or_default();
                    view.reason = Some(format!("{reason} (Not recorded: {error})"));
                }
                view.needs_user = self.needs_user(&view);
                view.action_status = view
                    .action_id
                    .as_ref()
                    .and_then(|id| self.get_action(id).ok())
                    .map(|action| action.status);
                view
            })
            .collect();
        DropQueueView {
            items,
            running: drops.running,
        }
    }

    /// Whether the file waits for the user: see `DropItemView::needs_user`.
    fn needs_user(&self, view: &DropItemView) -> bool {
        match view.state {
            DropState::NeedsSourceAuthorization | DropState::NeedsAnswer => true,
            DropState::HandedOver if view.action_id.is_none() => match &view.operation_id {
                // The window has not started its normal flow yet.
                None => true,
                Some(id) => self
                    .get_operation(id)
                    .is_ok_and(|operation| flow_is_open(operation.stage)),
            },
            _ => false,
        }
    }
}

/// An operation the user can still act on (the stages `cancel_operation`
/// cancels).
fn flow_is_open(stage: OperationStage) -> bool {
    matches!(
        stage,
        OperationStage::Observing
            | OperationStage::ReadyForAnalysis
            | OperationStage::Analyzing
            | OperationStage::NeedsClarification
            | OperationStage::RecommendationReady
            | OperationStage::NeedsManualLocation
            | OperationStage::AnalysisFailed
            | OperationStage::InvalidModelOutput
    )
}

fn find<'a>(drops: &'a Drops, drop_id: &str) -> Result<&'a DropItem, AppError> {
    drops
        .items
        .iter()
        .find(|item| item.view.drop_id == drop_id)
        .ok_or_else(|| invalid("There is no dropped file with this ID."))
}

fn check_drop_id(drop_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(drop_id) {
        Ok(())
    } else {
        Err(invalid("The dropped file's ID is not valid."))
    }
}

fn invalid(message: &str) -> AppError {
    AppError::analysis(ErrorCode::InvalidInput, message, false, None)
}

/// Why a dropped file whose text cannot be read goes to the manual pick. A
/// scanned PDF dropped while text recognition gets ready (often the drop
/// that started Traeva) is not read again by itself (§14.8), so the queue
/// says to drop it again. An encrypted PDF is not read at all (§14), even
/// one that opens without a password, so the queue says that (10/15, the
/// student's question about a protected handbook).
fn unread_reason(view: &OperationView) -> &'static str {
    let note = view
        .file
        .as_ref()
        .and_then(|file| file.extraction.note.as_deref());
    if note == Some(crate::ocr::OCR_GETTING_READY_NOTE) {
        "Text recognition is still getting ready (the first time can take about a minute): \
         drop the file again in a moment, or choose its folder by hand."
    } else if note == Some(crate::pdf::ENCRYPTED_NOTE) {
        "This PDF is protected by its author, so Traeva does not read it: choose its folder by hand."
    } else {
        "Traeva cannot read text in this file: choose its folder by hand."
    }
}

/// Why an operation stopped, for people: its error, or its stage.
fn stage_reason(view: &OperationView) -> String {
    view.error
        .as_ref()
        .map(|error| error.message.clone())
        .unwrap_or_else(|| format!("Stopped ({:?}).", view.stage))
}

/// "School/English" → ["School", "English"]; "" → []. None for a path that
/// is absolute, has an empty, "." or ".." segment.
fn segments_of(path: &str) -> Option<Vec<String>> {
    if path.is_empty() {
        return Some(Vec::new());
    }
    let segments: Vec<String> = path.split('/').map(str::to_string).collect();
    segments
        .iter()
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        .then_some(segments)
}

/// The folders the model may suggest in this root, as segment lists (the
/// same bounded listing as the analysis; protected folders are left out).
fn existing_folders(root: &Folder) -> Result<Vec<Vec<String>>, AppError> {
    let catalog = observe::build_catalog(root).map_err(|_| {
        AppError::authorization(
            ErrorCode::RootUnavailable,
            "Traeva could not list the folders of the target range.",
        )
    })?;
    Ok(catalog
        .folders
        .into_iter()
        .filter(|folder| folder.protected.is_none())
        .map(|folder| folder.segments)
        .collect())
}

// The two tables of migration 0010.

fn open_consent(store: &Store) -> Result<Option<ConsentRow>, AppError> {
    let conn = store.connection()?;
    let row = conn
        .query_row(
            "SELECT consent_id, notice_version, target_root_id, scopes_json, agreed_at_ms
             FROM organize_auto_consents WHERE withdrawn_at_ms IS NULL",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|err| sqlite_error("Cannot read the automatic mode", &err))?;
    let Some((consent_id, version, target_root_id, scopes_json, agreed_at_ms)) = row else {
        return Ok(None);
    };
    let scopes: Vec<String> = serde_json::from_str(&scopes_json).map_err(|_| {
        AppError::storage(
            ErrorCode::DatabaseCorrupt,
            "The automatic mode's saved folders cannot be read.",
        )
    })?;
    Ok(Some(ConsentRow {
        consent_id,
        notice_version: u32::try_from(version).unwrap_or(0),
        target_root_id,
        scopes,
        agreed_at_ms,
    }))
}

fn replace_consent(store: &Store, row: &ConsentRow, now_ms: i64) -> Result<(), AppError> {
    let fail = |err: rusqlite::Error| sqlite_error("Cannot save the automatic mode", &err);
    let mut conn = store.connection()?;
    let tx = conn.transaction().map_err(fail)?;
    tx.execute(
        "UPDATE organize_auto_consents SET withdrawn_at_ms = ?1 WHERE withdrawn_at_ms IS NULL",
        params![now_ms],
    )
    .map_err(fail)?;
    let scopes = serde_json::to_string(&row.scopes).map_err(|_| {
        AppError::storage(
            ErrorCode::DatabaseCorrupt,
            "The automatic mode's folders cannot be saved.",
        )
    })?;
    tx.execute(
        "INSERT INTO organize_auto_consents
             (consent_id, notice_version, target_root_id, scopes_json, agreed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            row.consent_id,
            row.notice_version,
            row.target_root_id,
            scopes,
            row.agreed_at_ms
        ],
    )
    .map_err(fail)?;
    tx.commit().map_err(fail)
}

fn withdraw_open(store: &Store, now_ms: i64) -> Result<(), AppError> {
    let conn = store.connection()?;
    conn.execute(
        "UPDATE organize_auto_consents SET withdrawn_at_ms = ?1 WHERE withdrawn_at_ms IS NULL",
        params![now_ms],
    )
    .map_err(|err| sqlite_error("Cannot turn the automatic mode off", &err))?;
    Ok(())
}

fn insert_event(
    store: &Store,
    drop_id: &str,
    batch_id: &str,
    position: usize,
    file_name: &str,
    now_ms: i64,
) -> Result<(), AppError> {
    let conn = store.connection()?;
    conn.execute(
        "INSERT INTO organize_drop_events
             (drop_id, batch_id, position, file_name, received_at_ms, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, 'received')",
        params![drop_id, batch_id, position as i64, file_name, now_ms],
    )
    .map_err(|err| sqlite_error("Cannot record the dropped file", &err))?;
    Ok(())
}

struct EventEnd<'a> {
    outcome: &'a str,
    reason: Option<&'a str>,
    root_id: Option<&'a str>,
    file_id: Option<&'a str>,
    consent_id: Option<&'a str>,
    plan_id: Option<&'a str>,
    action_id: Option<&'a str>,
    finished_at_ms: i64,
}

fn finish_event(store: &Store, drop_id: &str, end: &EventEnd<'_>) -> Result<(), AppError> {
    let conn = store.connection()?;
    conn.execute(
        "UPDATE organize_drop_events
         SET outcome = ?2, reason = ?3, root_id = ?4, file_id = ?5, consent_id = ?6,
             plan_id = ?7, action_id = ?8, finished_at_ms = ?9
         WHERE drop_id = ?1",
        params![
            drop_id,
            end.outcome,
            end.reason,
            end.root_id,
            end.file_id,
            end.consent_id,
            end.plan_id,
            end.action_id,
            end.finished_at_ms
        ],
    )
    .map_err(|err| sqlite_error("Cannot record what became of the dropped file", &err))?;
    Ok(())
}

/// Before an automatic move: the row names the folder, file, agreement and
/// plan it runs under (still `received`). Fails when the row is gone or
/// already finished.
fn bind_event(
    store: &Store,
    drop_id: &str,
    root_id: Option<&str>,
    file_id: &str,
    consent_id: &str,
    plan_id: &str,
) -> Result<(), AppError> {
    let conn = store.connection()?;
    let changed = conn
        .execute(
            "UPDATE organize_drop_events
             SET root_id = ?2, file_id = ?3, consent_id = ?4, plan_id = ?5
             WHERE drop_id = ?1 AND outcome = 'received'",
            params![drop_id, root_id, file_id, consent_id, plan_id],
        )
        .map_err(|err| sqlite_error("Cannot record the dropped file's plan", &err))?;
    if changed == 1 {
        Ok(())
    } else {
        Err(AppError::storage(
            ErrorCode::DatabaseCorrupt,
            "The dropped file's record is missing.",
        ))
    }
}

/// The plan and action the normal flow confirmed for a handed-over file.
fn note_event_action(
    store: &Store,
    drop_id: &str,
    file_id: Option<&str>,
    plan_id: &str,
    action_id: &str,
) -> Result<(), AppError> {
    let conn = store.connection()?;
    conn.execute(
        "UPDATE organize_drop_events
         SET file_id = coalesce(?2, file_id), plan_id = ?3, action_id = ?4
         WHERE drop_id = ?1 AND outcome = 'handed_over'",
        params![drop_id, file_id, plan_id, action_id],
    )
    .map_err(|err| sqlite_error("Cannot record what became of the dropped file", &err))?;
    Ok(())
}

#[cfg(test)]
#[path = "auto_drop_tests.rs"]
mod tests;
