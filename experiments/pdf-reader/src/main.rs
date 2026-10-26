//! `pdf-probe`: the experiment's test tool. It is the only part that
//! touches files: it reads the PDFs named on the command line (read-only)
//! and prints one JSON object per file. The reader itself only sees bytes.
//!
//!     pdf-probe [--text] FILE...
//!         read in this process
//!     pdf-probe --isolated [--text] [--timeout SECONDS] [--memory-mib MIB] FILE...
//!         read each file in a child process with a heap cap and a time limit
//!
//! The child (`pdf-probe --child MIB`) reads the PDF bytes from stdin. Its
//! heap cap is enforced by the allocator below, so it works the same on
//! macOS (where `RLIMIT_AS` is not enforced) and Linux. The text is printed
//! only with `--text`; it is for synthetic test files.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use traeva_pdf_experiment::{
    Code, MAX_PDF_BYTES, PDF_READER, read_pdf, scan_work, stopped_quality,
};

struct Capped;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CAP: AtomicUsize = AtomicUsize::new(usize::MAX);

fn reserve(size: usize) -> bool {
    let now = LIVE.fetch_add(size, Ordering::Relaxed).saturating_add(size);
    if now > CAP.load(Ordering::Relaxed) {
        LIVE.fetch_sub(size, Ordering::Relaxed);
        return false;
    }
    PEAK.fetch_max(now, Ordering::Relaxed);
    true
}

fn release(size: usize) {
    LIVE.fetch_sub(size, Ordering::Relaxed);
}

// SAFETY: every call forwards to the system allocator with the same layout;
// the counters only decide whether to forward.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if pointer.is_null() {
            release(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        release(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old && !reserve(new_size - old) {
            return std::ptr::null_mut();
        }
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if moved.is_null() {
            if new_size > old {
                release(new_size - old);
            }
        } else if new_size < old {
            release(old - new_size);
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Capped = Capped;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        return child(args.get(1));
    }
    let mut include_text = false;
    let mut isolated = false;
    let mut timeout = 20u64;
    let mut memory_mib = 1024usize;
    let mut files = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--text" => include_text = true,
            "--isolated" => isolated = true,
            "--timeout" => match rest.next().and_then(|v| v.parse().ok()) {
                Some(seconds) => timeout = seconds,
                None => return usage(),
            },
            "--memory-mib" => match rest.next().and_then(|v| v.parse().ok()) {
                Some(mib) => memory_mib = mib,
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => files.push(arg.clone()),
        }
    }
    if files.is_empty() {
        return usage();
    }
    for file in files {
        let bytes = match read_file(&file) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("{}", json!({ "file": file, "error": error }));
                continue;
            }
        };
        let mut result = if isolated {
            isolated_reading(&bytes, timeout, memory_mib)
        } else {
            reading(&bytes)
        };
        if !include_text && let Some(object) = result.as_object_mut() {
            object.remove("text");
        }
        let mut line = json!({
            "file": file,
            "sha256": hex(&Sha256::digest(&bytes)),
            "bytes": bytes.len(),
        });
        if let (Some(line), Some(result)) = (line.as_object_mut(), result.as_object()) {
            line.extend(result.clone());
        }
        println!("{line}");
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: pdf-probe [--isolated] [--text] [--timeout SECONDS] [--memory-mib MIB] FILE..."
    );
    ExitCode::from(2)
}

/// The largest file the probe passes to the reader. The reader gets the
/// whole file, so its quality report's `input_sha256` is the file's; the
/// reader itself refuses anything over `MAX_PDF_BYTES`. A larger file is
/// not read at all.
const MAX_PROBE_BYTES: usize = 4 * MAX_PDF_BYTES;

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PROBE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PROBE_BYTES {
        return Err(format!(
            "larger than the probe reads ({} MiB); not passed to the reader",
            MAX_PROBE_BYTES >> 20
        ));
    }
    Ok(bytes)
}

fn reading(bytes: &[u8]) -> Value {
    let started = Instant::now();
    let reading = read_pdf(bytes);
    let millis = started.elapsed().as_millis();
    let text = &reading.extraction.text;
    json!({
        "summary": reading.extraction.summary,
        "pages": reading.pages,
        "quality": reading.quality,
        "text_sha256": hex(&Sha256::digest(text.as_bytes())),
        "text": text,
        "millis": millis,
        // The raw-byte check run once more, after the timing.
        "scan_work": scan_work(bytes),
    })
}

fn child(memory_mib: Option<&String>) -> ExitCode {
    let Some(mib) = memory_mib.and_then(|v| v.parse::<usize>().ok()) else {
        return usage();
    };
    CAP.store(mib.saturating_mul(1 << 20), Ordering::Relaxed);
    let mut bytes = Vec::new();
    if std::io::stdin()
        .take(MAX_PROBE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_PROBE_BYTES
    {
        return ExitCode::from(3);
    }
    let mut result = reading(&bytes);
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "peak_heap_bytes".to_string(),
            json!(PEAK.load(Ordering::Relaxed)),
        );
    }
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{result}").is_err() {
        return ExitCode::from(3);
    }
    ExitCode::SUCCESS
}

fn isolated_reading(bytes: &[u8], timeout: u64, memory_mib: usize) -> Value {
    let started = Instant::now();
    let Ok(exe) = std::env::current_exe() else {
        return stopped(
            bytes,
            "crashed",
            "failed",
            "The isolated reader could not be started.",
        );
    };
    let Ok(mut process) = Command::new(exe)
        .arg("--child")
        .arg(memory_mib.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return stopped(
            bytes,
            "crashed",
            "failed",
            "The isolated reader could not be started.",
        );
    };
    let input = bytes.to_vec();
    let mut stdin = process.stdin.take();
    let writer = thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&input);
        }
    });
    let mut stdout = process.stdout.take();
    let reader = thread::spawn(move || {
        let mut out = String::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.take(256 << 20).read_to_string(&mut out);
        }
        out
    });
    let mut stderr = process.stderr.take();
    let errors = thread::spawn(move || {
        let mut err = String::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(64 << 10).read_to_string(&mut err);
        }
        err
    });
    let deadline = Duration::from_secs(timeout);
    let status = loop {
        match process.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = process.kill();
                let _ = process.wait();
                break None;
            }
        }
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    let err = errors.join().unwrap_or_default();
    let millis = started.elapsed().as_millis();
    let mut result = match status {
        None => stopped(
            bytes,
            "time_limit",
            "unsupported",
            &format!("The reader did not finish within {timeout} seconds in its isolated process."),
        ),
        Some(status) if status.success() => match serde_json::from_str::<Value>(out.trim()) {
            Ok(mut value) => {
                if let Some(object) = value.as_object_mut() {
                    object.insert("isolation".to_string(), json!({ "outcome": "finished" }));
                }
                value
            }
            Err(_) => stopped(
                bytes,
                "crashed",
                "failed",
                "The isolated reader returned no result.",
            ),
        },
        Some(_) if err.contains("memory allocation of") => stopped(
            bytes,
            "memory_limit",
            "unsupported",
            &format!(
                "The reader needed more than {memory_mib} MiB of memory in its isolated process."
            ),
        ),
        Some(_) => stopped(
            bytes,
            "crashed",
            "failed",
            "The reader's isolated process stopped unexpectedly.",
        ),
    };
    if let Some(isolation) = result.get_mut("isolation").and_then(Value::as_object_mut) {
        isolation.insert("millis".to_string(), json!(millis));
        isolation.insert("memory_mib".to_string(), json!(memory_mib));
        isolation.insert("timeout_seconds".to_string(), json!(timeout));
    }
    result
}

/// A reading the isolated process did not finish. The status is not the
/// reader's: it comes from this wrapper.
fn stopped(bytes: &[u8], outcome: &str, status: &str, note: &str) -> Value {
    let reason = match outcome {
        "time_limit" | "memory_limit" => Code::ResourceLimit,
        _ => Code::InternalError,
    };
    json!({
        "summary": {
            "reader": PDF_READER,
            "status": status,
            "chars": 0,
            "covered": [],
            "left_out": [],
            "note": note,
        },
        "pages": [],
        "quality": stopped_quality(bytes, reason, note),
        "text_sha256": hex(&Sha256::digest(b"")),
        "text": "",
        "isolation": { "outcome": outcome },
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
