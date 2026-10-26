//! Local validation CLI. Prints summaries/hashes, never extracted document text.
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, Read, Write},
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use traeva_core::OrganizeService;
use traeva_docx_validation::docx::{
    MAX_DOCX_BYTES, preview_docx_snapshot, read_docx, read_docx_cached,
};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_DOCX_BYTES as u64 {
        return Err("Input exceeds 20 MiB".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_DOCX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_DOCX_BYTES {
        return Err("Input grew beyond limit".into());
    }
    Ok(bytes)
}
fn report(
    bytes: &[u8],
    expected: Option<&str>,
    iterations: usize,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    let mut extraction = read_docx(bytes);
    for _ in 1..iterations {
        extraction = std::hint::black_box(read_docx(std::hint::black_box(bytes)));
    }
    let elapsed = started.elapsed();
    let matches = expected
        .map(|path| fs::read_to_string(path).map(|text| text == extraction.text))
        .transpose()
        .map_err(|e| e.to_string())?;
    let result = serde_json::json!({"input_sha256":hash(bytes),"input_bytes":bytes.len(),"summary":extraction.summary,"text_sha256":hash(extraction.text.as_bytes()),"expected_matches":matches,"iterations":iterations,"total_ms":elapsed.as_secs_f64()*1000.0,"mean_ms":elapsed.as_secs_f64()*1000.0/iterations as f64});
    if matches == Some(false) {
        return Err(format!("Expected text mismatch; summary: {}", result));
    }
    Ok(result)
}
fn cached_report(
    extraction: traeva_core::TextExtraction,
    expected: Option<&str>,
) -> Result<serde_json::Value, String> {
    let matches = expected
        .map(|path| fs::read_to_string(path).map(|s| s == extraction.text))
        .transpose()
        .map_err(|e| e.to_string())?;
    let result = serde_json::json!({"summary":extraction.summary,"text_sha256":hash(extraction.text.as_bytes()),"expected_matches":matches,"local_only":true});
    if matches == Some(false) {
        return Err(format!("Expected text mismatch: {result}"));
    }
    Ok(result)
}
fn source_state(path: &Path) -> serde_json::Value {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            serde_json::json!({"dev":m.dev(),"ino":m.ino(),"bytes":m.len(),"mtime_sec":m.mtime(),"mtime_nsec":m.mtime_nsec(),"ctime_sec":m.ctime(),"ctime_nsec":m.ctime_nsec()})
        }
        Err(e) => serde_json::json!({"error":e.to_string()}),
    }
}
fn emit(value: serde_json::Value) -> Result<(), String> {
    println!("{value}");
    std::io::stdout().flush().map_err(|e| e.to_string())
}
fn watch(path: &Path, data: &Path) -> Result<(), String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if data.exists() {
        return Err("Watch requires a new isolated app-data path".into());
    }
    let service = OrganizeService::start(data.to_path_buf());
    let root = service
        .authorize_root(path.parent().ok_or("No parent")?)
        .map_err(|e| format!("{e:?}"))?;
    let mut files = Vec::new();
    let register = || -> Result<String, String> {
        let file = service
            .register_file(&root.root_id, &path)
            .map_err(|e| format!("{e:?}"))?
            .file;
        service
            .start_memory(&file.file_id)
            .map_err(|e| format!("{e:?}"))?;
        Ok(file.file_id)
    };
    files.push(register()?);
    let record = |event: &str| -> Result<(), String> {
        let mut snapshots = Vec::new();
        for file in &files {
            for snapshot in service.list_snapshots(file).map_err(|e| format!("{e:?}"))? {
                let (saved, bytes) = service
                    .read_snapshot(&snapshot.snapshot_id)
                    .map_err(|e| format!("{e:?}"))?;
                if hash(&bytes) != saved.sha256 {
                    return Err("Snapshot digest mismatch".into());
                }
                snapshots
                    .push(serde_json::json!({"snapshot":saved,"reader":report(&bytes,None,1)?}));
            }
        }
        emit(
            serde_json::json!({"event":event,"at_ms":SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_millis(),"source":source_state(&path),"snapshots":snapshots}),
        )
    };
    record("start")?;
    // This watch stays bound to its initial file object, including after a Word replacement.
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let command: Vec<_> = line.split_whitespace().collect();
        match command.as_slice() {
            ["check"] => {
                let checks = service.check_memory().map_err(|e| format!("{e:?}"))?;
                emit(serde_json::json!({"event":"check","checks":checks}))?;
                record("verified")?;
            }
            ["poll", seconds] => {
                let seconds = seconds.parse::<u64>().map_err(|_| "Invalid poll seconds")?;
                if !(1..=60).contains(&seconds) {
                    return Err("Poll seconds must be 1..60".into());
                }
                let start = Instant::now();
                while start.elapsed().as_secs() < seconds {
                    let before = source_state(&path);
                    let began = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| e.to_string())?
                        .as_millis();
                    let checks = service.check_memory().map_err(|e| format!("{e:?}"))?;
                    emit(
                        serde_json::json!({"event":"poll","at_ms":began,"source_before":before,"source_after":source_state(&path),"checks":checks}),
                    )?;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                record("poll_finished")?;
            }
            ["stop"] => break,
            _ => return Err("Watch commands: check | poll SECONDS (1..60) | stop".into()),
        }
    }
    for file in files {
        service.stop_memory(&file).map_err(|e| format!("{e:?}"))?;
    }
    emit(serde_json::json!({"event":"stopped"}))
}
fn run() -> Result<serde_json::Value, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str){
        Some("memory-e2e") if (3..=4).contains(&args.len()) => {
            use traeva_docx_validation::memory_e2e::Session;
            let session = if let Some(chain) = args.get(3) {
                Session::resume(Path::new(&args[1]), Path::new(&args[2]), chain)?
            } else { Session::new(Path::new(&args[1]), Path::new(&args[2]))? };
            emit(serde_json::json!({"event":"ready","history":session.history()}))?;
            for line in std::io::stdin().lock().lines() {
                let input:serde_json::Value = serde_json::from_str(&line.map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                if input["op"] == "quit" { break; }
                emit(session.command(input))?;
            }
            Ok(serde_json::json!({"event":"process_exit","memory_stopped":false}))
        }
        Some("inspect-cache") if (2..=3).contains(&args.len()) => {
            let bytes = bytes(Path::new(&args[1]))?;
            Ok(serde_json::json!({"input_sha256":hash(&bytes),"input_bytes":bytes.len(),"reader":cached_report(read_docx_cached(&bytes), args.get(2).map(String::as_str))?}))
        }
        Some("preview") if (3..=4).contains(&args.len()) => {
            if !Path::new(&args[1]).is_dir() { return Err("Isolated app-data does not exist".into()); }
            let service = OrganizeService::start(args[1].clone().into());
            let (snapshot, extraction) = preview_docx_snapshot(&service, &args[2]).map_err(|e|format!("{e:?}"))?;
            Ok(serde_json::json!({"snapshot":snapshot,"preview":cached_report(extraction,args.get(3).map(String::as_str))?}))
        }
        Some("bench-cache") if args.len() == 3 => {
            let count = args[2].parse::<usize>().map_err(|_|"Invalid repeats")?;
            if !(1..=1000).contains(&count) { return Err("Repeats must be 1..1000".into()); }
            let bytes = bytes(Path::new(&args[1]))?;
            let expected = read_docx_cached(&bytes);
            let mut elapsed = Vec::new();
            for _ in 0..count {
                let start = Instant::now();
                let actual = read_docx_cached(&bytes);
                elapsed.push(start.elapsed().as_micros());
                if actual != expected { return Err("Nondeterministic result".into()); }
            }
            elapsed.sort_unstable();
            Ok(serde_json::json!({"input_sha256":hash(&bytes),"input_bytes":bytes.len(),"repeats":count,"p50_us":elapsed[count/2],"p95_us":elapsed[(count*95/100).min(count-1)],"max_us":elapsed[count-1],"build":"debug","reader":cached_report(expected,None)?}))
        }

        Some("watch") if args.len()==3=>{watch(Path::new(&args[1]),Path::new(&args[2]))?;Ok(serde_json::json!({"watch_complete":true}))},
        Some("inspect") if (2..=4).contains(&args.len())=>{let iterations=args.get(3).map(|s|s.parse::<usize>()).transpose().map_err(|_|"Invalid iterations")?.unwrap_or(1);if !(1..=10000).contains(&iterations){return Err("Iterations must be 1..10000".into());}report(&bytes(Path::new(&args[1]))?,args.get(2).map(String::as_str),iterations)}
        Some("capture") if args.len()==3=>{
            let path=fs::canonicalize(&args[1]).map_err(|e|e.to_string())?;let data=Path::new(&args[2]);
            if data.exists(){return Err("Capture requires a new isolated app-data path".into());}
            let service=OrganizeService::start(data.to_path_buf());let root=service.authorize_root(path.parent().ok_or("No parent")?).map_err(|e|format!("{e:?}"))?;
            let file=service.register_file(&root.root_id,&path).map_err(|e|format!("{e:?}"))?.file;service.start_memory(&file.file_id).map_err(|e|format!("{e:?}"))?;
            let snapshots=service.list_snapshots(&file.file_id).map_err(|e|format!("{e:?}"))?;let snapshot=snapshots.first().ok_or("Initial capture failed")?;
            let(saved,bytes)=service.read_snapshot(&snapshot.snapshot_id).map_err(|e|format!("{e:?}"))?;service.stop_memory(&file.file_id).map_err(|e|format!("{e:?}"))?;
            Ok(serde_json::json!({"snapshot":saved,"reader":report(&bytes,None,1)?}))
        }
        Some("snapshot") if (3..=4).contains(&args.len())=>{
            if !Path::new(&args[1]).is_dir(){return Err("Snapshot app-data does not exist".into());}
            let service=OrganizeService::start(args[1].clone().into());let(snapshot,bytes)=service.read_snapshot(&args[2]).map_err(|e|format!("{e:?}"))?;
            Ok(serde_json::json!({"snapshot":snapshot,"reader":report(&bytes,args.get(3).map(String::as_str),1)?}))
        }
        _=>Err("Usage: docx-probe inspect-cache FILE [EXPECTED] | preview ISOLATED_APP_DATA ID [EXPECTED] | bench-cache FILE REPEATS | inspect FILE [EXPECTED_TEXT [ITERATIONS]] | capture FILE NEW_APP_DATA | snapshot APP_DATA SNAPSHOT_ID [EXPECTED_TEXT] | watch FILE NEW_APP_DATA (stdin: check, poll SECONDS, stop)".into())
    }
}
fn main() {
    match run() {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
