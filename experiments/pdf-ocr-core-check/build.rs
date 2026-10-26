// The harness compiles the product allocator verbatim, avoiding a second
// implementation. Product registration still belongs to A.
fn main() {
    let source = "../../crates/traeva-core/src/pdf_child.rs";
    println!("cargo:rerun-if-changed={source}");
    let text = std::fs::read_to_string(source).unwrap();
    let start = text.find("static HEAP_CAP:").unwrap();
    let end = text[start..].find("#[cfg(test)]").unwrap() + start;
    let allocator = &text[start..end];
    assert!(
        allocator.contains("pub(crate) fn set_heap_cap"),
        "Use main f0f96ee or later for A's shared heap-cap entry."
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("product_allocator.rs"), allocator).unwrap();
}
