//! traeva-core holds all of Traeva's rules: storage, observation, plans,
//! execution, record checks and Undo, and Memory's shared contract and
//! readers. It does not depend on Tauri, so `cargo test -p traeva-core`
//! runs without opening a window.

#[cfg(not(target_os = "macos"))]
compile_error!("Traeva's first release supports macOS only (docs/architecture.md §1).");

mod analysis;
mod app_data;
mod auto_drop;
mod auto_link;
mod auto_memory;
mod candidates;
mod capture;
mod changes;
mod classify;
mod contracts;
mod docx;
mod execute;
mod export;
mod family;
mod family_associate;
mod family_correct;
mod family_decide;
mod family_merge;
mod family_move;
mod find;
mod lineage;
mod memory_consent;
mod memory_contracts;
mod model;
mod observe;
// On-device text recognition for scanned PDFs, the parent's side (#186,
// organize-spec §14.8). The child's side comes from a separate port.
mod ocr;
mod opened;
mod operation;
// The PDF reader (#186, organize-spec §14), moved unchanged from the
// reviewed experiment. Only `pdf_child` calls it (`pdf::read_pdf`), for
// Organize; Memory does not read PDF. The rest (the page table, the
// scanning-work report) is kept from the experiment and not used here.
#[allow(dead_code)]
mod pdf;
mod pdf_child;
// G-OCR's text recognition child for scanned PDFs (#186, organize-spec
// 14.8): rendering, Apple's Vision and the memory monitor. `ocr` starts and
// watches it; the diagnostic readings kept from the experiment are unused.
#[allow(dead_code, unused_imports)]
mod pdf_ocr;
// E's PPTX reader (#28, contract §4.4 v23). Core calls its two
// projections; the experiment entry `read_pptx` and the re-exported limits
// are for E's validation crate, which compiles the same file.
mod plan;
mod platform;
#[allow(dead_code, unused_imports)]
mod pptx;
mod reader;
mod recover;
mod relocate;
mod retrieval;
mod settings;
mod snapshot;
mod source;
mod store;
mod undo;
// F's XLSX reader (#28, contract §4.4 v23). Core calls its two
// projections; the typed workbook API (`read_xlsx`, `coordinate_changes`)
// is for F's validation crate, which compiles the same files.
mod works;
#[allow(dead_code, unused_imports)]
mod xlsx;

/// Folder-tree snapshots for zero-write checks, shared with tests/common/.
#[cfg(test)]
#[path = "../tests/common/snapshot.rs"]
mod test_snapshot;

/// The fixture shared by the plan and execution tests inside this crate.
#[cfg(test)]
mod test_fixture;

#[cfg(test)]
#[path = "source_hook_tests.rs"]
mod source_hook_tests;

#[cfg(test)]
#[path = "volume_tests.rs"]
mod volume_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

pub use app_data::APP_DATA_DIR_VAR;
pub use auto_drop::{
    AUTO_DROP_NOTICE_VERSION, AutoDropRequest, AutoDropSettingsView, DropItemView, DropQueueView,
    DropState, PetItemView, PetStateView, ScopeView, auto_drop_notice,
};
pub use auto_memory::{
    AUTO_MEMORY_LIMIT, AUTO_MEMORY_NOTICE_VERSION, AutoMemoryOutcome, AutoMemoryRequest,
    AutoMemoryView, auto_memory_notice,
};
pub use candidates::{
    CANDIDATE_NOTICE_VERSION, CandidateCaptureInfo, CandidateSwitchRequest, OccupantView,
    PlaceHolds,
};
pub use capture::{CheckOutcome, MEMORY_CHECK_INTERVAL, MemoryWatchView, WatchCheck, WatchStatus};
pub use changes::{
    CHANGES_PROMPT_VERSION, CHANGES_SCHEMA_VERSION, ChangeAction, ChangeEvidence, ChangeLinesView,
    ChangeView, DIFFER_VERSION, HunkLines, HunkText, PairState, PassStep, QuoteAt, VersionPairView,
};
pub use classify::{
    Candidate, Clarification, ClassificationResult, Disposition, MAX_QUESTIONS,
    OUTPUT_SCHEMA_VERSION, PROMPT_VERSION,
};
pub use contracts::{
    ActionStatus, ActionView, AnsweredView, AppDataSource, AppError, AppState, BuildOutcome,
    CandidateView, ClarificationAnswer, ClarificationOption, ErrorCode, ErrorStage,
    ExtractionStatus, ExtractionSummary, FileView, FolderCatalogView, FolderView, ModelStatus,
    NextStep, OperationStage, OperationView, PlanEnd, PlanKind, PlanLifecycle, PlanRequest,
    PlanStatus, PlanTarget, PlanView, PragmaSettings, QuestionView, RootStatus, RootView,
    StorageInfo, StorageStatus, SuggestionView, UncertainField, UndoPreview, UndoStatus,
    UserChoice,
};
pub use docx::{DOCX_CACHED_READER, DOCX_READER};
pub use export::{ExportPreview, ExportRequest, ExportedVersion};
pub use family::{
    CANDIDATE_LIMIT, CandidateCapture, CoverageGapView, EdgeKind, FamilyMemberView,
    FileWatchesView, GapEnd, MemberBasis, MemoryFamilyView, MemoryRecordView, PendingCounts,
    PlaceView, RecordState, RecordedGapView, RelationStatus, VersionRelationView,
};
pub use family_associate::{
    AssociateChoice, AssociateChoiceKind, AssociationEvidence, AssociationPreview,
    AssociationRequest, AssociationSubject, AssociationSubjectView, ContinueOption, FileStanding,
};
pub use family_correct::{RelationAction, RelationPreview, RelationRequest};
pub use family_decide::{
    CandidateChoice, CandidateDecision, CandidateDecisionOutcome, CandidateDecisionRequest,
    CandidateReason, CurrentObjectChoice, PendingCandidate, PendingCandidatesView, UnreadOccupant,
};
pub use family_merge::{
    MergePreview, MergeRequest, OverrideView, SameContentHint, WorkDecisionOutcome,
};
pub use family_move::{MovePreview, MoveQuery, MoveRequest, MoveTarget};
pub use find::{
    FIND_NOTICE_VERSION, FIND_PROMPT_VERSION, FileKind, FileTime, FindCoverage, FindMatch,
    FindResult, FindSetupView, FindState, FindView, FindWords, MatchPlace, TimeAgo, TimeUnit,
    WordsBy, find_notice,
};
pub use memory_consent::{MEMORY_NOTICE_VERSION, MemoryConsentView, memory_consent_notice};
pub use memory_contracts::{
    DocxPreview, GapKind, PairBasis, PairCheck, PairEvidence, PairRefusal, PreviewText,
    SnapshotRef, TextExtraction, TextStatus, TextSummary, UsablePair, VersionPreview,
};
pub use model::{ModelAnswer, ModelRequest, Usage};
pub use observe::{Evidence, Extraction, Omission};
pub use ocr::{
    MemoryProbe, OCR_DPI, OCR_ENABLED, OCR_FLAG, OCR_GETTING_READY_NOTE, OCR_MAX_PAGES,
    OCR_MAX_PIXELS, OCR_MEMORY_LIMIT, OCR_POLL_INTERVAL, OCR_PREPARE_FLAG, OCR_PREPARE_TIME_LIMIT,
    OCR_TIME_LIMIT, OcrHelper, OcrState, ProcessMemory, child_footprint, ocr_child_main,
    ocr_prepare_main, ocr_supported_here,
};
pub use opened::{OPENED_DIR, OpenedCopy, OpenedVersion};
pub use pdf_child::{
    CappedAlloc, PDF_HEAP_CAP, PDF_TEXT_FLAG, PDF_TIME_LIMIT, PdfHelper, pdf_child_main,
};
pub use reader::{
    CellField, CellValueType, MAX_MEMORY_TEXT_CHARS, NO_READER, PLAIN_TEXT_READER,
    PPTX_MEMORY_READER, PPTX_PREVIEW_READER, SheetVisibility, SlideRole, StructureLine,
    XLSX_MEMORY_READER, XLSX_PREVIEW_READER, escape_source_line, has_structure_lines, read_text,
    structure_line, structure_name, unescape_source_line,
};
pub use relocate::{RelocationOutcome, RelocationPreview, RelocationRequest};
pub use retrieval::{
    AskedBy, Coverage, QUESTION_PROMPT_VERSION, ReachedBy, SEARCH_PROMPT_VERSION, SearchAnswer,
    SearchCandidate, SearchOption, SearchQuestion, SearchState, SearchView,
};
pub use settings::{
    DEFAULT_MODEL, IMAGE_NOTICE_VERSION, ImageSending, KEYCHAIN_SERVICE, ModelSettingsInput,
    ModelSettingsView, NOTICE_VERSION, PROVIDER_OPENAI, consent_notice,
};
pub use snapshot::SNAPSHOT_DIR;
pub use source::{
    SourceAssociationView, SourceCandidateView, SourceCaptureState, SourceDownloadEvent,
    SourceDownloadState, SourceIngestOutcome, SourceLinkKind, SourceRecordView, SourceRelation,
    SourceRoot, SourceState, SourceStatus,
};
pub use store::{DATABASE_FILE_NAME, LATEST_SCHEMA_VERSION, LOCK_FILE_NAME};
pub use works::{WorkDecisionKind, WorkSummaryView};

use observe::{FolderCatalog, ReadError};
use platform::macos::{self, Folder, FsError, Observation, Overlap, Protection};
use store::{FileEvidence, FileRow, RootEvidence, RootRow, SeenFile, Store};

/// A registered file and its extracted text. The text stays in memory: this
/// type is deliberately not serializable, so it cannot be sent over IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedFile {
    pub file: FileView,
    pub extraction: Extraction,
}

/// The one service the Tauri layer holds. If storage could not start, the service
/// still exists but is blocked: it reports why and refuses all new operations.
pub struct OrganizeService {
    storage: Result<Store, AppError>,
    /// The latest folder list of each root, so a plan request can name a
    /// listed folder. In memory only: a restart makes old lists unusable.
    catalogs: Mutex<HashMap<String, CachedCatalog>>,
    /// Held for a whole run of a plan, so runs happen one at a time.
    execution: Mutex<()>,
    /// Chosen files on their way to a plan (reading, then analysis), by
    /// operation ID. In memory only.
    operations: Mutex<HashMap<String, operation::Operation>>,
    /// Where model requests go and where the API key is kept (#14).
    model: model::ModelSetup,
    /// Memory's last look at each watch (#32), for the window. In memory only.
    memory_checks: Mutex<capture::LastChecks>,
    /// This start's run of Memory's watcher (#31, `memory_runs`), once it ran.
    memory_run: Mutex<Option<family::RunState>>,
    /// Memory's searches (#34): the question and the answers stay here, in
    /// memory only, never in the database.
    memory_searches: Mutex<retrieval::Searches>,
    /// Files dropped on Traeva's icon (#168), in order. In memory only.
    drops: Mutex<auto_drop::Drops>,
    /// Held for a whole automatic start of Memory (#201): the limit check,
    /// the start and its record, so two moves finishing together cannot
    /// both pass the 50-record check (R2-AM-01, #227).
    auto_memory: Mutex<()>,
    /// How Organize reads PDF files in a separate process (#186); none: it
    /// does not read them.
    pdf_helper: Mutex<Option<pdf_child::PdfHelper>>,
    /// On-device text recognition for scanned PDFs (#186, §14.8): the
    /// recognizer, its preparation and the recognitions kept. In memory only.
    ocr: Mutex<ocr::Ocr>,
    /// Held while a recognition or preparation child runs: one at a time.
    ocr_child: Mutex<()>,
    /// Set when the app quits: running children stop, no new ones start.
    ocr_stop: AtomicBool,
    /// Find's searches (#198): the question and the results stay here, in
    /// memory only, never in the database.
    finds: Mutex<find::Finds>,
    /// Texts Find read, for the next search. In memory only.
    find_texts: Mutex<find::KeptTexts>,
    /// Milliseconds since 1970. Tests replace it to jump past the 15 minutes.
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    /// Tests only: a new object at a record's place is never linked by
    /// itself (auto_link.rs), so the tests of what happens when it cannot be
    /// (the report, candidates, decisions) still reach those paths.
    #[cfg(test)]
    no_auto_link: AtomicBool,
}

struct CachedCatalog {
    catalog_id: String,
    catalog: FolderCatalog,
}

impl OrganizeService {
    /// Opens storage in `app_data_dir`. Tests pass a temporary folder here.
    pub fn start(app_data_dir: PathBuf) -> OrganizeService {
        OrganizeService::start_with_clock(app_data_dir, Box::new(now_ms))
    }

    /// What the app starts with: `~/Library/Application Support/Traeva/`
    /// (architecture.md §4), unless `TRAEVA_APP_DATA_DIR` names a separate
    /// test folder (#65; app_data.rs has its rules). A test folder that
    /// breaks them blocks storage with the reason: Traeva never falls back to
    /// the normal folder then.
    pub fn start_for_app() -> OrganizeService {
        OrganizeService::start_from(
            std::env::var_os(app_data::APP_DATA_DIR_VAR),
            std::env::var_os("HOME"),
        )
    }

    /// `start_for_app` with the two environment values passed in.
    fn start_from(
        override_dir: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
    ) -> OrganizeService {
        let chosen = app_data::choose(override_dir, home.clone()).and_then(|chosen| {
            if let app_data::Chosen::Test(dir) = &chosen {
                app_data::make_test_folder(dir, home)?;
            }
            Ok(chosen)
        });
        match chosen {
            Ok(app_data::Chosen::Default(dir)) => OrganizeService::start(dir),
            Ok(app_data::Chosen::Test(dir)) => {
                let mut service = OrganizeService::start(dir.clone());
                if let Ok(store) = service.storage.as_mut() {
                    store.mark_test_folder();
                }
                // Its own Keychain account: the normal key is never used.
                service.model.key_account_suffix = app_data::test_key_suffix(&dir);
                service
            }
            Err(error) => OrganizeService::with_storage(Err(error), Box::new(now_ms)),
        }
    }

    /// Before anything is written, the app-data folder must not be inside a
    /// folder the user authorized (#47). Then a restart ends every plan that
    /// no action has used yet (organize-spec.md §3): the user has to preview
    /// again. Model requests that were still waiting are marked interrupted:
    /// their answers can no longer arrive. Unfinished snapshot copies are
    /// removed (#32). Then, before anything else can run, every unsettled
    /// action gets its record check (§7). If that cannot be recorded, storage
    /// is blocked and the next start checks again.
    fn start_with_clock(
        app_data_dir: PathBuf,
        clock: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> OrganizeService {
        let storage = check_app_data_isolation(&app_data_dir)
            .and_then(|()| Store::open(&app_data_dir))
            .and_then(|store| {
                store.invalidate_open_plans(clock(), "Traeva restarted")?;
                store.interrupt_analyses()?;
                auto_drop::interrupt_unfinished(&store, clock())?;
                snapshot::clear_incoming(store.app_data_dir());
                Ok(store)
            });
        let mut service = OrganizeService::with_storage(storage, clock);
        if service.storage.is_ok()
            && let Err(error) = service.check_unsettled_actions()
        {
            service.storage = Err(error);
        }
        service
    }

    fn with_storage(
        storage: Result<Store, AppError>,
        clock: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> OrganizeService {
        OrganizeService {
            storage,
            catalogs: Mutex::new(HashMap::new()),
            execution: Mutex::new(()),
            operations: Mutex::new(HashMap::new()),
            model: model::ModelSetup::new(),
            memory_checks: Mutex::new(HashMap::new()),
            memory_run: Mutex::new(None),
            memory_searches: Mutex::new(HashMap::new()),
            drops: Mutex::new(auto_drop::Drops::default()),
            auto_memory: Mutex::new(()),
            pdf_helper: Mutex::new(None),
            ocr: Mutex::new(ocr::Ocr::default()),
            ocr_child: Mutex::new(()),
            ocr_stop: AtomicBool::new(false),
            finds: Mutex::new(HashMap::new()),
            find_texts: Mutex::new(find::KeptTexts::default()),
            clock,
            #[cfg(test)]
            no_auto_link: AtomicBool::new(false),
        }
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// What the window shows first, and again after a refresh: storage,
    /// roots, recent actions and every action waiting for a decision.
    /// Reading it never runs or resubmits anything.
    pub fn get_app_state(&self) -> Result<AppState, AppError> {
        let app_version = env!("CARGO_PKG_VERSION").to_string();
        let store = match &self.storage {
            Ok(store) => store,
            Err(error) => {
                return Ok(AppState {
                    app_version,
                    storage: StorageStatus::Blocked {
                        error: error.clone(),
                    },
                    model: ModelStatus::NotConfigured,
                    roots: Vec::new(),
                    recent_actions: Vec::new(),
                    decisions_needed: Vec::new(),
                });
            }
        };
        let mut recent_actions = Vec::new();
        for action_id in store.recent_actions(50)? {
            recent_actions.push(self.get_action(&action_id)?);
        }
        let mut decisions_needed = Vec::new();
        for action_id in store.actions_needing_a_decision()? {
            decisions_needed.push(self.get_action(&action_id)?);
        }
        Ok(AppState {
            app_version,
            storage: StorageStatus::Ready(store.status()?),
            model: self.model_status(),
            roots: self.list_roots()?,
            recent_actions,
            decisions_needed,
        })
    }

    /// Authorizes the folder at `path` (picked by the user in the native folder
    /// picker). Only reads: nothing is written inside the folder. Authorizing an
    /// already active folder again returns the existing root.
    pub fn authorize_root(&self, path: &Path) -> Result<RootView, AppError> {
        let store = self.store()?;
        if !path.is_absolute() || path.to_str().is_none() {
            return Err(AppError::authorization(
                ErrorCode::InvalidInput,
                "The folder path must be an absolute path.",
            ));
        }
        let shown = path.display().to_string();
        let folder = macos::open_folder_path(path).map_err(|err| open_error(&shown, err))?;
        let facts = inspect_root(store, &folder, &shown)?;
        let new_root = RootRow {
            root_id: uuid::Uuid::new_v4().to_string(),
            display_path: facts.real_path,
            evidence: RootEvidence {
                fs_type: facts.fs_type,
                ..root_evidence_of(folder.observation())
            },
            status: RootStatus::Active,
            granted_at_ms: self.now(),
            revoked_at_ms: None,
        };
        let saved = store.insert_root_unless_active(&new_root)?;
        Ok(root_view(&saved, None))
    }

    /// Revokes a root for good. Unconsumed plans that use it can never run,
    /// because they keep pointing at this root_id; authorizing the folder again
    /// creates a new root_id. Revoking twice returns the revoked root.
    pub fn revoke_root(&self, root_id: &str) -> Result<RootView, AppError> {
        let store = self.store()?;
        check_root_id(root_id)?;
        match store.revoke_root(root_id, self.now())? {
            Some(row) => Ok(root_view(&row, None)),
            None => Err(unknown_root()),
        }
    }

    /// The active roots, each checked again on disk right now.
    pub fn list_roots(&self) -> Result<Vec<RootView>, AppError> {
        let store = self.store()?;
        let mut views = Vec::new();
        for row in store.active_roots()? {
            let problem = self.verified_root(&row.root_id).err();
            views.push(root_view(&row, problem));
        }
        Ok(views)
    }

    /// Registers the file at `picked` (an absolute path from the native file
    /// picker) inside root `root_id`: reads it twice with a pause (organize-spec.md
    /// §4.1), hashes the exact bytes, records the observation in app data and
    /// extracts its text. Nothing is written inside the root.
    pub fn register_file(&self, root_id: &str, picked: &Path) -> Result<ObservedFile, AppError> {
        self.register_file_with(root_id, picked, &mut |root, parent, name| {
            observe::read_stable(parent, name, root.id().dev)
        })
    }

    /// `register_file` with the stable read passed in, so tests can change the
    /// file between the two reads. Never uses text recognition: Memory and
    /// Source register files here too.
    fn register_file_with(
        &self,
        root_id: &str,
        picked: &Path,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<observe::StableRead, ReadError>,
    ) -> Result<ObservedFile, AppError> {
        self.register_file_reading(root_id, picked, read, ocr::OcrUse::Never)
    }

    /// `register_file_with` for Organize's own operations: a PDF without a
    /// text layer may be read by text recognition (organize-spec.md §14.8).
    pub(crate) fn register_file_for_organize(
        &self,
        root_id: &str,
        picked: &Path,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<observe::StableRead, ReadError>,
    ) -> Result<ObservedFile, AppError> {
        self.register_file_reading(root_id, picked, read, ocr::OcrUse::Allowed)
    }

    fn register_file_reading(
        &self,
        root_id: &str,
        picked: &Path,
        read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<observe::StableRead, ReadError>,
        ocr: ocr::OcrUse,
    ) -> Result<ObservedFile, AppError> {
        let store = self.store()?;
        let (root_row, root) = self.verified_root(root_id)?;
        let shown = picked.display().to_string();
        let (segments, name) = relative_location(&root_row.display_path, picked)?;
        let stable = read_in_root(&root, &segments, &name, &shown, read)?;
        let row = store.record_observation(&seen_file(
            &root_row.root_id,
            &segments,
            &name,
            &stable,
            self.now(),
        ))?;
        // A source confirmed for the same physical file under an older record
        // follows it to this one (#27). That is Source's bookkeeping: if it
        // cannot be saved, the source stays on the older record and choosing
        // the file still works as before.
        let _ = self.continue_registered_sources(&row);
        // The text comes from exactly the bytes that were hashed.
        let extraction = self.organize_extraction(&name, &stable.bytes, ocr);
        Ok(ObservedFile {
            file: file_view(&row, &root_row.display_path, &extraction),
            extraction,
        })
    }

    /// The bounded folder list of a root: at most 500 folders, 6 levels deep,
    /// never entering hidden folders, packages or project folders.
    pub fn list_folders(&self, root_id: &str) -> Result<FolderCatalogView, AppError> {
        let (row, root) = self.verified_root(root_id)?;
        let catalog = observe::build_catalog(&root).map_err(|_| {
            AppError::authorization(
                ErrorCode::RootUnavailable,
                format!("Traeva could not list the folders in {}.", row.display_path),
            )
        })?;
        // Keep this list so a plan request can name one of its folders. It
        // replaces the previous list of the root, whose IDs stop working.
        let catalog_id = uuid::Uuid::new_v4().to_string();
        let view = catalog_view(&row.root_id, &catalog_id, &catalog);
        self.catalogs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                row.root_id.clone(),
                CachedCatalog {
                    catalog_id,
                    catalog,
                },
            );
        Ok(view)
    }

    /// Opens an active root by its saved path and proves it is still the same
    /// folder (`RootEvidence::same_object`) before anything uses it. The
    /// returned handle anchors every later walk inside this root.
    pub(crate) fn verified_root(&self, root_id: &str) -> Result<(RootRow, Folder), AppError> {
        let store = self.store()?;
        check_root_id(root_id)?;
        let row = store.get_root(root_id)?.ok_or_else(unknown_root)?;
        let shown = row.display_path.clone();
        if row.status == RootStatus::Revoked {
            return Err(AppError::authorization(
                ErrorCode::RootRevoked,
                format!("The folder {shown} was removed from Traeva. Choose it again to use it."),
            ));
        }
        let folder =
            macos::open_folder_path(Path::new(&row.display_path)).map_err(|err| match err {
                FsError::SymbolicLink | FsError::NotADirectory => root_changed(&shown),
                other => open_error(&shown, other),
            })?;
        if !root_evidence_of(folder.observation()).same_object(&row.evidence) {
            return Err(root_changed(&shown));
        }
        inspect_root(store, &folder, &shown)?;
        Ok((row, folder))
    }

    fn store(&self) -> Result<&Store, AppError> {
        self.storage.as_ref().map_err(Clone::clone)
    }
}

/// Startup, before the first write (organize-spec.md §2.2; #47): no active
/// authorized folder may be the app-data folder or any folder above it.
/// Folders are compared by their evidence (`RootEvidence::same_object`),
/// walking up from the app-data folder itself, so a folder moved while
/// Traeva was closed, or a symbolic link left in its place, cannot hide that
/// the database now sits inside an authorized folder. If it does, or Traeva cannot tell, nothing
/// is written and storage stays blocked.
fn check_app_data_isolation(app_data_dir: &Path) -> Result<(), AppError> {
    let shown = app_data_dir.display();
    let cannot_tell = || {
        AppError::storage(
            ErrorCode::OverlapsAppData,
            format!(
                "Traeva could not check that its data folder {shown} is outside the folders you \
                 authorized, so it did not start and wrote nothing."
            ),
        )
    };
    // No database yet (first start) means no folder was ever authorized.
    let roots = store::saved_roots(app_data_dir)?;
    if roots.is_empty() {
        return Ok(());
    }
    let folder = macos::open_real_folder(app_data_dir).map_err(|_| cannot_tell())?;
    let around = macos::ancestors(folder.fd()).map_err(|_| cannot_tell())?;
    let inside = around.iter().any(|seen| {
        let seen = root_evidence_of(seen);
        roots.iter().any(|root| root.same_object(&seen))
    });
    if inside {
        return Err(AppError::storage(
            ErrorCode::OverlapsAppData,
            format!(
                "Traeva's data folder {shown} is now inside a folder you authorized, so Traeva \
                 did not start and wrote nothing. Move the data folder back out of it, then open \
                 Traeva again."
            ),
        ));
    }
    Ok(())
}

/// All root rules (organize-spec.md §2.1, §2.2, §4.2), with the app-data
/// folder this service actually uses.
fn inspect_root(store: &Store, folder: &Folder, shown: &str) -> Result<macos::RootFacts, AppError> {
    let app_data = macos::open_folder_path(store.app_data_dir())
        .map_err(|_| overlap_error(shown, Overlap::CannotTell))?;
    macos::inspect_root(folder, &app_data, &library_folders()).map_err(|err| rule_error(shown, err))
}

/// Folders that hold application data. Roots may not be inside them.
fn library_folders() -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from("/Library")];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(PathBuf::from(home).join("Library"));
    }
    folders
}

fn open_error(shown: &str, err: FsError) -> AppError {
    match err {
        FsError::SymbolicLink => AppError::authorization(
            ErrorCode::UnsupportedLocation,
            format!("{shown} is a symbolic link. Choose the folder it points to instead."),
        ),
        FsError::NotADirectory => AppError::authorization(
            ErrorCode::UnsupportedLocation,
            format!("{shown} is not a folder."),
        ),
        FsError::PermissionDenied => AppError::authorization(
            ErrorCode::RootUnavailable,
            format!("Traeva is not allowed to open {shown}."),
        ),
        _ => AppError::authorization(
            ErrorCode::RootUnavailable,
            format!("The folder {shown} is missing or cannot be opened."),
        ),
    }
}

fn rule_error(shown: &str, err: FsError) -> AppError {
    let protected = |why: String| {
        AppError::authorization(ErrorCode::ProtectedLocation, format!("{shown} {why}"))
    };
    match err {
        FsError::OverlapsAppData(overlap) => overlap_error(shown, overlap),
        FsError::Protected(Protection::Hidden) => {
            protected("is hidden or inside a hidden folder.".to_string())
        }
        FsError::Protected(Protection::Package(name)) => {
            protected(format!("is the package “{name}” or inside it."))
        }
        FsError::Protected(Protection::Project(marker)) => protected(format!(
            "is a project folder or inside one (found “{marker}”). Traeva does not reorganize projects."
        )),
        FsError::Protected(Protection::AppData) => {
            protected("is inside a Library folder, which holds app data.".to_string())
        }
        FsError::UnsupportedVolume(why) => AppError::authorization(
            ErrorCode::UnsupportedLocation,
            format!("{shown} is on an unsupported volume ({why})."),
        ),
        FsError::Dataless => AppError::authorization(
            ErrorCode::UnsupportedLocation,
            format!("{shown} is a cloud placeholder that is not downloaded to this Mac."),
        ),
        FsError::NotADirectory => AppError::authorization(
            ErrorCode::UnsupportedLocation,
            format!("{shown} is not a folder."),
        ),
        _ => AppError::authorization(
            ErrorCode::RootUnavailable,
            format!("Traeva could not check {shown}."),
        ),
    }
}

fn overlap_error(shown: &str, overlap: Overlap) -> AppError {
    let why = match overlap {
        Overlap::RootContainsAppData => "contains Traeva's own data folder",
        Overlap::RootInsideAppData => "is Traeva's own data folder or inside it",
        Overlap::CannotTell => {
            "might overlap Traeva's own data folder, and Traeva cannot rule it out"
        }
    };
    AppError::authorization(ErrorCode::OverlapsAppData, format!("{shown} {why}."))
}

/// Splits a picked absolute file path into folder names below the root plus
/// the file name. `canonicalize` only helps derive the names (both paths are
/// resolved the same way); the real boundary is the handle walk that follows.
/// The file name itself is not resolved, so a picked symbolic link stays a link
/// and is refused later.
fn relative_location(root_path: &str, picked: &Path) -> Result<(Vec<String>, String), AppError> {
    let shown = picked.display().to_string();
    let invalid = || {
        AppError::observation(
            ErrorCode::InvalidInput,
            format!("{shown} is not a valid file path."),
        )
    };
    if !picked.is_absolute() {
        return Err(invalid());
    }
    let name = picked
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?
        .to_string();
    let parent = picked.parent().ok_or_else(invalid)?;
    let parent = std::fs::canonicalize(parent).map_err(|_| {
        AppError::observation(ErrorCode::FileNotFound, format!("{shown} does not exist."))
    })?;
    let root = std::fs::canonicalize(root_path).map_err(|_| {
        AppError::observation(
            ErrorCode::FileUnreadable,
            format!("Traeva could not check where {shown} is."),
        )
    })?;
    let inside = parent.strip_prefix(&root).map_err(|_| {
        AppError::observation(
            ErrorCode::OutsideRoot,
            format!("{shown} is not inside the authorized folder {root_path}."),
        )
    })?;
    let mut segments = Vec::new();
    for part in inside.components() {
        segments.push(part.as_os_str().to_str().ok_or_else(invalid)?.to_string());
    }
    Ok((segments, name))
}

fn file_error(shown: &str, err: FsError) -> AppError {
    let unsupported =
        |why: &str| AppError::observation(ErrorCode::UnsupportedFile, format!("{shown} {why}"));
    match err {
        FsError::NotFound => {
            AppError::observation(ErrorCode::FileNotFound, format!("{shown} does not exist."))
        }
        FsError::Protected(Protection::Hidden) => {
            unsupported("is hidden. Traeva does not organize hidden files.")
        }
        FsError::Protected(Protection::Package(name)) => AppError::observation(
            ErrorCode::ProtectedLocation,
            format!("{shown} is inside the package “{name}”."),
        ),
        FsError::Protected(Protection::Project(marker)) => AppError::observation(
            ErrorCode::ProtectedLocation,
            format!("{shown} is inside a project folder (found “{marker}”)."),
        ),
        FsError::Protected(Protection::AppData) => AppError::observation(
            ErrorCode::ProtectedLocation,
            format!("{shown} is inside a folder that holds app data."),
        ),
        FsError::SymbolicLink => unsupported("is a symbolic link."),
        FsError::NotADirectory => unsupported("is not inside a plain folder."),
        FsError::NotSupportedFile(what) => unsupported(&format!("is a {what}.")),
        FsError::CrossVolume => unsupported("is on another volume."),
        FsError::Dataless => {
            unsupported("is a cloud placeholder that is not downloaded to this Mac.")
        }
        FsError::InvalidName(_) => AppError::observation(
            ErrorCode::InvalidInput,
            format!("{shown} is not a valid file path."),
        ),
        FsError::ChainChanged => AppError::observation(
            ErrorCode::FileChanging,
            format!("A folder above {shown} was moved while Traeva read the file."),
        ),
        _ => AppError::observation(
            ErrorCode::FileUnreadable,
            format!("Traeva could not read {shown}."),
        ),
    }
}

fn read_error(shown: &str, err: ReadError) -> AppError {
    match err {
        ReadError::Fs(fs_error) => file_error(shown, fs_error),
        ReadError::TooLarge => AppError::observation(
            ErrorCode::FileTooLarge,
            format!(
                "{shown} is larger than 20 MiB. Traeva does not read or move files that large."
            ),
        ),
        ReadError::StillChanging { attempts } => AppError::observation(
            ErrorCode::FileChanging,
            format!(
                "{shown} kept changing while Traeva read it ({attempts} attempts). \
                 Wait until it is saved, then try again."
            ),
        ),
    }
}

/// Reads file `name`, in the folders `segments` below the root, stably and
/// through handles opened from the root handle; then checks that those
/// folders did not move while it was read. Only reads.
fn read_in_root(
    root: &Folder,
    segments: &[String],
    name: &str,
    shown: &str,
    read: &mut dyn FnMut(&Folder, &Folder, &str) -> Result<observe::StableRead, ReadError>,
) -> Result<observe::StableRead, AppError> {
    let names: Vec<&str> = segments.iter().map(String::as_str).collect();
    let chain = macos::open_chain(root, &names).map_err(|err| file_error(shown, err))?;
    let parent = chain.last().unwrap_or(root);
    let stable = read(root, parent, name).map_err(|err| read_error(shown, err))?;
    // The folders must still be where they were while the file was read.
    macos::verify_chain(root, &chain).map_err(|err| file_error(shown, err))?;
    Ok(stable)
}

/// One stable read, ready to be recorded by `Store::record_observation`.
fn seen_file(
    root_id: &str,
    segments: &[String],
    name: &str,
    stable: &observe::StableRead,
    now_ms: i64,
) -> SeenFile {
    let mut relative_path = segments.join("/");
    if !relative_path.is_empty() {
        relative_path.push('/');
    }
    relative_path.push_str(name);
    SeenFile {
        root_id: root_id.to_string(),
        relative_path,
        basename: name.to_string(),
        sha256: stable.sha256.clone(),
        byte_length: stable.bytes.len() as u64,
        evidence: evidence_of(&stable.observed),
        now_ms,
    }
}

/// The physical evidence Traeva saves from one observation of a file.
fn evidence_of(observed: &Observation) -> FileEvidence {
    FileEvidence {
        dev: observed.id.dev,
        volume: macos::volume_uuid(observed.id.dev),
        ino: observed.id.ino,
        birthtime_ns: observed.birthtime_ns,
        mtime_ns: observed.mtime_ns,
        ctime_ns: observed.ctime_ns,
        link_count: observed.link_count,
    }
}

/// A folder's evidence as it is saved for a root, from one observation.
/// `fs_type` is left empty: it is never compared.
fn root_evidence_of(observed: &Observation) -> RootEvidence {
    RootEvidence {
        dev: observed.id.dev,
        volume: macos::volume_uuid(observed.id.dev),
        ino: observed.id.ino,
        birthtime_ns: observed.birthtime_ns,
        fs_type: String::new(),
    }
}

fn file_view(row: &FileRow, root_path: &str, extraction: &Extraction) -> FileView {
    FileView {
        file_id: row.file_id.clone(),
        file_revision: row.record_revision,
        root_id: row.root_id.clone(),
        relative_path: row.relative_path.clone(),
        display_path: format!("{root_path}/{}", row.relative_path),
        byte_length: row.byte_length,
        sha256: row.sha256.clone(),
        extraction: extraction.summary.clone(),
    }
}

fn catalog_view(root_id: &str, catalog_id: &str, catalog: &FolderCatalog) -> FolderCatalogView {
    FolderCatalogView {
        root_id: root_id.to_string(),
        catalog_id: catalog_id.to_string(),
        folders: catalog
            .folders
            .iter()
            .map(|folder| FolderView {
                folder_id: folder.folder_id.clone(),
                parent_id: folder.parent_id.clone(),
                relative_path: folder.segments.join("/"),
                depth: folder.segments.len(),
                protected: folder.protected.as_ref().map(|why| match why {
                    Protection::Project(marker) => format!("project folder (contains “{marker}”)"),
                    other => format!("{other:?}"),
                }),
            })
            .collect(),
        truncated: catalog.truncated,
    }
}

fn root_changed(shown: &str) -> AppError {
    AppError::authorization(
        ErrorCode::RootChanged,
        format!(
            "The folder at {shown} is not the folder you authorized (it was moved or replaced). \
             Choose it again."
        ),
    )
}

fn unknown_root() -> AppError {
    AppError::authorization(
        ErrorCode::UnknownRoot,
        "There is no authorized folder with this ID.",
    )
}

/// Root IDs are lowercase, hyphenated UUIDs, exactly as Traeva created them.
fn check_root_id(root_id: &str) -> Result<(), AppError> {
    if is_canonical_uuid(root_id) {
        Ok(())
    } else {
        Err(AppError::authorization(
            ErrorCode::InvalidInput,
            "The folder ID is not valid.",
        ))
    }
}

/// IDs are lowercase, hyphenated UUIDs, exactly as Traeva created them.
fn is_canonical_uuid(id: &str) -> bool {
    matches!(uuid::Uuid::try_parse(id), Ok(parsed) if parsed.hyphenated().to_string() == id)
}

fn root_view(row: &RootRow, problem: Option<AppError>) -> RootView {
    RootView {
        root_id: row.root_id.clone(),
        display_path: row.display_path.clone(),
        status: row.status,
        granted_at_ms: row.granted_at_ms,
        revoked_at_ms: row.revoked_at_ms,
        problem,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
