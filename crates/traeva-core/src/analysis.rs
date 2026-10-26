//! One analysis per operation: start, up to three questions, routing,
//! retry, manual choice and cancel (organize-spec.md §5, §6; #16; the
//! student's routing material is the comment on issue #16; three questions
//! one at a time since organize-spec §15, the student's decision of 10/14).
//!
//! The operation (operation.rs) reads the chosen file. A file with text the
//! model may read ends in `ready_for_analysis`; from there:
//!
//!   start_analysis ─▶ analyzing ─▶ recommendation_ready   (route: recommend)
//!                         │      ─▶ needs_clarification    (route: ask, ≤ 3 times)
//!                         │      ─▶ needs_manual_location  (route: manual)
//!                         │      ─▶ analysis_failed / invalid_model_output
//!                         │                ╰─ retry (same step) ─▶ analyzing
//!   answer_clarification ─┘  (the answer is locked first, then sent with
//!                              every earlier answer)
//!
//! Rules that hold everywhere:
//! - A result is published only if it belongs to the request the operation
//!   is still waiting for. Cancel, manual choice and retry move on, so a
//!   late result changes nothing (checked under the operations lock).
//! - A request not sent yet when the user moves on is never sent.
//! - Nothing here writes inside a root. A target becomes a plan only through
//!   `build_plan`, then preview and confirmation.
//! - No lock is held while reading the file or waiting for the model.

use crate::classify::{
    AnsweredQuestion, ClassificationResult, Classified, ClassifyRequest, MAX_QUESTIONS,
};
use crate::contracts::{
    AnsweredView, AppError, CandidateView, ClarificationAnswer, ErrorCode, ErrorStage, FileView,
    NextStep, OperationStage, OperationView, PlanRequest, PlanTarget, QuestionView, SuggestionView,
    UserChoice,
};
use crate::observe::{self, FolderRef, ReadError, StableRead};
use crate::operation::{Operation, check_operation_id, unknown_operation};
use crate::plan::{open_names, parse_new_folders, split_path, still_registered};
use crate::platform::macos::Folder;
use crate::store::{PlanRow, Store};
use crate::{OrganizeService, is_canonical_uuid};

/// "Confidence ≥ 0.70" in the routing (organize-spec.md §5; 0.80 until the
/// student lowered it in v0.12).
const RECOMMEND_AT: f64 = 0.70;
const MAX_ANSWER_CHARS: usize = 500;

/// What the core keeps about an operation's analysis, beyond its view.
/// Memory only: a restart ends it, and nothing is sent again by itself.
#[derive(Debug, Default)]
pub(crate) struct Flow {
    /// The model request the operation waits for, if any.
    pub request_id: Option<String>,
    /// Whether that request is an explicit retry (kept in its metadata).
    pub retry: bool,
    /// The latest question, once one was asked.
    pub asked: Option<QuestionView>,
    /// Every answered question with its answer, oldest first. Each answer
    /// is locked before it is sent.
    pub answered: Vec<AnsweredQuestion>,
    /// The latest published result, while its candidates may be chosen.
    pub published: Option<Published>,
    /// The request a background run has taken on. Another run for the same
    /// request finds it taken and does nothing, so a double click runs once.
    pub running: Option<String>,
    /// Every plan made in this operation, oldest first. Once any of them was
    /// confirmed, the operation can no longer be cancelled.
    pub plans: Vec<String>,
    /// The user confirmed sending this file's image for this analysis
    /// (§14.10, "ask each time"). Used only when sending images is on.
    pub send_image: bool,
    /// Goes up whenever the analysis moves on: a new request, a request's
    /// end (a result or a failure), a manual choice, a cancel. A plan is kept
    /// only if it did not change while the plan was made.
    pub epoch: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Published {
    pub classified: Classified,
    /// Set only when the result was routed to a recommendation.
    pub recommended_id: Option<String>,
}

/// The three ways a checked result can go (#16 material §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Recommend,
    Ask,
    Manual,
}

/// The #16 routing table, in order:
/// 1. Recommend: `recommend`, confidence ≥ 0.70, exactly one candidate, it
///    is the preferred one, it cites at least one text piece, and nothing is
///    left unsure (no uncertain_field, no question).
/// 2. Ask: otherwise, when the result carries a question and fewer than
///    `MAX_QUESTIONS` were answered.
/// 3. Manual: everything else (abstain, a question after the last answer,
///    or a result that should ask but has no question: Traeva never makes one up).
pub(crate) fn route(result: &ClassificationResult, answered: usize) -> Route {
    let single_supported = match result.candidates.as_slice() {
        [only] => {
            result.preferred_id.as_deref() == Some(only.candidate_id.as_str())
                && !only.evidence_ids.is_empty()
        }
        _ => false,
    };
    let recommend = result.disposition == crate::classify::Disposition::Recommend
        && result.confidence.is_some_and(|score| score >= RECOMMEND_AT)
        && single_supported
        && result.uncertain_field.is_none()
        && result.clarification.is_none();
    if recommend {
        Route::Recommend
    } else if result.clarification.is_some() && answered < MAX_QUESTIONS {
        Route::Ask
    } else {
        Route::Manual
    }
}

/// The model setup refused during the run (settings, consent or key changed
/// after the start): nothing was sent, and the setup is checked again when
/// the user retries, so this failure can be retried once it is fixed.
fn retry_after_setup(mut error: AppError) -> AppError {
    if matches!(
        error.code,
        ErrorCode::ModelNotConfigured | ErrorCode::ConsentRequired | ErrorCode::KeyUnavailable
    ) {
        error.retryable = true;
        error.allowed_next = vec![NextStep::TryAgain, NextStep::ChooseFolder];
    }
    error
}

/// Where a failed analysis leaves the operation (organize-spec.md §6, §9).
fn stage_after(error: &AppError) -> OperationStage {
    match error.code {
        ErrorCode::InvalidModelOutput => OperationStage::InvalidModelOutput,
        // Nothing to send: the user picks by hand.
        ErrorCode::NoUsableText => OperationStage::NeedsManualLocation,
        ErrorCode::FileChanged | ErrorCode::UnknownFile | ErrorCode::FileNotFound => {
            OperationStage::Stale
        }
        ErrorCode::ProtectedLocation | ErrorCode::UnsupportedFile | ErrorCode::FileTooLarge => {
            OperationStage::Blocked
        }
        _ if error.stage == ErrorStage::Authorization => OperationStage::Blocked,
        _ => OperationStage::AnalysisFailed,
    }
}

/// What the plan request names, before the operation is consulted.
enum Picked {
    Candidate(String),
    /// A listed folder, plus the new folders inside it.
    Folder(FolderRef, Vec<String>),
}

/// A target picked for `build_plan`, with how it was picked.
#[derive(Debug, Clone)]
pub(crate) struct PlanChoice {
    pub folder: FolderRef,
    pub new_folders: Vec<String>,
    pub user_choice: UserChoice,
    pub analysis_request_id: Option<String>,
    /// The operation's epoch when the choice was made, if any.
    pub epoch: Option<u64>,
}

impl OrganizeService {
    /// Starts the analysis of a read file (`ready_for_analysis`), or retries
    /// a failed step (`analysis_failed` / `invalid_model_output`, when the
    /// error allows it): a retry after answers sends the locked answers
    /// again. Returns at once in `analyzing`; then run
    /// `run_analysis` in the background. While already analyzing, it only
    /// returns the operation (a double click sends nothing more). Model
    /// settings, consent and key are checked first: a refusal changes nothing.
    pub fn start_analysis(&self, operation_id: &str) -> Result<OperationView, AppError> {
        self.start_analysis_sending(operation_id, false)
    }

    /// `start_analysis`, saying whether the user just confirmed sending this
    /// file's image (§14.10, "ask each time"). With sending images off the
    /// flag does nothing; "without asking" sends without it.
    pub fn start_analysis_sending(
        &self,
        operation_id: &str,
        send_image: bool,
    ) -> Result<OperationView, AppError> {
        check_operation_id(operation_id)?;
        let file_id = {
            let operations = self.operations();
            let operation = operations.get(operation_id).ok_or_else(unknown_operation)?;
            if operation.view.stage == OperationStage::Analyzing {
                return Ok(operation.view.clone());
            }
            startable(&operation.view)?;
            operation
                .view
                .file
                .as_ref()
                .map(|file| file.file_id.clone())
        };
        self.sending_setup(file_id.as_deref())?;

        let mut operations = self.operations();
        let operation = operations
            .get_mut(operation_id)
            .ok_or_else(unknown_operation)?;
        if operation.view.stage == OperationStage::Analyzing {
            return Ok(operation.view.clone());
        }
        let retry = startable(&operation.view)?;
        operation.flow.request_id = Some(uuid::Uuid::new_v4().to_string());
        operation.flow.retry = retry;
        operation.flow.send_image = send_image;
        // A retry drops the failed step's result; candidates of an older
        // result can no longer be chosen.
        operation.flow.published = None;
        operation.view.suggestion = None;
        operation.view.error = None;
        operation.view.stage = OperationStage::Analyzing;
        operation.view.generation += 1;
        operation.flow.epoch += 1;
        Ok(operation.view.clone())
    }

    /// Answers the current question. Checked: it is the latest question,
    /// one option of it or the user's own words (1–500 characters), and it
    /// was not answered yet. The answer is locked first, then the operation
    /// goes to `analyzing`; run `run_analysis` next, which sends every
    /// answer so far. The same answer again changes nothing and sends
    /// nothing; another answer is refused (to change it, cancel and choose
    /// the file again).
    pub fn answer_clarification(
        &self,
        operation_id: &str,
        answer: &ClarificationAnswer,
    ) -> Result<OperationView, AppError> {
        check_operation_id(operation_id)?;
        if !is_canonical_uuid(&answer.question_id) {
            return Err(AppError::analysis(
                ErrorCode::InvalidInput,
                "The question ID is not valid.",
                false,
                None,
            ));
        }
        let file_id = {
            let operations = self.operations();
            let operation = operations.get(operation_id).ok_or_else(unknown_operation)?;
            if let Some(repeat) = answerable(operation, answer)? {
                return Ok(repeat);
            }
            operation
                .view
                .file
                .as_ref()
                .map(|file| file.file_id.clone())
        };
        self.sending_setup(file_id.as_deref())?;

        let mut operations = self.operations();
        let operation = operations
            .get_mut(operation_id)
            .ok_or_else(unknown_operation)?;
        if let Some(repeat) = answerable(operation, answer)? {
            return Ok(repeat);
        }
        let asked = operation.flow.asked.clone().ok_or_else(no_question)?;
        let text = answer_text(&asked, answer)?;
        operation.flow.answered.push(AnsweredQuestion {
            question_id: asked.question_id.clone(),
            uncertain_field: asked.uncertain_field,
            question: asked.question.clone(),
            options: asked.options.iter().map(|o| o.text.clone()).collect(),
            answer: text.clone(),
        });
        operation.view.answered.push(AnsweredView {
            question_id: asked.question_id.clone(),
            number: asked.number,
            question: asked.question.clone(),
            answer: text.clone(),
        });
        operation.view.answer = Some(text);
        operation.flow.request_id = Some(uuid::Uuid::new_v4().to_string());
        operation.flow.retry = false;
        // The last result's candidates end with the answer.
        operation.flow.published = None;
        operation.view.suggestion = None;
        operation.view.stage = OperationStage::Analyzing;
        operation.view.generation += 1;
        operation.flow.epoch += 1;
        Ok(operation.view.clone())
    }

    /// Leaves the analysis for a manual choice: from `ready_for_analysis`,
    /// `analyzing`, `needs_clarification` or a failure. A request still
    /// waiting is cancelled, and its late result is dropped. Candidates of
    /// the latest result, if any, stay on offer (not preselected).
    pub fn choose_manual_location(&self, operation_id: &str) -> Result<OperationView, AppError> {
        check_operation_id(operation_id)?;
        let (view, waiting) = {
            let mut operations = self.operations();
            let operation = operations
                .get_mut(operation_id)
                .ok_or_else(unknown_operation)?;
            let waiting = switch_to_manual(operation)?;
            (operation.view.clone(), waiting)
        };
        if let Some(request_id) = waiting {
            let _ = self.cancel_model_request(&request_id);
        }
        Ok(view)
    }

    /// Runs the request the operation now waits for: reads the file again,
    /// asks the model (unless the user moved on first) and publishes the
    /// result, routed. Slow; run it in the background. Nothing happens for an
    /// operation that is not analyzing.
    pub fn run_analysis(&self, operation_id: &str) {
        self.run_analysis_with(operation_id, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        });
    }

    /// `run_analysis` with the stable read passed in, for tests.
    pub(crate) fn run_analysis_with(
        &self,
        operation_id: &str,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) {
        // What to ask, taken under the lock; the lock is not held after.
        // The run takes the request on, so a second run for it does nothing.
        let (request, file) = {
            let mut operations = self.operations();
            let Some(operation) = operations.get_mut(operation_id) else {
                return;
            };
            let (Some(request_id), Some(file)) = (&operation.flow.request_id, &operation.view.file)
            else {
                return;
            };
            if operation.view.stage != OperationStage::Analyzing
                || operation.flow.running.as_ref() == Some(request_id)
            {
                return;
            }
            let request_id = request_id.clone();
            let file = file.clone();
            operation.flow.running = Some(request_id.clone());
            let request = ClassifyRequest {
                request_id,
                file_id: file.file_id.clone(),
                file_revision: file.file_revision,
                target_root_id: operation.view.target_root_id.clone(),
                answered: operation.flow.answered.clone(),
                generation: operation.view.generation,
                retry: operation.flow.retry,
                send_image: operation.flow.send_image,
            };
            (request, file)
        };

        let outcome = self.classify_with(&request, read, &mut || {
            self.mark_sending(operation_id, &request.request_id)
        });
        // Back from the model: the file and both folders must still be the
        // ones the request was made for (organize-spec.md §5).
        let fresh = match &outcome {
            Ok(_) => self.still_fresh(&file, &request.target_root_id, read),
            Err(_) => Ok(()),
        };

        let mut operations = self.operations();
        let Some(operation) = operations.get_mut(operation_id) else {
            return;
        };
        if operation.view.stage != OperationStage::Analyzing
            || operation.flow.request_id.as_deref() != Some(request.request_id.as_str())
        {
            return; // the user moved on: this result is too late
        }
        operation.flow.request_id = None;
        operation.flow.retry = false;
        operation.view.generation += 1;
        operation.flow.epoch += 1;
        match (outcome, fresh) {
            (Ok(classified), Ok(())) => publish(operation, classified),
            (Ok(_), Err(error)) | (Err(error), _) => {
                operation.view.stage = stage_after(&error);
                operation.view.error = Some(retry_after_setup(error));
            }
        }
    }

    /// Asked right before a request is sent: is the operation still waiting
    /// for exactly this request? If so, the request counts as sent.
    fn mark_sending(&self, operation_id: &str, request_id: &str) -> bool {
        let mut operations = self.operations();
        let Some(operation) = operations.get_mut(operation_id) else {
            return false;
        };
        let current = operation.view.stage == OperationStage::Analyzing
            && operation.flow.request_id.as_deref() == Some(request_id);
        if current {
            operation.view.model_requests += 1;
            operation.view.generation += 1;
        }
        current
    }

    /// The registered file is still there, unchanged (`still_registered`:
    /// the same object, size and mtime; a moved ctime alone is settled by
    /// the hash), and both folders are still authorized. build_plan checks
    /// the same facts again; the full hash is checked again at the next
    /// request and right before execution.
    fn still_fresh(
        &self,
        file: &FileView,
        target_root_id: &str,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<StableRead, ReadError>,
    ) -> Result<(), AppError> {
        let store = self.store()?;
        let changed = || {
            AppError::analysis(
                ErrorCode::FileChanged,
                "The file changed, moved or was replaced while Traeva waited for the model. \
                 Choose it again.",
                false,
                Some(&file.file_id),
            )
        };
        let (row, present) = store.get_file(&file.file_id)?.ok_or_else(changed)?;
        if !present || row.record_revision != file.file_revision {
            return Err(changed());
        }
        let (_, root) = self.verified_root(&row.root_id)?;
        self.verified_root(target_root_id)?;
        let (names, name) = split_path(&row.relative_path);
        let chain = open_names(&root, &names).map_err(|_| changed())?;
        let parent = chain.last().unwrap_or(&root);
        let mut read_here = |parent: &Folder, name: &str, _: i64| read(&root, parent, name);
        if still_registered(parent, &name, root.id().dev, &row, &mut read_here) {
            Ok(())
        } else {
            Err(changed())
        }
    }

    /// The target `build_plan` should use, and how it was chosen (#16
    /// confirmation 3). A candidate is taken only from the operation's
    /// current result, with Traeva's own copy of its folder; a folder comes
    /// from the root's listing. Picking a folder while an analysis is still
    /// going (or failed) is the user's manual choice: the operation moves to
    /// `needs_manual_location` and stays there even if this plan is then
    /// refused (a taken name, a changed folder), so another folder can be
    /// picked. Only malformed input (bad folder names) changes nothing.
    pub(crate) fn plan_choice(&self, request: &PlanRequest) -> Result<PlanChoice, AppError> {
        // A listed folder is looked up first: it needs no operation.
        let picked = match &request.target {
            PlanTarget::Candidate { candidate_id } => Picked::Candidate(candidate_id.clone()),
            PlanTarget::Folder {
                catalog_id,
                folder_id,
                new_folders,
            } => {
                // Bad input changes nothing: names are checked before the
                // operation is touched.
                parse_new_folders(new_folders).map_err(|why| {
                    AppError::plan(
                        ErrorCode::InvalidFolderName,
                        why,
                        Some(&request.file_id),
                        None,
                    )
                })?;
                Picked::Folder(
                    self.listed_folder(
                        &request.target_root_id,
                        catalog_id,
                        folder_id,
                        &request.file_id,
                    )?,
                    new_folders.clone(),
                )
            }
        };
        let Some(operation_id) = &request.operation_id else {
            return match picked {
                Picked::Folder(folder, new_folders) => Ok(PlanChoice {
                    folder,
                    new_folders,
                    user_choice: UserChoice::Manual,
                    analysis_request_id: None,
                    epoch: None,
                }),
                Picked::Candidate(_) => Err(AppError::plan(
                    ErrorCode::InvalidInput,
                    "A candidate can only be chosen within its operation.",
                    Some(&request.file_id),
                    None,
                )),
            };
        };
        check_operation_id(operation_id)?;

        let (choice, waiting) = {
            let mut operations = self.operations();
            let operation = operations
                .get_mut(operation_id)
                .ok_or_else(unknown_operation)?;
            let matches = operation.view.file.as_ref().is_some_and(|file| {
                file.file_id == request.file_id && file.file_revision == request.file_revision
            }) && operation.view.target_root_id == request.target_root_id;
            if !matches {
                return Err(AppError::plan(
                    ErrorCode::InvalidInput,
                    "This choice does not match the operation's file or target folder.",
                    Some(&request.file_id),
                    None,
                ));
            }
            let recommending = operation.view.stage == OperationStage::RecommendationReady;
            let mut waiting = None;
            let (folder, new_folders, user_choice) = match picked {
                Picked::Candidate(candidate_id) => {
                    let published = match operation.view.stage {
                        OperationStage::RecommendationReady
                        | OperationStage::NeedsManualLocation => operation.flow.published.as_ref(),
                        _ => None,
                    }
                    .ok_or_else(|| outdated(&request.file_id))?;
                    let index = published
                        .classified
                        .result
                        .candidates
                        .iter()
                        .position(|candidate| candidate.candidate_id == candidate_id)
                        .ok_or_else(|| outdated(&request.file_id))?;
                    let candidate = &published.classified.result.candidates[index];
                    // A recommendation has exactly one candidate, the
                    // recommended one (route()): choosing it accepts it.
                    // Without a recommendation, picking one of the offered
                    // candidates is the user's own choice.
                    let user_choice = if recommending {
                        UserChoice::Accept
                    } else {
                        UserChoice::Manual
                    };
                    (
                        published.classified.folders[index].clone(),
                        candidate.new_segments.clone(),
                        user_choice,
                    )
                }
                Picked::Folder(folder, new_folders) => {
                    let user_choice = if recommending {
                        UserChoice::Correct
                    } else {
                        if operation.view.stage != OperationStage::NeedsManualLocation {
                            waiting = switch_to_manual(operation)?;
                        }
                        UserChoice::Manual
                    };
                    (folder, new_folders, user_choice)
                }
            };
            let analysis_request_id = operation
                .flow
                .published
                .as_ref()
                .map(|published| published.classified.result.request_id.clone());
            let choice = PlanChoice {
                folder,
                new_folders,
                user_choice,
                analysis_request_id,
                epoch: Some(operation.flow.epoch),
            };
            (choice, waiting)
        };
        if let Some(request_id) = waiting {
            let _ = self.cancel_model_request(&request_id);
        }
        Ok(choice)
    }

    /// Saves a plan made from `plan_choice` and records it in its operation,
    /// in one step under the operations lock (then the database, the same
    /// order cancel_operation uses): only if the analysis did not move on
    /// meanwhile, and so that a cancel or another plan can never slip in
    /// between saving and recording. An old suggestion never comes back as
    /// a preview, and the operation always knows its latest plan.
    pub(crate) fn save_operation_plan(
        &self,
        store: &Store,
        plan: &PlanRow,
        operation_id: &str,
        epoch: u64,
    ) -> Result<(), AppError> {
        let mut operations = self.operations();
        let operation = operations
            .get_mut(operation_id)
            .ok_or_else(unknown_operation)?;
        if operation.flow.epoch != epoch {
            // Only a cancel moves a recommendation or a manual choice on.
            let mut error = AppError::plan(
                ErrorCode::SuggestionOutdated,
                "The operation was cancelled while the plan was being made, so no plan was \
                 saved. Choose the file again.",
                Some(&plan.file_id),
                None,
            );
            error.allowed_next = vec![NextStep::ChooseFile];
            return Err(error);
        }
        store.insert_plan(plan)?;
        operation.view.plan_id = Some(plan.plan_id.clone());
        operation.flow.plans.push(plan.plan_id.clone());
        operation.view.generation += 1;
        Ok(())
    }
}

/// Puts a checked, fresh result in place, routed (#16 material §2).
fn publish(operation: &mut Operation, classified: Classified) {
    let answered = operation.flow.answered.len();
    let result = &classified.result;
    let route = route(result, answered);
    let mut recommended_id = None;
    match route {
        Route::Recommend => {
            operation.view.stage = OperationStage::RecommendationReady;
            recommended_id = result.preferred_id.clone();
        }
        Route::Ask => {
            // #15 only accepts a question together with its uncertain_field.
            if let (Some(clarification), Some(field)) =
                (&result.clarification, result.uncertain_field)
            {
                let question = QuestionView {
                    question_id: clarification.question_id.clone(),
                    uncertain_field: field,
                    question: clarification.question.clone(),
                    options: clarification.options.clone(),
                    number: answered as u32 + 1,
                    max_questions: MAX_QUESTIONS as u32,
                };
                operation.flow.asked = Some(question.clone());
                operation.view.question = Some(question);
                // The new question has no answer yet; earlier ones stay in
                // `answered`.
                operation.view.answer = None;
            }
            operation.view.stage = OperationStage::NeedsClarification;
        }
        Route::Manual => operation.view.stage = OperationStage::NeedsManualLocation,
    }
    let published = Published {
        classified,
        recommended_id,
    };
    operation.view.suggestion = match operation.view.stage {
        OperationStage::RecommendationReady | OperationStage::NeedsManualLocation => {
            Some(suggestion_view(&published))
        }
        _ => None,
    };
    operation.flow.published = Some(published);
}

fn suggestion_view(published: &Published) -> SuggestionView {
    let result = &published.classified.result;
    SuggestionView {
        request_id: result.request_id.clone(),
        summary: result.summary.clone(),
        confidence: result.confidence,
        recommended_id: published.recommended_id.clone(),
        candidates: result
            .candidates
            .iter()
            .zip(&published.classified.folders)
            .map(|(candidate, folder)| CandidateView {
                candidate_id: candidate.candidate_id.clone(),
                folder_path: folder.segments.join("/"),
                new_segments: candidate.new_segments.clone(),
                reason: candidate.reason.clone(),
                evidence_ids: candidate.evidence_ids.clone(),
            })
            .collect(),
    }
}

/// From the stages that allow it, moves to `needs_manual_location`. Returns
/// the request that was waiting, to be cancelled outside the lock.
fn switch_to_manual(operation: &mut Operation) -> Result<Option<String>, AppError> {
    match operation.view.stage {
        OperationStage::ReadyForAnalysis
        | OperationStage::Analyzing
        | OperationStage::NeedsClarification
        | OperationStage::AnalysisFailed
        | OperationStage::InvalidModelOutput => {}
        _ => {
            return Err(wrong_stage(
                &operation.view,
                &format!(
                    "The operation is “{}”, so it cannot switch to picking the folder by hand.",
                    stage_name(&operation.view)
                ),
            ));
        }
    }
    let waiting = operation.flow.request_id.take();
    operation.flow.retry = false;
    operation.view.stage = OperationStage::NeedsManualLocation;
    operation.view.error = None;
    operation.view.suggestion = operation.flow.published.as_ref().map(suggestion_view);
    operation.view.generation += 1;
    operation.flow.epoch += 1;
    Ok(waiting)
}

/// May an analysis start (or be retried) now? Returns whether it is a retry.
/// A failure can be retried when its error says so.
fn startable(view: &OperationView) -> Result<bool, AppError> {
    match view.stage {
        OperationStage::ReadyForAnalysis => Ok(false),
        OperationStage::AnalysisFailed | OperationStage::InvalidModelOutput => {
            if view.error.as_ref().is_some_and(|error| error.retryable) {
                Ok(true)
            } else {
                Err(wrong_stage(
                    view,
                    "This failure cannot be retried. Pick the folder by hand, or cancel.",
                ))
            }
        }
        _ => Err(wrong_stage(
            view,
            &format!(
                "An analysis cannot start while the operation is “{}”.",
                stage_name(view)
            ),
        )),
    }
}

/// Checks an answer against the operation. `Some(view)` means it is the
/// same answer again: nothing to do. An error means it cannot be accepted.
fn answerable(
    operation: &Operation,
    answer: &ClarificationAnswer,
) -> Result<Option<OperationView>, AppError> {
    let asked = operation.flow.asked.as_ref().ok_or_else(no_question)?;
    if asked.question_id != answer.question_id {
        return Err(AppError::analysis(
            ErrorCode::InvalidInput,
            "This is not the current question of this operation.",
            false,
            None,
        ));
    }
    let text = answer_text(asked, answer)?;
    let locked = operation
        .flow
        .answered
        .last()
        .filter(|locked| locked.question_id == asked.question_id);
    if let Some(locked) = locked {
        if locked.answer == text {
            return Ok(Some(operation.view.clone()));
        }
        return Err(wrong_stage(
            &operation.view,
            "The answer is already locked and cannot be changed. To start over, cancel and \
             choose the file again.",
        ));
    }
    if operation.view.stage != OperationStage::NeedsClarification {
        return Err(wrong_stage(
            &operation.view,
            &format!(
                "The question cannot be answered while the operation is “{}”.",
                stage_name(&operation.view)
            ),
        ));
    }
    Ok(None)
}

/// The answer's text: the chosen option's, or the user's own words
/// (not blank, at most 500 characters). Exactly one of the two.
fn answer_text(asked: &QuestionView, answer: &ClarificationAnswer) -> Result<String, AppError> {
    let bad = |message: &str| AppError::analysis(ErrorCode::InvalidInput, message, false, None);
    match (&answer.option_id, &answer.text) {
        (Some(option_id), None) => asked
            .options
            .iter()
            .find(|option| &option.option_id == option_id)
            .map(|option| option.text.clone())
            .ok_or_else(|| bad("This option is not one of the question's options.")),
        (None, Some(text)) => {
            if text.trim().is_empty() || text.chars().count() > MAX_ANSWER_CHARS {
                Err(bad("The answer must have 1 to 500 characters."))
            } else {
                Ok(text.clone())
            }
        }
        _ => Err(bad(
            "Answer with one option or with your own words, not both.",
        )),
    }
}

fn wrong_stage(view: &OperationView, message: &str) -> AppError {
    AppError::analysis(
        ErrorCode::WrongStage,
        message,
        false,
        view.file.as_ref().map(|file| file.file_id.as_str()),
    )
}

/// The stage as the window spells it, e.g. "needs_manual_location".
fn stage_name(view: &OperationView) -> String {
    serde_json::to_value(view.stage)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn no_question() -> AppError {
    AppError::analysis(
        ErrorCode::WrongStage,
        "No question was asked in this operation.",
        false,
        None,
    )
}

fn outdated(file_id: &str) -> AppError {
    AppError::plan(
        ErrorCode::SuggestionOutdated,
        "This candidate is not part of the current suggestion any more (cancelled, answered or \
         retried since). Choose again.",
        Some(file_id),
        None,
    )
}

#[cfg(test)]
#[path = "analysis_tests.rs"]
mod tests;
