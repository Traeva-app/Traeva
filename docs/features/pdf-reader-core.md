# The PDF reader in the core (#186)

> Historical PDF reader migration report for #186. The initial Organize-only decision below describes the scope at that stage. The current product uses the PDF text layer in Organize and Find; Memory does not read PDFs. See the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md) for current behavior. This note does not broaden the original LIMITED PASS.

On 10/12 the student decided that only Organize reads the PDF text layer ([#186](https://github.com/P30g53/Traeva/issues/186), organize-spec §14). A's #189 already built the child process and the wiring: `pdf_child` calls `crate::pdf::read_pdf`, and at that time `pdf.rs` was only a placeholder. This step moves the reviewed reader from the experiment into the core and replaces that placeholder file:

- Above the existing `mod pdf;` in `lib.rs`, only `#[allow(dead_code)]` and a comment were added. The core uses only `read_pdf`; the page table, the scan workload report and other parts brought from the experiment have no callers for now.
- This step did not change `reader.rs`, `observe.rs`, `pdf_child.rs`, Memory or Tauri. Memory does not read PDF.

## Source and version

- `pdf-1` is `pdf-experiment-9`: `81ddd8036d60b14d2d631ad05ae57fa025945625` of PR #148, `experiments/pdf-reader/src/`.
- Codex-R4's fifth round concluded a scoped [LIMITED PASS](https://github.com/P30g53/Traeva/pull/148#issuecomment-6135900327) on this commit. This **does not equal** resource safety, nor a Go for integration, so §14.3 requires parsing in a child process.
- What was moved is the base version without quality diagnostics. Turning #158's quality report (suspected multi-column, invisible text layer) into a rule for "is the text usable" needs a separate decision by the student, so it was not moved.

## Files

| Core file | From the experiment |
|---|---|
| `src/pdf.rs` | `src/lib.rs` (replaces #189's placeholder file) |
| `src/pdf_scan.rs` | `src/scan.rs` |
| `src/pdf_preflight.rs` | `src/preflight.rs` |
| `src/pdf_document.rs` | `src/document.rs` |
| `src/pdf_layout.rs` | `src/layout.rs` |
| `src/pdf_tests.rs` | Newly written tests, using only normal samples |

The experiment's `contract.rs` (a copy of the core contract) is not moved; the core's `memory_contracts` is used instead. `main.rs` (the test tool `pdf-probe`) is not moved either.

## Entry point

```rust
pub(crate) fn read_pdf(bytes: &[u8]) -> TextExtraction  // crate::pdf::read_pdf
pub const PDF_READER: &str = "pdf-1";
```

- **Input**: one fixed set of bytes; it is a pure function. It does not open paths, write files, go online, execute JavaScript or decompress to disk.
- **Output**: the core's `TextExtraction`. `summary.status` is `complete`, `partial`, `unsupported` or `failed`; `summary.note` gives the reason (without file content); `text` is in drawing order, with a `[Page N]` line before each page.
- **Reads only the text layer**: it does not read metadata, forms, attachments or annotations.
  - If form fields or annotations exist, they are written to `left_out`, and the status is `partial`;
  - Exception: Link and Popup annotations do not count; a PDF whose only annotations are of these two kinds is still `complete`.
- **Version with the page table**: `read_with_pages` returns the same result plus a per-page table, for later use when needed.

## The only changes (logic unchanged)

`tests/fixtures/pdf/relocate.py` reads the experiment source from `81ddd80` and generates the core files from one fixed change table. The `--check` mode verifies that the current core files are identical, character for character, to the generated result. PR #148 is not merged yet and main has no `experiments/pdf-reader`, so #148's branch (`claude/optimistic-allen-nazsq7`) must be fetched first.

The change table contains only these:

1. The test module at the end of each file is not moved. Those tests mostly use boundary and malformed inputs, and they stay in the experiment.
2. Module paths: the split files are attached in `pdf.rs` with `#[path]`, and references to the entry module use `super` instead of `crate`.
3. The shared contract uses the core's types (`memory_contracts`, `reader::MAX_MEMORY_TEXT_CHARS`).
4. The reader name is changed to `pdf-1`, and its version note is updated.
5. `read_pdf` returns only `TextExtraction`; the original read with the page table is renamed `read_with_pages`.
6. The entry module's doc text, and the hook for the core tests.

Finally it is formatted with rustfmt (edition 2024), matching `cargo fmt`.

## Limits (public constants in `pdf.rs`)

For the rules below, exceeding one stops the read and returns unsupported, with no text:

| Rule | Value |
|---|---:|
| Input size (Organize's own limit is 20 MiB) | 32 MiB |
| Object count (declared and actual) | 200,000 |
| Page count (the whole document stops; it does not read only the first 500 pages, see below) | 500 |
| Stream count | 50,000 |
| Filters per stream | 4 |
| Decoded size of one non-image stream (with or without filters) | 16 MiB |
| Decoded size of one image stream before it reaches the image codec | 64 MiB |
| Total decoded size of all non-image streams / total decoding work | 128 MiB / 256 MiB |
| Total content streams of one page | 32 MiB |
| Distinct candidate values of one indirect length object | 64 |
| Offset table of one object stream / total member data read in one pass | 64 KiB / 512 MiB |
| Glyphs drawn (whole-document total) / draw operations | 2,000,000 / 1,000,000 |
| Encrypted PDFs | Never opened |

A few other rules do not "stop":

| Rule | Value | When exceeded |
|---|---:|---|
| Text length (shared limit) | 1,000,000 characters | Truncated, status `partial` |
| Nesting depth of dictionaries and arrays | 64 | Stops if exceeded in the file's own stream dictionaries and in containers decoded at open time; elsewhere, the part beyond the limit is no longer checked |
| Drawn page content cached by hayro | 64 MiB | Reopen the document to release the cache (the cache holds at most about 64 MiB plus one page) |

These are **in-process soft limits**. Some allocations inside hayro (font parsing, CMap, cross-reference table) and the paths listed in the experiment README's "Round 6 → Residual risks" are not directly bounded by these constants. So the hard limit relies on the §14.3 child process.

**One mismatch with the spec, for A and the student to decide**: organize-spec §14.2 gives "over the page count" as an example of `partial`; this reader returns `unsupported` for the whole document, with no text, when the page count exceeds 500, and Organize falls back to manual pick. The move does not change logic, so this was moved as is. Either change the spec's example, or later open a separate item to make the reader read only the first 500 pages (this changes the read result and needs R re-verification).

## Memory and time (for setting the child process limits)

**Conclusions first**:

- **The maximum memory and time within the limits are not established.** Below are the measured maximums over 182 existing inputs, not a bound within the limits.
- **Estimated from the code, several paths within the limits can exceed 512 MiB**:
  - When decoding nested forms, each level takes at most 16 MiB and at most 50 levels exist at once, about 800 MiB;
  - The glyph limit counts the whole document, and all glyphs kept for a single page are held in memory, on the order of 200 MiB at the limit;
  - There are also allocations inside hayro that the constants do not bound.

  These are all estimates, not measured. Anything over the child process limit is stopped, counts as failed under §14.3, falls back to manual pick, and is not sent to the model.

### Measurements

Measured by the developer, not R4's results. Environment: macOS 27.0.1, Apple M3 Pro, release build.

- **Method**: using the `pdf-experiment-9` probe, each input is read once in an isolated child process. The heap is counted by the allocator (not RSS), with a 512 MiB limit; the time limit is 30 s.
- **Inputs**: all existing inputs in the experiment, 182 in total, including malformed, boundary and stress inputs.
- **Raw records**: `docs/features/pdf-reader-core-resources.json`.

**All 182 finished normally within the limits**: no memory overrun, no timeout, no crash.

| Input category | Count | Largest input | Largest peak heap | Longest `read_pdf` |
|---|---:|---:|---:|---:|
| Normal samples (program-generated p01–p16, 4 real exports) | 20 | 98 KB | 5.1 MiB | 0.21 s |
| Samples that reach the glyph, draw and form-expansion limits | 3 | 28 KB | 30 MiB | 0.58 s |
| Other adversarial and corrupt samples (including one over the 32 MiB input limit, rejected right after it is read in) | 28 | 34.6 MB | 64 MiB | 0.06 s |
| Defect reproductions from three rounds (including inputs near the 16 MiB single-stream limit) | 75 | 17.8 MB | 64 MiB | 0.07 s |
| Stress inputs aimed at scan time (64 KiB–8 MiB, 7 kinds) | 56 | 8.4 MB | 44 MiB | 2.32 s |

- **Peak heap**: counted by the allocator, including the input bytes themselves and a preflight that the probe runs again after `read_pdf`, so it is larger than `read_pdf`'s own peak. A's child process also caps the heap with a counting allocator, so the numbers can be compared directly; if RSS or rlimit is used instead, these numbers cannot be applied directly.
- **Time**:
  - Counts only `read_pdf` itself in the child process, not starting the child process, passing the bytes in through the pipe, or writing the result;
  - The slowest kind takes 0.62, 1.54 and 2.32 s at 2, 4 and 8 MiB, that is 0.29–0.38 s per MiB;
  - 20 MiB was not measured; extrapolated from this slope it is about 6–7.7 s (on this machine).

### View on A's tentative 512 MiB / 15 s

- **Memory**: 512 MiB is 8 times the maximum over the 182 measured inputs. Heavier paths within the limits (estimated above) may be stopped and fall back to manual pick. This is a result §14.3 allows, but no text can then be read from such PDFs.
- **Time**: the estimate for 20 MiB is 6–7.7 s, so 15 s leaves only about 1.9–2.5x headroom, and the child process's wall-clock time must also add startup, transfer and output. Slower machines have less headroom.
- **Recommendation**: R4 should run additional measurements in the child-process setup (already scheduled in #186), especially inputs close to 20 MiB and slower machines.

## panic

- **How it is used**: the reader catches hayro's panics with `catch_unwind`, and stops hayro with `resume_unwind` when the glyph or draw limit is reached. Both require `panic = "unwind"`, which is the workspace's current default.
- **If changed to `panic = "abort"`**: these cases become child process crashes. Under §14.3 they still fall back to manual pick, but the reader's own explanation text is lost.
- **What cannot be caught**: stack overflow and abort cannot be caught anyway; only the child process can back them up.

## Tests

- **Tests in the core**: `src/pdf_tests.rs`, with 4 normal synthetic samples in `tests/fixtures/pdf/` (three pages of English, two pages of Chinese, images only, encrypted).
  - Each file's summary and body text are exactly the same as in the reviewed experiment;
  - The statuses are complete, complete, unsupported and unsupported respectively;
  - The same bytes always give the same result.
- **What stays in the experiment** (on PR #148's branch, not on main):
  - The experiment's own tests: contract, byte truncation and mutation, defect reproduction, range model, linear scan;
  - The experiment's other samples: malformed, boundary, stress;
  - All of R4's re-verification is in `experiments/pdf-reader`. Whether #148 is merged, and in what form it is kept, is the student's decision.

## Shared dependency change (A has decided to accept it)

**flate2 gains the `zlib-rs` feature, and the decompression backend of the whole workspace changes from miniz_oxide to zlib-rs.**

- **Reason**: hayro-interpret 0.8.0 unconditionally turns on hayro-syntax's `unsafe` feature, and this feature brings in `flate2 = { features = ["zlib-rs"] }`. Cargo features are only additive, so the core cannot turn it off; when flate2 has both zlib-rs and rust_backend, it uses zlib-rs.
- **Scope of impact**: all zip reading/writing and direct decompression in the core, that is DOCX, PPTX, XLSX, and the App itself. The PDF reader already ran on zlib-rs in the experiment, so the PDF part matches what R4 tested.
- **A's judgment** (A owns shared dependencies):
  - The Office readers' compression ratio check uses the sizes in the file headers;
  - The output of deflate decompression is deterministic by the standard;
  - On corrupt compressed data, the two backends may report the error at different positions, but the core code always treats it as a failure and does not look at the error content.

  A will record this in the contract, and at final verification will ask R to run a regression with real Word, PPT and Excel files.
- **Test adjustment**: `repetitive_long_body_is_unsupported_not_partial_truncation` in `docx_tests.rs` used to generate the archive on the fly at the default compression level, and under zlib-rs it did not reach the 200:1 compression ratio the test assumes. Now this test writes the archive itself with compression level 9, and it passes under both backends; neither the assertions nor the read logic changed. A will notify D on the PR.
- **Independent verification crates**: `tests/docx_reader`, `tests/pptx_reader` and `tests/xlsx_reader` all reference traeva-core as a path dependency, and their Cargo.lock files also gained the PDF reader's packages.
- **`cargo tree -p traeva-core -e features -i flate2`**:
  - On main (`7ea9397`), flate2 is turned on only by `rust_backend` (miniz_oxide);
  - On this branch, there is also `zlib-rs`, turned on by hayro-syntax, and the same holds in the App's (traeva-app) dependency tree.

## Dependencies and licenses

- **New dependencies**: `hayro-interpret`, `hayro-syntax` (`=0.8.0`), `kurbo` (`=0.13.1`), and the 21 packages they bring in, 24 in total; versions and checksums are the same as locked in the experiment.
- **Packages the core already has**: `flate2`, `serde` and `memchr` use the core's existing versions, the same as the experiment. Three other packages also use the core's existing versions, which are older than those locked in the experiment:

  | Package | Core | Experiment | Effect on the PDF reader |
  |---|---|---|---|
  | `libc` | 0.2.189 | 0.2.190 | Not in the PDF reader's dependency tree |
  | `smallvec` | 1.16.2 | 1.16.3 | The only change is in one branch of `try_grow`, which hayro's and kurbo's usage never reaches |
  | `syn` | 3.0.6 | 3.0.8 | Used only at build time for proc-macros |

- **License notices**: see `docs/third-party-notices.md`.
  - The PDFium fonts and Adobe CMaps that hayro compiles into the binary by default are BSD-3 licensed;
  - The newly added brotli family is also BSD-3;
  - All these notices must be included when distributing.

## Future maintenance

- After merging, the core's `pdf*.rs` is authoritative. Future fixes change the core first.
- The experiment directory (PR #148's branch) keeps `pdf-experiment-9`/`-10` and the evidence R4 used, and no longer evolves on its own. If R4 is still to re-verify in the experiment, both sides must be changed together, or the experiment must be changed to compile the core's source (following F's XLSX approach). A and the student decide.

## What is not verified

- R4's LIMITED PASS is limited to the scope it tested:
  - The full stress family, residual paths inside the library, N4, and real-corpus compatibility (including the false-rejection rate of the 64-candidate-value limit) are all not covered by acceptance;
  - GUI and real macOS exports are not covered by acceptance either.
- The maximum memory and time within the limits are not established; the time for a 20 MiB input was not measured.
- This core copy of the code has no independent re-verification by R yet.
- The regression of DOCX, PPTX and XLSX under zlib-rs with real files is done by R at final verification.
