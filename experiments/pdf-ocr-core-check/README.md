# OCR core module handoff to A

> Historical OCR module-handoff verification. The unregistered-module and no-product-wiring statements below describe this migration stage. Current Organize OCR support is described in the [README](../../README.md#supported-content) and [Organize rules](../../docs/features/organize-spec.md). The [English development log](evidence/dev-log.en.md) translates the original evidence log without replacing the file recorded in its manifest. Original failures and acceptance limits remain unchanged.

This directory verifies the unregistered module in `crates/traeva-core/src/pdf_ocr/`. Only new files are added; product Cargo, lib.rs, Tauri, Organize and UI are not modified. It is implemented per [A's latest narrowed contract](https://github.com/P30g53/Traeva/issues/211#issuecomment-6140676858) and keeps the resource rules of the [original coordination decision](https://github.com/P30g53/Traeva/issues/211#issuecomment-6140147939), Refs #186. The old experiment [#215](https://github.com/P30g53/Traeva/pull/215) keeps the first failures, the full set of ordinary samples and the original report.

## Source and migration differences

The source is #215 `f7923f8f1e0c5055741182e4dd756330cbe7ff72`, with the main implementation at `c2854919e7d75d8d27a08bbc2c3871f36fcb20e2`. The first check found that the five PDF source files on main `bebf841` match the experiment's dependencies; the final port is based on `f0f96eee7b7e9b953937be17421ddcafbfd2849d` and uses the heap-setting function that A opened up in #218. The old pdf_child was not copied; main's R2 Partial fix and Find method are kept.

Mechanical changes: imports inside the module now use `super`; the reader now uses `crate::pdf`; TextStatus uses the core contract; macOS conditional compilation replaces the separate experiment feature; the original render/Vision algorithm and the synthetic bitmap are copied. Per-page pixel release, an independent RenderCache per page, and the Vision autoreleasepool all come from the verified experiment; they are not claimed as new inventions in this change.

Explicit functional changes:

- Default 3 pages, automatic language on; keeps 200 dpi / 8M pixels / at least 10 non-whitespace characters per page.
- The product entry only accepts a PDF that has no usable text layer anywhere in the document. If any page is `Text` or `TextWithOmissions`, it is rejected after the same single PDF parse and before rendering and Vision; mixed PDFs and text-layer-only PDFs both take the original text-layer path.
- The product output is tightened to 20 MiB of byte input and strictly three fields; if any render/recognition fails or the final text exceeds 1,000,000 Unicode scalars, the whole text is discarded.
- It succeeds only when OCR actually produced text, and the status is always partial. The text is rebuilt only from the text lines of successfully OCR'd pages, at most three `[Page N]` sections, keeping the original PDF page numbers; pages that were not recognized get no empty heading. The OCR note states up front the page count, possible errors and the number of omitted pages, at most 300 characters.
- macOS <13 is rejected before any Vision object is created; revision <3 is rejected before the automatic-language setter. Pure boundary tests are not the same as real-machine coverage on an old OS.
- The memory API keeps both the raw kernel peak and `max(raw_peak,current)`. The first real test hit a raw peak lower than the current value; that failure is kept. This normalization is still only an observed lower bound, not a hard cap.

## A's registration checklist and API

Register the module in core and export the entry points below; A is responsible for app flags and parent scheduling. The module itself creates no window or product background service.

| API | Input and result | A-side responsibility |
|---|---|---|
| `ocr_child_main(args: &[String]) -> i32` | stdin ≤20 MiB; stdout a single `{status,text,note}`; the entry sets the original `set_heap_cap(512 << 20)`, validates arguments and starts the parent-exit watchdog | The App installs the existing CappedAlloc and passes the parent PID; the parent side handles the single task slot, the 15-second budget, cancellation and reaping |
| `ocr_prepare_main(args: &[String]) -> i32` | Reads no stdin/file; fixed 960×240 synthetic bitmap; validates the parent PID and starts the watchdog; success is only `{"status":"ready"}` + exit 0 | Separate flag, pass the parent PID, 90-second budget, cancellation, prepare on every app launch |
| `process_memory(pid: u32) -> Option<ProcessMemory>` | Current physical footprint, normalized lifetime peak, raw kernel peak, in bytes | 5 ms target interval, 384 MiB soft threshold; reject on None; keep the running maximum across samples; final check before reap |
| `child_parent_watchdog(parent_pid: u32) -> Result<(), String>` | Called inside both entries; verifies getppid at startup, then checks every 50 ms; on parent exit `_exit(125)` | The parent PID is passed in through the parent's launch arguments; the child must not infer it by itself; no need to start a second watchdog thread |

Both entries must receive exactly one `--parent-pid <PID>`; the PID must be greater than 1, representable as a macOS process ID, and match the actual parent process. The recognition entry may also receive `--max-pages 3 --dpi 200 --max-pixels 8000000`; when these are omitted, the same defaults apply. Only the approved values are accepted; no tuning is opened up. The prepare entry only accepts the parent PID. Unknown arguments, repeated arguments, missing values and invalid values all exit 2 and never reach reading or Vision. The compatibility export `ocr_prepare_child_main` is an alias of `ocr_prepare_main` with the same signature.

`ProcessMemory` fields: `footprint_bytes`, `lifetime_peak_bytes`, `kernel_lifetime_peak_bytes`. The last one is the uncorrected system value; the first two are used for the stop decision. macOS process memory covers Vision's allocations inside the child process, not the memory of other system-service processes. The pixel budget also does not limit the decoded size of source images or the PDF's internal caches.

For the prepare entry, exit 2 means argument/parent-process validation failed or preparation did not finish, and exit 3 means writing the output failed. For the recognition entry, exit 2 also covers failing to read stdin / exceeding 20 MiB, and exit 3 means writing the output failed; after exit 0 the result must still be judged by the JSON status. The parent side must limit stdout/stderr, parse strictly, re-check cancellation and the time limit after parsing, and check memory and reaping; any anomaly drops the text, with no automatic retry. Product heap rules, parent scheduling and status wiring remain A's to maintain; this module does not replace A's supervisor.

| Recognition result | JSON status / text | Parent status and next step |
|---|---|---|
| No usable text layer in the whole document, at least one page OCR'd, no errors on any attempted page | partial / text of at most three actual OCR pages | The source is determined by the successful OCR call path, metadata marked ocr; never guessed from the note; no automatic move |
| Mixed PDF, text-layer-only PDF | unsupported / empty; no rendering, no Vision call | Should take the original pdf_child; does not enter the OCR fallback |
| No recognizable scanned page found, too few recognized characters, encrypted/unsupported, total text over the limit | unsupported / empty | Fall back to manual pick |
| Reader failure, or render/recognition failure on any attempted page | failed / empty | Fall back to manual pick; revoke Ready per A's rules |
| Child timeout, cancellation, memory over the limit, monitoring failure, crash, or cannot be reaped | The child's answer is not accepted | Revoke Ready; the single task slot is released only after reaping is confirmed; if not reaped, it stays unavailable |

The original reader classifies a page as ImagePage only when it has "no readable text and images cover at least 50%"; OCR reuses this page table. If any page in the document has a usable text layer, OCR is not attempted; pages with smaller images, pure vector graphics or unmapped fonts may also not trigger it. Content beyond the first 3 qualifying scanned pages is not recognized; the note covers omitted pages and the original reader's other omissions. Finished OCR preparation does not prove that the caches for all languages stay valid permanently.

`read_with_ocr` and the verification tool's `--inspect` keep the original experiment's detailed diagnostic behavior for the frozen page-by-page migration comparison, and may still attempt OCR on a mixed PDF. The product entry uses the separate `read_scanned_document` path, which first checks whether the whole document qualifies; diagnostic results must not pass as product output.

## Exact dependencies

These are the direct dependencies A needs to add; keep the default features, objc2 only for the macOS target. The exactly resolved transitive dependencies are in this directory's separate Cargo.lock (from #215, with only the verification crate's package name replaced). After A updates the product Cargo/lock, A needs to generate the third-party notices and pin the final product build.

```toml
hayro = "=0.8.0"
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "=0.6.4"
objc2-foundation = "=0.3.2"
objc2-core-foundation = "=0.3.2"
objc2-core-graphics = "=0.3.2"
objc2-vision = { version = "=0.3.2", features = ["objc2-core-graphics", "objc2-image-io"] }
```

Reused from what core already has: libc 0.2.189, serde 1.0.229 + derive, serde_json 1.0.151; the PDF chain already has hayro-interpret 0.8.0, hayro-syntax 0.8.0 (direct dependency with default off), kurbo 0.13.1, flate2 1.1.10, memchr 2.8.3. hayro enables the underlying image/decode features across the build, so turning off defaults on a direct dependency cannot be used to claim that JPEG2000/JBIG2/CCITT are absent.

## Reproduce

In a local checkout on macOS 13+, Rust 1.98, with dependencies already cached:

```sh
cargo fmt --manifest-path experiments/pdf-ocr-core-check/Cargo.toml --check
cargo test --offline --locked --manifest-path experiments/pdf-ocr-core-check/Cargo.toml pdf_ocr::
cargo test --offline --locked --manifest-path experiments/pdf-ocr-core-check/Cargo.toml --bin traeva-pdf-ocr-core-check
cargo clippy --offline --locked --manifest-path experiments/pdf-ocr-core-check/Cargo.toml --all-targets -- -D warnings
cargo build --release --offline --locked --manifest-path experiments/pdf-ocr-core-check/Cargo.toml
python3 experiments/pdf-ocr-core-check/check.py --source /path/to/frozen-215/experiments/pdf-ocr --source-sha <exact-port-code-sha>
python3 experiments/pdf-ocr-core-check/lifecycle_extra.py --driver experiments/pdf-ocr-core-check/target/release/traeva-pdf-ocr-core-check --source-sha <exact-port-code-sha> --sample /path/to/frozen-215/experiments/pdf-ocr/samples/s03-mixed-clear.pdf
```

`--source` points to a full checkout of #215; first build the old `pdf-ocr-probe` per its README. check.py first checks the PDF hashes in truth, then signs the new experiment App and calls prepare and recognition with an empty env. The monitoring driver attaches its own PID for the child, so there is no need to type the parent PID on the command line.

The detailed diagnostics still compare the original experiment's text, source and omissions character by character and page by page; the verdicts are not relaxed. The product wire is verified separately against the narrowed contract: a document with a text layer must be rejected; a scanned document's text must equal the text lines of the original experiment's actual OCR pages, at most three non-empty page sections, with no empty heading for trailing pages. Known recognition differences are still recorded as FAIL; the s09/s10 label failures in the old frozen evaluation are also not exempted or re-judged. Migration equality is not acceptance of recognition quality.

`lifecycle_extra.py` uses three different signed bundle IDs and checks prepare cancellation, prepare timeout, parent-process exit and the later retries; `--sample` may be omitted, and when given only the frozen s03 is accepted. A new bundle ID is still not evidence of a freshly installed Mac. Both scripts create a new evidence file on every run and save incrementally, keep the first failures, and do not modify the original samples or expectations; signed runs must be scheduled by the experiment to run serially.

The standalone harness compiles these real core source files by path; build.rs extracts the existing allocator section verbatim from the product pdf_child, and the test binary only installs that allocator, with no separately written algorithm. Before the product registers the module, a normal root build does not compile OCR at all; this harness prevents a "new module that was never compiled" from being falsely reported as passing. The test filter deliberately excludes the original pdf's three fixture tests; this is not a new re-verification of them. The signed bundle is an experiment CLI bundle, not a Tauri GUI / release signing / notarization acceptance.

Additional reproduction with the formal bundle metadata:

```sh
python3 experiments/pdf-ocr-core-check/product_metadata.py --plist /path/to/generated/Traeva.app/Contents/Info.plist --source /path/to/frozen-215/experiments/pdf-ocr --driver experiments/pdf-ocr-core-check/target/release/traeva-pdf-ocr-core-check --source-sha <exact-port-code-sha>
```

This command only reads the generated plist and creates a new signed CLI bundle in the experiment directory; it does not launch the original product App. The verdict covers two preparations, 15 frozen texts and at most 3 page sections. The first-round out-of-bundle comparison records are in this directory's evidence; all failures are kept. For results and integration boundaries see the [report](REPORT.md); for evidence integrity see the [manifest](evidence/manifest.json).

The strict entry also confirms that the actual OCR page numbers are positive integers and strictly increasing; an empty text or a 0/repeated/out-of-order page number discards the whole text. If any line of the OCR text, after stripping leading whitespace, starts with the reserved marker `[Page `, it also returns failed/empty text, to avoid confusion with section boundaries; the recognized text is not deleted or rewritten to manufacture a success. Mentioning that string in the middle of ordinary text is not affected. A's parent side must still validate the output independently.
