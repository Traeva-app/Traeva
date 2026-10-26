//! Conservative Undo (organize-spec.md §8). Student material for #11:
//! https://github.com/P30g53/Traeva/issues/11#issuecomment-6001832176
//! (shared with the Recovery part of #10).
//!
//! Undo puts a file back where an organize action took it from. It only
//! changes the location: it never restores old bytes, never calls a model,
//! never overwrites, renames or picks another place, and never re-creates a
//! deleted folder.
//!
//!   1. `preview_undo` checks the organize action and the files, then saves
//!      a new, immutable undo plan (kind `undo`, no folders to create). It
//!      only looks; nothing is written inside a root.
//!   2. Confirming that plan runs it through the same executor as any plan
//!      (#9). Its success commits the undo, the organize action's `undone`,
//!      the file record and one path-history entry in one transaction.
//!   3. Then the empty folders the organize action created are removed with
//!      the record check's rules (#10). A folder that stays is the cleanup
//!      warning; the file is back either way.
//!
//! An interrupted undo is checked like any action (#10), with the reverse
//! plan's two ends.

use crate::contracts::{
    ActionStatus, AppError, ErrorCode, PlanKind, PlanLifecycle, UndoPreview, UndoStatus,
};
use crate::execute::{is_expected_file, unresolved};
use crate::observe::{self, ReadError, StableRead};
use crate::plan::{chain_matches, links, load_plan, new_folder_paths, open_names, split_path};
use crate::platform::macos::{self, Folder, FsError};
use crate::recover::is_unsettled;
use crate::store::{
    ActionRow, Entry, ExpectedSource, FolderReceipt, JournalRow, MoveResult, PlanRow, Store,
};
use crate::{OrganizeService, evidence_of, is_canonical_uuid};

/// What the checks before an undo found: the file at its organized place,
/// read in full, and the folders on both sides.
struct UndoCheck {
    current: StableRead,
    current_chain: Vec<Folder>,
    original_chain: Vec<Folder>,
}

impl OrganizeService {
    /// Asks to undo organize action `action_id` (organize-spec.md §9
    /// `preview_undo`). Returns a new undo plan to confirm, the undo that
    /// already put the file back, or the undo that is in progress. Anything
    /// that changed since the organize action is an `undo_conflict`.
    pub fn preview_undo(&self, action_id: &str) -> Result<UndoPreview, AppError> {
        let store = self.store()?;
        let original = load_action(store, action_id)?;
        let nothing = |message: &str| {
            AppError::execution(
                ErrorCode::NothingToUndo,
                message,
                None,
                None,
                Some(action_id),
            )
        };
        if original.kind == PlanKind::Undo {
            return Err(nothing(
                "This action is an undo; it cannot be undone. Organize the file again instead.",
            ));
        }
        // An undo that already put the file back, or one still in progress.
        for (undo_id, status) in store.undo_actions_of(action_id)?.into_iter().rev() {
            if status == ActionStatus::Committed {
                return Ok(UndoPreview::AlreadyUndone {
                    undo: self.get_action(&undo_id)?,
                });
            }
            if is_unsettled(status) {
                return Ok(UndoPreview::UndoInProgress {
                    undo: self.get_action(&undo_id)?,
                });
            }
        }
        if original.status != ActionStatus::Committed {
            return Err(nothing(
                "Only a file Traeva moved can be put back, and this action did not move one.",
            ));
        }
        let organize = load_plan(store, &original.plan_id)?;
        let committed = committed_file(store, action_id)?.ok_or_else(|| {
            nothing("The record of this move is incomplete, so it cannot be undone.")
        })?;

        let check = self.check_undo(&organize, &committed, action_id)?;
        let (file, _) = store.get_file(&organize.file_id)?.ok_or_else(|| {
            nothing("The file's record is missing, so this move cannot be undone.")
        })?;
        let (original_folders, _) = split_path(&organize.source_relative_path);
        let plan = PlanRow {
            plan_id: uuid::Uuid::new_v4().to_string(),
            kind: PlanKind::Undo,
            file_id: organize.file_id.clone(),
            source_root_id: organize.target_root_id.clone(),
            source_relative_path: organize.target_path(),
            expected_source: ExpectedSource {
                file_revision: file.record_revision,
                sha256: check.current.sha256.clone(),
                byte_length: check.current.bytes.len() as u64,
                evidence: evidence_of(&check.current.observed),
                folder_chain: links(&check.current_chain),
            },
            target_root_id: organize.source_root_id.clone(),
            target_segments: original_folders,
            basename: organize.basename.clone(),
            folder_chain: links(&check.original_chain),
            folders_to_create: Vec::new(),
            created_at_ms: self.now(),
            lifecycle: PlanLifecycle::Open,
            lifecycle_changed_at_ms: None,
            lifecycle_reason: None,
            action_id: None,
            undo_of_action_id: Some(action_id.to_string()),
            user_choice: None,
            analysis_request_id: None,
        };
        // Another unsettled action involving the file or either location.
        if let Some(busy) = store.conflicting_action(&plan)? {
            return Err(unresolved(&plan.file_id, None, &busy));
        }
        store.insert_plan(&plan)?;
        Ok(UndoPreview::Planned {
            plan: self.plan_view(store, &plan)?,
        })
    }

    /// The material's checks before an undo (organize-spec.md §8): both
    /// roots authorized and on one volume; the file still at its organized
    /// place, the same object with the same bytes as when it was committed;
    /// every folder above its old place still the recorded one; the old name
    /// free. Something that changed is an `undo_conflict`; a file or folder
    /// that cannot be read right now is reported as that, never as a pass.
    fn check_undo(
        &self,
        organize: &PlanRow,
        committed: &MoveResult,
        action_id: &str,
    ) -> Result<UndoCheck, AppError> {
        let conflict = |why: &str| {
            AppError::execution(
                ErrorCode::UndoConflict,
                format!("{why} The file stays where it is."),
                Some(&organize.file_id),
                None,
                Some(action_id),
            )
        };
        // A changed or unauthorized folder is a conflict; one that cannot be
        // opened right now is an error to try again later.
        let root = |root_id: &str| -> Result<Folder, AppError> {
            match self.verified_root(root_id) {
                Ok((_, root)) => Ok(root),
                Err(error) if error.code == ErrorCode::RootUnavailable => Err(error),
                Err(error) => Err(conflict(&error.message)),
            }
        };
        let current_root = root(&organize.target_root_id)?;
        let original_root = root(&organize.source_root_id)?;
        if current_root.id().dev != original_root.id().dev {
            return Err(conflict("The two folders are no longer on one volume."));
        }
        let unreadable = |what: &str| {
            AppError::observation(
                ErrorCode::FileUnreadable,
                format!("Traeva could not read {what}, so it cannot check the undo."),
            )
        };

        // The file, where Traeva put it.
        let (names, name) = split_path(&organize.target_path());
        let current_chain = match open_names(&current_root, &names) {
            Ok(chain) => chain,
            Err(FsError::PermissionDenied | FsError::Other(_)) => {
                return Err(unreadable("the folder the file is in"));
            }
            Err(_) => {
                return Err(conflict(
                    "The folder Traeva put the file in was moved, replaced or deleted.",
                ));
            }
        };
        let parent = current_chain.last().unwrap_or(&current_root);
        match macos::observe(parent, &name) {
            Ok(found) if is_expected_file(&found, &committed.evidence) => {}
            Ok(_) => {
                return Err(conflict(
                    "Another item is where Traeva put the file. Even a copy with the same \
                     content is not the file Traeva moved.",
                ));
            }
            Err(FsError::NotFound) => {
                return Err(conflict(
                    "The file is no longer where Traeva put it; it was moved or deleted.",
                ));
            }
            Err(_) => return Err(unreadable("the file")),
        }
        let current = match observe::read_stable(parent, &name, current_root.id().dev) {
            Ok(read) => read,
            Err(ReadError::StillChanging { .. }) => {
                return Err(AppError::observation(
                    ErrorCode::FileChanging,
                    "The file is being changed right now. Try again when it is saved.",
                ));
            }
            Err(ReadError::Fs(FsError::PermissionDenied | FsError::Other(_))) => {
                return Err(unreadable("the file"));
            }
            Err(_) => return Err(conflict("The file changed after Traeva moved it.")),
        };
        if !is_expected_file(&current.observed, &committed.evidence) {
            return Err(conflict("The file was replaced after Traeva moved it."));
        }
        if current.sha256 != committed.sha256 {
            return Err(conflict(
                "The file was edited after Traeva moved it. Undo only moves files; it never \
                 brings back old content.",
            ));
        }

        // The old place: every folder above it still the recorded one, and
        // the old name free. A deleted folder is never re-created.
        let (old_names, old_name) = split_path(&organize.source_relative_path);
        let original_chain = match open_names(&original_root, &old_names) {
            Ok(chain) if chain_matches(&chain, &organize.expected_source.folder_chain) => chain,
            Err(FsError::PermissionDenied | FsError::Other(_)) => {
                return Err(unreadable("the folder the file came from"));
            }
            _ => {
                return Err(conflict(
                    "The folder the file came from was moved, replaced or deleted. Undo does \
                     not re-create it.",
                ));
            }
        };
        match macos::observe(original_chain.last().unwrap_or(&original_root), &old_name) {
            Err(FsError::NotFound) => {}
            Ok(_) => {
                return Err(conflict(
                    "Something else now has the file's old name there. Undo never replaces \
                     it or uses another name.",
                ));
            }
            Err(_) => return Err(unreadable("the folder the file came from")),
        }
        Ok(UndoCheck {
            current,
            current_chain,
            original_chain,
        })
    }

    /// After an undo is committed (live or by a record check): remove the
    /// empty folders the organize action created, innermost first, with the
    /// record check's rules and the intent and result in the undo's journal.
    /// Nothing here can undo the undo: a folder that stays is only the
    /// cleanup warning.
    pub(crate) fn clean_up_after_undo(&self, store: &Store, undo_id: &str, original_id: &str) {
        let Ok(Some(original)) = store.get_action(original_id) else {
            return;
        };
        let Ok(plan) = load_plan(store, &original.plan_id) else {
            return;
        };
        let Ok(rows) = store.journal(original_id) else {
            return;
        };
        let receipts = receipts_in_order(&plan, &rows);
        // A database error here leaves the remaining folders in place; they
        // show up in the undo's `folders_kept`.
        let _ = self.remove_empty_folders(store, undo_id, &plan, &receipts);
    }
}

/// Whether an organize action can be undone, from saved records only, and
/// its latest undo that is committed or in progress. `root_revoked`: a root
/// of its plan was removed from Traeva (also a saved record).
pub(crate) fn undo_status(
    action: &ActionRow,
    undos: &[(String, ActionStatus)],
    root_revoked: bool,
) -> (UndoStatus, Option<String>) {
    if action.kind == PlanKind::Undo {
        return (UndoStatus::NotAvailable, None);
    }
    if let Some((id, _)) = undos
        .iter()
        .rev()
        .find(|(_, status)| *status == ActionStatus::Committed)
    {
        return (UndoStatus::Undone, Some(id.clone()));
    }
    if let Some((id, _)) = undos.iter().rev().find(|(_, status)| is_unsettled(*status)) {
        return (UndoStatus::InProgress, Some(id.clone()));
    }
    match action.status {
        ActionStatus::Committed if root_revoked => (UndoStatus::RootRevoked, None),
        ActionStatus::Committed => (UndoStatus::Available, None),
        _ => (UndoStatus::NotAvailable, None),
    }
}

/// The receipts of an organize action, in the order its plan creates the
/// folders. A receipt out of that order is not used.
pub(crate) fn receipts_in_order(plan: &PlanRow, rows: &[JournalRow]) -> Vec<FolderReceipt> {
    let paths = new_folder_paths(plan);
    let mut receipts = Vec::new();
    for row in rows {
        if let Some(Entry::FolderReceipt(receipt)) = &row.entry {
            if paths.get(receipts.len()) != Some(&receipt.path) {
                break;
            }
            receipts.push(receipt.clone());
        }
    }
    receipts
}

/// What the organize action's final transaction recorded about the moved
/// file: its bytes' hash and its identity at the new place.
fn committed_file(store: &Store, action_id: &str) -> Result<Option<MoveResult>, AppError> {
    Ok(store
        .journal(action_id)?
        .into_iter()
        .find_map(|row| match row.entry {
            Some(Entry::MoveResult(result)) => Some(result),
            _ => None,
        }))
}

fn load_action(store: &Store, action_id: &str) -> Result<ActionRow, AppError> {
    let refuse =
        |code: ErrorCode, message: &str| AppError::execution(code, message, None, None, None);
    if !is_canonical_uuid(action_id) {
        return Err(refuse(
            ErrorCode::InvalidInput,
            "The action ID is not valid.",
        ));
    }
    store
        .get_action(action_id)?
        .ok_or_else(|| refuse(ErrorCode::UnknownAction, "There is no action with this ID."))
}

#[cfg(test)]
#[path = "undo_tests.rs"]
mod tests;
