# Unregistered XLSX core module

> Historical XLSX module-relocation report. The title and statements about an unregistered module, unmerged helper or Unsupported dispatch describe the relocation stage. XLSX is now connected to Memory; current behavior is in the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md). The original failures and review limits below are retained.

Canonical sources now live in `crates/traeva-core/src/xlsx.rs`,
`xlsx_package.rs`, `xlsx_xml.rs`, and `xlsx_projection.rs`. The parser/ZIP/XML
logic and projection output are unchanged from R3-reviewed #155 head
`8f274c186a24cfaa4902fced09e8b4edb818969f`:
[R3 LIMITED PASS](https://github.com/P30g53/Traeva/pull/155#issuecomment-6117066467).
Only module paths, crate-to-parent imports and rustfmt import ordering changed.
Parser profile v1, shared xlsx-1/xlsx-preview-1, candidate-3, all safety/resource
limits and the original nine failures are retained. No version bump is needed
for a path-only relocation with identical behavior.

This PR is stacked on #155, which depends on the unmerged A #151 helper at
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`. Latest main read for this task was
`6886a569b2eb553d809c4c6dc74e533eb5d30b35` (contract v23 merged). Neither main/R2
nor shared helper source is changed here. `lib.rs` does not register mod xlsx,
and read_text still returns Unsupported for .xlsx. A owns exports, dispatch,
preview command, permissions and subsequent production integration.

The new standalone `tests/xlsx_reader` follows E's #[path] pattern and compiles
the **same core source**, with actual core types/helper reexports, its own lock
and target/evidence directories. Old experiment crates are compatibility exports:
their API still resolves the canonical core source, with no second parser body.
The original research, audit, FAIL and native Excel evidence is not rewritten.
The raw compatibility manifest now links existing core for projection helpers;
historical dependency audit evidence describes the original parser dependency
graph, not a new absence-of-network-library claim for the bridge.

Entrypoints: read_xlsx(bytes, limits) -> typed Workbook; read_xlsx_memory(bytes)
and read_xlsx_preview(bytes) -> existing TextExtraction; experimental
project_verified_snapshot checks already-verified supplied hash/length and input
ceiling. No new shared DTO/schema/API is established. Production input must still
come from read_snapshot, never live-source fallback. Model/UsablePair/consent
are later integration responsibilities.

Memory includes visible sheets only; preview labels hidden/very hidden. Literal,
formula and saved cache remain separate; cache is always not verified. Only
structure cannot manufacture original evidence or Partial. Part is internal
Workbook context per A/B's decision. No logical-row alignment, format/business
inference, formula evaluation, macro execution or link refresh is added.

## Developer relocation checks

- Existing 22 focused assertions executed against core source: PASS.
- Existing 24 F-owned synthetic package input hashes verified; 48 memory/preview
  text hashes, reader/status/scalar count/actual omission lists exactly MATCH the
  reviewed #155 observed records. These are migration parity observations, not
  extra independent tests or native Excel saves.
- Mechanical proof: parser body identical; ZIP/XML change only crate to super;
  projection changes imports/order only. Equality check's first import-order
  assertion was corrected after retaining the diff; no parser body changed.
- Offline locked core validation lib/probe, Clippy all-targets, fmt PASS.
- Old raw and projection compatibility crates compile with warnings denied.
- Initial formatter diagnostic: probe.rs was not created yet. Initial direct
  projection shim compile failed because it tried to compile parser dependencies;
  fixed by exporting the raw compatibility crate's canonical implementation,
  without adding root dependencies. Raw compile diagnostics are retained.

Reproduce:

```sh
cargo test --offline --locked --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml --lib
cargo clippy --offline --locked --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml --check
cargo run --offline --locked --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml --bin xlsx-core-probe -- OWN_SYNTHETIC.xlsx memory
```

Evidence under that validation crate includes source-normalized relocation proof,
hash parity and raw first diagnostics. Its independent lock and the compatibility
lock use baseline root registry versions; root manifests/lock remain untouched.
This does not repeat R3's original 122-input safety matrix or #155's independent
76 judgments. R3 independently rechecks the new exact SHA.

**Independent native Excel X15 remains owed/BLOCKED**, separate from F's #137
Excel16.78 developer saves and R3's source/build/evidence limited acceptance.
F did not repeat GUI saves. R3 must use its own synthetic material, freeze its own
expectations, coordinate #65 and bind its own workbook/app before Office work.
Developer results cannot replace independent Office PASS. Historical Snapshot /
App / analysis / retrieval / preview / evidence UI / notice v4 and revocation
are not accepted by relocation; no model call, UI slot, merge or Issue closure.
