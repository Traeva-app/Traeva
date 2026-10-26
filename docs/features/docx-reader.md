# #66 Limited DOCX Reader: implementation and validation

> Historical DOCX reader report for the commits named below. Pending decisions, PR states and integration gaps describe that review stage. DOCX is now connected to Memory; current formats and limits are in the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md). The original test results remain limited to their recorded versions.

The latest follow-up is [Word save lifecycle validation](docx-word-lifecycle.md), based on main `6cc3a60742c54cac6e3e056eb422cf2af36d2bd2`. Its [machine evidence](../../crates/traeva-core/tests/fixtures/docx/word-lifecycle-validation.json) binds the updated Reader notes and tests to source hashes. The results below retain the earlier #100/#106 baselines and are not silently rebased onto the new follow-up.

As of 2026-10-10, docx-2 fixes the defects identified in [A's #100 review](https://github.com/P30g53/Traeva/pull/100#issuecomment-6101165382). The earlier docx-1 blanket PASS is superseded. GitHub records #100 merged by P30g53 at 19:43:45 UTC, merge `7b4062dffc2f16969f946e020374541d491a5523`; D issued no merge action. This follow-up adds actual rich Word-save fixtures and validation. **The October 13 Go/No-Go decision remains pending**. Production wiring is A's unmerged [Draft #103](https://github.com/P30g53/Traeva/pull/103). Full Word Memory and Codex-R acceptance have not passed. #66 remains open and does not block #26/#37.

## Version and isolation

- Earlier #106 baseline main: `6f14e3a7dbdb9b60f2d7945054fc1d8556f8ccdf`, contract v12. Earlier Reader repair/performance runs used core baseline `1c0ff381a0331e270e8f38c3e0e21a56c4ee478e`; the A integration run used A `449e95f` plus frozen D source/tests and migration.
- Reader: `docx-2`; earlier #100/#106 source SHA-256 `4ebddd23b43a2b3f14253a04f4f348b7a8105bc5c3c74ec694d012bfb2333626`. The notes-only follow-up source SHA-256 is `765946eb65cf3e48f0daf90367b7a3ea37d339503a4b7d65efeaf45c49371fc2`.
- Environment: macOS 27.0.1 arm64, Rust 1.98.1, Microsoft Word 16.78.
- D's independent worktree uses explicit temporary synthetic roots/app-data. No default Traeva data, user document, sealed material or model API was accessed.
- Evidence: [validation.json](../../crates/traeva-core/tests/fixtures/docx/validation.json), [r2-review.json](../../crates/traeva-core/tests/fixtures/docx/r2-review.json), [rich Word evidence](../../crates/traeva-core/tests/fixtures/docx/word-rich-validation.json), [fixture provenance](../../crates/traeva-core/tests/fixtures/docx/README.md).
- Original Word UI operations: [D claim](https://github.com/P30g53/Traeva/issues/65#issuecomment-6094849892), [D return](https://github.com/P30g53/Traeva/issues/65#issuecomment-6094914735). Rich Word save: [D claim after R2 return](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101504827), [D return](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101701197), [retention-check correction](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101713094). Only the D synthetic document was opened/saved/closed; the slot has been returned.

## Changes after A's review

| Finding | Current behavior / regression |
|---|---|
| Preflight and zip can select different EOCD/directories | Refuse nonzero archive comments, then compare each raw library entry with preflight name/raw name, local/central/data offsets, flags, method, CRC and sizes. Nested-EOCD fixture proves the library selects FAKE BODY while the control reads REAL BODY; old Reader accepted it, docx-2 refuses it without text. |
| AES extra overrides method-0 headers | Extra whitelist permits only validated inert Office growth hints. AES, Unicode aliases, ZIP64 and unknown extras are Unsupported. Stored packed/unpacked lengths must match. The AES fixture proves the library actually sees Deflate before checking explicit rejection. |
| Ancillary filename guesses cause false/missing omissions | Follow document relationship types to case-insensitive part names; only actual story content is omitted. Empty headers, separator-only notes, external templates and harmless web links cause no omission. Renamed/case-varied headers and foreign story text are reported. No targets are opened. |
| Complex fields reject the whole document | Omit instructions and cached results from begin through end, including nested fields; retain surrounding supported text as Partial. Invalid/unclosed field controls fail with no prefix. Independent review found and fixed a cross-paragraph concatenation defect; retained paragraphs keep LF boundaries. |
| Repacked fixtures conceal Word container compatibility | Add layout-preserving Word v1/v2 fixtures. Only equal-length core.xml payload and two CRC fields change; all other bytes/offsets/sizes/extras are identical. Independent full byte comparison confirmed this. Repacked fixtures remain separately labelled. |
| CPU work / status / coverage issues | Count attributes in event budget and charge namespace resolution before work; bound QName/URI bytes. Reject case-fold duplicate parts and network-path targets. UTF-16 without BOM and template content type are Unsupported. Paragraph-mark revisions are reported. Stored/Deflate truncation report the actual tail count. |

Non-critical `mso-contentType` processing instructions remain explicitly Unsupported. The optional relaxation proposed by A was not implemented; SharePoint/Teams compatibility is untested and may be limited.

## Reader behavior

`read_docx(&[u8]) -> TextExtraction` opens no path, makes no network request, executes no document content and writes no ZIP entry. Consumers supply verified immutable Snapshot bytes. Main-body UTF-8 transitional/strict WordprocessingML paragraphs, runs, hyperlink labels, tabs, explicit breaks and empty paragraphs are supported. Namespace URIs determine meaning. Character references are decoded; CR/CRLF normalize to LF. There is no artificial trailing newline.

| Status | Meaning |
|---|---|
| Complete | Supported main-body stored text is retained; no detected actual content omission. Empty body can be Complete. |
| Partial | Supported text remains, with actual omissions or truncation named in left_out. All corruption checks continue after text truncation. |
| Unsupported | Encryption, unsupported ZIP/XML/encoding/features or a budget limit. Currently also no supported text with unsupported content; empty text/coverage plus a note. |
| Failed | Damaged/truncated/inconsistent package or XML. Discard every extracted prefix. |

Tables, headers/footers, notes, comments, drawings/text boxes, embedded objects, fields, foreign/unknown body elements and revisions are not extracted. Revisions are neither accepted nor rejected. List numbers, style/layout/pagination and hidden-text visibility are not interpreted. Stored text can differ from visible Word text. This is not full OOXML/XSD validation, OCR, legacy DOC, PDF, PPTX or XLSX support.

The student approved **200:1 ratio**, **Unsupported when omitted content leaves no supported text**, and **renewed Memory consent v2 before any DOCX send** in [A's #100 reply](https://github.com/P30g53/Traeva/pull/100#issuecomment-6101881983). The lifecycle follow-up makes the first two explanations explicit: the Unsupported note lists the actual omitted categories, and highly repetitive long documents exceeding 200:1 are Unsupported with no retained prefix, rather than truncated Partial text. Go/No-Go and production enablement remain pending; no automatic send scope was enabled here.

## Safety budgets

| Resource | Limit |
|---|---:|
| ZIP input / entries | 20 MiB / 512 |
| One expanded entry / package total | 8 MiB / 32 MiB |
| Declared per-entry expansion ratio | 200:1 |
| One XML part / nesting | 4 MiB / 64 |
| Events including attributes, package total | 500,000 |
| Namespace-resolution work | 1,000,000 active-binding scan units |
| Per-element attributes / active bindings | 64 / 256 |
| QName / namespace URI | 256 / 1,024 bytes |
| Retained text | 1,000,000 Unicode scalar values |

Canonical ASCII names cannot traverse/escape; case-fold duplicates fail. Local/central records agree and cover the local-record region without overlaps/gaps/unreferenced records. Only Stored/raw Deflate, ordinary 32-bit descriptors and validated Office growth hints are supported. ZIP64, multivolume, symlinks, encryption, archive comments and other extras/methods are refused. Every entry, including ignored binaries, is bounded and checked for actual length and CRC. Deflate must reach StreamEnd and consume exactly its bytes. All XML parts are validated, including unused ones; DTD/entities and processing instructions are Unsupported. Error notes never include document content.

XML parts are retained in memory for relationship-based story inspection, bounded by the 32 MiB expanded package total. These are logical parsing/resource caps, not an OS process-memory/time sandbox. Concurrent Reader limits are not measured.

Dependencies stay pinned to quick-xml 0.42.0, zip 8.6.0, flate2 1.1.10 (Rust backend) and crc32fast 1.5.2. A maintains workspace dependencies/lock. Advisory context: [quick-xml namespace allocation](https://rustsec.org/advisories/RUSTSEC-2026-0195.html), [duplicate attributes](https://rustsec.org/advisories/RUSTSEC-2026-0194.html), [ZIP extraction paths](https://rustsec.org/advisories/RUSTSEC-2025-0168.html). The Reader does not invoke ZIP filesystem extraction; this is not an exhaustive dependency audit.

## Executed verification

| Check | Actual result / boundary |
|---|---|
| D Reader + real core Snapshot APIs | 79 passed, 0 failed: 76 Reader + 3 Snapshot, on current main core plus the rich-fixture test. Standalone fmt/Clippy passed. No default-data writes or cloud calls. |
| Separate Reader development reviewer | Frozen source hash unchanged; earlier test hash in r2-review.json: 78 passed, fmt/Clippy passed; 28 prior independent inputs rerun. This is not Codex-R acceptance. |
| Incremental independent fixture review | [rich-review.json](../../crates/traeva-core/tests/fixtures/docx/rich-review.json): 79 passed; byte-range proof and helper reproduction passed for v1/v2/rich. This reviewer performed no Word GUI or rich Snapshot capture. |
| Layout-preserving actual Word v1/v2 | Complete and exact expected text; originals and scrubbed fixtures independently checked. |
| Original immutable Word v1 Snapshot, fresh process/docx-2 | Original hash, 12,229 bytes and expected text preserved after earlier Word v2 save/source rename. No family/lineage inference. |
| Earlier A #103 + D source + test migration, isolated tree | A head `449e95fca08498ad57585ee2b8d9f74b61ac8973` shared bytes/actual workspace lock unchanged. Earlier 78-test D hash: 829 workspace tests passed, 0 failed, 10 ignored; fmt/Clippy passed. Public snapshot_text is asserted as docx-2, with no none fallback. This count does not include the new rich golden test or later main Source changes. A must rerun its updated head. |
| Current-main core regression | `cargo test -p traeva-core --offline --locked`: 728 passed, 0 failed, 9 ignored; workspace fmt/Clippy passed. Initial sandbox run had 82 loopback bind permission failures, all retained in validation.json; rerun with approved local mock-server access passed. No real provider enabled. |
| Current-main frontend regression | Build passed; 89 tests passed, 0 failed on `6f14e3a`. |
| Actual rich Word Save As | 18,335-byte actual Word ZIP retains all eight checked features. Original and layout-preserving scrubbed fixture match all 108 expected scalars as Partial, with comments, fields, notes, headers/footers, tables and revisions disclosed. New golden test checks the exact text and omission set. |
| Rich immutable Snapshot, source renamed, fresh process | Original hash `ec860ece…ee02` and SnapshotRef preserved; expected text matched. Earlier standalone CLI and A449+D public read_text example both passed; a freshly rebuilt standalone CLI on current main also passed after source rename. Full Word lifecycle/semantic/retrieval remains untested. |

The old docx-1 report and measurements are retained in [validation-docx1.json](../../crates/traeva-core/tests/fixtures/docx/validation-docx1.json). The earlier independent-review.json is explicitly superseded. First revision lint/fmt failures and the independently found paragraph bug are retained in r2-review.json. Rich validation retains two failed SEQ-retention checks: Word had normalized it to fldSimple, rather than merely changing case as D's first UI return comment incorrectly stated. The corrected check inspects both forms. Scrubbing also initially failed on one author field and then on the packed-length search; the helper now accepts the existing author fields and uses a valid intermediate sync flush when needed.

Reproduction on #100 before wiring:

```sh
cargo test --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --offline --locked
cargo clippy --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --all-targets --offline --locked -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --check
```

A's wiring branch receives D's prepared migration to `tests/docx_snapshot.rs` and `examples/docx_probe.rs`, removing the nested crate and second lock. #100 retains its standalone harness so it can be kept without production wiring if No-Go. Validate the migrated branch with `cargo test --workspace --offline --locked`, workspace Clippy/fmt and `cargo run -p traeva-core --example docx_probe -- inspect FILE EXPECTED_TEXT`. Shared manifests/lib/dispatch/contracts remain A-authored.

## Performance and remaining validation

Release means on layout-preserving Word v1/v2 are 1.016 / 0.989 ms over 200 reads each (64/72 scalars); the actual rich Word fixture is 1.596 ms over 200 reads (108 scalars). The synthetic 10,000-paragraph document is 433,762 bytes / 888,889 scalars: 12.076 ms over 10 reads. Whole-process peak RSS for that selected case was 14,303,232 bytes (about 13.6 MiB). These earlier release measurements use frozen docx-2 on core `1c0ff381`. This is one arm64 host and selected inputs, not worst-case guarantees or production concurrency evidence.

Before October 13: Codex-R must independently verify the exact proposed commits and remaining Word lifecycle cases, including continuous/no-change saves, atomic replacement, move/rename, restart and writes racing capture. The rich Save As/feature-retention/immutable Snapshot checks above are complete but do not cover that whole matrix. For optional student reproduction, copy [rich-source.docx](../../crates/traeva-core/tests/fixtures/docx/rich-source.docx) into a new synthetic folder, open the copy in Word, Save As a new DOCX, close it, and compare locally with [rich-source.txt](../../crates/traeva-core/tests/fixtures/docx/rich-source.txt), keeping the six omission labels. These are stored-text expectations, not visible field/list rendering. Never edit a fixture or a real user document in place.

At October 13, report correctness, failures, limits, timings and actual integration work to the student. **No Go has been decided here.** Only an approved Go allows the conditional Word → Semantic Change → Retrieval phase. A's dispatch is ready as Draft; B already consumes snapshot_text and Reader coverage, but the complete Word path has not been verified. Expected bounded effort remains A wiring/regression under one day and approved lifecycle/semantic/retrieval checks roughly one to two days, subject to their schedules. Consent-scope decisions and any old-Word opening review can add work.

No-Go preserves the Reader/tests/report and stops expansion without affecting #26/#37. Old Word opening stays disabled pending Protected View/quarantine review. No PDF/PPTX/XLSX, shared schema/DTO/migration/command changes, automatic merge or Issue closure.
