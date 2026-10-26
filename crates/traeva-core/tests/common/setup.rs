//! One service on a temporary app-data folder, with one authorized root that
//! holds the sample tree, an empty "Taxes" folder and a project folder
//! ("web", which has a package.json). Everything goes through the public API.

use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;
use traeva_core::{
    AppError, BuildOutcome, FileView, FolderCatalogView, OrganizeService, PlanRequest, PlanTarget,
    PlanView,
};

use super::{open_for_inspection, snapshot};

pub struct Setup {
    pub app: tempfile::TempDir,
    pub root_dir: tempfile::TempDir,
    service: Option<OrganizeService>,
    pub root_id: String,
}

impl Setup {
    pub fn new() -> Setup {
        let app = tempfile::Builder::new()
            .prefix("traeva-appdata-")
            .tempdir()
            .unwrap();
        let root_dir = tempfile::Builder::new()
            .prefix("traeva-root-")
            .tempdir()
            .unwrap();
        snapshot::fill_sample_tree(root_dir.path());
        fs::create_dir(root_dir.path().join("Taxes")).unwrap();
        fs::create_dir(root_dir.path().join("web")).unwrap();
        fs::write(root_dir.path().join("web/package.json"), "{}").unwrap();
        let service = OrganizeService::start(app.path().join("Traeva"));
        let root_id = service.authorize_root(root_dir.path()).unwrap().root_id;
        Setup {
            app,
            root_dir,
            service: Some(service),
            root_id,
        }
    }

    pub fn service(&self) -> &OrganizeService {
        self.service.as_ref().unwrap()
    }

    /// Quits and starts Traeva again on the same app data.
    pub fn restart(&mut self) {
        self.service = None;
        self.service = Some(OrganizeService::start(self.app.path().join("Traeva")));
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root_dir.path().join(relative)
    }

    pub fn register(&self, relative: &str) -> FileView {
        self.service()
            .register_file(&self.root_id, &self.path(relative))
            .unwrap()
            .file
    }

    pub fn catalog(&self) -> FolderCatalogView {
        self.service().list_folders(&self.root_id).unwrap()
    }

    pub fn request(
        &self,
        file: &FileView,
        catalog: &FolderCatalogView,
        folder: &str,
        new: &[&str],
    ) -> PlanRequest {
        let folder_id = catalog
            .folders
            .iter()
            .find(|f| f.relative_path == folder)
            .unwrap_or_else(|| panic!("{folder} is not listed"))
            .folder_id
            .clone();
        PlanRequest {
            file_id: file.file_id.clone(),
            file_revision: file.file_revision,
            target_root_id: catalog.root_id.clone(),
            operation_id: None,
            target: PlanTarget::Folder {
                catalog_id: catalog.catalog_id.clone(),
                folder_id,
                new_folders: new.iter().map(|n| n.to_string()).collect(),
            },
        }
    }

    pub fn build(
        &self,
        file: &FileView,
        folder: &str,
        new: &[&str],
    ) -> Result<BuildOutcome, AppError> {
        let catalog = self.catalog();
        self.service()
            .build_plan(&self.request(file, &catalog, folder, new))
    }

    pub fn plan(&self, file: &FileView, folder: &str, new: &[&str]) -> PlanView {
        match self.build(file, folder, new).unwrap() {
            BuildOutcome::Planned { plan } => plan,
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    pub fn db(&self) -> Connection {
        open_for_inspection(&self.app.path().join("Traeva"))
    }

    pub fn count(&self, table: &str) -> i64 {
        self.db()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// The action's journal as (step, phase), in order.
    pub fn journal(&self, action_id: &str) -> Vec<(String, String)> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT step, phase FROM action_events WHERE action_id = ?1 ORDER BY sequence")
            .unwrap();
        stmt.query_map([action_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// Makes the temporary database refuse one kind of write, as a full disk
    /// would. `on` is the trigger's event and condition. The app has no
    /// switch for this.
    pub fn refuse(&self, on: &str) {
        self.db()
            .execute_batch(&format!(
                "CREATE TRIGGER test_refuse {on} BEGIN SELECT RAISE(ABORT, 'disk full (test)'); END;"
            ))
            .unwrap();
    }

    pub fn allow_again(&self) {
        self.db()
            .execute_batch("DROP TRIGGER test_refuse;")
            .unwrap();
    }
}

/// `[("confirm", "result"), ...]` as owned strings, to compare with `journal`.
pub fn steps(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(step, phase)| (step.to_string(), phase.to_string()))
        .collect()
}
