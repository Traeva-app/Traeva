# #66: Explicit cache Reader and local Snapshot plain-text preview

> Historical local-preview implementation report. The pending Go decision, wiring status and API observations below belong to the named commits. For current DOCX support, see the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md). This report remains developer evidence for its original scope.

Date: 2026-10-10 (America/Los_Angeles). This is D's development evidence and wiring notes, **not R1's independent acceptance or a PASS for the whole Word Memory**.

Implementation commit: `705d17df1f9db3e1d2eb201d4916e2635e4247a9`; starting main: `61b0afc85aa25de09103a6a46aafcafde62dfc71` (the student has merged #126, which includes the R126-01 fix). The [machine evidence](../../crates/traeva-core/tests/fixtures/docx/local-reader-validation.json) stores input hashes, Reader statuses, text hashes, expectation matches, run log hashes, initial failures and source file checks.

## Delivered items and technical recommendations

| Item | D's technical recommendation | Actual delivered boundary |
|---|---|---|
| Cached display text of structurally complete complex fields | Conditional Go, for R1 review | A compilable, runnable explicit Reader; the cache may be stale and must not be used as up-to-date citation data |
| Local plain-text preview of historical DOCX | Core/Reader Go, UI pending A/B | Gets and verifies bytes from the real `read_snapshot`, and returns in-memory plain text and the original SnapshotRef; no Word, no external access, no writes to the original file |
| Formal opening of historical DOCX in Word | **No-Go** | Open allowlist unchanged; an empty research rejection list does not authorize opening Word |
| Formal Word Memory / Semantic Change / Retrieval / cloud AI | Not enabled this round | Does not change #103/#124, does not enter the current default Reader dispatch or the model sending chain, does not affect #26/#37 |

The final decision on 10/13 is still made by the student. R1's LIMITED PASS on #126 is bound to the original research scope and the old commit; it cannot be inherited by this implementation.

## How to call

The new code is an ordinary Rust module and does not depend on a build script that generates/replaces source:

```rust
// explicit entry points in crates/traeva-core/src/docx.rs
read_docx_cached(bytes: &[u8]) -> TextExtraction
preview_docx_snapshot(service: &OrganizeService, snapshot_id: &str)
    -> Result<(SnapshotRef, TextExtraction), AppError>
```

- Version: `docx-cache-local-1`. The original `read_docx` is still `docx-2` and still excludes cached results; the existing `reader::read_text` / `snapshot_text` dispatch is unchanged.
- Both Readers use the existing ZIP/XML safety checks through the same `extract_with`, and only choose different body visitors. They share the result summary; the original Reader's 78 regressions pass.
- `preview_docx_snapshot` calls only `read_snapshot`. The extension of the name at capture time must be `.docx` (case-insensitive); the type is not chosen by the current file name or current content, and there is no fallback from source.
- It returns existing types; there is no new DTO, schema, migration or shared file change. `TextExtraction` is still not serializable, and Debug still does not show the body text; this function does not store preview text and does not call the model.
- A corrupt/missing Snapshot or an invalid ID returns the original `AppError`, `stage: memory`; a corrupt or unsupported DOCX inside a valid Snapshot returns an empty `TextExtraction` that keeps the Reader's failure status. The two kinds of failure must not be lumped together as "no historical body text".
- This API reads/parses synchronously; for desktop calls A must arrange background work, and UI waiting/cancel handling goes to B. Large packages must not be processed directly on the UI thread.

This round does not change core public registration, dependency manifests, IPC or UI. The independent verification crate compiles these source files directly and reuses main's real `OrganizeService`, not a mock Snapshot service.

## Citation field rules and known limits

Only when every unfinished complex field has already reached `fldChar separate` may the `w:t`, tab/explicit line break at that position enter the output. An inner result inside an outer code area is not output. `instrText`, `delInstrText` and marked field metadata do not enter the body; fields are not evaluated/updated, and Zotero/EndNote databases are not parsed.

- A field with a cache is still `Partial`: freshness unknown; `dirty=true` additionally notes that it may be stale. `dirty=false`, a missing marker, or the most recent Word save cannot prove freshness.
- Missing separator or empty result: no display value is made up; Partial if there is other readable body text, otherwise Unsupported.
- Orphaned/duplicate separator, end without begin, unclosed field or invalid dirty value: Failed, and every prefix already read is discarded.
- A field boundary falls in an omitted area, or a field crosses a content/property revision: the whole document is Unsupported. The revision acceptance rule was not decided by D; the existing scope of not accepting revisions is kept.
- Simple fields are still omitted. Tables, other stories, comments, images, text boxes, embedded objects, unknown namespaces and similar are still bound by the original Limited Reader; this is no basis for claiming full understanding of Word documents. Omitted content that actually exists is described in the summary.
- When the known `ADDIN ZOTERO_ITEM`, `ADDIN ZOTERO_BIBL`, `CSL_CITATION` or `ADDIN EN.CITE` appear in the output, the document is conservatively rejected, including across runs and with letter case/whitespace variations. Ordinary mentions of these words may also be rejected.
- **R126-02 kept**: content that has already been flattened into ordinary `w:t`, with none of the markers above and no structural origin, cannot be reliably told apart from ordinary body text. A unit test explicitly keeps the boundary that such content may be Complete; the summary warns that unmarked flattened text cannot be identified reliably. It must not be claimed that all flattened internal data of citation managers can be recognized or excluded. If acceptance requires a guarantee that all such content is excluded, it should be No-Go.

The field stack is at most 32 levels. Package limits follow the existing Reader: 20 MiB input, 512 entries, 8 MiB decompressed per entry, 32 MiB total, compression ratio 200, 4 MiB per XML, XML depth 64, 500,000 event/attribute work, 1,000,000 namespace work, 1,000,000 Unicode scalar characters of kept body text. Encryption, malformed compression, ZIP bombs, CRC/size/directory mismatches, corrupt XML/DTD and similar are still rejected. There is no path that decompresses to disk, executes the document or opens relationship targets.

## R126-01 fix

Fix commit: `e344eab2b29b8f41584fef6a6a1cad2d1b5f55d2`, which entered main when the student merged #126.

The research copy's rejection check now checks **the final Unsupported/Failed status of `read_docx`**, not only whether the internal `extract` returns an error. Both the synthetic foreign-namespace `fldChar` package and the corrupt package return rejection reasons. The research crate's 112 tests and Clippy pass.

This is only a fix to the research negative filter: neither Complete, Partial nor an empty reason list can authorize starting Word. The formal `copy_for_opening` still gives `can_open=false` for DOCX. The No-Go on historical quarantine origin, active content/external access isolation and the formal publish boundary is not lifted.

The new module required the old research crate's source generator to exclude sibling module declarations, and its baseline harness to export the existing service types; this PR makes only compatibility fixes. The new Reader/preview itself does not use this generator; both the old research's first compile failure and the 112 PASS after the fix are kept.

## Verification results

Environment: macOS 27.0.1 (26A434), arm64, Rust 1.98.1. The real Word samples come from six earlier Save As runs in Word 16.78, with D's own synthetic test material as content; Word was not started again this time, and no real personal documents or plugin libraries were used.

| Check | Actual result |
|---|---|
| New implementation standalone crate | 127 PASS: 78 original Reader, 34 cache, 8 preview, 7 existing Snapshot/lifecycle tests |
| Merged research crate compatibility | 112 PASS, including the R126-01 regression |
| core offline regression | 791 PASS, 10 ignored; ignored does not count as executed and passed |
| Frontend regression / build | 108 PASS / build PASS (frontend not changed this round) |
| fmt / Clippy for the workspace and the two standalone crates | PASS |
| Six synthetic seeds + six packages saved by real Word | 12/12 exactly match the predefined text expectations; body text did not enter logs or GitHub |
| Reopening a real historical Snapshot | A copy of the old test database, reopened through the actual core and hash-verified; 74-character body text matches |
| New real Word Snapshot capture/preview | Real capture/read_snapshot, 53 characters match; same after source rewrite, move, delete and several service restarts |
| Original file / Snapshot protection | Preview does not change bytes, inode, permissions, mtime/ctime; app-data directory/file contents unchanged, no opened directory, no viewer launched |
| Corrupt/missing/symlink/invalid Snapshot | Returns an error; no fallback even when a valid source is present; no leak of the prefix already read |
| Default dispatch and open allowlist | Default snapshot_text does not get cached text; DOCX can_open still false |

Real Word results: ordinary citation Partial/53, nested Partial/20, missing/old result Partial/29, revision Unsupported/0, malformed field after Word save Unsupported/0, ordinary historical body Complete/74. The malformed seed itself is Failed/0. Word removing the dirty marker does not change the "freshness unknown" note.

Debug in-memory reads: the 12,121-byte Word sample 100 times, p50 **11.324 ms**, p95 **12.451 ms**, max 18.912 ms; an approximately 1 MiB synthetic cache (1,000,001 characters) 10 times, p50 **62.961 ms**, p95 **65.702 ms**, keeping 1,000,000 characters and noting the tail truncation. This does not include Snapshot disk verification, IPC, UI or Word, and it is not a production SLA. Slower measurements under parallel compile/test load are kept; no fixed latency is promised from them.

Initial failures were not deleted: the Clippy large-err check caused by the existing AppError, the old research harness missing service type exports, and a hidden-file fixture that wrongly assumed it could be captured (the core actually rejects it correctly at the Observation stage). They were fixed in place: keep the shared AppError; complete the standalone harness; change the fixture to verify the existing rejection without bypassing the hidden-file protection. Finally 127/112 tests pass.

D has finished the implementation self-check; **independent re-verification by R1 on the PR's final commit is still pending**. This round does not take the #65 UI slot; there are no new windows, and the main screen is not used. Verification with real Zotero/EndNote plugins, other Office/macOS, release/RSS, full desktop preview, real AI or Word→Semantic→Retrieval was not run. The repository currently has no CI workflow; these are local results with logs kept.

## Rerun and A/B handoff

```sh
cargo test --offline --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml
cargo clippy --offline --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --all-targets -- -D warnings
cargo test --offline --manifest-path crates/traeva-core/tests/word_research/Cargo.toml
cargo test --offline -p traeva-core
cargo fmt --all --check
cargo clippy --offline --workspace --all-targets -- -D warnings
npm test
npm run build
```

The synthetic tests use isolated app-data under `/private/tmp` on their own and are fully offline. Part of the core regression uses a loopback mock, not a real provider. Do not pass the formal app-data to the verification tools.

```sh
cargo run --offline --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --bin docx-probe -- inspect-cache SYNTHETIC.docx EXPECTED.txt
cargo run --offline --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --bin docx-probe -- preview ISOLATED_APP_DATA SNAPSHOT_ID EXPECTED.txt
cargo run --offline --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --bin docx-probe -- bench-cache SYNTHETIC.docx 100
```

The CLI outputs summary/hash/match results, **not the body text**. The Rust API returns the body text in memory, for later local preview wiring. The real Word/Snapshot packages, the isolated DB and the raw logs stay in D's local `/private/tmp/traeva-d-docx-local-20261010/`; no personal documents are used, and the packages are not uploaded to GitHub.

Estimated remaining work:

- **A: 0.5–1 person-day**. Review D's module and the existing parsing dependencies, and unify the public export, background calls and the IPC boundary for local-only preview. `TextExtraction` cannot be used directly as a DTO; if the window needs body text, A separately defines bounded data/paging and states the local-only use. No shared wiring diff was submitted this time, and `read_text` is not switched to the cache Reader by default.
- **B: about 1 person-day**. Choose the real snapshot_id, and show the version, coverage/failure/staleness warnings and bounded body text. Use only text nodes/escaped plain text, never executed as HTML/Markdown; viewer auto-open, external content loading and model sending are prohibited. A formal Word open button stays disallowed.
- **R1: 0.5–1 person-day**. Pin the new PR head, and independently run the mechanism, Snapshot, and safety/default routing regressions; if desktop verification is needed, schedule the UI slot separately per #65, with the window on the Dell secondary screen.
- **Plugins/formal Word Memory: separately estimated at 1–2+ person-days, and not started this round**. Real plugin origin verification, line references matching the Reader version, sending scope/dedicated permission, Semantic Change/Retrieval wiring and the full flow must be checked separately by A/B/R; a cloud Go cannot be inferred from the local preview Go.

These are development effort estimates, not schedule promises from A/B. The reviewable Core/Reader is delivered first, without waiting for A/B core work to finish; #103/#124 are not changed, and D does not merge or close #66 on its own.
