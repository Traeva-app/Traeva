use std::{env, fs, path::PathBuf};

fn main() {
    // Compile the production ZIP/XML guards verbatim. Only this standalone
    // research target substitutes its body visitor; production is untouched.
    let source = "../../src/docx.rs";
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-changed=cached_body.rs");
    let reader = fs::read_to_string(source).unwrap();
    // This legacy prototype substitutes the visitor; sibling production
    // modules must not be resolved relative to its generated OUT_DIR file.
    let reader = reader.replace(
        r#"#[path = "docx_cached.rs"]
mod cached;
#[path = "docx_preview.rs"]
mod preview;
pub use cached::{DOCX_CACHED_READER, read_docx_cached};
pub use preview::preview_docx_snapshot;

"#,
        "",
    );
    let (guards, _) = reader.split_once("fn body_text(").unwrap();
    let guards = guards
        .lines()
        .filter(|line| !line.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\"docx-2\"", "\"docx-cache-research-1\"")
        .replace(
            "main body paragraphs, runs, tabs and explicit line breaks",
            "supported main-body text and stored complex-field display text",
        )
        .replace(
            "Only supported main-body text was retained; see left_out. Formatting and layout are not interpreted.",
            "Stored field display text may be missing or stale. No field or citation database was evaluated or refreshed. Field codes and internal metadata were excluded; see left_out.",
        );
    let combined = format!(
        "{guards}\n{}",
        fs::read_to_string("cached_body.rs").unwrap()
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("reader.rs"),
        combined,
    )
    .unwrap();
}
