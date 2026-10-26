//! Record checks after an interruption (organize-spec.md §7). Student
//! material for #10:
//! https://github.com/P30g53/Traeva/issues/10#issuecomment-5988968392
//!
//! A record check only looks and records. It never moves, renames,
//! overwrites or deletes a user's file, never finishes an interrupted move
//! and never moves anything back. The one change it may make on disk is the
//! spec's single exception: removing an empty folder that the action itself
//! created and holds a receipt for.
//!
//! Every action that is not settled (prepared, running, needs_user_decision)
//! is checked at startup, oldest first, before anything else can run:
//!
//!   1. An intact journal without a move intent means the move was never
//!      called: failed_no_move (the material's supplement).
//!   2. Otherwise look at both names, read-only:
//!      - the source is still the expected file with the same bytes, the
//!        target is empty, roots and folder chains are valid → failed_no_move;
//!      - the source is empty, the target is the expected file with the same
//!        bytes, and roots, chains and journal all match → committed, with
//!        the file record and one path-history entry in one transaction;
//!      - anything else → needs_user_decision.
//!
//!      Reading a file takes a moment (two full reads with a pause). A
//!      decision based on such a read is only kept if both ends still look
//!      the same afterwards: roots, folders on the way and both names, by
//!      identity (#46), and the file that was read is still the very version
//!      that was read: same size, times and flags (#54). Otherwise →
//!      needs_user_decision.
//!   3. Only once a failed_no_move is saved: remove the action's empty
//!      folders that it holds receipts for, innermost first, each with an
//!      intent and a result in the journal.

use crate::contracts::{ActionStatus, ActionView, AppError, ErrorCode};
use crate::execute::is_expected_file;
use crate::observe::{self, ReadError, StableRead};
use crate::plan::{chain_matches, load_plan, new_folder_paths, open_names, split_path};
use crate::platform::macos::{self, Folder, FsError};
use crate::store::{
    ActionProblem, ActionRow, Basis, Entry, FolderReceipt, FolderRemoval, FolderToRemove, Found,
    JournalRow, MoveIntent, MoveRecord, MoveResult, Phase, PlanRow, Reconciliation, Store,
    same_volume,
};
use crate::{OrganizeService, evidence_of};

/// Reads a file in full, twice with a pause (`observe::read_stable`). Tests
/// pass their own, to change things while a file is being read.
pub(crate) type Read<'a> = dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError> + 'a;

/// The facts a record check takes from a journal once it is known to be
/// intact and to match the plan.
struct JournalFacts {
    receipts: Vec<FolderReceipt>,
    move_intent: bool,
}

/// What a record check saw at one end. `NotChecked` covers a root that is
/// no longer authorized or no longer the same folder, and any folder on the
/// way that is not the expected one: nothing found behind them counts.
struct Look {
    found: Found,
    /// The stable read, when the expected file is there unchanged.
    read: Option<StableRead>,
}

impl Look {
    fn not_checked() -> Look {
        Look {
            found: Found::NotChecked,
            read: None,
        }
    }
}

impl OrganizeService {
    /// At startup, before anything else can run: check every unsettled
    /// action. A database error stops the check, and the caller blocks
    /// storage with it; the next start checks the same actions again.
    pub(crate) fn check_unsettled_actions(&self) -> Result<(), AppError> {
        let store = self.store()?;
        let _turn = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for action_id in store.unresolved_actions()? {
            self.check_action(store, &action_id, &mut observe::read_stable)?;
        }
        Ok(())
    }

    /// Checks one action again on request (organize-spec.md §9
    /// `reconcile_action`), with the same rules as at startup. A settled
    /// action is only reported. This never moves anything.
    pub fn reconcile_action(&self, action_id: &str) -> Result<ActionView, AppError> {
        self.reconcile_action_with(action_id, &mut observe::read_stable)
    }

    /// `reconcile_action` with the file read passed in, so tests can change
    /// things while a file is being read.
    pub(crate) fn reconcile_action_with(
        &self,
        action_id: &str,
        read: &mut Read<'_>,
    ) -> Result<ActionView, AppError> {
        self.get_action(action_id)?; // checks the ID and that the action exists
        let store = self.store()?;
        // A run of this action may still hold the lock; check after it.
        let _turn = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(action) = store.get_action(action_id)?
            && is_unsettled(action.status)
        {
            self.check_action(store, action_id, read)?;
        }
        self.get_action(action_id)
    }

    fn check_action(
        &self,
        store: &Store,
        action_id: &str,
        read: &mut Read<'_>,
    ) -> Result<(), AppError> {
        let Some(action) = store.get_action(action_id)? else {
            return Ok(());
        };
        let plan = load_plan(store, &action.plan_id)?;
        let rows = store.journal(action_id)?;
        let journal = read_journal(&action, &plan, &rows);
        let receipts = journal.as_ref().map_or(&[][..], |j| j.receipts.as_slice());
        let source = self.look_at_source(&plan, read);
        let target = self.look_at_target(&plan, receipts, read);
        let (mut status, mut basis) = decide(journal.as_ref(), &source, &target);
        let (mut source_now, mut target_now) = (source.found, target.found);
        // Both decisions that rest on a read must still hold after it: look at
        // both ends again without reading (#46). The end that was read must
        // also still be the very version that was read (#54).
        if matches!(basis, Basis::MovedAsPlanned | Basis::StillAtSource) {
            let again = (
                self.glance_at_source(&plan, source.read.as_ref()),
                self.glance_at_target(&plan, receipts, target.read.as_ref()),
            );
            if again != (source_now, target_now) {
                (status, basis) = (ActionStatus::NeedsUserDecision, Basis::Uncertain);
                (source_now, target_now) = again;
            }
        }
        let check = Reconciliation {
            status,
            basis,
            journal_matches: journal.is_some(),
            source_now,
            target_now,
        };
        let now = self.now();

        if status == ActionStatus::Committed {
            let Some(moved) = &target.read else {
                return Ok(()); // decide() only says committed after a read
            };
            let result = MoveResult {
                sha256: moved.sha256.clone(),
                evidence: evidence_of(&moved.observed),
            };
            store.commit_recovered_move(
                &MoveRecord {
                    action_id,
                    file_id: &plan.file_id,
                    from_root_id: &plan.source_root_id,
                    from_path: &plan.source_relative_path,
                    to_root_id: &plan.target_root_id,
                    to_path: &plan.target_path(),
                    basename: &plan.basename,
                    byte_length: moved.bytes.len() as u64,
                    result: &result,
                    now_ms: now,
                    undo_of: plan.undo_of_action_id.as_deref(),
                },
                &check,
            )?;
            // A recovered undo cleans up after itself just like a live one.
            if let Some(original) = &plan.undo_of_action_id {
                self.clean_up_after_undo(store, action_id, original);
            }
            return Ok(());
        }

        // Already waiting for a decision, and nothing new was found: keep
        // the record as it is, so checking again adds nothing.
        if action.status == ActionStatus::NeedsUserDecision
            && status == ActionStatus::NeedsUserDecision
            && last_check(&rows) == Some(&check)
        {
            return Ok(());
        }
        store.record_check(action_id, &problem(&check), &check, now)?;
        if status == ActionStatus::FailedNoMove
            && let Some(journal) = &journal
        {
            self.remove_empty_folders(store, action_id, &plan, &journal.receipts)?;
        }
        Ok(())
    }

    /// The source end: the root, the folders above the file, then the file.
    fn look_at_source(&self, plan: &PlanRow, read: &mut Read<'_>) -> Look {
        match self.source_folder(plan) {
            Some((parent, dev)) => look_at(&parent, &source_name(plan), plan, dev, read),
            None => Look::not_checked(),
        }
    }

    /// The target end: the root, the existing folders, each folder this
    /// action created (still the one in its receipt), then the file name.
    fn look_at_target(
        &self,
        plan: &PlanRow,
        receipts: &[FolderReceipt],
        read: &mut Read<'_>,
    ) -> Look {
        match self.target_folder(plan, receipts) {
            Some((parent, dev)) => look_at(&parent, &plan.basename, plan, dev, read),
            None => Look::not_checked(),
        }
    }

    /// The source end again, without reading: what is at the name now.
    fn glance_at_source(&self, plan: &PlanRow, read: Option<&StableRead>) -> Found {
        match self.source_folder(plan) {
            Some((parent, _)) => glance(&parent, &source_name(plan), plan, read),
            None => Found::NotChecked,
        }
    }

    /// The target end again, without reading: what is at the name now.
    fn glance_at_target(
        &self,
        plan: &PlanRow,
        receipts: &[FolderReceipt],
        read: Option<&StableRead>,
    ) -> Found {
        match self.target_folder(plan, receipts) {
            Some((parent, _)) => glance(&parent, &plan.basename, plan, read),
            None => Found::NotChecked,
        }
    }

    /// The folder that should hold the file at the source, opened again from
    /// the root, with the root's device. None when the root or a folder on
    /// the way is not the expected one.
    fn source_folder(&self, plan: &PlanRow) -> Option<(Folder, i64)> {
        let (_, root) = self.verified_root(&plan.source_root_id).ok()?;
        let (names, _) = split_path(&plan.source_relative_path);
        let mut chain = match open_names(&root, &names) {
            Ok(chain) if chain_matches(&chain, &plan.expected_source.folder_chain) => chain,
            _ => return None,
        };
        let dev = root.id().dev;
        Some((chain.pop().unwrap_or(root), dev))
    }

    /// The folder that should hold the file at the target, opened again
    /// from the root. None when the root, an existing folder or a folder
    /// this action created changed, or not every folder was created (then
    /// the target was never reached).
    fn target_folder(&self, plan: &PlanRow, receipts: &[FolderReceipt]) -> Option<(Folder, i64)> {
        let (_, root) = self.verified_root(&plan.target_root_id).ok()?;
        let mut chain = open_target_chain(&root, plan, receipts)?;
        if chain.len() != plan.target_segments.len() {
            return None;
        }
        let dev = root.id().dev;
        Some((chain.pop().unwrap_or(root), dev))
    }

    /// The material's exception: after a failed_no_move is saved, remove
    /// this action's empty folders that it holds receipts for, innermost
    /// first. A folder stays when it is not empty (a `.DS_Store` counts), is
    /// not the folder in its receipt, its root or chain changed, or another
    /// unsettled action relies on it. Every removal has an intent before it
    /// and a result after it; without a saved intent nothing is removed.
    pub(crate) fn remove_empty_folders(
        &self,
        store: &Store,
        action_id: &str,
        plan: &PlanRow,
        receipts: &[FolderReceipt],
    ) -> Result<(), AppError> {
        if receipts.is_empty() {
            return Ok(());
        }
        let Ok((_, root)) = self.verified_root(&plan.target_root_id) else {
            return Ok(());
        };
        let Some(mut chain) = open_target_chain(&root, plan, receipts) else {
            return Ok(());
        };
        let existing = plan.target_segments.len() - plan.folders_to_create.len();
        let in_use = folders_in_use(store, action_id)?;
        let paths = new_folder_paths(plan);
        while chain.len() > existing {
            let Some(folder) = chain.pop() else { break };
            let path = &paths[chain.len() - existing];
            let used = in_use.iter().any(|saved| saved.is(&folder));
            if !macos::is_empty_folder(&folder).unwrap_or(false) || used {
                break; // it stays, and so does every folder around it
            }
            store.append_event(
                action_id,
                "cleanup",
                Phase::Intent,
                &FolderToRemove {
                    path: path.clone(),
                    dev: folder.id().dev,
                    ino: folder.id().ino,
                },
                self.now(),
            )?;
            let parent = chain.last().unwrap_or(&root);
            let removed = matches!(
                macos::remove_empty_folder(parent, folder.name(), folder.id()),
                Ok(()) | Err(FsError::ChangedButUnconfirmed)
            );
            store.append_event(
                action_id,
                "cleanup",
                Phase::Result,
                &FolderRemoval {
                    path: path.clone(),
                    removed,
                },
                self.now(),
            )?;
            if !removed {
                break;
            }
        }
        Ok(())
    }
}

/// prepared, running and needs_user_decision are not settled yet.
pub(crate) fn is_unsettled(status: ActionStatus) -> bool {
    matches!(
        status,
        ActionStatus::Prepared | ActionStatus::Running | ActionStatus::NeedsUserDecision
    )
}

/// after a crash, decide if the file was moved, not moved, or if the user has to decide
fn decide(journal: Option<&JournalFacts>, source: &Look, target: &Look) -> (ActionStatus, Basis) {
    let has_journal = journal.is_some();
    let has_move_intent = match journal {
        Some(facts) => facts.move_intent,
        None => false,
    };
    let still_at_source = source.found == Found::ExpectedFile && target.found == Found::Nothing;
    let moved_to_target = source.found == Found::Nothing && target.found == Found::ExpectedFile;

    if has_journal && !has_move_intent {
        return (ActionStatus::FailedNoMove, Basis::NoMoveIntent);
    }
    if still_at_source {
        return (ActionStatus::FailedNoMove, Basis::StillAtSource);
    }
    if has_journal && moved_to_target {
        return (ActionStatus::Committed, Basis::MovedAsPlanned);
    }
    (ActionStatus::NeedsUserDecision, Basis::Uncertain)
}

/// read the journal and check it against the plan, return None if anything doesn't match
fn read_journal(action: &ActionRow, plan: &PlanRow, rows: &[JournalRow]) -> Option<JournalFacts> {
    let folders = new_folder_paths(plan);
    let mut receipts: Vec<FolderReceipt> = Vec::new();
    let mut waiting: Option<String> = None;
    let mut move_intent = false;
    let mut index = 0;
    while index < rows.len() {
        let row = &rows[index];
        if row.sequence != index as i64 + 1 {
            return None;
        }
        let entry = match &row.entry {
            Some(entry) => entry,
            None => return None,
        };
        if index == 0 {
            match entry {
                Entry::Confirmed(confirmed) => {
                    if confirmed.plan_id != action.plan_id {
                        return None;
                    }
                }
                _ => return None,
            }
        } else {
            match entry {
                Entry::FolderIntent(intent) => {
                    let next_folder = folders.get(receipts.len());
                    if move_intent || waiting.is_some() || next_folder != Some(&intent.path) {
                        return None;
                    }
                    waiting = Some(intent.path.clone());
                }
                Entry::FolderReceipt(receipt) => {
                    if waiting.as_ref() != Some(&receipt.path) {
                        return None;
                    }
                    waiting = None;
                    receipts.push(receipt.clone());
                }
                Entry::MoveIntent(intent) => {
                    if move_intent
                        || waiting.is_some()
                        || receipts.len() != folders.len()
                        || !matches_plan(intent, plan)
                    {
                        return None;
                    }
                    move_intent = true;
                }
                Entry::Settlement(_) | Entry::Reconciliation(_) => {}
                _ => return None,
            }
        }
        index += 1;
    }
    if rows.is_empty() {
        return None;
    }
    Some(JournalFacts {
        receipts,
        move_intent,
    })
}

fn matches_plan(intent: &MoveIntent, plan: &PlanRow) -> bool {
    let expected = &plan.expected_source;
    intent.from_root_id == plan.source_root_id
        && intent.from_path == plan.source_relative_path
        && intent.to_root_id == plan.target_root_id
        && intent.to_path == plan.target_path()
        && intent.sha256 == expected.sha256
        && intent.byte_length == expected.byte_length
        && intent.evidence == expected.evidence
}

/// What is at `name` now, read-only. The expected object is read in full
/// (twice, with a pause) to compare its bytes with the plan's hash.
fn look_at(
    parent: &Folder,
    name: &str,
    plan: &PlanRow,
    root_dev: i64,
    read: &mut Read<'_>,
) -> Look {
    let expected = &plan.expected_source;
    let (found, read) = match macos::observe(parent, name) {
        Err(FsError::NotFound) => (Found::Nothing, None),
        Err(_) => (Found::NotChecked, None),
        Ok(now) if is_expected_file(&now, &expected.evidence) => match read(parent, name, root_dev)
        {
            Ok(read)
                if is_expected_file(&read.observed, &expected.evidence)
                    && read.sha256 == expected.sha256 =>
            {
                (Found::ExpectedFile, Some(read))
            }
            _ => (Found::ChangedFile, None),
        },
        Ok(_) => (Found::SomethingElse, None),
    };
    Look { found, read }
}

/// What is at `name` now, without reading it. `ExpectedFile` means the
/// expected file object; when this end was just read (`read`), it must also
/// still be exactly the version that read saw: same size, times and flags.
/// Otherwise the bytes may have changed since, and it is a `ChangedFile`.
fn glance(parent: &Folder, name: &str, plan: &PlanRow, read: Option<&StableRead>) -> Found {
    match macos::observe(parent, name) {
        Err(FsError::NotFound) => Found::Nothing,
        Err(_) => Found::NotChecked,
        Ok(now) if !is_expected_file(&now, &plan.expected_source.evidence) => Found::SomethingElse,
        Ok(now) if read.is_some_and(|read| now != read.observed) => Found::ChangedFile,
        Ok(_) => Found::ExpectedFile,
    }
}

fn source_name(plan: &PlanRow) -> String {
    split_path(&plan.source_relative_path).1
}

/// The existing target folders (still the planned ones), then each folder
/// the action created, as long as it is still the folder in its receipt.
/// None when the existing part changed.
fn open_target_chain(
    root: &Folder,
    plan: &PlanRow,
    receipts: &[FolderReceipt],
) -> Option<Vec<Folder>> {
    let existing = plan.target_segments.len() - plan.folders_to_create.len();
    let mut chain = match open_names(root, &plan.target_segments[..existing]) {
        Ok(chain) if chain_matches(&chain, &plan.folder_chain) => chain,
        _ => return None,
    };
    for (receipt, name) in receipts.iter().zip(&plan.folders_to_create) {
        let parent = chain.last().unwrap_or(root);
        let Ok(mut next) = macos::open_chain(parent, &[name.as_str()]) else {
            break;
        };
        let folder = next.remove(0);
        let observed = folder.observation();
        if observed.id.ino != receipt.ino
            || observed.birthtime_ns != receipt.birthtime_ns
            || !same_volume(&receipt.volume, &macos::volume_uuid(observed.id.dev))
        {
            break;
        }
        chain.push(folder);
    }
    Some(chain)
}

/// A folder another unsettled action relies on, as it was saved.
struct SavedFolder {
    volume: Option<String>,
    ino: u64,
}

impl SavedFolder {
    /// The open `folder` may be this saved one: the same inode number on the
    /// same volume (`same_volume`). When unsure, it counts, so the folder stays.
    fn is(&self, folder: &Folder) -> bool {
        self.ino == folder.id().ino
            && same_volume(&self.volume, &macos::volume_uuid(folder.id().dev))
    }
}

/// Folders other unsettled actions rely on: the folders above their source,
/// the existing folders of their target, and the folders they created.
fn folders_in_use(store: &Store, except: &str) -> Result<Vec<SavedFolder>, AppError> {
    let mut saved = Vec::new();
    for (action_id, plan) in store.unresolved_plans()? {
        if action_id == except {
            continue;
        }
        // Its roots too: a folder this action created may since have been
        // authorized as a root of its own (#48).
        for root_id in [&plan.source_root_id, &plan.target_root_id] {
            if let Some(root) = store.get_root(root_id)? {
                saved.push(SavedFolder {
                    volume: root.evidence.volume,
                    ino: root.evidence.ino,
                });
            }
        }
        for link in plan
            .expected_source
            .folder_chain
            .iter()
            .chain(&plan.folder_chain)
        {
            saved.push(SavedFolder {
                volume: link.volume.clone(),
                ino: link.ino,
            });
        }
        for row in store.journal(&action_id)? {
            if let Some(Entry::FolderReceipt(receipt)) = row.entry {
                saved.push(SavedFolder {
                    volume: receipt.volume,
                    ino: receipt.ino,
                });
            }
        }
    }
    Ok(saved)
}

fn last_check(rows: &[JournalRow]) -> Option<&Reconciliation> {
    rows.iter().rev().find_map(|row| match &row.entry {
        Some(Entry::Reconciliation(check)) => Some(check),
        _ => None,
    })
}

/// The reason saved with the action, for people.
fn problem(check: &Reconciliation) -> ActionProblem {
    let (code, message) = match check.basis {
        Basis::NoMoveIntent => (
            ErrorCode::RunInterrupted,
            "Traeva stopped before it moved the file (it quit, crashed or could not record \
             the result). The file was not moved. Preview again to organize it."
                .to_string(),
        ),
        Basis::StillAtSource => (
            ErrorCode::RunInterrupted,
            "Traeva stopped around the move, but the file is still at its old location, \
             unchanged, and nothing is at the new location. The file was not moved. Preview \
             again to organize it."
                .to_string(),
        ),
        Basis::MovedAsPlanned | Basis::Uncertain => (
            ErrorCode::ResultUnconfirmed,
            format!(
                "Traeva cannot tell what happened to the file. At its old location: {}. At the \
                 new location: {}.{} Nothing will be moved automatically; check both locations.",
                describe(check.source_now),
                describe(check.target_now),
                if check.journal_matches {
                    ""
                } else {
                    " The action's journal does not match its plan."
                }
            ),
        ),
    };
    ActionProblem { code, message }
}

fn describe(found: Found) -> &'static str {
    match found {
        Found::Nothing => "nothing",
        Found::ExpectedFile => "the file, unchanged",
        Found::ChangedFile => "the file, but not with the previewed content",
        Found::SomethingElse => "a different item",
        Found::NotChecked => "not checked (a folder or its authorization changed)",
    }
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;

/// O18-C (#21): the same record check after the process really was killed.
#[cfg(test)]
#[path = "crash_tests.rs"]
mod crash_tests;
