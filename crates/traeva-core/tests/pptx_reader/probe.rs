use sha2::{Digest, Sha256};
use std::{env, fs, process};
use traeva_pptx_validation::pptx::{read_pptx, read_pptx_memory, read_pptx_preview};

fn main() {
    let args: Vec<String> = env::args().collect();
    if !(3..=4).contains(&args.len()) {
        eprintln!(
            "Usage: pptx-probe INPUT.pptx EXPECTED.txt [legacy|memory|preview] (synthetic only)"
        );
        process::exit(2);
    }
    let reader = match args.get(3).map(String::as_str).unwrap_or("legacy") {
        "legacy" => read_pptx,
        "memory" => read_pptx_memory,
        "preview" => read_pptx_preview,
        _ => {
            eprintln!("Unknown projection");
            process::exit(2);
        }
    };
    let bytes = fs::read(&args[1]).expect("synthetic PPTX input");
    let expected = fs::read_to_string(&args[2]).expect("synthetic expected text");
    let out = reader(&bytes);
    println!(
        "{}",
        serde_json::json!({
            "sha256": hash(&bytes), "bytes": bytes.len(),
            "summary": out.summary, "text_matches": out.text == expected,
            "text_sha256": hash(out.text.as_bytes())
        })
    );
    if out.text != expected {
        process::exit(1);
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
