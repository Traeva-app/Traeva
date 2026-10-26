//! Confirming a plan and running it (organize-spec.md §2.3, §6). Student
//! material for #9:
//! https://github.com/P30g53/Traeva/issues/9#issuecomment-5987940299
//!
//! One plan becomes at most one action: the database enforces
//! `actions.plan_id NOT NULL UNIQUE`, and asking again returns that action.
//! Every change on disk follows the same order:
//!
//!   1. save the intent in the journal and commit it,
//!   2. make the change (the fd-relative mkdir or no-replace move of #6),
//!   3. check the result, then save it.
//!
//! If an intent cannot be saved, its change is not made. No database
//! transaction is open while a folder or file is changed. Success is
//! reported only after the move was checked and the final transaction (file
//! record, path history, action status) committed.
//!
//! Once the action exists, every failure ends in `settle`: `failed_no_move`
//! when there is enough evidence the file did not move, `needs_user_decision`
//! otherwise. Nothing is retried or moved back automatically; trying again
//! takes a new plan and a new confirmation.

use crate::contracts::{ActionStatus, ActionView, AppError, ErrorCode, RootStatus};
use crate::observe::{self, ReadError, StableRead};
use crate::plan::{ReadyPlan, Revalidation, is_expired, load_plan, new_folder_paths, plan_ends};
use crate::platform::macos::{self, Folder, FsError, Kind, NewFolderName, ObjectId, Observation};
use crate::store::{
    ActionProblem, Confirmation, Entry, FileEvidence, FolderIntent, FolderReceipt, JournalRow,
    MoveIntent, MoveRecord, MoveResult, Phase, PlanRow, Preconditions, Seen, Settlement, Store,
};
use crate::{OrganizeService, evidence_of, is_canonical_uuid, undo};

/// Points in a run where tests step in, for example to put a file at the
/// target at the worst possible moment. The app passes a function that does
/// nothing.
// The folder index is only read by tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Checkpoint {
    /// The intent for new folder `n` (0 = outermost) is saved; mkdir is next.
    BeforeMkdir(usize),
    /// New folder `n` was created; checking it is next.
    AfterMkdir(usize),
    /// Every new folder exists; the check before the move is next.
    BeforeMoveCheck,
    /// The move intent is saved; the move is next.
    BeforeMove,
    /// The move call returned success; the checks after the move are next.
    AfterMove,
}

/// How far a failed run got. This decides the final status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// The move was never tried, so this action did not move the file.
    BeforeMove,
    /// The move call failed. Both names are looked at before "not moved".
    MoveFailed,
    /// The move happened, or may have: never "nothing happened".
    AfterMove,
}

#[derive(Debug)]
struct Failure {
    reached: Reached,
    code: ErrorCode,
    message: String,
}

impl Failure {
    fn new(reached: Reached, code: ErrorCode, message: impl Into<String>) -> Failure {
        Failure {
            reached,
            code,
            message: message.into(),
        }
    }
}

/// What a run holds: the plan and the handles the pre-execution check
/// opened, plus every folder this action created.
struct Run<'a> {
    store: &'a Store,
    action_id: String,
    plan: PlanRow,
    source_root: Folder,
    source_chain: Vec<Folder>,
    target_root: Folder,
    /// The existing target folders, then each folder this action created.
    target_chain: Vec<Folder>,
}

impl Run<'_> {
    fn source_parent(&self) -> &Folder {
        self.source_chain.last().unwrap_or(&self.source_root)
    }

    fn target_parent(&self) -> &Folder {
        self.target_chain.last().unwrap_or(&self.target_root)
    }

    /// Every folder of the target path exists (the existing ones and the
    /// ones this action created).
    fn target_ready(&self) -> bool {
        self.target_chain.len() == self.plan.target_segments.len()
    }

    fn expected(&self) -> &FileEvidence {
        &self.plan.expected_source.evidence
    }
}

impl OrganizeService {
    /// Confirms plan `plan_id` and runs it (organize-spec.md §6 `apply_plan`).
    /// Returns the action: committed, or failed with the real reason. A plan
    /// an action already used returns that action and never runs again.
    pub fn apply_plan(&self, plan_id: &str) -> Result<ActionView, AppError> {
        self.apply_plan_with(
            plan_id,
            &mut |parent, name, root_dev| observe::read_stable(parent, name, root_dev),
            &mut |_| {},
        )
    }

    /// `apply_plan` with the stable read and the checkpoints passed in, so
    /// tests can skip the pause and change things in the middle of a run.
    pub(crate) fn apply_plan_with(
        &self,
        plan_id: &str,
        read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
        checkpoint: &mut dyn FnMut(Checkpoint),
    ) -> Result<ActionView, AppError> {
        let store = self.store()?;
        let plan = load_plan(store, plan_id)?;
        // A double click, or a retry after a timeout: report, never run again.
        if let Some(action_id) = &plan.action_id {
            return self.get_action(action_id);
        }
        // One run at a time. This lock is taken before any database lock.
        let _turn = self
            .execution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A click that waited here while this same plan ran gets that action,
        // whatever its result, before the file's other actions are looked at.
        let plan = load_plan(store, plan_id)?;
        if let Some(action_id) = &plan.action_id {
            return self.get_action(action_id);
        }
        // An unsettled action of this file, or one at either of its locations.
        if let Some(busy) = store.conflicting_action(&plan)? {
            return Err(unresolved(&plan.file_id, Some(plan_id), &busy));
        }
        // #8's checks ①–⑦ (a failed one ends the plan, no action is created).
        let ready = match self.revalidate_plan_with(plan_id, read)? {
            Revalidation::AlreadyConsumed { action_id } => return self.get_action(&action_id),
            Revalidation::Ready(ready) => ready,
        };

        // The confirmation: one short transaction.
        let now = self.now();
        let stale = |message: &str| {
            AppError::plan(
                ErrorCode::StalePlan,
                message,
                Some(&ready.plan.file_id),
                Some(plan_id),
            )
        };
        if is_expired(ready.plan.created_at_ms, now) {
            return Err(stale("This plan is older than 15 minutes. Preview again."));
        }
        let action_id = uuid::Uuid::new_v4().to_string();
        let confirmation = store
            .confirm_plan(&ready.plan, &action_id, &preconditions(&ready), now)
            .map_err(|error| AppError {
                file_id: Some(ready.plan.file_id.clone()),
                plan_id: Some(plan_id.to_string()),
                ..error
            })?;
        match confirmation {
            Confirmation::Created => {}
            Confirmation::AlreadyUsed(existing) => return self.get_action(&existing),
            Confirmation::PlanClosed => {
                return Err(stale("This plan can no longer run. Preview again."));
            }
            Confirmation::RootRevoked => {
                return Err(stale(
                    "A folder this plan uses is no longer authorized. Preview again.",
                ));
            }
            Confirmation::FileBusy(busy) => {
                return Err(unresolved(&ready.plan.file_id, Some(plan_id), &busy));
            }
            Confirmation::NotUndoable => {
                // The organize action was undone or changed meanwhile: this
                // undo plan can never run. Preview the undo again.
                return Err(self.invalidate_plan(
                    store,
                    &ready.plan,
                    ErrorCode::NothingToUndo,
                    "the organize action can no longer be undone",
                    "The organize action this undo belongs to is no longer committed. Nothing \
                     was moved.",
                ));
            }
        }

        // The plan is used now. From here on, every failure goes to `settle`.
        let mut run = Run {
            store,
            action_id,
            plan: ready.plan,
            source_root: ready.source_root,
            source_chain: ready.source_chain,
            target_root: ready.target_root,
            target_chain: ready.target_chain,
        };
        match self.run(&mut run, read, checkpoint) {
            Ok(()) => {
                // A committed undo then removes the organize action's empty folders.
                if let Some(original) = &run.plan.undo_of_action_id {
                    self.clean_up_after_undo(store, &run.action_id, original);
                }
                self.get_action(&run.action_id)
            }
            Err(failure) => self.settle(&run, failure),
        }
    }

    /// The saved action, read again from the database. Never runs anything.
    pub fn get_action(&self, action_id: &str) -> Result<ActionView, AppError> {
        let store = self.store()?;
        let refuse =
            |code: ErrorCode, message: &str| AppError::execution(code, message, None, None, None);
        if !is_canonical_uuid(action_id) {
            return Err(refuse(
                ErrorCode::InvalidInput,
                "The action ID is not valid.",
            ));
        }
        let action = store
            .get_action(action_id)?
            .ok_or_else(|| refuse(ErrorCode::UnknownAction, "There is no action with this ID."))?;
        let plan = load_plan(store, &action.plan_id)?;
        let (source, target) = plan_ends(store, &plan)?;
        let journal = store.journal(action_id)?;
        let folders = FolderSummary::of(&journal);
        let mut root_revoked = false;
        for root_id in [&plan.source_root_id, &plan.target_root_id] {
            if matches!(store.get_root(root_id)?, Some(row) if row.status == RootStatus::Revoked) {
                root_revoked = true;
            }
        }
        let (undo_status, undo_action_id) =
            undo::undo_status(&action, &store.undo_actions_of(action_id)?, root_revoked);
        // For a committed undo: the organize action's folders it did not remove.
        let folders_kept = match &plan.undo_of_action_id {
            Some(original) if action.status == ActionStatus::Committed => {
                folders_kept_after_undo(store, original, &folders.removed)?
            }
            _ => Vec::new(),
        };
        let problem = action.problem.map(|problem| {
            AppError::execution(
                problem.code,
                problem.message,
                Some(&plan.file_id),
                Some(&plan.plan_id),
                Some(action_id),
            )
        });
        Ok(ActionView {
            action_id: action.action_id,
            plan_id: plan.plan_id,
            kind: action.kind,
            file_id: plan.file_id,
            status: action.status,
            source,
            target,
            folders_created: folders.created,
            folders_removed: folders.removed,
            folders_without_receipt: folders.without_receipt,
            folders_kept,
            created_at_ms: action.created_at_ms,
            committed_at_ms: action.committed_at_ms,
            recovered_at_ms: action.recovered_at_ms,
            problem,
            undo_of_action_id: plan.undo_of_action_id.clone(),
            undo_status,
            undo_action_id,
        })
    }

    /// Everything after the confirmation, in the order of the student material.
    fn run(
        &self,
        run: &mut Run,
        read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
        checkpoint: &mut dyn FnMut(Checkpoint),
    ) -> Result<(), Failure> {
        let store = run.store;
        let not_moved = |code: ErrorCode, message: String| {
            Failure::new(
                Reached::BeforeMove,
                code,
                format!("{message} Nothing was moved."),
            )
        };

        // prepared → running, saved before anything changes on disk.
        store.mark_running(&run.action_id).map_err(|error| {
            not_moved(
                error.code,
                format!(
                    "Traeva could not record that it started ({}).",
                    error.message
                ),
            )
        })?;

        // New folders, outermost first: intent → mkdir → check → receipt.
        let paths = new_folder_paths(&run.plan);
        for (index, path) in paths.iter().enumerate() {
            let name = NewFolderName::parse(&run.plan.folders_to_create[index]).map_err(|_| {
                not_moved(
                    ErrorCode::InvalidFolderName,
                    format!("“{path}” is not a valid folder name."),
                )
            })?;
            store
                .append_event(
                    &run.action_id,
                    "mkdir",
                    Phase::Intent,
                    &FolderIntent { path: path.clone() },
                    self.now(),
                )
                .map_err(|error| {
                    not_moved(
                        error.code,
                        format!(
                            "Traeva could not record that it is about to create “{path}”, so it \
                             did not create it ({}).",
                            error.message
                        ),
                    )
                })?;
            checkpoint(Checkpoint::BeforeMkdir(index));
            let folder = macos::create_folder(run.target_parent(), &name).map_err(|error| {
                match error {
                    FsError::AlreadyExists => not_moved(
                        ErrorCode::TargetConflict,
                        format!(
                            "“{path}” already exists, so Traeva did not use it. Choose another folder."
                        ),
                    ),
                    FsError::ChangedButUnconfirmed => not_moved(
                        ErrorCode::OperationRefused,
                        format!(
                            "“{path}” was created but could not be checked, so Traeva does not \
                             treat it as its own."
                        ),
                    ),
                    other => not_moved(
                        ErrorCode::OperationRefused,
                        format!("macOS did not create “{path}” ({}).", refusal(&other)),
                    ),
                }
            })?;
            run.target_chain.push(folder);
            checkpoint(Checkpoint::AfterMkdir(index));
            // The new folder must be at its name, under the unchanged folders.
            macos::verify_chain(&run.target_root, &run.target_chain).map_err(|_| {
                not_moved(
                    ErrorCode::StalePlan,
                    format!(
                        "A target folder changed while Traeva created “{path}”, so Traeva does \
                         not treat it as its own."
                    ),
                )
            })?;
            let created = run.target_parent().observation();
            let receipt = FolderReceipt {
                path: path.clone(),
                dev: created.id.dev,
                volume: macos::volume_uuid(created.id.dev),
                ino: created.id.ino,
                birthtime_ns: created.birthtime_ns,
            };
            store
                .append_event(&run.action_id, "mkdir", Phase::Result, &receipt, self.now())
                .map_err(|error| {
                    not_moved(
                        error.code,
                        format!(
                            "“{path}” was created, but its receipt could not be saved ({}).",
                            error.message
                        ),
                    )
                })?;
        }

        // Right before the move: everything again, with a full hash.
        checkpoint(Checkpoint::BeforeMoveCheck);
        self.check_before_move(run, read)?;
        let to_path = run.plan.target_path();
        let intent = MoveIntent {
            from_root_id: run.plan.source_root_id.clone(),
            from_path: run.plan.source_relative_path.clone(),
            to_root_id: run.plan.target_root_id.clone(),
            to_path: to_path.clone(),
            sha256: run.plan.expected_source.sha256.clone(),
            byte_length: run.plan.expected_source.byte_length,
            evidence: run.expected().clone(),
        };
        store
            .append_event(&run.action_id, "move", Phase::Intent, &intent, self.now())
            .map_err(|error| {
                not_moved(
                    error.code,
                    format!(
                        "Traeva could not record that it is about to move the file, so it did \
                         not move it ({}).",
                        error.message
                    ),
                )
            })?;

        // The move: one atomic step that never replaces anything.
        checkpoint(Checkpoint::BeforeMove);
        let name = run.plan.basename.as_str();
        match macos::move_no_replace(run.source_parent(), name, run.target_parent(), name) {
            Ok(()) => {}
            Err(FsError::AlreadyExists) => {
                return Err(Failure::new(
                    Reached::MoveFailed,
                    ErrorCode::TargetConflict,
                    format!(
                        "Something named “{to_path}” appeared just before the move. Nothing was \
                         replaced; choose another folder."
                    ),
                ));
            }
            Err(FsError::ChangedButUnconfirmed) => {
                return Err(Failure::new(
                    Reached::AfterMove,
                    ErrorCode::ResultUnconfirmed,
                    "The file was moved, but macOS could not confirm the change is saved on \
                     disk. Nothing will be moved back automatically.",
                ));
            }
            Err(other) => {
                return Err(Failure::new(
                    Reached::MoveFailed,
                    ErrorCode::OperationRefused,
                    format!("macOS did not move the file ({}).", refusal(&other)),
                ));
            }
        }

        // The move call succeeded. That alone is not success.
        checkpoint(Checkpoint::AfterMove);
        let moved = self.check_after_move(run, read)?;

        // The final transaction. Only once it commits is the action a success.
        let result = MoveResult {
            sha256: moved.sha256,
            evidence: evidence_of(&moved.observed),
        };
        store
            .commit_move(&MoveRecord {
                action_id: &run.action_id,
                file_id: &run.plan.file_id,
                from_root_id: &run.plan.source_root_id,
                from_path: &run.plan.source_relative_path,
                to_root_id: &run.plan.target_root_id,
                to_path: &to_path,
                basename: name,
                byte_length: moved.bytes.len() as u64,
                result: &result,
                now_ms: self.now(),
                undo_of: run.plan.undo_of_action_id.as_deref(),
            })
            .map_err(|error| {
                Failure::new(
                    Reached::AfterMove,
                    ErrorCode::ResultUnconfirmed,
                    format!(
                        "The file was moved and checked, but the record could not be saved ({}). \
                         Nothing will be moved back automatically.",
                        error.message
                    ),
                )
            })
    }

    /// Right before the move intent: both roots still authorized and the same
    /// folders, both folder chains unchanged, and the file still exactly the
    /// previewed one (identity and a full hash).
    fn check_before_move(
        &self,
        run: &Run,
        read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
    ) -> Result<(), Failure> {
        let not_moved = |code: ErrorCode, message: &str| {
            Failure::new(
                Reached::BeforeMove,
                code,
                format!("{message} Nothing was moved."),
            )
        };
        self.roots_unchanged(run)
            .map_err(|error| not_moved(error.code, &error.message))?;
        macos::verify_chain(&run.source_root, &run.source_chain).map_err(|_| {
            not_moved(
                ErrorCode::FileChanged,
                "A folder above the file was moved or replaced.",
            )
        })?;
        macos::verify_chain(&run.target_root, &run.target_chain).map_err(|_| {
            not_moved(
                ErrorCode::StalePlan,
                "A target folder was moved or replaced.",
            )
        })?;
        let source = match read(
            run.source_parent(),
            &run.plan.basename,
            run.source_root.id().dev,
        ) {
            Ok(source) => source,
            Err(ReadError::StillChanging { .. }) => {
                return Err(not_moved(
                    ErrorCode::FileChanging,
                    "The file was being changed.",
                ));
            }
            Err(_) => {
                return Err(not_moved(
                    ErrorCode::FileChanged,
                    "The file moved, was replaced or could not be read.",
                ));
            }
        };
        if !is_expected_file(&source.observed, run.expected()) {
            return Err(not_moved(
                ErrorCode::FileChanged,
                "The file was replaced by another file.",
            ));
        }
        if source.sha256 != run.plan.expected_source.sha256 {
            return Err(not_moved(
                ErrorCode::FileChanged,
                "The file's content changed after the preview.",
            ));
        }
        Ok(())
    }

    /// After the move: the old name is empty, the target name is the moved
    /// file with exactly the previewed bytes, and both chains and both roots
    /// are unchanged. (The move itself already made both folders durable.)
    ///
    /// Reading the content takes a moment (two full reads with a pause), so
    /// the names, chains and roots are checked before the read and again
    /// after it (#46). A change during the read is never recorded as success.
    /// After the read, the file must also still be exactly the version that
    /// was read (#54): any change to it, even with its size and modification
    /// time put back, moves its change time, and the hash just read would
    /// then be stale.
    fn check_after_move(
        &self,
        run: &Run,
        read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
    ) -> Result<StableRead, Failure> {
        let unconfirmed = |why: &str| {
            Failure::new(
                Reached::AfterMove,
                ErrorCode::ResultUnconfirmed,
                format!(
                    "The file was moved, but Traeva could not confirm the result: {why} Nothing \
                     will be moved back automatically."
                ),
            )
        };
        let name = run.plan.basename.as_str();
        // The file was checked on the source root's volume; its device
        // number is today's, not the one saved in the plan.
        let moved_id = ObjectId {
            dev: run.source_root.id().dev,
            ino: run.expected().ino,
        };
        // Returns what the target name shows now.
        let still_in_place = || {
            let at_target = macos::confirm_moved(
                run.source_parent(),
                name,
                run.target_parent(),
                name,
                moved_id,
            )
            .map_err(|_| {
                unconfirmed("the old name is not empty, or the target is not the moved file.")
            })?;
            macos::verify_chain(&run.source_root, &run.source_chain)
                .and_then(|()| macos::verify_chain(&run.target_root, &run.target_chain))
                .map_err(|_| unconfirmed("a folder on the way changed during the move."))?;
            self.roots_unchanged(run)
                .map_err(|_| unconfirmed("an authorized folder changed during the move."))?;
            Ok(at_target)
        };
        still_in_place()?;
        let moved = read(run.target_parent(), name, run.target_root.id().dev)
            .map_err(|_| unconfirmed("the moved file could not be read to check its content."))?;
        if !is_expected_file(&moved.observed, run.expected())
            || moved.sha256 != run.plan.expected_source.sha256
        {
            return Err(unconfirmed(
                "the file at the target is not exactly the previewed file.",
            ));
        }
        // The read took a moment: everything must still be where it was, and
        // the file must still be the version that was read.
        if still_in_place()? != moved.observed {
            return Err(unconfirmed("the moved file changed after Traeva read it."));
        }
        Ok(moved)
    }

    /// Both roots are still authorized, and the folders at their paths are
    /// still the ones whose handles this run holds.
    fn roots_unchanged(&self, run: &Run) -> Result<(), AppError> {
        for (root_id, held) in [
            (&run.plan.source_root_id, &run.source_root),
            (&run.plan.target_root_id, &run.target_root),
        ] {
            let (_, now) = self.verified_root(root_id)?;
            if now.id() != held.id() {
                return Err(AppError::execution(
                    ErrorCode::StalePlan,
                    "An authorized folder was moved or replaced.",
                    None,
                    None,
                    None,
                ));
            }
        }
        Ok(())
    }

    /// The one failure handling of the student material: look at both
    /// names, decide `failed_no_move` or `needs_user_decision`, and try to
    /// save that. If even that cannot be saved, the storage error is returned
    /// as it is: the journal saved so far stays, and nothing claims the
    /// failure was recorded.
    fn settle(&self, run: &Run, failure: Failure) -> Result<ActionView, AppError> {
        let name = run.plan.basename.as_str();
        let source_now = seen(run.source_parent(), name, run.expected());
        let target_now = if run.target_ready() {
            seen(run.target_parent(), name, run.expected())
        } else {
            Seen::Unknown
        };
        // Enough evidence that this action did not move the file: the move
        // was never tried, or the move call failed while the old name still
        // holds the file and the target name does not.
        let not_moved = match failure.reached {
            Reached::BeforeMove => true,
            Reached::MoveFailed => {
                source_now == Seen::ExpectedFile && target_now != Seen::ExpectedFile
            }
            Reached::AfterMove => false,
        };
        let (status, problem) = if not_moved {
            (
                ActionStatus::FailedNoMove,
                ActionProblem {
                    code: failure.code,
                    message: failure.message,
                },
            )
        } else if failure.reached == Reached::MoveFailed {
            (
                ActionStatus::NeedsUserDecision,
                ActionProblem {
                    code: ErrorCode::ResultUnconfirmed,
                    message: format!(
                        "{} Traeva cannot tell where the file is now. Nothing will be moved \
                         automatically.",
                        failure.message
                    ),
                },
            )
        } else {
            (
                ActionStatus::NeedsUserDecision,
                ActionProblem {
                    code: failure.code,
                    message: failure.message,
                },
            )
        };
        let settlement = Settlement {
            status,
            code: problem.code,
            source_now,
            target_now,
        };
        match run
            .store
            .settle_action(&run.action_id, &problem, &settlement, self.now())
        {
            Ok(()) => self.get_action(&run.action_id),
            Err(error) => Err(AppError {
                message: format!(
                    "{} The action could not be recorded as finished, so it still shows as in \
                     progress.",
                    error.message
                ),
                file_id: Some(run.plan.file_id.clone()),
                plan_id: Some(run.plan.plan_id.clone()),
                action_id: Some(run.action_id.clone()),
                ..error
            }),
        }
    }
}

/// The folders an action's journal mentions, for the window.
struct FolderSummary {
    created: Vec<String>,
    removed: Vec<String>,
    without_receipt: Vec<String>,
}

impl FolderSummary {
    fn of(journal: &[JournalRow]) -> FolderSummary {
        let mut summary = FolderSummary {
            created: Vec::new(),
            removed: Vec::new(),
            without_receipt: Vec::new(),
        };
        let mut intended = Vec::new();
        for row in journal {
            match &row.entry {
                Some(Entry::FolderIntent(intent)) => intended.push(intent.path.clone()),
                Some(Entry::FolderReceipt(receipt)) => summary.created.push(receipt.path.clone()),
                Some(Entry::FolderRemoval(removal)) if removal.removed => {
                    summary.removed.push(removal.path.clone());
                }
                _ => {}
            }
        }
        summary.without_receipt = intended
            .into_iter()
            .filter(|path| !summary.created.contains(path))
            .collect();
        summary
    }
}

/// The folders an organize action created (with receipts) that its
/// committed undo did not remove.
fn folders_kept_after_undo(
    store: &Store,
    original_id: &str,
    removed: &[String],
) -> Result<Vec<String>, AppError> {
    let Some(original) = store.get_action(original_id)? else {
        return Ok(Vec::new());
    };
    let organize = load_plan(store, &original.plan_id)?;
    let receipts = undo::receipts_in_order(&organize, &store.journal(original_id)?);
    Ok(receipts
        .into_iter()
        .map(|receipt| receipt.path)
        .filter(|path| !removed.contains(path))
        .collect())
}

/// What the action expects, saved with it at the confirmation.
fn preconditions(ready: &ReadyPlan) -> Preconditions {
    let plan = &ready.plan;
    Preconditions {
        source_root_id: plan.source_root_id.clone(),
        source_path: plan.source_relative_path.clone(),
        target_root_id: plan.target_root_id.clone(),
        target_path: plan.target_path(),
        folders_to_create: new_folder_paths(plan),
        sha256: ready.source.sha256.clone(),
        byte_length: ready.source.bytes.len() as u64,
        evidence: evidence_of(&ready.source.observed),
    }
}

/// The error when an earlier action that is not settled yet involves this
/// file, or the plan's source or target location.
pub(crate) fn unresolved(file_id: &str, plan_id: Option<&str>, action_id: &str) -> AppError {
    AppError::execution(
        ErrorCode::UnresolvedAction,
        "An earlier action that is still running or whose result needs checking involves \
         this file or one of these locations. Traeva will not plan or move anything there \
         until that action is settled.",
        Some(file_id),
        plan_id,
        Some(action_id),
    )
}

/// The same physical file as expected (`FileEvidence::same_object`).
pub(crate) fn is_expected_file(found: &Observation, expected: &FileEvidence) -> bool {
    found.kind == Kind::File && evidence_of(found).same_object(expected)
}

fn seen(parent: &Folder, name: &str, expected: &FileEvidence) -> Seen {
    match macos::observe(parent, name) {
        Ok(found) if is_expected_file(&found, expected) => Seen::ExpectedFile,
        Ok(_) => Seen::SomethingElse,
        Err(FsError::NotFound) => Seen::Nothing,
        Err(_) => Seen::Unknown,
    }
}

/// A short reason for people, without raw error numbers.
fn refusal(error: &FsError) -> &'static str {
    match error {
        FsError::PermissionDenied => "no permission",
        FsError::NotFound => "the file or a folder on the way is gone",
        FsError::CrossVolume => "it would leave the volume",
        FsError::UnsupportedVolume(_) => "the volume does not allow it",
        _ => "the system refused",
    }
}

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;
