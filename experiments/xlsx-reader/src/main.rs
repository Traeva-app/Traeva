use serde_json::json;
use std::fs::File;
use std::io::{Read, Write};
use traeva_xlsx_experiment::{Limits, coordinate_changes, read_xlsx};

fn load(path: &std::ffi::OsStr, cap: usize) -> std::io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("input must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn run() -> (serde_json::Value, bool) {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 && args.len() != 2 {
        return (
            json!({"error": "usage: xlsx-probe SYNTHETIC.xlsx [AFTER.xlsx]"}),
            false,
        );
    }
    let limits = Limits::default();
    let first = match load(&args[0], limits.input_bytes) {
        Ok(b) => b,
        Err(_) => return (json!({"error": "input_read_failed"}), false),
    };
    let before = match read_xlsx(&first, &limits) {
        Ok(w) => w,
        Err(e) => return (json!({"error": e}), false),
    };
    if args.len() == 1 {
        return (json!({"workbook": before}), true);
    }
    let second = match load(&args[1], limits.input_bytes) {
        Ok(b) => b,
        Err(_) => return (json!({"error": "input_read_failed"}), false),
    };
    let after = match read_xlsx(&second, &limits) {
        Ok(w) => w,
        Err(e) => return (json!({"error": e}), false),
    };
    match coordinate_changes(&before, &after) {
        Ok(changes) => (
            json!({"before": before, "after": after, "coordinate_changes": changes,
            "diff_scope": "positional evidence only; cache freshness unverified; semantic alignment not inferred"}),
            true,
        ),
        Err(e) => (json!({"error": e}), false),
    }
}

fn main() {
    let (value, success) = run();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let written =
        serde_json::to_writer_pretty(&mut out, &value).is_ok() && out.write_all(b"\n").is_ok();
    if !success || !written {
        std::process::exit(1);
    }
}
