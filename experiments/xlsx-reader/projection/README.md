# XLSX Memory / local preview projection candidate

> Historical projection candidate. The unregistered adapter, stacked PR and waiting conditions below describe the original handoff. Current XLSX support is described in the [README](../../../README.md#supported-content) and [shared contract](../../../docs/development/shared-contract.md). See the [English translation of the path-redaction record](evidence/home-path-redaction/README.en.md); the original record and hashes are retained.

An unregistered adapter of the reviewed typed Workbook into the existing core
TextExtraction, prepared for the student's full Memory slice. Formal wiring
waits for R2's #26 run to finish. No second parser, shared DTO, snapshot store,
UI, differ or sender is added.

Main/R2 baseline: `c6a711d7f3155aee0f30b82eb44aa27cc267e523` / contract v22.
This F branch is **stacked on unmerged #151**, fixed dependency
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`, to compile/call A's real helpers.
F's PR diff against that dependency changes only this directory. A owns the
shared source and #150/#151; F did not edit or merge it. The parser src/manifest
remain identical to reviewed `6de78846225a65321089ec5794254407f35d632d`.

Interfaces:

```rust
read_xlsx_memory(bytes: &[u8]) -> TextExtraction   // A identity xlsx-1
read_xlsx_preview(bytes: &[u8]) -> TextExtraction  // A identity xlsx-preview-1
project_verified_snapshot(&SnapshotRef, &[u8], ProjectionMode)
    -> Result<(SnapshotRef, TextExtraction), &'static str>
```

Shared names, structure_name and escape_source_line are imported from the actual
pinned A implementation, not copied. Its reader.rs SHA-256 is
`e0eb50657197eee4b343d38d55c13294a2c89635d8d8575df3dd12bef9bdca55`.
F's private revision is `f-xlsx-projection-candidate-3`, separate from the unchanged
parser profile v1 and shared names. [A's final decisions](https://github.com/P30g53/Traeva/pull/150#issuecomment-6116720334)
resolved F's [specific questions](https://github.com/P30g53/Traeva/pull/150#issuecomment-6116636392).
[GRAMMAR.md](GRAMMAR.md) states the implemented boundaries.

Memory excludes hidden/veryHidden sheets; preview includes and marks them.
Literals/formulas/caches have separate typed labels and escaped original lines.
Caches remain unverified and are never recalculated. Blank/missing cache states
are separate generated lines, with no invented source. Part stays in Workbook;
user text locations stop at ordinal/ID/name/A1/field. Only generated structure
never becomes a successful body. Rejections carry no prefix. All actual omissions
and the million-scalar limit propagate; parser resource caps never increase.

The snapshot wrapper is an extra check of bytes already verified by core, not
an acquisition API: A must call read_snapshot, preserve AppErrors and avoid live
source fallback. Core UsablePair and B's new consent are necessary for future
analysis/sending. No typed Workbook IPC or model permission is granted here.

Reproduce this candidate's focused checks (no full matrix or GUI):

```sh
cargo test --offline --locked --manifest-path experiments/xlsx-reader/projection/Cargo.toml
cargo clippy --offline --locked --manifest-path experiments/xlsx-reader/projection/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path experiments/xlsx-reader/projection/Cargo.toml --check
```

The standalone lock uses no registry package/version absent from baseline root
lock. Root manifests/lock are untouched. Actual core types/helpers are compiled,
so its existing dependencies are linked, but these entry points invoke no model,
GUI or network service. Opt-in `F_XLSX_PROJECTION_EVIDENCE` captures only F's own
in-memory synthetic inputs/output into an existing private directory; otherwise
tests do not write evidence. Full packages/outputs stay ignored in evidence/local.

[REPORT.md](REPORT.md) records 22 final distinct developer checks, initial FAILs,
actual hashes and scope. [PROTOCOL-ALIGNED.md](PROTOCOL-ALIGNED.md) is the current
frozen expectation; PROTOCOL.md / PROTOCOL-V23.md are retained earlier phases.
[#134 R3 attempt3](https://github.com/P30g53/Traeva/pull/134#issuecomment-6113535039)
and the original nine FAIL/fixes are referenced, not rerun. #137 native developer
Excel evidence at `47006ba0744f278c7cb0d83557d3c554313e4896` remains separate.
[R3 latest #137](https://github.com/P30g53/Traeva/pull/137#issuecomment-6115341493)
only passed source/build/evidence scope; independent real Excel X15 is BLOCKED.
No Office repeat occurred. R3 must independently review this new SHA, and future
App/Snapshot/analysis/retrieval/preview/consent still require exact-head testing.
# Canonical source relocation

Active implementation is now `crates/traeva-core/src/xlsx_projection.rs`, loaded through the raw compatibility crate. This crate keeps its prior API and evidence; it contains no independent projection body. The new `crates/traeva-core/tests/xlsx_reader` validates the canonical source. See [relocation notes](../../../docs/features/xlsx-reader-core.md); the original #155 evidence below is retained, not rewritten.
