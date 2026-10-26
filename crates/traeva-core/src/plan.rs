//! Immutable ActionPlans (organize-spec.md §3, §6). Student material for #8:
//! https://github.com/P30g53/Traeva/issues/8#issuecomment-5983494019
//!
//! A plan's execution content (source, target, folders to create, expected
//! file) is saved once and never changed; the database refuses such updates.
//! Only the lifecycle changes, and only from `open` to a final state:
//! cancelled, superseded or invalidated. Whether a plan can be confirmed is
//! worked out each time it is read, never stored.
//!
//! Nothing here writes inside a root: building, reading, cancelling and
//! checking a plan only look at the filesystem.

use crate::contracts::{
    AppError, BuildOutcome, ErrorCode, PlanEnd, PlanKind, PlanLifecycle, PlanRequest, PlanStatus,
    PlanView, RootStatus,
};
use crate::observe::{FolderRef, ReadError, StableRead};
use crate::platform::macos::{
    self, Folder, FsError, Kind, NameProblem, NewFolderName, Observation,
};
use crate::store::{ChainLink, ExpectedSource, FileRow, PlanRow, Store};
use crate::{OrganizeService, is_canonical_uuid};

/// A preview can be confirmed for 15 minutes after it was made.
pub const PREVIEW_LIFETIME_MS: i64 = 15 * 60 * 1000;
/// At most this many new folders, nested, per plan (organize-spec.md §4).
pub const MAX_NEW_FOLDERS: usize = 3;

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

/// Expired after 15 minutes. A clock that reads earlier than the plan's
/// creation cannot be trusted either, so that also counts as expired.
pub fn is_expired(created_at_ms: i64, now_ms: i64) -> bool {
    now_ms < created_at_ms || now_ms - created_at_ms >= PREVIEW_LIFETIME_MS
}

/// What `get_plan` reports, from stored facts only (no filesystem checks).
/// An action that used the plan decides everything; then the stored
/// lifecycle; then authorization; then time.
pub fn plan_status(plan: &PlanRow, roots_active: bool, now_ms: i64) -> PlanStatus {
    if plan.action_id.is_some() {
        return PlanStatus::Consumed;
    }
    match plan.lifecycle {
        PlanLifecycle::Cancelled => PlanStatus::Cancelled,
        PlanLifecycle::Superseded => PlanStatus::Superseded,
        PlanLifecycle::Invalidated => PlanStatus::Invalidated,
        PlanLifecycle::Open if !roots_active => PlanStatus::RootRevoked,
        PlanLifecycle::Open if is_expired(plan.created_at_ms, now_ms) => PlanStatus::Expired,
        PlanLifecycle::Open => PlanStatus::Confirmable,
    }
}

/// Check the new folders to be created (outermost first): only allow at most 3 folders,
/// then check if each name follows the naming rules.
pub fn parse_new_folders(names: &[String]) -> Result<Vec<NewFolderName>, String> {
    let count = names.len();
    if count > MAX_NEW_FOLDERS {
        return Err(format!(
            "At most {MAX_NEW_FOLDERS} new folders can be created at once ({count} were given)."
        ));
    }
    let mut valid_folders = Vec::new();
    for name in names {
        match NewFolderName::parse(name) {
            Ok(folder) => valid_folders.push(folder),
            Err(problem) => {
                return Err(format!(
                    "“{name}” cannot be a folder name: {}.",
                    explain(problem)
                ));
            }
        }
    }
    Ok(valid_folders)
}

fn explain(problem: NameProblem) -> &'static str {
    match problem {
        NameProblem::Empty => "it is empty",
        NameProblem::DotOrDotDot => "“.” and “..” are not names",
        NameProblem::PathSeparator => "it contains “/” or “\\”",
        NameProblem::Colon => "it contains “:”",
        NameProblem::ControlCharacter => "it contains a control character",
        NameProblem::LeadingDot => "it starts with “.” (a hidden name)",
        NameProblem::EdgeWhitespace => "it starts or ends with a space",
        NameProblem::TooLong => "it is longer than 255 bytes",
        NameProblem::PackageExtension => "it ends like a package (for example “.app”)",
        NameProblem::ProjectMarker => {
            "it is a project file name (for example “Cargo.toml”), so the folder around it would \
             become a project that Traeva leaves alone"
        }
    }
}

/// split a path into folder names and the file name
pub(crate) fn split_path(relative_path: &str) -> (Vec<String>, String) {
    match relative_path.rfind('/') {
        None => (Vec::new(), relative_path.to_string()),
        Some(index) => {
            let folder_path = &relative_path[..index];
            let file_name = &relative_path[index + 1..];
            let mut folders = Vec::new();
            for folder in folder_path.split('/') {
                folders.push(folder.to_string());
            }
            (folders, file_name.to_string())
        }
    }
}

/// make a list of paths for the new folders (outer ones first)
pub(crate) fn new_folder_paths(plan: &PlanRow) -> Vec<String> {
    let all_folders = &plan.target_segments;
    let existing = all_folders.len() - plan.folders_to_create.len();
    let mut paths = Vec::new();
    let mut depth = existing + 1;
    while depth <= all_folders.len() {
        paths.push(all_folders[..depth].join("/"));
        depth += 1;
    }
    paths
}

pub(crate) fn open_names(root: &Folder, names: &[String]) -> Result<Vec<Folder>, FsError> {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    macos::open_chain(root, &names)
}

pub(crate) fn links(chain: &[Folder]) -> Vec<ChainLink> {
    chain
        .iter()
        .map(|folder| ChainLink {
            name: folder.name().to_string(),
            dev: folder.id().dev,
            volume: macos::volume_uuid(folder.id().dev),
            ino: folder.id().ino,
        })
        .collect()
}

/// The open chain is still the saved one, folder by folder
/// (`ChainLink::same_folder`).
pub(crate) fn chain_matches(chain: &[Folder], expected: &[ChainLink]) -> bool {
    chain.len() == expected.len()
        && links(chain)
            .iter()
            .zip(expected)
            .all(|(now, saved)| now.same_folder(saved))
}

/// The file at `parent`/`name` is still the registered file. Normally
/// nothing visible about it changed. When only its ctime moved, the bytes
/// decide: macOS and other apps add attributes (such as com.apple.macl)
/// without touching the content, but an edit that restores mtime also moves
/// only ctime. So one stable read must give exactly the registered hash.
/// The check after the model and build_plan both use this; execution checks
/// the full hash again (`revalidate_plan_with`).
pub(crate) fn still_registered(
    parent: &Folder,
    name: &str,
    root_dev: i64,
    file: &FileRow,
    read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
) -> bool {
    let Ok(now) = macos::observe(parent, name) else {
        return false;
    };
    if !same_but_ctime(&now, file) {
        return false;
    }
    if now.ctime_ns == file.evidence.ctime_ns {
        return true;
    }
    read(parent, name, root_dev)
        .is_ok_and(|read| same_but_ctime(&read.observed, file) && read.sha256 == file.sha256)
}

/// The registered object, with the same size, mtime and link count. Only
/// its ctime is left to compare.
fn same_but_ctime(now: &Observation, file: &FileRow) -> bool {
    let saved = &file.evidence;
    now.kind == Kind::File
        && crate::evidence_of(now).same_object(saved)
        && now.size == file.byte_length
        && now.mtime_ns == saved.mtime_ns
        && now.link_count == saved.link_count
}

// ---------------------------------------------------------------------------
// Building, reading and cancelling plans
// ---------------------------------------------------------------------------

/// What `revalidate_plan` found right before execution.
#[derive(Debug)]
pub(crate) enum Revalidation {
    /// An action already used this plan. The executor returns that action and
    /// never runs the plan again.
    AlreadyConsumed { action_id: String },
    /// Every check passed. The executor (#9) continues with these handles.
    Ready(ReadyPlan),
}

#[derive(Debug)]
pub(crate) struct ReadyPlan {
    pub plan: PlanRow,
    pub source_root: Folder,
    pub source_chain: Vec<Folder>,
    pub target_root: Folder,
    pub target_chain: Vec<Folder>,
    pub source: StableRead,
}

impl OrganizeService {
    /// Builds and saves a new immutable plan for moving a registered file into
    /// a listed folder (and up to 3 new folders inside it), or to a candidate
    /// of its operation's current suggestion (#16). Only looks: no folder is
    /// created and nothing moves. Any older open plan of the same file
    /// becomes superseded.
    pub fn build_plan(&self, request: &PlanRequest) -> Result<BuildOutcome, AppError> {
        let store = self.store()?;
        if !is_canonical_uuid(&request.file_id) {
            return Err(AppError::plan(
                ErrorCode::InvalidInput,
                "The file ID is not valid.",
                None,
                None,
            ));
        }
        let file_id = request.file_id.as_str();
        let fail =
            |code: ErrorCode, message: String| AppError::plan(code, message, Some(file_id), None);

        let (file, present) = store.get_file(file_id)?.ok_or_else(|| {
            fail(
                ErrorCode::UnknownFile,
                "There is no registered file with this ID.".to_string(),
            )
        })?;
        // An earlier action of this file is not settled yet (#9). Checked
        // first so that this is the reason given, even if the file changed.
        if let Some(busy) = store.unresolved_action_of_file(file_id)? {
            return Err(crate::execute::unresolved(file_id, None, &busy));
        }
        if !present || file.record_revision != request.file_revision {
            return Err(fail(
                ErrorCode::FileChanged,
                format!(
                    "{} changed since you chose it. Choose the file again.",
                    file.relative_path
                ),
            ));
        }
        // The target and how it was chosen (analysis.rs): a listed folder,
        // or a candidate of the operation's current suggestion.
        let choice = self.plan_choice(request)?;
        let new_folders = parse_new_folders(&choice.new_folders)
            .map_err(|why| fail(ErrorCode::InvalidFolderName, why))?;
        let listed = choice.folder.clone();

        let (source_root_row, source_root) = self.verified_root(&file.root_id)?;
        let (target_root_row, target_root) = self.verified_root(&request.target_root_id)?;
        if source_root.id().dev != target_root.id().dev {
            return Err(fail(
                ErrorCode::UnsupportedLocation,
                "The file and the target folder are on different volumes; the first release \
                 moves files within one volume only."
                    .to_string(),
            ));
        }

        // The file must still be the registered object, unchanged.
        let shown_source = format!("{}/{}", source_root_row.display_path, file.relative_path);
        let file_changed = || {
            fail(
                ErrorCode::FileChanged,
                format!(
                    "{shown_source} changed, moved or was replaced since you chose it. Choose it again."
                ),
            )
        };
        let (source_names, basename) = split_path(&file.relative_path);
        let source_chain = match open_names(&source_root, &source_names) {
            Ok(chain) => chain,
            Err(FsError::Protected(why)) => {
                return Err(crate::file_error(&shown_source, FsError::Protected(why)));
            }
            Err(_) => return Err(file_changed()),
        };
        let source_parent = source_chain.last().unwrap_or(&source_root);
        if !still_registered(
            source_parent,
            &basename,
            source_root.id().dev,
            &file,
            &mut crate::observe::read_stable,
        ) {
            return Err(file_changed());
        }

        // The chosen folder must still be the folder that was listed.
        let shown_target = format!(
            "{}/{}",
            target_root_row.display_path,
            listed.segments.join("/")
        );
        let outdated = || {
            fail(
                ErrorCode::FolderListOutdated,
                format!(
                    "{shown_target} changed since the folder list was made. List the folders again."
                ),
            )
        };
        let target_chain = open_names(&target_root, &listed.segments).map_err(|_| outdated())?;
        let target_folder = target_chain.last().unwrap_or(&target_root);
        if target_folder.id() != listed.id {
            return Err(outdated());
        }

        // Already there: nothing to plan, no action, nothing to undo. Like
        // any new choice, it ends the file's earlier plans that no action
        // used (organize-spec.md §3, #71), so a plan for another place
        // cannot still be confirmed.
        if new_folders.is_empty() && target_folder.id() == source_parent.id() {
            store.supersede_open_plans(
                &file.file_id,
                self.now(),
                "the user chose the place the file is in now",
            )?;
            return Ok(BuildOutcome::AlreadyInPlace {
                file_id: file.file_id,
            });
        }

        // Preview-time check only; the move itself never replaces anything.
        let first_new = new_folders
            .first()
            .map_or(basename.as_str(), NewFolderName::as_str);
        match macos::observe(target_folder, first_new) {
            Err(FsError::NotFound) => {}
            Ok(_) => {
                return Err(fail(
                    ErrorCode::TargetConflict,
                    format!(
                        "“{first_new}” already exists in {shown_target}. Choose another folder, \
                         or pick the existing folder from the list."
                    ),
                ));
            }
            Err(_) => return Err(outdated()),
        }

        let mut target_segments = listed.segments.clone();
        target_segments.extend(new_folders.iter().map(|name| name.as_str().to_string()));
        let plan = PlanRow {
            plan_id: uuid::Uuid::new_v4().to_string(),
            kind: PlanKind::Organize,
            file_id: file.file_id.clone(),
            source_root_id: file.root_id.clone(),
            source_relative_path: file.relative_path.clone(),
            expected_source: ExpectedSource {
                file_revision: file.record_revision,
                sha256: file.sha256.clone(),
                byte_length: file.byte_length,
                evidence: file.evidence.clone(),
                folder_chain: links(&source_chain),
            },
            target_root_id: request.target_root_id.clone(),
            target_segments,
            basename,
            folder_chain: links(&target_chain),
            folders_to_create: new_folders
                .iter()
                .map(|name| name.as_str().to_string())
                .collect(),
            created_at_ms: self.now(),
            lifecycle: PlanLifecycle::Open,
            lifecycle_changed_at_ms: None,
            lifecycle_reason: None,
            action_id: None,
            undo_of_action_id: None,
            user_choice: Some(choice.user_choice),
            analysis_request_id: choice.analysis_request_id.clone(),
        };
        // An unsettled action at either of this plan's locations (#10).
        if let Some(busy) = store.conflicting_action(&plan)? {
            return Err(crate::execute::unresolved(file_id, None, &busy));
        }
        // Within an operation, the plan is saved only if the analysis did
        // not move on meanwhile, and recorded in the same step.
        match (&request.operation_id, choice.epoch) {
            (Some(operation_id), Some(epoch)) => {
                self.save_operation_plan(store, &plan, operation_id, epoch)?
            }
            _ => store.insert_plan(&plan)?,
        }
        Ok(BuildOutcome::Planned {
            plan: self.plan_view(store, &plan)?,
        })
    }

    /// The saved plan plus its current status, worked out now from stored
    /// facts. Execution still checks everything again.
    pub fn get_plan(&self, plan_id: &str) -> Result<PlanView, AppError> {
        let store = self.store()?;
        let plan = load_plan(store, plan_id)?;
        self.plan_view(store, &plan)
    }

    /// Cancels an open plan. A plan an action already used is final: this only
    /// reports it. Cancelling twice changes nothing.
    pub fn cancel_plan(&self, plan_id: &str) -> Result<PlanView, AppError> {
        let store = self.store()?;
        let plan = load_plan(store, plan_id)?;
        if plan.action_id.is_none() {
            store.close_plan(
                plan_id,
                PlanLifecycle::Cancelled,
                self.now(),
                "cancelled by the user",
            )?;
        }
        let plan = load_plan(store, plan_id)?;
        self.plan_view(store, &plan)
    }

    /// All checks right before execution (the #8 student material's ①–⑦).
    /// The executor (#9) calls this under its lock and only continues on
    /// `Ready`. The stable read is passed in (production: `read_stable`), so
    /// tests can change the file between the two reads without waiting.
    pub(crate) fn revalidate_plan_with(
        &self,
        plan_id: &str,
        read: &mut dyn FnMut(&Folder, &str, i64) -> Result<StableRead, ReadError>,
    ) -> Result<Revalidation, AppError> {
        let store = self.store()?;
        let plan = load_plan(store, plan_id)?;
        let refuse = |code: ErrorCode, message: &str| {
            AppError::plan(code, message, Some(&plan.file_id), Some(plan_id))
        };

        // ① An action already used this plan: report it, never run it again.
        if let Some(action_id) = &plan.action_id {
            return Ok(Revalidation::AlreadyConsumed {
                action_id: action_id.clone(),
            });
        }
        // ② Only an open plan can run.
        if plan.lifecycle != PlanLifecycle::Open {
            let why = plan.lifecycle_reason.as_deref().unwrap_or("closed");
            return Err(refuse(
                ErrorCode::StalePlan,
                &format!("This plan can no longer run ({why}). Preview again."),
            ));
        }
        // ③ 15 minutes, worked out from created_at. Nothing is stored.
        if is_expired(plan.created_at_ms, self.now()) {
            return Err(refuse(
                ErrorCode::StalePlan,
                "This plan is older than 15 minutes. Preview again.",
            ));
        }

        // From here on, a failed check ends the plan for good (invalidated).
        // The one exception: the file could not be read right now.
        let end = |code: ErrorCode, reason: &str, message: &str| {
            self.invalidate_plan(store, &plan, code, reason, message)
        };

        // ⑥ Both roots: still authorized, still the same folders, one volume.
        let source_root = match self.verified_root(&plan.source_root_id) {
            Ok((_, root)) => root,
            Err(error) => {
                return Err(end(
                    ErrorCode::StalePlan,
                    "a root changed or was revoked",
                    &error.message,
                ));
            }
        };
        let target_root = match self.verified_root(&plan.target_root_id) {
            Ok((_, root)) => root,
            Err(error) => {
                return Err(end(
                    ErrorCode::StalePlan,
                    "a root changed or was revoked",
                    &error.message,
                ));
            }
        };
        if source_root.id().dev != target_root.id().dev {
            return Err(end(
                ErrorCode::StalePlan,
                "the roots are on different volumes",
                "The file and the target folder are no longer on one volume.",
            ));
        }

        // ⑤ The folders above the file, then the file itself.
        let (source_names, basename) = split_path(&plan.source_relative_path);
        let source_chain = match open_names(&source_root, &source_names) {
            Ok(chain) if chain_matches(&chain, &plan.expected_source.folder_chain) => chain,
            _ => {
                return Err(end(
                    ErrorCode::FileChanged,
                    "a folder above the file changed",
                    "A folder above the file was moved or replaced. Choose the file again.",
                ));
            }
        };
        let parent = source_chain.last().unwrap_or(&source_root);
        let source = match read(parent, &basename, source_root.id().dev) {
            Ok(read) => read,
            Err(ReadError::StillChanging { .. }) => {
                return Err(refuse(
                    ErrorCode::FileChanging,
                    "The file kept changing, so its hash could not be checked. Try again in a moment.",
                ));
            }
            Err(ReadError::Fs(FsError::PermissionDenied | FsError::Other(_))) => {
                return Err(refuse(
                    ErrorCode::FileUnreadable,
                    "The file could not be read, so its hash could not be checked.",
                ));
            }
            Err(_) => {
                return Err(end(
                    ErrorCode::FileChanged,
                    "the file moved, was replaced or was deleted",
                    "The file is no longer the one in this plan. Choose it again.",
                ));
            }
        };
        if !crate::evidence_of(&source.observed).same_object(&plan.expected_source.evidence) {
            return Err(end(
                ErrorCode::FileChanged,
                "the file was replaced",
                "The file was replaced by another file. Choose it again.",
            ));
        }
        // ④ The exact bytes must still be the ones that were previewed.
        if source.sha256 != plan.expected_source.sha256 {
            return Err(end(
                ErrorCode::FileChanged,
                "the file content changed",
                "The file's content changed after the preview. Choose it again.",
            ));
        }

        // ⑥ The existing target folders are still the planned ones.
        let existing = plan.target_segments.len() - plan.folders_to_create.len();
        let target_chain = match open_names(&target_root, &plan.target_segments[..existing]) {
            Ok(chain) if chain_matches(&chain, &plan.folder_chain) => chain,
            _ => {
                return Err(end(
                    ErrorCode::StalePlan,
                    "the target folder changed",
                    "The target folder was moved or replaced. Preview again.",
                ));
            }
        };
        let target_folder = target_chain.last().unwrap_or(&target_root);

        // ⑦ Nothing may occupy the target name or the first folder to create.
        let first_new = plan.folders_to_create.first().unwrap_or(&plan.basename);
        match macos::observe(target_folder, first_new) {
            Err(FsError::NotFound) => {}
            Ok(_) => {
                return Err(end(
                    ErrorCode::TargetConflict,
                    "the target name is taken",
                    &format!("“{first_new}” now exists at the target. Choose another folder."),
                ));
            }
            Err(_) => {
                return Err(end(
                    ErrorCode::StalePlan,
                    "the target could not be checked",
                    "Traeva could not check the target folder. Preview again.",
                ));
            }
        }

        Ok(Revalidation::Ready(ReadyPlan {
            plan,
            source_root,
            source_chain,
            target_root,
            target_chain,
            source,
        }))
    }

    /// Marks an open, unused plan invalidated and returns the error to report.
    /// If even that cannot be saved, the database error is reported instead.
    pub(crate) fn invalidate_plan(
        &self,
        store: &Store,
        plan: &PlanRow,
        code: ErrorCode,
        reason: &str,
        message: &str,
    ) -> AppError {
        match store.close_plan(
            &plan.plan_id,
            PlanLifecycle::Invalidated,
            self.now(),
            reason,
        ) {
            Ok(_) => AppError::plan(code, message, Some(&plan.file_id), Some(&plan.plan_id)),
            Err(db_error) => db_error,
        }
    }

    /// The folder `folder_id` of the listing `catalog_id`, which must be the
    /// latest listing of `root_id`.
    pub(crate) fn listed_folder(
        &self,
        root_id: &str,
        catalog_id: &str,
        folder_id: &str,
        file_id: &str,
    ) -> Result<FolderRef, AppError> {
        let outdated = || {
            AppError::plan(
                ErrorCode::FolderListOutdated,
                "This folder choice comes from an old folder list. List the folders again.",
                Some(file_id),
                None,
            )
        };
        let catalogs = self
            .catalogs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cached = catalogs
            .get(root_id)
            .filter(|cached| cached.catalog_id == catalog_id)
            .ok_or_else(outdated)?;
        let folder = cached
            .catalog
            .folders
            .iter()
            .find(|folder| folder.folder_id == folder_id)
            .ok_or_else(outdated)?;
        if folder.protected.is_some() {
            return Err(AppError::plan(
                ErrorCode::ProtectedLocation,
                "This folder is a project folder and can never be a target.",
                Some(file_id),
                None,
            ));
        }
        Ok(folder.clone())
    }

    pub(crate) fn plan_view(&self, store: &Store, plan: &PlanRow) -> Result<PlanView, AppError> {
        let active = |root_id: &str| -> Result<bool, AppError> {
            Ok(matches!(store.get_root(root_id)?, Some(row) if row.status == RootStatus::Active))
        };
        let status = plan_status(
            plan,
            active(&plan.source_root_id)? && active(&plan.target_root_id)?,
            self.now(),
        );
        let reason = match status {
            PlanStatus::Confirmable => None,
            PlanStatus::Consumed => Some(
                "An action already used this plan; see that action for the result.".to_string(),
            ),
            PlanStatus::Expired => Some("Older than 15 minutes. Preview again.".to_string()),
            PlanStatus::RootRevoked => {
                Some("A folder this plan uses is no longer authorized.".to_string())
            }
            PlanStatus::Cancelled | PlanStatus::Superseded | PlanStatus::Invalidated => {
                plan.lifecycle_reason.clone()
            }
        };
        let (source, target) = plan_ends(store, plan)?;
        Ok(PlanView {
            plan_id: plan.plan_id.clone(),
            kind: plan.kind,
            file_id: plan.file_id.clone(),
            file_revision: plan.expected_source.file_revision,
            source,
            target,
            basename: plan.basename.clone(),
            folders_to_create: new_folder_paths(plan),
            created_at_ms: plan.created_at_ms,
            expires_at_ms: plan.created_at_ms + PREVIEW_LIFETIME_MS,
            lifecycle: plan.lifecycle,
            status,
            can_confirm: status == PlanStatus::Confirmable,
            reason,
            action_id: plan.action_id.clone(),
            undo_of_action_id: plan.undo_of_action_id.clone(),
            user_choice: plan.user_choice,
            analysis_request_id: plan.analysis_request_id.clone(),
        })
    }
}

/// Both ends of a plan, with display paths from the saved roots.
pub(crate) fn plan_ends(store: &Store, plan: &PlanRow) -> Result<(PlanEnd, PlanEnd), AppError> {
    let end = |root_id: &str, relative_path: String| -> Result<PlanEnd, AppError> {
        let display_path = match store.get_root(root_id)? {
            Some(row) => format!("{}/{relative_path}", row.display_path),
            None => relative_path.clone(),
        };
        Ok(PlanEnd {
            root_id: root_id.to_string(),
            relative_path,
            display_path,
        })
    };
    Ok((
        end(&plan.source_root_id, plan.source_relative_path.clone())?,
        end(&plan.target_root_id, plan.target_path())?,
    ))
}

pub(crate) fn load_plan(store: &Store, plan_id: &str) -> Result<PlanRow, AppError> {
    if !is_canonical_uuid(plan_id) {
        return Err(AppError::plan(
            ErrorCode::InvalidInput,
            "The plan ID is not valid.",
            None,
            None,
        ));
    }
    store.get_plan(plan_id)?.ok_or_else(|| {
        AppError::plan(
            ErrorCode::UnknownPlan,
            "There is no plan with this ID.",
            None,
            None,
        )
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
