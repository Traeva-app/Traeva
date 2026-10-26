//! Test-only driver for the existing Memory and local DOCX preview APIs.
use crate::docx::preview_docx_snapshot;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};
use traeva_core::{CandidateDecisionRequest, CandidateSwitchRequest, OrganizeService};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn error(e: traeva_core::AppError) -> Value {
    json!({"error":e})
}

pub struct Session {
    service: OrganizeService,
    chain: String,
    source: PathBuf,
}

impl Session {
    pub fn new(source: &Path, data: &Path) -> Result<Self, String> {
        let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
        if !source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("docx"))
            || !source.is_file()
        {
            return Err("Use a synthetic .docx source file".into());
        }
        if data.exists() {
            return Err("Requires fresh isolated app-data".into());
        }
        let data = fs::canonicalize(data.parent().ok_or("No data parent")?)
            .map_err(|e| e.to_string())?
            .join(data.file_name().ok_or("No data name")?);
        let root = source.parent().ok_or("No source parent")?;
        if data.starts_with(root) || root.starts_with(&data) {
            return Err("App-data must be outside the source root".into());
        }
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&data)
            .map_err(|e| e.to_string())?;
        let service = OrganizeService::start(data.clone());
        let authorized = service.authorize_root(root).map_err(|e| format!("{e:?}"))?;
        let record = service
            .start_memory_at(&authorized.root_id, &source)
            .map_err(|e| format!("{e:?}"))?;
        fs::write(
            data.join("docx-memory-e2e-session.json"),
            json!({"chain":record.chain_id,"source":source}).to_string(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            service,
            chain: record.chain_id,
            source,
        })
    }

    pub fn resume(source: &Path, data: &Path, chain: &str) -> Result<Self, String> {
        if !data.is_dir() {
            return Err("Existing isolated app-data required".into());
        }
        let marker = fs::read(data.join("docx-memory-e2e-session.json"))
            .map_err(|_| "Requires this driver's isolated session marker")?;
        if marker.len() > 8192 {
            return Err("Invalid session marker".into());
        }
        let marker: Value = serde_json::from_slice(&marker).map_err(|e| e.to_string())?;
        if marker["chain"] != chain || marker["source"].as_str().map(Path::new) != Some(source) {
            return Err("Session identity differs from marker".into());
        }
        let service = OrganizeService::start(data.to_path_buf());
        service.memory_record(chain).map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            service,
            chain: chain.into(),
            source: source.into(),
        })
    }

    pub fn history(&self) -> Value {
        let record = match self.service.memory_record(&self.chain) {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        let family = match self.service.memory_family(&record.family_id) {
            Ok(f) => f,
            Err(e) => return error(e),
        };
        let previews:Vec<_> = family.members.iter().flat_map(|m| &m.snapshots).map(|s| {
            match preview_docx_snapshot(&self.service, &s.snapshot_id) {
                Ok((saved,text)) => json!({"snapshot":saved,"preview":{"summary":text.summary,"text_sha256":digest(text.text.as_bytes())},"verified_bytes_sha256":saved.sha256}),
                Err(e) => json!({"snapshot":s,"error":e}),
            }
        }).collect();
        let source = fs::symlink_metadata(&self.source).map(|m|json!({"dev":m.dev(),"ino":m.ino(),"bytes":m.len(),"mtime_sec":m.mtime(),"mtime_nsec":m.mtime_nsec()})).unwrap_or_else(|e|json!({"missing":true,"error":e.to_string()}));
        json!({"chain_id":self.chain,"source":source,"record":record,"family":family,"previews":previews,"pairs":self.service.usable_pairs_of_family(&record.family_id).map(|p|json!(p)).unwrap_or_else(error),"scope":"core_only_local_test"})
    }

    pub fn command(&self, request: Value) -> Value {
        let result = match request["op"].as_str() {
            Some("history") => self.history(),
            Some("check") => self
                .service
                .check_memory()
                .map(|c| json!({"checks":c}))
                .unwrap_or_else(error),
            Some("enable_candidates") => match self.service.candidate_capture(&self.chain) {
                Err(e) => error(e),
                Ok(info) => self
                    .service
                    .set_candidate_capture(&CandidateSwitchRequest {
                        chain_id: self.chain.clone(),
                        on: true,
                        notice_version: Some(info.notice_version),
                        occupant_token: info.unread_occupant.map(|o| o.occupant_token),
                        place_token: Some(info.place_token),
                    })
                    .map(|r| json!({"record":r}))
                    .unwrap_or_else(error),
            },
            Some("pending") => self
                .service
                .pending_candidates(&self.chain)
                .map(|p| json!({"page":p}))
                .unwrap_or_else(error),
            Some("decide") => match serde_json::from_value::<CandidateDecisionRequest>(
                request["request"].clone(),
            ) {
                Ok(r) if r.chain_id == self.chain => self
                    .service
                    .decide_pending_candidates(&r)
                    .map(|o| json!({"decision":o,"basis":"explicit_synthetic_test_author_choice"}))
                    .unwrap_or_else(error),
                _ => json!({"driver_error":"Invalid decision or different record"}),
            },
            _ => {
                json!({"driver_error":"Commands: history, check, enable_candidates, pending, decide, quit"})
            }
        };
        json!({"op":request["op"],"result":result})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(temp: &tempfile::TempDir) -> PathBuf {
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let source = root.join("synthetic.docx");
        fs::write(
            &source,
            include_bytes!("../fixtures/docx/word-layout-v1.docx"),
        )
        .unwrap();
        source
    }

    #[test]
    fn memory_e2e_app_data_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::Builder::new()
            .prefix("traeva-d-e2e-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let path = source(&temp);
        let data = temp.path().with_extension("app-data");
        let session = Session::new(&path, &data).unwrap();
        let mode = fs::metadata(&data).unwrap().permissions().mode() & 0o777;
        drop(session);
        fs::remove_dir_all(&data).unwrap();
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn memory_e2e_rejects_existing_or_overlapping_data_without_modifying_it() {
        let temp = tempfile::Builder::new()
            .prefix("traeva-d-e2e-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let path = source(&temp);
        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("sentinel"), b"keep").unwrap();
        assert!(Session::new(&path, &existing).is_err());
        assert!(Session::new(&path, &path.parent().unwrap().join("data")).is_err());
        assert_eq!(fs::read(existing.join("sentinel")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&existing).unwrap().count(), 1);
        assert!(!path.parent().unwrap().join("data").exists());
    }

    #[test]
    fn memory_e2e_resume_requires_matching_test_session_and_refuses_other_chain() {
        let temp = tempfile::Builder::new()
            .prefix("traeva-d-e2e-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let path = source(&temp);
        assert!(Session::resume(&path, temp.path(), "unknown").is_err());
        assert!(!temp.path().join("traeva.sqlite3").exists());
        let data = temp.path().join("data");
        let session = Session::new(&path, &data).unwrap();
        let chain = session.chain.clone();
        let history = session.history();
        assert!(Session::resume(&path, &data, "different").is_err());
        assert!(Session::resume(&path.with_file_name("other.docx"), &data, &chain).is_err());
        let refused = session.command(
            json!({"op":"decide", "request":{"chain_id":"other", "page_token":"token","items":[]}}),
        );
        assert!(refused["result"]["driver_error"].is_string());
        assert_eq!(session.history()["family"], history["family"]);
        drop(session);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            Session::resume(&path, &data, &chain).unwrap().history()["previews"],
            history["previews"]
        );
    }
}
