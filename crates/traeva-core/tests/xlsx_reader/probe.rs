use sha2::{Digest, Sha256};
use std::{env, fs, process};
use traeva_xlsx_validation::xlsx::{Limits, read_xlsx, read_xlsx_memory, read_xlsx_preview};

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: xlsx-core-probe SYNTHETIC.xlsx typed|memory|preview");
        process::exit(2);
    }
    let bytes = fs::read(&args[1]).expect("own synthetic XLSX input");
    if args[2] == "typed" {
        match read_xlsx(&bytes, &Limits::default()) {
            Ok(workbook) => println!("{}", serde_json::json!({"workbook": workbook})),
            Err(error) => {
                println!("{}", serde_json::json!({"error": error}));
                process::exit(1);
            }
        }
        return;
    }
    let extraction = match args[2].as_str() {
        "memory" => read_xlsx_memory(&bytes),
        "preview" => read_xlsx_preview(&bytes),
        _ => process::exit(2),
    };
    println!(
        "{}",
        serde_json::json!({"summary": extraction.summary,
        "text_sha256": hash(extraction.text.as_bytes())})
    );
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
