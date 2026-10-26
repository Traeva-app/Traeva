# #28 PPTX Reader independent experiment

> Historical standalone PPTX experiment report. Statements that production dispatch is unchanged or returns reader `none` describe this experiment, before integration. PPTX is now connected to Memory; see the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md). An [English fixture guide](../../crates/traeva-core/tests/fixtures/pptx/README.en.md) includes the later metadata-cleanup notes. Original experiment results remain tied to their recorded versions.

This implements `read_pptx(&[u8]) -> TextExtraction` in an isolated worktree on main `069208aa37f07ba6a1539b188c94b67ab975ba60`, after reading DOCX, shared-contract §4.2/§4.4 and Snapshot APIs. The user explicitly authorized the PPTX experiment on 2026-10-11; the existing #28 backlog restriction still applies to production expansion. This PR only references #28 and does not close it.

The Reader opens no path, writes no file, invokes no application, performs no network request and executes no document content. Its standalone harness imports existing core types without changing them. PowerPoint GUI operations and synthetic file I/O occur only in the test workflow. Production `read_text`/`snapshot_text` still return reader `none` for PPTX; a regression asserts that boundary.

## Supported text and ordering

The package-level officeDocument relationship identifies the presentation part. `p:sldIdLst` and its relationship IDs determine slide order, including hidden slides; filenames never determine order. Relationships resolve case-insensitively to checked internal package members. Explicit title/ctrTitle placeholders and matching layout-inherited placeholder types produce `[Title]`. Other supported shapes produce `[Text]`, in shape-tree XML order, including grouped shapes. Notes are associated through each slide's notesSlide relationship; supported body and non-placeholder note shapes produce `[Notes]`.

Text uses LF paragraph boundaries, explicit DrawingML breaks, decoded entities/CDATA and Unicode scalar counts. Generated `[Slide N]`/role labels make the output unambiguous without adding shared DTOs; their characters count toward the existing 1,000,000-character cap. These labels are experimental extraction syntax, not a shared interface decision. Any future output-rule change must bump `pptx-experiment-1`.

The result is stored text, not a rendered slide or visual reading order. Formatting, layout, bullet numbering, hidden-text visibility and slideshow behavior are not interpreted. Template/master placeholder sample text is not copied into slide text. Non-placeholder master/layout/notes-master text is disclosed with visibility unresolved. Slide thumbnails in notes are not duplicated.

## Status and omissions

| Status | Behavior |
|---|---|
| Complete | Supported stored-text subset retained with no detected actual omission. An empty deck/slide can be Complete. No general claim of full PowerPoint reading. |
| Partial | Supported source text retained, plus actual omitted categories or exact truncation tail in left_out and a note. |
| Unsupported | Unsupported package type/encoding/features, encryption/macros or exceeded budget. Also used when actual omissions leave no supported source text: generated labels do not rescue a picture-only deck. Text/coverage/left_out are empty; note names the reason. |
| Failed | Corrupt/truncated ZIP/XML, CRC mismatch, duplicate IDs/parts, missing internal target or inconsistent slide/notes relationships. Every extracted prefix is discarded. |

Actual detections include pictures and alternative text, charts, tables, SmartArt, animations/timing, transitions/timed advance, dynamic fields, audio/video, embedded objects, comments, unknown/alternate content, external content targets, unreferenced slide/notes content and inherited static text. Empty comments, empty timing and empty compatibility containers do not invent omissions. External hyperlink labels are retained; destinations are never accessed. Unsupported binary formats and ignored feature bodies are not validated as full PowerPoint/XSD schemas.

## ZIP/XML safety and budgets

The experiment retains an isolated copy of DOCX ZIP/XML checks in `pptx_package.rs`. Local/central records and the ZIP library's view must agree on names, offsets, flags, method, CRC and sizes. Records cover the local region without overlaps/gaps. ASCII member names cannot traverse/alias; case-fold duplicates fail. Every member, including ignored binary content, is bounded, length-checked and CRC-checked. Raw Deflate must reach StreamEnd and consume exactly its bytes. Signed/unsigned 32-bit data descriptors and validated inert Office growth hints are allowed; ZIP64, multivolume, symlinks, encryption, archive comments and unknown/AES/Unicode-path extras are refused.

All XML/.rels parts are checked, including unused ones. XML content types under non-XML filenames are checked too. Namespace URIs determine element meaning. DTD/entities, processing instructions and non-UTF-8 encodings are Unsupported; damaged XML fails. Attribute separators, expanded duplicate attributes, XML characters, names and namespace declarations are checked. Relationship targets never become filesystem paths or URLs; missing internal targets fail. Macros are refused through member names, content types and relationship types. Notes backlinks, duplicate slide/notes targets and external structural targets fail closed.

| Resource | Limit |
|---|---:|
| Input / ZIP entries | 20 MiB / 512 |
| One expanded member / package total | 8 MiB / 32 MiB |
| Expansion ratio per member | 200:1 |
| XML part / depth | 4 MiB / 64 |
| Total parser events including attributes | 500,000 |
| Namespace scan work / active bindings | 1,000,000 / 256 |
| Attributes per element / QName / namespace URI | 64 / 256 bytes / 1,024 bytes |
| Ordered slides / retained Unicode scalars | 200 / 1,000,000 |

Repeated semantic parsing is charged to the same XML budget. Corruption checks continue beyond text truncation. Limits are logical parser caps, not an OS memory/time sandbox or measured concurrency guarantee. A long repetitive document can exceed the ratio limit and become Unsupported before text truncation.

## Executed validation

Environment: macOS 27.0.1 arm64, Rust 1.98.1, Microsoft PowerPoint 16.78. All data is synthetic and isolated; no real model, credentials, formal Traeva data or shared UI instance was used. PowerPoint was placed and verified on the Dell AW3418DW secondary display after opens/saves.

- Standalone Reader/Snapshot tests, formatter and strict Clippy passed; exact counts and source hashes are in [validation.json](../../crates/traeva-core/tests/fixtures/pptx/validation.json). Coverage includes strict/aliased namespaces, Unicode/CRLF, grouped shapes, inherited titles, reversed order/hidden slides, notes, category omissions, empty content, resource caps, all truncations of a small archive, deterministic mutations and post-truncation corruption.
- Existing DOCX adversarial ZIP seeds for nested EOCD, AES method override and Unicode-path alias are reused before presentation parsing. Other tests cover traversal, header/CRC disagreements, symlinks, Deflate tails, entry/expanded-total/XML/namespace budgets and external targets.
- Four actual PowerPoint saves match their exact expected stored text. Basic v2 changes one note. Both rich saves retain picture/chart/table/animation/transition and disclose six categories as Partial. The corrected rich source and its saved output reopen without repair.
- Snapshot tests use existing APIs with temporary synthetic roots/app-data only: read verified bytes, refuse silent adoption after atomic replacement, survive source rename and service restart, and keep missing/corrupt/unknown snapshot errors at stage Memory. This is contract compatibility evidence, not production PPTX Memory enablement.
- Required repository regression: 792 core tests passed, 10 existing ignored; workspace fmt/strict Clippy and `npm run build` passed on the pinned baseline. Root workspace checks do not compile the unregistered experimental Reader; its separate checks above do.

Failures are retained: first compilation type inference and sha2 hex formatting errors; first Reader run 42 PASS/2 FAIL (a namespace-alias fixture accidentally changed `http:` and a label-tail expectation was one character off); 82 initial core mock-server failures from sandbox loopback restrictions, followed by an approved offline rerun; first synthetic rich timing required PowerPoint repair; first retention check looked at the pre-save part filename after PowerPoint renumbered slides. A later golden test retained a 57 PASS/1 FAIL run: animation property strings were redundantly labelled as unsupported body text. The fix reports those properties under animations/timing while still disclosing unknown inline body text. Corrected checks follow actual saved members/order. These are developer iteration records, not failures erased from a frozen evaluation.

## Reuse and A/B follow-up

| Area | Reused here | Future integration owner/work |
|---|---|---|
| Shared extraction/Snapshot contract | Existing TextExtraction/TextSummary/TextStatus and cap; existing checked Snapshot read API in isolated tests | A reviews and registers pure PPTX dispatch only if separately authorized. No new shared fields/migration proposed as approved. |
| Container safety | DOCX preflight, raw Stored/Deflate/CRC, namespace-aware XML validation and resource policy | A may extract one reviewed common Office-package helper; this isolated copy deliberately does not refactor DOCX/shared code. Dependency/lock deduplication also belongs to A. |
| Test tooling | Existing DOCX metadata scrubber unchanged; existing adversarial ZIP fixtures; temporary data conventions | A migrates the nested validation harness into normal tests only during approved integration, then reruns current-head regression. |
| Presentation semantics | New slide-order graph, title placeholders/layout inheritance, notes association, feature omissions | A/R/student review exact semantics and corpus compatibility. Real-save evidence is development validation, not independent Codex-R acceptance. |
| Semantic changes/retrieval/UI | No connection in this PR | B must consume reader version and Partial/Unsupported/Failed coverage faithfully, verify evidence offsets/labels and real Snapshot changes; never claim complete reading. New AI send consent, UI, watcher and source lifecycle decisions require separate scope. |

Production Memory/Organize/Tauri IPC/AI registration, migrations, shared interfaces and main program are untouched. No Go/No-Go, CAC/Gate PASS, #28 closure, PR merge or cloud authorization is inferred from this experiment.

Reproduce from the repository root:

```sh
cargo test --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --offline --locked
cargo clippy --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --all-targets --offline --locked -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --check
cargo run --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --offline --locked --bin pptx-probe -- crates/traeva-core/tests/fixtures/pptx/powerpoint-rich-clean.pptx crates/traeva-core/tests/fixtures/pptx/expected.txt
```

## Unregistered projection preparation

The student authorized E to prepare the full-Memory output adaptation in an
isolated branch while R2 finishes its current #26 verification. Baseline is
`c6a711d7f3155aee0f30b82eb44aa27cc267e523`; production wiring, permission notices,
shared DTOs, root dependencies/lock and Tauri registration are not changed here.
The old `read_pptx` / `pptx-experiment-1` output and four Office development
fixtures remain available. Their acceptance does not cover the new projections.

The preparation follows A/B's actual replies on #65 and the finalized v23:
[A's proposal](https://github.com/P30g53/Traeva/issues/65#issuecomment-6116379340),
[B's corrections](https://github.com/P30g53/Traeva/issues/65#issuecomment-6116391271),
[#150 f19c31b](https://github.com/P30g53/Traeva/pull/150) and the actual Draft
helper [#151 3d1a4a9](https://github.com/P30g53/Traeva/pull/151).
The helper is not treated as a released shared API. E's format confirmation and
boundary questions are in
[#150](https://github.com/P30g53/Traeva/pull/150#issuecomment-6116693656).

| Explicit pure entry | Reader version | Selected stored text |
|---|---|---|
| `read_pptx` | `pptx-experiment-1` | Original experimental syntax, including hidden slides |
| `read_pptx_memory` | `pptx-1` | Visible slides' titles/text and associated Notes |
| `read_pptx_preview` | `pptx-preview-1` | All slides/Notes, hidden slides marked; local preview only |

All return the existing, non-serializable `TextExtraction`. They open no path,
write no file, call no model and grant no sending permission. A must obtain
`(SnapshotRef, bytes)` from `read_snapshot`, retain the exact SnapshotRef/Reader
binding, then choose the entry. Unknown/missing/corrupt snapshots remain Memory
AppError; package damage in verified bytes is an extraction status. Tests exercise
both projections from one checked tuple after the original is changed. Production
`snapshot_text` still dispatches PPTX to `none`.

New projection grammar follows v23 (#150 reviewed head
`f19c31b6c7ad6f5095668a07e76a9ca0a290e10e`, merged). The #151 helper code is
still Draft; a helper-compatible producer is not production registration:

- `⟦slide N · id X⟧`, with ` · hidden` only in preview; `X` is validated package
  `sldId`, not a permanent identity. `N` remains the original presentation ordinal,
  so Memory filtering can leave gaps. A confirmed this interpretation in
  [the v23 clarification](https://github.com/P30g53/Traeva/pull/150#issuecomment-6116720334).
- `⟦title⟧`, `⟦text⟧`, `⟦notes⟧` each occupy their own LF-separated line. Notes stay
  in their slide block. Page blocks have one connecting LF, without an invented
  blank source line; original paragraph/line whitespace remains stored text.
- Source lines starting with `⟦` or `\` gain one leading `\`, without trimming.
  Lines like `[Slide 3]` are original text under the new grammar. Positions count
  the escaped projection's Unicode scalars, including labels/LF/escapes, rather
  than UTF-8 bytes, UTF-16 units or graphemes. A/B's shared helper must classify
  only the agreed Reader names; generated and unrecognized structure are never
  original-word evidence. Display may remove the one escape, but QuoteAt stays
  bound to the projected coordinate.
- The producer does not implement a second shared recognizer or a mapping DTO.
  Its streaming prefix escape is checked against the actual #151 helper. A owns
  the final shared helper/constant source and registration; B owns whole-text
  evidence lookup, structure-only/reorder decisions and UI/consent changes.

The A/B responsibilities above describe this experiment's original integration stage. Current Find and Memory work follows [AGENTS.md §4](../../AGENTS.md#4-session-responsibilities-and-shared-files): the student owns the core implementation, agents handle permitted wiring and review, and R independently verifies changes. The original experiment evidence and implementation attribution remain unchanged.

The package is still fully validated when a slide is excluded or text is cut.
`show` accepts XML boolean lexical values `0`/`false` and `1`/`true` with XML
whitespace; absent means visible, an invalid value fails the new projections.
Visibility here is this slide flag, not rendered shape/style visibility.
Actual unsupported-feature categories remain **package-global**, including
features in intentionally excluded content; hidden-slide/Notes policy exclusion
is separately counted. No supported source text after selection is Unsupported
with empty output, even if labels could be generated. Blank legacy behavior is
preserved; it is not the new projection's source-evidence rule.

The million-scalar cap is unchanged. Complete labels (with their delimiters) and
the two-scalar source escape prefix are atomic. At the first boundary that cannot
fit safely, output stops and may keep slightly fewer than the cap. `left_out`
counts the full selected/escaped projection minus the actual retained scalars.
No partial label becomes apparent source evidence, no non-BMP scalar is split,
and subsequent hidden/unused content is still checked. A confirmed these atomic
boundaries and the single-LF page separator in the same v23 clarification.

For ordinary reorder B can group valid output by sldId, ignore only the page
ordinal and compare all role/source/Notes lines. Changed content or incomplete ID
sets cannot establish pure reorder. Missing/duplicate/invalid IDs still fail the
original package checks rather than being accepted for comparison. Even matching
projected pages cannot prove that unsupported images/layout/animation stayed the
same; structural conclusions must name the stored-text scope and quality.

Validation expectations were frozen before execution in E's isolated evidence
directory. The first locked build exposed the pre-existing nested-lock metadata
gap after DOCX dependency registration; only four `traeva-core` dependency edges
were refreshed in this validation crate, with no package version or root-lock
change. A test-helper lifetime compilation error and the first failing added
last-page-move check (extra page separator) are retained. The latter changed the
candidate's page delimiter; prior passes are not relabelled as independent PASS.

Use the existing probe with a final `memory` or `preview` argument and an
independently prepared expected text; the old two-argument call remains legacy:

```sh
cargo run --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --offline --locked --bin pptx-probe -- OWN.pptx OWN-memory.txt memory
cargo run --manifest-path crates/traeva-core/tests/pptx_reader/Cargo.toml --offline --locked --bin pptx-probe -- OWN.pptx OWN-preview.txt preview
```

The actual A helper compatibility check compiles a temporary validation crate
with a direct dependency on A's unmodified core checkout, plus E's real source
via `#[path]`. It does not copy a same-rule recognizer. It requires exactly
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`, checks that checkout is clean, records
the actual `reader.rs` hash, and refuses to overwrite a previous attempt directory:

```sh
python3 crates/traeva-core/tests/pptx_reader/check_a151.py --a-checkout OWN-A-151-CHECKOUT --output OWN-NEW-COMPAT-DIRECTORY
```

The new source/visibility/truncation behavior still needs R3 on its exact new
commit. R3's prior 59-input offline and 22-judgment native PowerPoint LIMITED PASS
remain the old-version evidence. No new Office GUI, real-model, packaged App or
integrated Memory/consent/retrieval validation is claimed here. Root wiring and
permission notice v4 must wait for the agreed R2 handoff and A/B integration.

Development checks: 76 standalone tests passed (56 legacy Reader, 17 projection,
3 Snapshot), plus 5 actual-helper compatibility tests at each pinned #151 head.
Both the standalone and actual-helper crates passed strict Clippy. Workspace
format/Clippy and frontend build passed; core passed 877 tests with 10 intentionally
ignored after the approved loopback-mock rerun. The first sandbox run (692 passed,
82 failed, 4 ignored) and focused PermissionDenied diagnostic are retained.
Exact source/protocol/log hashes and check scope are in
`crates/traeva-core/tests/pptx_reader/projection-validation.json`; no result here
is R3's new-version PASS.
