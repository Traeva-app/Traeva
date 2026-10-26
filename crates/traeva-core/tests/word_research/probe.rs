use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use traeva_core::OrganizeService;
use traeva_word_research::prototype::{MAX_DOCX_BYTES, opening_reasons, read_docx};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn input(path: &str) -> Result<Vec<u8>, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > MAX_DOCX_BYTES as u64 {
        return Err("Input exceeds the research budget".into());
    }
    let mut data = Vec::new();
    (&mut f)
        .take(MAX_DOCX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > MAX_DOCX_BYTES {
        return Err("Input grew beyond budget".into());
    }
    Ok(data)
}
fn report(bytes: &[u8], expected: Option<&str>) -> Result<serde_json::Value, String> {
    let result = read_docx(bytes);
    let matches = expected
        .map(|p| fs::read_to_string(p).map(|s| s == result.text))
        .transpose()
        .map_err(|e| e.to_string())?;
    let value = serde_json::json!({"input_sha256":hash(bytes),"input_bytes":bytes.len(),"summary":result.summary,"text_sha256":hash(result.text.as_bytes()),"expected_matches":matches,"opening_reasons":opening_reasons(bytes)});
    if matches == Some(false) {
        return Err(format!("Expected text mismatch: {value}"));
    }
    Ok(value)
}
fn run() -> Result<serde_json::Value, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("inspect") if (2..=3).contains(&args.len()) => report(&input(&args[1])?, args.get(2).map(String::as_str)),
        Some("capture") if args.len() == 3 => {
            let p = fs::canonicalize(&args[1]).map_err(|e| e.to_string())?;
            let data = Path::new(&args[2]);
            if data.exists() { return Err("Capture requires fresh isolated app-data".into()); }
            let service = OrganizeService::start(data.to_path_buf());
            let root = service.authorize_root(p.parent().ok_or("No parent")?).map_err(|e| format!("{e:?}"))?;
            let f = service.register_file(&root.root_id, &p).map_err(|e| format!("{e:?}"))?.file;
            service.start_memory(&f.file_id).map_err(|e| format!("{e:?}"))?;
            let snapshots = service.list_snapshots(&f.file_id).map_err(|e| format!("{e:?}"))?;
            let snapshot = snapshots.first().ok_or("No snapshot")?;
            let (saved, bytes) = service.read_snapshot(&snapshot.snapshot_id).map_err(|e| format!("{e:?}"))?;
            service.stop_memory(&f.file_id).map_err(|e| format!("{e:?}"))?;
            Ok(serde_json::json!({"snapshot":saved,"reader":report(&bytes,None)?}))
        }
        Some("copy") if args.len() == 3 => {
            if !Path::new(&args[1]).is_dir() { return Err("App-data does not exist".into()); }
            let service = OrganizeService::start(args[1].clone().into());
            let (snapshot, bytes) = service.read_snapshot(&args[2]).map_err(|e| format!("{e:?}"))?;
            let reasons = opening_reasons(&bytes);
            if !reasons.is_empty() {
                return Ok(serde_json::json!({"research_refused":true,"reasons":reasons,"snapshot":snapshot}));
            }
            let copy = service.copy_for_opening(&args[2]).map_err(|e| format!("{e:?}"))?;
            if copy.can_open { return Err("Production whitelist unexpectedly permits this input".into()); }
            let original_copy = Path::new(&copy.path);
            let research_copy = original_copy.with_extension("docx");
            // Keep the extensionless core copy and create another file without
            // replacing anything. This lab path never launches a viewer.
            let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&research_copy).map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            file.set_permissions(fs::Permissions::from_mode(0o444)).map_err(|e| e.to_string())?;
            let meta = fs::symlink_metadata(&research_copy).map_err(|e| e.to_string())?;
            if meta.nlink() != 1 || meta.mode() & 0o777 != 0o444 || hash(&input(research_copy.to_str().ok_or("Invalid path")?)?) != snapshot.sha256 {
                return Err("Research copy failed identity/mode/hash checks".into());
            }
            Ok(serde_json::json!({"snapshot":snapshot,"research_path":research_copy,"production_can_open":copy.can_open,"mode":meta.mode()&0o777,"ino":meta.ino(),"nlink":meta.nlink(),"quarantine_required_before_word":true,"opened":false}))
        }
        Some("bench") if args.len() == 3 => {
            let count = args[2].parse::<usize>().map_err(|_| "Invalid repeat count")?;
            if !(1..=1000).contains(&count) { return Err("Repeat count must be 1..=1000".into()); }
            let bytes = input(&args[1])?;
            let mut elapsed = Vec::with_capacity(count);
            let expected = read_docx(&bytes);
            for _ in 0..count {
                let start = std::time::Instant::now();
                let actual = read_docx(&bytes);
                elapsed.push(start.elapsed().as_micros());
                if actual != expected { return Err("Nondeterministic Reader output".into()); }
            }
            elapsed.sort_unstable();
            Ok(serde_json::json!({"input_sha256":hash(&bytes),"input_bytes":bytes.len(),"repeats":count,"unit":"microseconds","p50":elapsed[count/2],"p95":elapsed[(count*95/100).min(count-1)],"max":elapsed[count-1],"summary":expected.summary,"text_sha256":hash(expected.text.as_bytes()),"build":"debug"}))
        }
        Some("snapshot") if (3..=4).contains(&args.len()) => {
            if !Path::new(&args[1]).is_dir() { return Err("App-data does not exist".into()); }
            let service = OrganizeService::start(args[1].clone().into());
            let (snapshot, bytes) = service.read_snapshot(&args[2]).map_err(|e| format!("{e:?}"))?;
            Ok(serde_json::json!({"snapshot":snapshot,"reader":report(&bytes,args.get(3).map(String::as_str))?}))
        }
        _ => Err("Usage: word-research inspect FILE [EXPECTED] | capture FILE NEW_APP_DATA | copy APP_DATA ID | snapshot APP_DATA ID [EXPECTED] | bench FILE REPEATS. This tool never launches Word.".into()),
    }
}
fn main() {
    match run() {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
