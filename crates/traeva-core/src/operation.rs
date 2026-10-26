//! A chosen file as an operation the window polls (organize-spec.md §6, §9;
//! architecture.md §3): reading it here, then its analysis (analysis.rs).
//!
//! Reading takes a moment (two full reads with a pause), so the Tauri layer
//! starts it in the background and the window asks for the operation about
//! every 300 ms while it reads or analyzes. An operation lives in memory
//! only: a restart ends it, and nothing about it needs a record check,
//! because reading and analyzing never write inside a root.
//!
//! Every change raises `generation`. A reading that finishes after the user
//! cancelled is dropped, so a late answer never replaces a newer state.

use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;

use crate::analysis::Flow;
use crate::contracts::{AppError, ErrorCode, OperationStage, OperationView};
use crate::observe::{self, ReadError, StableRead};
use crate::platform::macos::Folder;
use crate::{OrganizeService, is_canonical_uuid};

/// One operation: what the window sees, and what only the core keeps.
#[derive(Debug)]
pub(crate) struct Operation {
    pub view: OperationView,
    /// The analysis facts that stay in the core (#16).
    pub flow: Flow,
}

impl OrganizeService {
    /// Starts an operation for a file picked in `source_root_id`, to be
    /// organized into `target_root_id`. Both roots must be usable now.
    /// Returns at once, in stage `observing`; `run_operation` reads the file.
    pub fn start_operation(
        &self,
        source_root_id: &str,
        target_root_id: &str,
    ) -> Result<OperationView, AppError> {
        self.verified_root(source_root_id)?;
        self.verified_root(target_root_id)?;
        let view = OperationView {
            operation_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            stage: OperationStage::Observing,
            source_root_id: source_root_id.to_string(),
            target_root_id: target_root_id.to_string(),
            file: None,
            error: None,
            question: None,
            answer: None,
            answered: Vec::new(),
            suggestion: None,
            model_requests: 0,
            plan_id: None,
            action_id: None,
        };
        self.operations().insert(
            view.operation_id.clone(),
            Operation {
                view: view.clone(),
                flow: Flow::default(),
            },
        );
        Ok(view)
    }

    /// Reads and records the picked file for the operation. This is slow on
    /// purpose (see `observe::read_stable`), so the Tauri layer runs it in
    /// the background. Nothing happens for an unknown or cancelled operation.
    pub fn run_operation(&self, operation_id: &str, picked: &Path) {
        self.run_operation_with(operation_id, picked, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        });
    }

    /// `run_operation` with the stable read passed in, for tests.
    pub(crate) fn run_operation_with(
        &self,
        operation_id: &str,
        picked: &Path,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) {
        let Some(started) = self
            .operations()
            .get(operation_id)
            .map(|operation| operation.view.clone())
        else {
            return;
        };
        if started.stage != OperationStage::Observing {
            return;
        }
        // No lock is held while reading: the window can poll or cancel meanwhile.
        let result = self.register_file_for_organize(&started.source_root_id, picked, read);

        let mut operations = self.operations();
        let Some(Operation { view, .. }) = operations.get_mut(operation_id) else {
            return;
        };
        if view.generation != started.generation {
            return; // cancelled meanwhile: this answer is too late
        }
        view.generation += 1;
        match result {
            Ok(observed) => {
                // Text the model may read, or an image it may look at
                // (§14.10): the user can start an analysis. Neither (not
                // .txt/.md, not UTF-8, blank): pick by hand.
                let summary = &observed.extraction.summary;
                let may_look = summary.image_input
                    && self.image_sending_now() != crate::settings::ImageSending::Off;
                view.stage = if summary.usable_text || may_look {
                    OperationStage::ReadyForAnalysis
                } else {
                    OperationStage::NeedsManualLocation
                };
                view.file = Some(observed.file);
            }
            Err(error) => {
                view.stage = if error.code == ErrorCode::FileChanging {
                    OperationStage::WaitingForFile
                } else {
                    OperationStage::Blocked
                };
                view.error = Some(error);
            }
        }
    }

    /// The operation as it is now, with the action its plan became once
    /// confirmed. Asking never starts anything.
    pub fn get_operation(&self, operation_id: &str) -> Result<OperationView, AppError> {
        check_operation_id(operation_id)?;
        let mut view = self
            .operations()
            .get(operation_id)
            .map(|operation| operation.view.clone())
            .ok_or_else(unknown_operation)?;
        if let Some(plan_id) = &view.plan_id {
            view.action_id = self
                .store()?
                .get_plan(plan_id)?
                .and_then(|plan| plan.action_id);
        }
        Ok(view)
    }

    /// Cancels an operation before anything was confirmed: while reading,
    /// at any analysis stage, or while a target is being chosen. A request
    /// still waiting is cancelled (its late answer is dropped), the question
    /// and candidates end, and an unused plan made in the operation is
    /// cancelled too. A finished operation (blocked, stale, waiting for the
    /// file, cancelled) stays as it is, so cancelling twice changes nothing.
    /// Once the operation's plan was confirmed, its action decides what
    /// happens: the cancel is refused and nothing changes (organize-spec.md
    /// §6, §9: a confirmed plan is never cancelled as if nothing happened).
    pub fn cancel_operation(&self, operation_id: &str) -> Result<OperationView, AppError> {
        check_operation_id(operation_id)?;
        let (view, waiting) = {
            let mut operations = self.operations();
            let operation = operations
                .get_mut(operation_id)
                .ok_or_else(unknown_operation)?;
            let open = matches!(
                operation.view.stage,
                OperationStage::Observing
                    | OperationStage::ReadyForAnalysis
                    | OperationStage::Analyzing
                    | OperationStage::NeedsClarification
                    | OperationStage::RecommendationReady
                    | OperationStage::NeedsManualLocation
                    | OperationStage::AnalysisFailed
                    | OperationStage::InvalidModelOutput
            );
            if !open {
                return Ok(operation.view.clone());
            }
            // Every plan made in the operation is checked, not only the
            // latest: one may have been confirmed while another was made.
            // Closing a plan and confirming it cannot both happen (the
            // database lets only one of them change an open plan), and the
            // lock keeps a new plan from being saved meanwhile.
            let mut confirmed = None;
            for plan_id in operation.flow.plans.clone() {
                let plan = self.cancel_plan(&plan_id)?;
                if let Some(action_id) = plan.action_id {
                    confirmed = Some((plan.file_id, action_id));
                }
            }
            if let Some((file_id, action_id)) = confirmed {
                let mut error = AppError::analysis(
                    ErrorCode::WrongStage,
                    "A plan made in this operation was already confirmed, so it cannot be \
                     cancelled; its action shows what happens.",
                    false,
                    Some(&file_id),
                );
                error.action_id = Some(action_id);
                return Err(error);
            }
            operation.view.stage = OperationStage::Cancelled;
            operation.view.generation += 1;
            operation.view.question = None;
            operation.view.answer = None;
            operation.view.answered.clear();
            operation.view.suggestion = None;
            operation.view.error = None;
            operation.flow.published = None;
            operation.flow.epoch += 1;
            let waiting = operation.flow.request_id.take();
            (operation.view.clone(), waiting)
        };
        if let Some(request_id) = waiting {
            let _ = self.cancel_model_request(&request_id);
        }
        Ok(view)
    }

    pub(crate) fn operations(&self) -> MutexGuard<'_, HashMap<String, Operation>> {
        self.operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn check_operation_id(operation_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(operation_id) {
        Ok(())
    } else {
        Err(AppError::observation(
            ErrorCode::InvalidInput,
            "The operation ID is not valid.",
        ))
    }
}

pub(crate) fn unknown_operation() -> AppError {
    AppError::observation(
        ErrorCode::UnknownOperation,
        "There is no such operation; operations end when Traeva restarts. Choose the file again.",
    )
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
