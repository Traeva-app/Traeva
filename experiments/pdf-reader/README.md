# TRAEVA PDF Reader experiment (Claude-G, Refs #28)

The **restricted, isolated PDF Reader experiment** that the student approved on 2026-10-11 ([#28 record](https://github.com/P30g53/Traeva/issues/28#issuecomment-6115168054); the rules were synced on main by A). It is not CAC Required, does not block #26/#37, and does not approve formal Memory, Organize, Tauri or model integration.

- Separate crate: it has its own `[workspace]` and `Cargo.lock`, is not in the main workspace, adds no dependency to core, and does not change shared files, DTOs/schema, migrations, registration or UI.
- The Reader only accepts fixed bytes in memory: it does not open paths, write files, use the network, run JavaScript, open attachments or decompress to disk. Files are read only in the test tool `pdf-probe`.
- For dependency selection, licensing and deployment see [RESEARCH.md](RESEARCH.md); for failures and corrections during development see [evidence/dev-log.md](evidence/dev-log.md).
- All results here are **developer self-tests**, not an independent Codex-R4 PASS.
- For the run reports `run-15` … `run-21`, the committed files are copies with the local account information removed (#37 privacy scan); all other bytes are unchanged; see [EVIDENCE-PRIVACY.md](EVIDENCE-PRIVACY.md).
- **Current version `pdf-experiment-10`**: `pdf-experiment-9` (fixes the defects Codex-R4 confirmed over four rounds and the N1–N3 items the student included; see "Round 3", "Round 5", "Round 6", "Round 7", PR #148) plus the quality diagnostics (see "Round 4"). The text is still in drawing order and identical character for character to `pdf-experiment-9` (see "Round 7 → Sync to quality diagnostics"). `pdf-experiment-8` is 7 plus quality diagnostics, `pdf-experiment-6` is 5 plus quality diagnostics, and `pdf-experiment-4` is the quality-diagnostics version from before the fixes were added; `pdf-experiment-2` is a layout experiment on another branch (PR #149) and is not merged here.

## Running

In this directory (`experiments/pdf-reader/`):

```sh
python3 fixtures/generate.py          # generate program-synthesized fixtures into fixtures/generated/ (git-ignored)
python3 fixtures/defects/generate.py  # reproduction samples of R4's defects into fixtures/defects/generated/ (git-ignored)
python3 fixtures/defects2/generate.py # reproduction samples of R4's round-2 defects into fixtures/defects2/generated/ (git-ignored)
python3 fixtures/defects3/generate.py # reproduction samples of R4's round 3 (PDF-08) into fixtures/defects3/generated/ (git-ignored)
python3 fixtures/stress/generate.py   # PDF-09 stress inputs into fixtures/stress/generated/ (git-ignored)
python3 fixtures/layout/generate.py   # layout samples (input only, from #149) into fixtures/layout/generated/
python3 fixtures/quality/generate.py  # quality diagnostic samples into fixtures/quality/generated/
cargo test --locked                   # unit tests + contract drift + truncation/mutation/repeated reads + three rounds of defect regressions + preflight old/new comparison and linearity + quality rules
python3 validate.py [--base OLD_PROBE] # all checks + 158 fixtures, writes evidence/runs/run-NN-<commit>.json
python3 stress.py                     # frozen scan-time protocol, writes evidence/stress/stress-NN-<commit>.json
python3 evidence/round6/repro_run.py  # N1/N2 reproduction, isolated child process, writes evidence/round6/n1-n2-NN-<commit>.json
cargo run --release --locked --bin pdf-probe -- --text fixtures/exported/e01-libreoffice.pdf
cargo run --release --locked --bin pdf-probe -- --isolated --timeout 20 --memory-mib 512 FILE...
python3 audit_dependencies.py --rustsec <advisory-db clone directory>
```

Requires Rust 1.98 and Python 3 (reportlab and pypdf are used only to generate fixtures). `fixtures/export.py` uses headless LibreOffice and Chromium to export real PDFs; the outputs are committed and usually do not need to be re-run.

**Adversarial samples (`a*`, `c*`) are given only to the offline reader; do not open them in a desktop viewer or on the GitHub web page.** They are not committed and are only generated locally; `fixtures/manifest.json` records each file's SHA-256, and after regeneration `generate.py --check` verifies them (the pypdf-encrypted a18/a19 get a different random salt each time, so their hashes are not compared).

## Interface

```rust
pub fn read_pdf(bytes: &[u8]) -> PdfReading;
pub struct PdfReading { pub extraction: TextExtraction, pub pages: Vec<PageReading>, pub quality: Quality }
pub const PDF_READER: &str = "pdf-experiment-10";
```

- `TextExtraction` / `TextSummary` / `TextStatus` / `MAX_MEMORY_TEXT_CHARS` have exactly the same fields as core's shared contract (shared-contract §4.4). Because traeva-core cannot compile on Linux and the experiment cannot add dependencies to core, a copy is kept here (`src/contract.rs`); `tests/contract.rs` reads the core source directly and fails as soon as the definitions change. At formal integration, delete the copy and use core's types instead.
- `pages` (each page's status, its exact character range in text, and its omissions) and `quality` (the quality diagnostics, see "Round 4") belong to this experiment; the shared contract has no corresponding fields.
- The same bytes always give the same result (checked by repeated reads in `tests/robustness.rs`). The verified bytes returned by `read_snapshot` can be fed in directly; the Snapshot API itself cannot run on Linux (see "Untested").

## Output rules (how to "read" the result)

1. **Pages**: each page first gets a line `[Page N]` (N is the physical page order in the file, starting at 1, not the printed page number), then that page's lines. Page boundaries are defined by `pages[].start/end` (Unicode scalar offsets); if the text itself already contains a line `[Page 3]`, looking only at text would be confusing. This marker syntax is experiment syntax, not a shared-interface decision; changing it requires bumping `pdf-experiment-N`.
2. **Order = the PDF's drawing order**; there is no reordering by coordinates.
   - Two columns drawn left column first, then right column (LibreOffice, Word, LaTeX and Chromium usually do this) come out correct.
   - A PDF that draws each line across both columns comes out alternating left and right, with a TAB in the middle (page 2 of p05 verifies this limitation specifically). From round 4 on, such pages get an `interleaved_columns` warning; the order is unchanged.
   - Things drawn later come later; for example, Chromium draws a rotated inline block after the whole line of text (e02).
   - This is only "stored text"; it does not mean the layout or reading order is understood.
3. **Lines**: adjacent drawn glyphs count as the same line if their directions differ by no more than about 10°, the baseline offset is no more than 0.6 of the larger font size, and there is no backward jump of more than 0.6 of a font size; otherwise a new line starts. A gap within a line larger than 2 font sizes is written as a TAB (any gap counts, including one between two space glyphs); a space glyph or a gap larger than 0.15 of a font size is written as one space (between two CJK characters it must be larger than 0.5). Spaces are not repeated, and trailing whitespace is trimmed. Hyphenated word breaks are not joined; paragraphs, headings, headers and footers are not recognized; no blank lines are output.
4. **Characters**:
   - The ligatures ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ are split into letters; all whitespace is written as a space; control characters are removed.
   - Glyphs with no Unicode mapping, mapped to U+FFFD, or in the private use area do not enter text; they are counted as unmapped and disclosed.
   - The same string drawn repeatedly at the same position (fill plus stroke, "fake bold") is kept only once.
   - Glyphs drawn outside the visible page (with 1 extra font size allowed on each side) or at zero font size do not enter text; they are counted and disclosed.
5. **Rotation**: first straighten according to the page's `/Rotate`, then look at the line direction; lines deviating from horizontal by more than 5° become separate lines, and `covered` says "rotated text on page N".
6. **Invisible text** (render mode 3, common in the OCR layer on scans) is kept in text, and `covered` notes "accuracy not checked". Annotations (including the appearance of form controls) are not drawn and are disclosed separately.

## Status rules

| status | condition |
|---|---|
| Complete | At least one page has non-whitespace text, and no omission of any kind was found |
| Partial | There is text, and `left_out` lists the omissions actually found (page numbers merged by kind); or the text was truncated beyond 1,000,000 characters |
| Unsupported | Encrypted (including permissions-only files that open with an empty password); over any resource limit; unsupported filters (LZW, unknown, filter lists given indirectly, image codecs used on non-images); **no page has usable text** (scans/images only, unmapped glyphs, blank pages) — never returns Complete with empty text |
| Failed | Not a PDF / no header; the file is truncated (no `%%EOF` or `startxref`); `startxref` points to the wrong place; corrupted compressed data (Flate missing the zlib header, or a missing or wrong Adler-32, also counts); a page's `/Contents` points to a nonexistent object, to an object that is neither null nor a content stream, or contains members that are not content streams (a `/Contents` that is null, or that references an object whose value is null, is a blank page); the page count declared by the page tree does not match the actual count; hayro cannot read the structure; an internal hayro panic |

For Unsupported/Failed, text is empty, `covered`/`left_out` are empty, and `note` explains the reason (without file content).

## Omission disclosure (`left_out`)

Only what was **actually found** in these bytes is listed; detection is heuristic:

| Kind | How it is judged | Key points of the wording |
|---|---|---|
| Scanned/image page | The page has no text, and images cover ≥ 50% (when several overlap, their areas are added, which overestimates) | Needs OCR, which this reader does not do |
| Images | The page draws an image (of any size) | Image content is not read; no OCR |
| Unmapped characters | Glyphs without a reliable Unicode value | States the count |
| Undecodable fonts | The page has text-showing instructions but not a single glyph came out, or hayro reports an unsupported font | — |
| Table/chart grid | At least 3 horizontal lines and 3 vertical lines at different positions (length ≥ 10 pt) | Text is kept in drawing order; the row and column structure is not kept |
| Vector graphics | ≥ 10 paths with curves/diagonal lines on a page with text; any one on a page without text | Graphics are not read; text inside them is kept |
| Math formulas | ≥ 2 math operators/math letters on the same page, or a TeX/OpenType math font is used | Symbols are kept; the typesetting (fractions, superscripts and subscripts, roots) is not |
| Annotations | Anything in `/Annots` other than Link, Popup and Widget (comments, highlights, stamps, attachments, etc.) | Not read |
| Form fields | Widget annotations; or `/AcroForm` has fields but no Widget; XFA | Values are not read |
| Off-page text | See output rule 4 | States the count |
| Document level | JavaScript (the `/JS` key, `/S /JavaScript`, the `/JavaScript` name tree) and embedded files (`/EmbeddedFiles`, `/EF`, `/Type /EmbeddedFile`): all looked up in the dictionaries hayro parsed, with name escapes decoded; same-named words in text, strings or comments do not count; optional content layers hidden by default; streams in external files | Not run, not opened, not read |
| Truncation | Over 1,000,000 characters | "the last N characters … (from page P on)"; later pages are `not_read` |

**Not detectable** (recorded honestly): tables without drawn lines; formulas expressed with ordinary letters or images; charts inside images (counted only as images); text in render mode 7 (clipping only); garbled text caused by wrong ToUnicode mappings (cannot be found without OCR); "hidden" text that is white, covered by images, or tiny; vertical CJK text (untested; may come out one character per line).

## Safety boundary and resource limits

| Limit | Value | Where it is enforced |
|---|---:|---|
| Input size | 32 MiB | Preflight (before hayro) |
| Declared object count (`/Size`) / actual object count | 200,000 | Preflight / after opening |
| Stream count | 50,000 | Preflight |
| Each non-image stream (with or without filters) | 16 MiB | Preflight. Filtered streams are read to the end of the file with decoders that have their own caps (Flate, ASCIIHex, ASCII85, RunLength), so a fake `endstream` or `/Length` cannot hide data (a17); for unfiltered streams the length is computed by hayro's data-range rule (round 6). After hayro opens the file, the data length of every stream it reads must be one of the lengths preflight computed |
| Each image stream (before the image codec) | 64 MiB | Preflight |
| Predictor row buffer (Flate, LZW) | Same as that stream's cap | Preflight, computed from `/Columns`, `/Colors` and `/BitsPerComponent`; cannot be checked when the parameters are stored elsewhere |
| Total of all non-image streams / total decode work | 128 MiB / 256 MiB | Preflight; decode work includes the bytes each filter reads in and writes out |
| The offset table of one object stream (hayro rereads it for every member it reads) | 64 KiB | Preflight; every stream with `/N` and `/First` counts |
| Number of member reads one full traversal can do × the largest offset table | 512 MiB | Preflight (before hayro opens the file) |
| Distinct candidate values of one length object (indirect `/Length`) | 64 | Preflight; when exceeded, the file's own streams and the containers decoded at open time stop, and other candidates record a refusal (round 7) |
| Filter chain length | 4 | Preflight |
| Dictionary/array nesting | 64 | Preflight (hayro itself: objects 64, form/pattern/Type3 nesting 50) |
| Page count | 500 | After opening, before drawing any page |
| Total content streams per page | 32 MiB | Before drawing the page, using the sizes preflight measured; content streams that preflight did not measure are refused outright |
| Page content cached by hayro | Reopen the document after 64 MiB | While drawing pages |
| Drawn glyphs / drawing events | 2,000,000 / 1,000,000 | While drawing pages; on reaching the limit, hayro is aborted with `resume_unwind` |
| Output text | 1,000,000 Unicode scalars (shared limit) | Summary |
| Encryption | Always Unsupported | Preflight follows `startxref`, `/Prev` and `/XRefStm` to read the trailers and xref stream dictionaries (escaped forms such as `/Encr#79pt` count too, a20, d23), and does not search the whole text for the word; then hayro opens the file with a password no file would use, and whenever hayro treats the file as encrypted (including when it finds the encryption dictionary itself while rebuilding the xref), it cannot open it, and the result is also Unsupported |

**These are application-level soft limits; they do not mean resource safety is complete:**

- hayro's other internal allocations (font parsing, CMaps, xref tables, which grow roughly with input size) are not directly constrained by the limits above.
- There is no in-process wall-clock time limit (to keep "same bytes, same result", only deterministic work limits are used).
- Preflight's total work grows linearly with file size (round 6 fixed the quadratic scanning that had existed since round 3; the literal-string query part is strictly O(n log n); round 7 removed two places where a shared value was copied for each candidate or reread in full: `/Filter` lists and the candidate values of an indirect `/Length`), without relying on a budget; the tables preflight builds once and the shared results it records take about 1–3 times the file size in memory (measured on an 8 MiB stress input: about 18 MiB extra). hayro's own super-linear or unconstrained paths are listed in "Round 6 → Residual risks"; **there is no claim that `read_pdf` is fully bounded**.
- When nested forms are decoded, each level is at most 16 MiB, and at most 50 levels exist at the same time.
- Preflight's own temporary memory is about 2 times a single limit (a03 measured peak: 64 MiB).
- A stream marked as an image but referenced as a font makes hayro run the image codec when it loads the font; preflight cannot block this.
- hayro panics can be caught only with `panic = "unwind"`; stack overflows and aborts cannot be caught.

**Hard limits rely on a separate process.** `pdf-probe --isolated` reads in a child process: heap memory is capped by an allocator counter (macOS does not enforce `RLIMIT_AS`, so it is not used), and the parent process kills a child that runs past the wall-clock time. Measured: a 16 MiB heap cap stops p18 (`memory_limit`), and a 0-second time limit stops p16 (`time_limit`). Whether formal integration uses a separate process is decided by A.

Measured (Linux x86_64, release, in-process, development machine, not a performance promise): ordinary fixtures ≤ 8 ms with peak heap ≤ 2 MiB; p16 (120 pages, 1.08 million glyphs) 0.49 s, 5.1 MiB; work-limit cases (p17, p18, a09) ≤ 1.4 s; the various compression bombs ≤ 0.1 s in preflight, with a peak of ≤ 64 MiB.

## Round 3: R4 defect fixes (`pdf-experiment-3`)

Codex-R4 rebuilt `07ce381` natively on macOS and reviewed it; verdict FAIL: the original 51 samples passed, and the 27 items R4 added independently were 12 PASS / 15 FAIL, grouped into 5 classes. **G did not obtain R4's original inputs**; the 24 samples in `fixtures/defects/` are reproductions G built from R4's description: expectations frozen first, failures produced on the unfixed code, then the fix.

| Class | R4's description | Fix | Reproduction samples |
|---|---|---|---|
| 1 P1 | Stream location did not skip strings and comments: with a nested Form plus a legal `/Memo (>>)`, or a comment between the dictionary and `stream`, 17 MiB of content bypassed the 16 MiB cap and returned Complete | Read the dictionary from the object header (`12 0 obj`) onward using PDF syntax: strings, hex strings, comments and nested dictionaries are all skipped as wholes. After hayro opens the file, every stream it can parse must already have been measured by preflight, otherwise Unsupported | d01–d05 |
| 2 P2 | A raw byte search across the whole file treated `/Encrypt`, `/Size`, `/JS` and `/EmbeddedFiles` in text or comments as structural fields | `/Encrypt` and `/Size` are read only from the trailers and xref stream dictionaries on the cross-reference chain; JavaScript and attachments are looked up only in the dictionaries hayro parsed | d06–d08 |
| 3 P2 | Legal escapes such as `/Java#53cript`, `/J#53`, `/Embedded#46iles` let real scripts and attachments go unreported | Same as above: parsed names are already decoded | d09–d12 |
| 4 P2 | Nonexistent references or string members in a `/Contents` array were silently skipped, still returning Complete | Check the raw members of `/Contents` one by one: each must be a reference to a content stream; otherwise the whole file is Failed | d13–d16 |
| 5 P2 | Flate did not check the full zlib wrapper and the Adler-32 | A zlib header (without a preset dictionary) and a matching Adler-32 are required, otherwise the data is treated as corrupt; corrupt data is still measured in both reading modes, so the caps stay in effect. Images are still only size-checked (image content is not read anyway) | d17–d21 |

In addition, d22–d24 test cross-reference streams, object streams and `/Prev` incremental updates (round 1's samples all used traditional xref tables; there were no samples for reading these structures before).

| Step | commit | Result |
|---|---|---|
| Reproduction samples and frozen expectations committed **first** | `8b28131` | The code is still that of `07ce381` |
| run 05 (unfixed) | `8b28131` | 57 PASS / 18 FAIL; all original 51 passed; all 17 samples designed as defects failed as R4 described, and d04 also failed (the false-refusal form of the same location defect; its "control" label was wrong) |
| run 06 (fixed) | `acd0f8c` | 75/75; `cargo test` (debug) FAIL: a u8 overflow inside a new unit test |
| run 07 | `e647446` | 75/75; clippy FAIL: the `% 31 == 0` written while fixing the test |
| run 08 | `78e747a` | **75/75**; fmt, clippy, all tests, hashes and both isolation limits PASS |

- Compared one by one with the round-1 binary (status, text, left_out, note): 85 of 103 files are fully identical. These 103 include the 51 original samples, 28 layout samples (including 3 real exports) and 24 reproduction samples. The 18 that changed are exactly the 18 that failed in run 05.
- Time and peak heap are essentially unchanged: p16 479 → 501 ms, a09 1,340 → 1,388 ms, peak heap fully identical.
- **Behavior changes to know about**:
  - "Raw deflate" Flate streams without a zlib header (written by a few old generators) now count as corrupt; in a non-image stream they make the whole file Failed.
  - A `/Contents` that references a missing object is also Failed.

## Round 4: quality diagnostics (`pdf-experiment-4`)

The student's decision on 10/12: CAC does not enable #149's automatic reordering, and PDF output stays in drawing order; after the base fixes, add lightweight quality diagnostics and explicit hints.

- `PdfReading` gains `quality`. Text, status, `covered`, `left_out`, note and the page table are all the same as in `pdf-experiment-3`: run 10 compared them one by one against a binary built from `e2d836f` (run 11 did the same comparison; results in the PR description), and all 107 files matched.
- No model calls, no OCR, no layout recovery. The diagnostics do not change the text order, do not touch page offsets, and do not relax any resource limit.

### Report structure

```json
{
  "reader": "pdf-experiment-4",
  "input_sha256": "<SHA-256 of the bytes read>",
  "text_available": true,
  "reading_order": "drawing_order_unverified",
  "diagnostics": [{ "level": "...", "code": "...", "pages": [2], "message": "...", "basis": "..." }],
  "limitations": ["..."]
}
```

- **Binding**: `reader` and `input_sha256` bind the diagnostics to the Reader version and the exact bytes (Snapshot).
  - `pages` is the physical page order, consistent with `pages[].page` and with `[Page N]` in the text.
  - An empty `pages` means the whole file.
- **`reading_order` is always `drawing_order_unverified`**: no diagnostic claims that the reading order has been checked, and "no warning" does not count as checked either.
- **`limitations`**: capability limits that hold for every file, kept separate from the file-specific `diagnostics`:
  - the reading order is not checked;
  - having all the characters does not mean the order is correct;
  - images are not read; there is no OCR;
  - table rows and columns are not recovered;
  - column detection is heuristic.

### Three levels

| level | Meaning | Source |
|---|---|---|
| `confirmed_omission` | Something that really is in the file but did not enter the text | Exactly each entry of `left_out`, with the same page numbers, no more and no less; normal files have none |
| `suspected_risk` | The text is there, but measured signs suggest there may be a problem | `interleaved_columns` (suspected interleaving of multiple columns), `invisible_text` (an invisible text layer, often the OCR layer on a scan) |
| `stopped` | Why there is no text | Exactly one entry for Unsupported/Failed, with a reason code |

- Codes for confirmed omissions: `image_page`, `images`, `unmapped_characters`, `undecoded_fonts`, `table_grid`, `drawings`, `math_layout`, `annotations`, `form_fields`, `xfa_form`, `javascript`, `embedded_files`, `hidden_layers`, `external_content`, `outside_page`, `truncated_text`.
- Codes for stop reasons: `not_a_pdf`, `truncated`, `damaged`, `encrypted`, `resource_limit`, `unsupported_feature`, `unsupported_structure`, `internal_error`, `no_text_layer`, `no_unicode`, `undecodable_fonts`, `no_text`.
- Every diagnostic has a `message` (an English hint for people) and a `basis` (the detection basis: what was measured, including counts and page numbers). For stop entries, the basis is the original note.

Examples (actual output of run 10):

| Situation | Sample | Status | Diagnostics |
|---|---|---|---|
| Normal text | p01 | Complete | None: `diagnostics` is empty, and `reading_order` is still unverified |
| Suspected interleaved columns | p05 | Complete | `suspected_risk interleaved_columns [2]`: “Text was extracted, but these pages may have columns read across, so lines from different columns may alternate. The order was kept as drawn.” Basis: “page 2: on 4 rows, text on both sides of the same gap (x 168 to 330 pt) is drawn one side right after the other” |
| Table | p10 | Partial | `confirmed_omission table_grid [1]`: “Table text was extracted in drawing order. Its rows and columns were not recovered, so check which values belong together.” |
| Mixed text and images | p09 | Partial | `image_page [2]` and `images [1]`: “Images were not read and there is no OCR: any text or meaning in them is not in the text.” |
| Scan | p08 | Unsupported | `stopped no_text_layer [1, 2]`: “The file has no usable text layer, and OCR is not supported.” |
| Unmapped font | q01 | Unsupported | `stopped no_unicode` |
| Encrypted, damaged, over a limit | a18–a20, c01–c08, a01, etc. | Unsupported/Failed | `stopped encrypted`, `damaged`, `truncated`, `resource_limit`, etc. |

"Suspected columns" is only a warning: it does not clear the text, change the status or refuse the file. p05 is still Complete.

### Basis for detecting interleaved columns

Only the parsed glyph coordinates and the drawing order are used; there is no keyword search.

1. Only upright lines are considered; rotated lines do not count.
2. **Fragments**: within the same line, a break is made between two **different text operations** (`Tj`, `TJ`, `'`, `"`) when the gap is ≥ 0.95 font sizes. Large gaps inside the same operation (justified `Tw` or `TJ` spacing) do not cause a break.
3. **Observation**: two pieces of text drawn one right after the other on the same baseline have a gap of ≥ 0.95 font sizes between them, and the fragments on both sides are each ≥ 6 font sizes wide. Both of these situations count:
   - within the same line (a sentence from the left column followed by a sentence from the right column, merged into one line);
   - two lines drawn next to each other, with baselines differing by ≤ 0.5 font sizes and no horizontal overlap.
4. When the page has a ruled grid (≥ 3 horizontal lines and ≥ 3 vertical lines), gaps crossed by a vertical line are cell borders and do not count.
5. A page is flagged only when ≥ 3 different lines on it have such a gap at the same position (a vertical strip 0.95 median font sizes wide). The basis states the number of lines and the x range of the gap.

### False positives and false negatives (frozen before the run)

All were written in advance in `NOTES` in `fixtures/quality/expectations.py`, and the run results match them exactly.

- **Known false positives** (a warning even though the text was read correctly):
  - q03: an unruled table whose long cells are drawn line by line.
  - q04: form-style "label: value" pairs.
  - Three or more lines drawn line by line with long text on both sides cannot be told apart, by coordinates, from reading across two columns.
- **Known false negatives**:
  - l05: both columns drawn bottom-up, one column finished before the other; no line is split.
  - l06 (each word drawn separately and in reverse order) and l25 (justified, one operation per word): the fragments near the column gap are all single words, less than 6 font sizes wide.
  - l19: only two lines.
  - l14: an unruled table drawn by column; the rows and columns are not recovered, and there is no hint either.
  - l16 page 1: labels drawn by column, read as two strings.
  - A whole line drawn across two columns in one text operation with large `TJ` offsets: there is no break inside the same operation, so it cannot be found (no sample).
- **Out of scope**: l02 is out of order within a single column; that is not a column risk and is not reported.
- **Controls** (should not be reported, and were not):
  - p05 page 1, l03 (two columns drawn by column);
  - x01–x03 (real two- and three-column exports from LibreOffice and Chromium, drawn by column);
  - l15: wide gaps, tab stops and label lines in a single column; only one line has long fragments;
  - l12, l13, l23: ruled tables with short cells;
  - q02: the same long cells as q03, but with grid lines;
  - the l22 numbered list, p10, p15 and the other ordinary files.

**No warning does not mean the reading order is correct**: l05, l06 and l25 have no warning, yet their order is wrong.

### Verification

| Step | commit | Result |
|---|---|---|
| Quality samples, rules and expectations committed **first** (including the known false negatives and false positives) | `2343231` | Diagnostics not yet implemented |
| Implementation | `93148af` | — |
| run 09 (first run) | `93148af` | 106/107. a16 FAIL: pdf-probe gave the reader only the first 32 MiB + 1 byte of the file, and the `input_sha256` in the report was of that prefix, not of the file |
| probe fix | `be8edb6` | The probe gives the reader whole files of ≤ 128 MiB (the reader still refuses files > 32 MiB); larger files are not read |
| run 10 | `be8edb6` | 107/107. fmt, clippy, all tests, hashes, the baseline comparison and both isolation limits passed. The file name contains dirty only because run 09's evidence file had not been committed yet at the time |
| Basis wording fix and docs | see the PR description | Only the English wording of basis changed (counts moved after the colon); for run 11 see `evidence/runs/` |

- **107 samples**: 51 from round 1, 24 R4 reproductions, 4 quality samples, 25 layout samples, 3 real exports. The layout samples are used only as input; their reading-order expectations belong to #149 and are not used here.
- **Baseline comparison**: `--base` uses pdf-experiment-3 built from `e2d836f`. For all 107 files, the status, character count, `covered`, `left_out`, note, text and page table (page, status, start and end offsets) are all identical.
- **Time and memory** are essentially unchanged: p16 502 → 496 ms, a09 1,336 → 1,351 ms. p16's peak heap rose from 5.3 MiB to 6.7 MiB (fragment coordinates are stored for each line); the rest are identical or nearly identical.
- `tests/quality.rs` checks the same set of rules in debug tests.

### Information prepared for A/B wiring (not implemented)

- This round delivers diagnostics and status only inside the experiment. There is no change to shared DTOs/schema, Memory, Tauri or the production UI. This is not a Go for formal PDF integration and does not authorize real model calls.
- The actual destination is confirmed by A/B. Binding keys that can be used directly:
  - `quality.reader`, equal to `summary.reader`;
  - `quality.input_sha256`, equal to the SHA-256 of the Snapshot bytes;
  - `diagnostics[].pages`, the physical page order, corresponding to `pages[].start/end`.
- Recommended use:
  - `stopped` maps to the status explanation;
  - `confirmed_omission` maps one to one to `left_out`;
  - `suspected_risk` is only a hint and should not block saving or analysis;
  - `limitations` is fixed text, not a finding about this file.

## Round 5: R4 round-2 defect fixes (`pdf-experiment-5`)

R4 re-verified #148 `e2d836f` and #158 `7f79cd8` on macOS:
- All of the original 27 items passed: the original 15 failures are fixed, and the 12 controls did not regress; the quality-diagnostics special check also passed.
- Overall still FAIL, with 2 P2 items confirmed.

Notes:
- R4's original inputs and logs were not uploaded. The 24 samples in `fixtures/defects2/` were built by G from the description in R4's comment: expectations frozen first, failures produced on the unfixed code (run 12), then the fix.
- s15 in R4's report is a problem with R4's own test expectation; as the student asked, the run logic is not changed for it.

| ID | R4's description | Fix | Samples |
|---|---|---|---|
| R4-PDF-06 | Object headers were searched for in the raw bytes of the whole file, so an example `8 0 obj << ... >> stream` in text or comments was treated as a stream: with `/Filter /Missing` it returned Unsupported, and with `/FlateDecode` plus `stream abc` it returned Failed | See below | f01–f14, f24 |
| R4-PDF-07 | When `/Contents` points to an object that really exists and whose value is null, the whole file was Failed, and even the text of normal pages was cleared | A reference that resolves to a legal null is treated as a blank page with no content. References to nonexistent objects, references to other types, and array members that are not streams (including null) are still Failed | f15–f23 |

### How R4-PDF-06 was fixed

In four layers; no resource limit was relaxed.

1. **All candidates are still measured.**
   - The dictionary after every `obj` keyword is a candidate (whitespace and comments may come in between), wherever it is in the file, and all of them are measured with the capped decoders.
   - The reason: when the xref is broken, hayro scans the whole file to rebuild it itself, and its scan does not skip strings and stream data. So the rule "the candidates cover everything hayro might read" must stay.
   - The size limits still take effect immediately for all candidates: 16 MiB per stream, the 128/256 MiB totals, the stream count (f14).
2. **Tell which objects are the file's own.**
   - Starting from the file header, read one object after another using PDF syntax. Dictionaries, arrays, strings, hex strings and comments are skipped as wholes.
   - Stream data is skipped by `/Length`; `/Length` may be a direct value or the integer value of an indirect object. The value is used only when exactly `endstream` comes after `/Length`; otherwise it falls back to the first `endstream`.
   - Where there is no complete object, reading continues token by token, so object headers met there count as the file's own: for damaged parts, it is better to check more.
   - **The file's own streams**: all findings still take effect before hayro, including damage, unknown filters, and filters whose size cannot be measured.
   - **Pseudo-objects in strings, comments, hex strings or other stream data**: only subject to the size limits. Other findings are recorded first and lead to a refusal only when hayro actually reads them.
3. **Check by position after hayro opens the file.**
   - For every stream hayro reads, look up the preflight record by the start positions of its dictionary and its data in the file. In unencrypted files both are slices of the original bytes, with byte-exact positions.
   - No record found: Unsupported; an unmeasured stream must not be decoded.
   - A pseudo-object with a recorded finding is found: refuse based on that finding.
   - `objects()` is read twice: if hayro rebuilt the xref during reading, the second pass sees the rebuilt objects.
4. **Object headers allow comments between the numbers and `obj`.** ISO 32000 7.2.4 treats a comment as one whitespace, and hayro reads it the same way. Previously preflight could not find streams written in this legal form and misjudged them as Unsupported (f24).

**Exceptions that are still refused** (frozen as a known limitation, f10):
- A pseudo-object that looks like an object stream (`/Type /ObjStm`) or an xref stream is still refused if it uses a filter hayro can decode but preflight cannot measure (LZW, indirect filter lists, image codecs used on non-images). The reason is that when hayro rebuilds the xref, it decodes object streams before any check.
- When a pseudo object stream uses an unknown filter (which hayro cannot decode either), it is not refused (f09).

### Verification

| Step | commit | Result |
|---|---|---|
| Reproduction samples and frozen expectations committed **first** | `2bbea68` | The code is still that of `e2d836f` |
| run 12 (unfixed) | `2bbea68` | 86/99. The 13 failures are exactly the 12 marked as defects (f01–f09, f16, f17, f22) plus f24; all controls and all original 75 passed |
| run 13 (fixed) | `94a7fb2` | **99/99**; fmt, clippy, all tests, hashes and both isolation limits passed |

- **Compared one by one with v3**: using the binary built from `e2d836f` (pdf-experiment-3), comparing the status, character count, `covered`, `left_out`, note, text and page table.
  - 131 files: the original 75, 24 from the second round, 25 layout samples, 3 real layout exports, 4 quality samples.
  - 118 are fully identical; the 13 that changed are exactly the 13 that failed in run 12.
- **Time**: on the same machine, alternating with v3, 9 runs each, median: p16 510 → 514 ms, a09 1,432 → 1,371 ms, a15 (over 50,000 streams) 10 → 36 ms.
- **Peak heap**: a15 11.5 → 14.2 MiB; the extra is the object table and the `endstream` index; the other samples differ by only a few KB.
- **Coverage of `fixtures/defects2/`**:
  - pseudo-objects appearing in text, file comments, content-stream comments, dictionary strings (with a hex string next to them), image pixel data, and data containing the word `endstream` (direct and indirect `/Length`);
  - controls with real streams: decompression bombs, corrupt data and unknown filters are still refused;
  - fake bombs hidden in data are still subject to the limits;
  - several null cases: direct null, indirect null, null in an object stream, "text page + null blank page";
  - cases that are still Failed: a missing reference, a number, an ordinary dictionary, a null member in an array.

**Behavior changes to know about**:
- Object examples in text, comments, strings or other stream data no longer affect the result.
- When a `/Contents` reference resolves to null, that page is a blank page; if all pages are blank, the existing no-text rule returns Unsupported.
- A null member in a `/Contents` array is still Failed: the rule "null is the same as the entry not being there" applies to dictionary values, not to array members. pypdf skips such members, while poppler outputs nothing.

### Sync to quality diagnostics (PR #158, `pdf-experiment-6`)

- #158 syncs #148's `c3a28cf` with a merge (merge commit `b2ccd01`), with no rebase and no force-push, and the quality-diagnostics feature is kept. The merged version is `pdf-experiment-6`, i.e. 5 plus the quality report.
- Every stop point brought in by the merge got a reason code. The stop reasons for the 24 second-round samples were frozen before the merged version's first run (`6de5c13`, entries added only).
- **run 14** (`6de5c13`): 131/131 passed; fmt, clippy, all tests, hashes and both isolation limits passed.
- **Compared with pdf-experiment-5**: `--base` uses pdf-experiment-5 built from `c3a28cf`; for all 131 files, the text, status, `left_out`, note and page table are all identical.
- **Diagnostics compared with v4**: the diagnostics of the original 107 samples are identical, entry by entry, to run 11 (v4).
- **Time**: on the same machine, alternating, 9 runs each, median: p16 497 → 475 ms, a09 1,465 → 1,487 ms, a15 43 → 61 ms (the extra is the input hash and the report).

## Round 6: R4 round-3 defect fixes (`pdf-experiment-7`)

R4 re-verified #148 `c3a28cf` (v5) and #158 `f47d9b3` (v6) on macOS: R4-PDF-06/07 PASS; overall FAIL, with two P2 resource defects confirmed:

- **R4-PDF-08**: unfiltered non-image streams are not subject to the 16 MiB per-stream cap; preflight only proved where the stream data starts, not where it ends.
- **R4-PDF-09**: preflight's work is quadratic on some legal files.

The scope the student set for this round: PDF-08; PDF-09 (linearize, no budget); minimal bounded fixes for N1, N2 and N3. N4 and content-quality items are deferred. G-PDF's handoff ([HANDOFF-round6.md](HANDOFF-round6.md), `fixtures/defects3/`, `fixtures/stress/`, `evidence/round6/`) is the starting point. This round was developed and self-tested on macOS arm64 (session G4).

### Steps

| Step | commit | Result |
|---|---|---|
| G-PDF's handoff commit (author changed to the student; content and tree unchanged) | `368aa5e` | defects3 reproductions and frozen expectations, the stress protocol, v5 records on Linux |
| run 15 (unfixed, macOS) | `368aa5e` | 111 / 126; the 15 failures = 8 PDF-08 defects + g14–g17 + g25–g27; all original 99 passed |
| stress-02 (unfixed) | `8e29d40` | 34 / 56, 18 time-outs; S2, S3, S4 and S6 take about 4× per doubling |
| n1-n2-01 (unfixed) | `2134bb1` | N1 hit the heap cap in the isolated subprocess (in-process it would abort outright); N2 with 1k / 4k / 16k members: 49 ms / 0.6 s / 9.8 s, 64k over 60 s |
| g14–g17 re-frozen on an independent basis | `d13514c` | Basis: `fixtures/defects3/range_basis.py` / `.json` |
| g25–g27 marked unverified | `01f62a1` | Run and recorded as usual; only the shared contract is checked; not counted as PASS/FAIL |
| Fix (`pdf-experiment-7`) | `dba679a` | run 16: 123 PASS / 0 FAIL / 3 unverified; clippy FAIL (a type spelling in the linearity test; clippy was not re-run before committing) |
| clippy fix; pdf-probe reports `scan_work` | `24bcc63`, `dd70a72` | Reading results unchanged |
| run 17 | `dd70a72` | **123 PASS / 0 FAIL / 3 unverified** (126 in total); fmt, clippy, all tests, release build, both isolation limits, and the hashes of defects/defects2/defects3/exports all passed; `fixtures/generate.py --check` is still recorded as false because of the Linux font path, and passed with the font wrapper |
| stress-03 | `dd70a72` | 52 / 56, no time-outs; for the 7 families, the time ratio of 8 MiB to 1 MiB is 7.8–9.9 (the protocol limit is 16); the 4 failures are S5 at 128 KiB–1 MiB, the same as v5 (see "Pending the student's decision") |
| n1-n2-02 | `dd70a72` | N1 is Unsupported before hayro (12 ms); N2 with 1k / 4k members is read normally in 46 ms / 0.59 s, and 16k / 64k members are refused by the member cap (≤ 25 ms) |
| compare-01 (v5 vs v7, one by one) | `dd70a72` | 115 of 126 files fully identical (status, character count, covered, left_out, note, text hash, full page table, isolation result); the 11 that changed = 8 PDF-08 defects + g25–g27 |
| Multi-perspective review (part of the development process, not R4's independent review) | `dd3b0a4` | 3 read-only review agents each looked at one aspect, and each "must fix" finding was re-checked by another agent: 5 items confirmed, plus a few that looked valid but were not re-checked (dev-log entry 29) |
| Review fixes (still `pdf-experiment-7`; the fixes came before the first push) | `c10d23a` | See "after review" in the items below |
| run 18 | `c10d23a` | **123 PASS / 0 FAIL / 3 unverified**; fmt, clippy, all tests, release build, both isolation limits and the hashes of each sample set all passed (`fixtures/generate.py --check` as in run 17, passed with the font wrapper) |
| stress-04 | `c10d23a` | 52 / 56, no time-outs; for the 7 families, the time ratio of 8 MiB to 1 MiB is 7.8–10.6; the 4 failures are still S5 at 128 KiB–1 MiB |
| n1-n2-03 | `c10d23a` | Same as n1-n2-02: N1 is Unsupported before hayro (12 ms); N2 with 1k / 4k members is read normally in 49 ms / 0.62 s, and 16k / 64k are refused by the 64 KiB offset-table cap |
| compare-02 (v5 vs v7) | `c10d23a` | Same as compare-01: 115 of 126 files fully identical, and the same 11 changed |

### Invariants and approach

**PDF-08: every non-image stream is subject to the 16 MiB per-stream cap, with or without filters; a stream's data range is computed by hayro's own rule and checked again after hayro opens the file.**

- The range rule is taken from the hayro 0.8.0 source (`object/stream.rs` in `hayro-syntax`):
  - `parse_proper`: exactly one newline after `stream`, data `/Length` bytes long, then only whitespace, then `endstream`.
  - Otherwise `parse_fallback`: from the first `stream` after the dictionary up to the first `endstream`, with trailing whitespace removed.
- A direct `/Length` is read with hayro's number grammar (sign, fraction truncated, must fit in a u32).
- An indirect `/Length`: hayro may resolve it to any value it can find, or may not resolve it at all (it does not resolve it when reading cross-reference streams or rebuilding the cross-reference), so all of these lengths count. Integers stored in object streams also count as possible values.
- For each candidate, preflight computes all the lengths hayro might take. Without filters, the largest of them is compared with the cap; if it is over, Unsupported: "A stream's data is larger than this reader's limit of 16 MiB."
- After hayro opens the file, the data length of every stream it reads (including page content) must be one of the lengths preflight computed; otherwise Unsupported, with no text. When the document is reopened because of the cache, this is checked again.
- No new "Failed when `/Length` does not match" rule was added. g14–g16 (`/Length` too short, too long, missing) read text by hayro's fallback rule and are Complete; g17 has no text in the range after the fallback and is Unsupported (no text). The re-freeze basis is in `range_basis.json`: hayro's rule rewritten in Python agrees with pypdf's reading results.

**PDF-09: preflight's total work grows linearly with file size (the literal-string end query part is strictly O(n log n), see below); no budget; all candidates are still measured.**

- The questions preflight needs to ask (where whitespace and comments end, where the next keyword is, where hex strings and literal strings end) are now all answered by looking up tables built once (new module `src/scan.rs`):
  - one table entry per 64 bytes; each query scans at most one block.
  - Literal-string ends come from the nesting depth of unescaped parentheses across the whole file, plus a tree of per-block minimums. When a string is read from any `(`, from the next byte on the reading agrees with this global depth.
- When dictionaries or arrays are read from different starting points and reach the same element in the same state, the rest of the reading is exactly the same. Each read records its result at the first element in each new 64-byte block, and later reads that get there take it directly. This way there is at most one recorded result per block (for the same state).
- If a dictionary read from its start joins someone else's result midway, the keys it needs are read along the chain of "joined reads", each segment only once.
- After review: when many dictionaries merge into the same long dictionary, they get the positions of the same batch of values. Each key records the start and end positions of its value when the dictionary is read; numbers and `/Filter` values are read only once per position; names compare only the bytes needed. This way, a shared long value is not reread by each dictionary.
- Decode work (the existing 256 MiB cap) now also counts the bytes each filter reads in, not only the output.
- The complexity, stated honestly: when a literal string does not end in the current block, one lookup is done in the block-minimum tree, O(log(n/64)) each time. This part is strictly O(n log n); with the 32 MiB input cap, the log factor is no more than 19. Every other query scans at most one 64-byte block.
- Correctness: the old direct reading is kept in the tests (`scan::reference`) and compared with the new implementation at every position of the generated inputs; the results are fully identical.
- Deliberately broken mutations are all caught: key merging, the depth of recorded results, and the result after an over-deep member. The last is an input added after review; the original inputs could not catch it.
- There are two linearity tests:
  - one counts elements read, with inputs constructed for several kinds of "multiple starting points merging" (it slows down and fails when shared results are turned off);
  - one counts elements plus value bytes read, with inputs where many dictionaries share long values (it fails when the number cache is turned off).

**N1: the predictor row buffer is computed from the decode parameters, and anything over the per-stream cap is refused before hayro.**

- hayro allocates one row of `/Columns × /Colors × /BitsPerComponent` before decoding, regardless of how short the data actually is (`filter/lzw_flate.rs`).
- Preflight finds each Flate/LZW filter's parameters the way hayro does: for a single filter, the `/DP` or `/DecodeParms` dictionary; for a filter array, the member at the same position in the `/DP` or `/DecodeParms` array. If the row size exceeds that stream's cap (non-image 16 MiB, image 64 MiB), Unsupported.
- When the parameters are stored elsewhere in the file (an indirect reference), they cannot be checked: the file's own streams, object streams and cross-reference streams are refused outright; pseudo-objects record a finding and are refused only when hayro actually reads them.
- All candidate dictionaries are checked, including those not followed by a `stream` keyword (see N3).

**N2: the cost of handling object stream members is limited by a structural quantity (changed after review to count bytes).**

- Every time hayro reads an object stream member, it rereads that object stream's whole offset table (`ObjectStream::new` in `xref.rs`). So the cost depends on how many bytes the offset table takes (from the start of the decoded data to the end of the N-th pair, including whitespace and comments), not on how many pairs there are.
- hayro recognizes a stream as an object stream by whether it has `/N` and `/First`, not by `/Type`; preflight recognizes it the same way.
- One object stream's offset table is at most 64 KiB (`MAX_OBJSTM_TABLE_BYTES`, about 6,000 pairs in the common form). Preflight measures it by actually reading the offset table once, the way hayro reads it.
- The maximum number of member reads one traversal can do: the number of entries a cross-reference stream can list by `/W`, plus the number of pairs hayro adds from object streams when rebuilding the cross-reference. This count × the bytes of the largest offset table is at most 512 MiB (`MAX_OBJSTM_SCAN`). It is checked before hayro opens the file, because hayro may already traverse all objects while opening (when the page tree is broken).
- On this machine, reading through an object stream of 4,000 pairs (an offset table of about 28 KiB) takes about 0.6 s.
- The earlier formulation "object count × member count ≤ 2^25" had a hole: with few pairs but each very long, the cost of one read could approach 16 MiB. Changed after review.
- hayro's scan when rebuilding the cross-reference (`fallback_xref_map`, which scans to the end of the file on an unclosed literal string): whether it is reachable through this reader and whether it would be slow is **unverified**.

**N3: minimal bounded fix.**

- For containers that hayro decodes as soon as it opens the file (object streams, cross-reference streams), preflight stops outright when it finds a resource problem: the size cannot be measured, it is over the cap, the predictor row is over the cap, or the offset table is over the cap. The same applies to those without a `stream` keyword. The way hayro takes the data follows its `parse_fallback`: the data after the next `stream`, decoded with this dictionary's filters.
- The per-stream cap applies to all candidates preflight can see: a dictionary after any `obj` that names filters hayro would decode is measured on the data hayro would take, even when no `stream` keyword follows.
- hayro skips filter names it does not recognize and runs the remaining filters as usual; preflight now measures the same way. The file's own streams with an unknown filter are still refused. A dictionary that names only filters hayro does not recognize and has no data (for example `/Filter /Adobe.PPKLite` in a signature dictionary) is not decoded by hayro and is not measured.
- After review: hayro gives a stream its role by how the file references it, not by `/Type`:
  - the object at a cross-reference section offset is read by hayro as a cross-reference stream;
  - a stream with `/N` and `/First` is read by hayro as an object stream.
  These two kinds also count as "decoded at open", with these rules:
  - If a comment between the dictionary and the `stream` keyword contains the word `stream` and a newline, hayro's fallback rule starts taking data from there. That range must also be measured.
  - Corrupt data, parameters stored elsewhere, or no filters with an indirect `/Length` whose value cannot be found: all stop outright.
  - A dictionary that cannot be read also stops outright as long as there is a `stream` anywhere after it (previously only the part before `endobj` was considered).
- After review: references are recognized the way hayro reads them (numbers may have a sign or a decimal point; only whitespace is allowed between the parts). Members of a `/DecodeParms` array other than dictionaries, `null` and references make the predictor row uncheckable.
- g25–g27 are the samples built for N3 at the handoff; on v5 they did not reach the target path, so they are marked unverified. On v7 they are Unsupported (per-stream cap), recorded only.

Timing (macOS arm64, release, pdf-probe direct mode, development machine, not a performance promise; time-out = no result by the 5 s cut-off):

| 8 MiB input | v5 (stress-02) | v7 (stress-04) |
|---|---:|---:|
| S1 normal large string | 53 ms | 82 ms |
| S2 candidates in comments (R4's legal family) | time-out | 180 ms |
| S3 unclosed literal string | time-out | 365 ms |
| S4 unclosed hex string | time-out | 329 ms |
| S5 no `endstream` | 24 ms | 47 ms |
| S6 comment lines | time-out | 102 ms |
| S7 nested dictionaries | 4,573 ms | 2,367 ms |

- Peak resident memory (stress inputs, subprocess): v5 about 30 MiB, v7 about 49 MiB; the extra is the tables built once (one entry per 64 bytes).
- The largest peak heap among the samples is still a03's 64 MiB; p16 226 → 224 ms, a09 627 → 608 ms.

### Measured passes and static hardening (recorded separately)

| Item | Measured (has samples and run records) | Static hardening (derived from the hayro source, no reproduction sample) |
|---|---|---|
| PDF-08 | g03, g04, g07–g09, g11, g20, g21 changed from Complete to Unsupported; the controls g01, g02, g05, g06, g10, g12–g16, g23, g24 are unchanged; g17 is unchanged | Refuse when a length hayro reads is not among the lengths preflight computed; check again after the document is reopened |
| PDF-09 | All 7 families of the frozen protocol are linear; the merging test that counts elements; the shared-long-value test | Linearity argument: each table query covers at most one block; at most one result is recorded per block and state; each literal-string query is O(log n) |
| N1 | The 737-byte reproduction sample is Unsupported before hayro (v5 hit the heap cap in the isolated subprocess) | Refuse or record a finding when the parameters are stored indirectly; dictionaries without the keyword are checked too |
| N2 | 1k / 4k members are read normally; 16k / 64k members (offset table over 64 KiB) are refused in preflight | Byte-based offset-table cap; "member reads × offset table" checked before opening; object streams recognized by `/N` and `/First` |
| N3 | g25–g27 are now Unsupported (unverified samples, recorded only) | Object streams/cross-reference streams without the keyword; containers recognized by role; the containers' other fallback range; the part after unknown filter names is measured anyway |

### Residual risks (inside hayro, outside the experiment's control; **no claim that `read_pdf` is fully bounded**)

- **The scan when rebuilding the cross-reference**: when hayro scans the whole file to rebuild it, it scans to the end of the file on an unclosed literal string, which may be quadratic. Whether it is reachable through this reader is unverified.
- **Dictionaries probed during the rebuild**: during the rebuild, hayro probes every `<<` after whitespace (not necessarily after `obj`); `/Type /ObjStm` ones are decoded on the spot. Those not after `obj` are not preflight candidates, so they are not subject to the caps. Not reproduced.
- **Pseudo-streams at draw time**: when a page uses a dictionary as a stream (`Dict::get::<Stream>`), hayro takes the data after the next `stream` and decodes it with this dictionary's filters. Candidates after `obj` whose filters can be measured are now subject to the per-stream cap. Inline dictionaries in dictionary values, filters preflight cannot measure (for example LZW on a pseudo-object), decode parameters stored elsewhere for pseudo-objects, and the total when the same pseudo-stream is decoded repeatedly are not constrained. Not reproduced.
- **Member reads**: at draw time, each object stream member fetched rereads an offset table of at most 64 KiB; the number of member fetches at draw time is not directly controlled by the experiment (the drawing-event cap is only an indirect constraint).
- **Dictionaries in object streams treated as streams**: hayro takes the data from the decoded object stream data by the fallback rule and decodes it with that member dictionary's filters. Preflight does not check inside object streams. Not reproduced.
- **Unfiltered pseudo-streams**: at draw time, when an unfiltered dictionary is read as a stream, its data is no larger than the file size; not measured.
- **Corrupt image data**: after both flate2 reading modes fail, hayro switches to a more lenient decoder (a port of pdf.js) and keeps decoding; the output may exceed the size preflight measured. Images are only size-checked; corrupt data in object streams and cross-reference streams now stops outright.
- **Choice of cross-reference chain** (N4, deferred): hayro uses the last `startxref` in the file, while preflight uses the one before the last `%%EOF`; when the two differ, hayro may read a trailer that preflight never read.
- **How `/Length` is written in early containers**: if a length object is stored in an integer form that hayro can read but preflight does not recognize, preflight cannot get that value. The post-open length check blocks ordinary streams but not containers decoded at open.
- **When cross-reference offsets overlap**, hayro parses the same bytes several times. Not reproduced.
- **The post-open object check** (`scan_dict` walks the nested dictionaries hayro parsed, level by level): each time hayro fetches a level it skips the subtree again, so the work is about nesting depth × size. Still linear (S7 8 MiB about 2,367 ms), but with a large constant.
- The existing residual risks are unchanged: hayro's internal allocations for font parsing, CMaps, cross-reference tables and so on; panics can be caught only with `panic = "unwind"`; stack overflows and aborts cannot be caught.

### Scan-work budget (candidate, **not implemented**)

Form: a deterministic `c × file size + k`; when it is used up, Unsupported (resource_limit) with no text. This round's preflight is already linear and is bounded without a budget. It is kept as a candidate for the student to decide whether to add it as an extra line of defense.

Compatibility analysis (`evidence/round6/scan-work-*.json`, pdf-probe's `scan_work`):

- Dictionary/array elements preflight read plus the value bytes read (the post-review counting): existing samples at most 0.16 per byte, stress inputs at most 0.41 per byte (scan-work-02).
- Bytes read and written by decoders: samples at most 504 times the file size. (Decode volume already has a 256 MiB total cap.)
- So, for example, with "work ≤ file bytes + 1,000,000", none of the existing samples, the stress inputs or R4's legal comment family would be refused.
- Risks:
  - If the constant is set small, normal large files with many small dictionaries would be wrongly refused.
  - A budget adds, on top of "same bytes, same result", another threshold that depends on implementation details: if the implementation changes later (for example the granularity of shared results), the same file's result may change.
  - The linear implementation is already bounded; a budget is only extra insurance.

### Pending the student's decision

- **Decided in round 7**: the student chose to correct the expectations according to the current rules (`1eae524`, see "Round 7"). What follows is the explanation from that time.
- **S5's frozen expectations**: S5 at 128 KiB–1 MiB was frozen as Complete; v5 and v7 both give Unsupported ("total of non-image streams over 128 MiB").
  - The reason: by hayro's fallback rule, every stream header without `endstream` takes data all the way to the next real `endstream`, and each candidate's length is added toward the existing total cap.
  - Two options: one, re-freeze S5's expectations by hayro's rule in a separate commit; two, change how the total is computed so that unfiltered data is counted only once by file bytes (overlapping ranges are not added again). The latter changes the meaning of an existing limit and needs the student's decision. Not changed this round.

### Sync to quality diagnostics (PR #158, `pdf-experiment-8`)

- #158 syncs #148's `8b0971b` (all round-6 commits) with a merge, with no rebase and no force-push, and the quality-diagnostics feature is kept. The merged version is `pdf-experiment-8`, i.e. 7 plus the quality report.
- Every stop point brought in by the merge got a reason code: resource-limit cases are `resource_limit`, things preflight did not measure or cannot check are `unsupported_structure`, and filters that cannot be measured are `unsupported_feature`.
- The stop reasons of the round-3 samples were frozen before the merged version's first run (a separate commit, entries added only; g25–g27 are unverified and not frozen).
- **run 19** (`17a1616`): of 158 samples, 155 PASS / 0 FAIL / 3 unverified (g25–g27). fmt, clippy, all tests, the hashes of each sample set and both isolation limits passed; `fixtures/generate.py --check` as in #148, passed with the font wrapper.
- **Compared with pdf-experiment-7**: `--base` uses pdf-experiment-7 built from #148 `c10d23a`; for all 158 files, the text, status, `left_out`, note and page table are all identical.
- **Compared with pdf-experiment-6** (compare-03, quality-01): 147 of 158 files are fully identical in both reading results and quality reports. The 11 that changed are the 8 PDF-08 defects and g25–g27, and the new stop reasons in the quality reports are all `resource_limit`.
- **stress-05**: 52 / 56, no time-outs, all 7 families linear. S5's 4 failures are the same as in #148. **n1-n2-04**: the same as #148.

## Round 7: the two items from R4 round 4 (`pdf-experiment-9`)

R4 round 4 independently re-verified #148 `8b0971b` (v7) and #158 `c12893b` (v8) ([#148 report](https://github.com/P30g53/Traeva/pull/148#issuecomment-6134091083)): the tested PDF-06/07/08 and the original PDF-09 reproduction families passed; overall acceptance of the Reader did not pass, with two items left:

- **R4-PDF-10 (P2)**: when the same `/Filter` value is shared by many candidate dictionaries, cache hits still deep-copy it one by one, and each candidate also keeps its own copy, so memory grows with "candidate count × list length". R4 has an anomalous observation on the probe; R4 did not perform the confirmation by calling `read_pdf` directly.
- **Four S5 FAILs**: R4 computed independently, from the bytes and the source, that S5 at 128 KiB–1 MiB has a potential non-image total under the current rules that exceeds the 128 MiB cap by 3–195 times. The original Complete expectation conflicts with the current rules; the expectation was wrong, not an accumulation bug.

The student's decision (2026-10-12):

- change S5's expectations according to the current rules; the rules, caps and counting method all stay unchanged;
- fix the copying in R4-PDF-10;
- for the same kind of issue found by the pre-push review (candidate values of an indirect `/Length`), add a structural cap of 64.

### Steps

| Step | commit | Result |
|---|---|---|
| S5 expectations corrected according to the current rules | `1eae524` | Only the four S5 items at 128 KiB–1 MiB changed: Complete became Unsupported, with the note for the non-image 128 MiB cap. The basis is R4's computation, written in `fixtures/stress/protocol.py`. The other 52 items and the rest of the protocol are unchanged; stress-02 to stress-05 keep the original expectations and those 4 FAILs |
| stress-06 (unfixed v7, corrected protocol) | `1eae524` (evidence `519aa68`) | 56 / 56, no time-outs; for the 7 families, the time ratio of 8 MiB to 1 MiB is 5.4–8.9 |
| Read-only review before the push (development process, not R4) | uncommitted fix | The filter-list fix is complete and the results are unchanged; 2 more items of the same kind were confirmed, both in the candidate values of an indirect `/Length` (dev-log entry 36) |
| Fix (`pdf-experiment-9`) | `0c72d05` | run 20: **123 PASS / 0 FAIL / 3 unverified**; fmt, clippy, all tests, release build, both isolation limits and the hashes of each sample set all passed (`fixtures/generate.py --check` as before, passed with the font wrapper) |
| stress-07 | `0c72d05` | 56 / 56, **no time-outs**; for the 7 families, the time ratio of 8 MiB to 1 MiB is 4.8–9.9; peak resident memory for 8 MiB inputs 31–49 MiB |
| compare-04 (v7 vs v9, one by one) | `0c72d05` | 182 files (126 samples + 56 stress inputs) are **all fully identical** to v7 (status, character count, covered, left_out, note, text hash, full page table, isolation result) |

### Invariants and approach

**R4-PDF-10: the `/Filter` list at one position is stored only once; each candidate holds only a shared reference to it, so the memory a candidate takes does not depend on the list length.**

- Before: a cache hit in `Scan::filter_list` returned a copy of the whole list, and `DictInfo` then stored that copy in each candidate.
- Now:
  - The list is parsed once per position and stored as a shared immutable list (`Rc<[Vec<u8>]>`).
  - Cache hits and `DictInfo` get references to the same single copy; only the pointer is copied.
- The reading results are unchanged: the check order, conditions and notes were not changed (compare-04).
- Test: `filter_lists_are_shared` reads the same dictionary twice and asserts that it gets the same list. A mutation that turns cache hits back into copies is caught by it.
- No budget or threshold was added. `scan_work` already counted shared values only once; after the fix it matches the memory actually held.

**The same kind of issue: each distinct candidate value of a length object is stored only once, with at most 64 values (`MAX_LENGTH_VALUES`, set by the student).**

- Cause: an indirect `/Length` may resolve to any definition of that object in the file, so preflight records every value it finds. Before, this table:
  - was not deduplicated and had no cap;
  - was copied in full by the object-boundary computation every time it met one of the file's own streams;
  - was read in full by every stream that referenced it, each keeping its own copy of the result.

  The cost was "number of holders × number of values", which violates round 6's "linear preflight".
- Now:
  - Each value is stored only once, in the order it was found.
  - It is read in place, without copying.
  - When there are more than 64 distinct values, the streams that reference it cannot be checked before reading:
    - the file's own streams, or containers decoded at open: stop, Unsupported ("A stream's length could not be checked before reading.");
    - other candidates: record a refusal, refused only when hayro actually reads them.
- With 64 or fewer, the reading results are unchanged: deduplication changes neither "the first matching value" nor the set of "all matching values".
- The post-open length check now uses binary search. The list was already sorted with each value once, so the results are unchanged.
- Normal files usually have only one or two values: one more each time an incremental update rewrites the object.
- Tests:
  - `length_values_are_kept_once_and_bounded`: deduplication and the cap;
  - `length_objects_with_too_many_values_stop_their_streams`: 64 are read as usual; at 65, the file's own streams stop and pseudo-objects only record a refusal.
  - Both mutations, removing the cap check and removing the deduplication, are caught.

**S5: the expectations were corrected according to the current rules; the rules are unchanged.**

- The current rule: every stream header without `/Length` and `endstream` takes data by hayro's fallback rule up to the next `endstream`, and each candidate's range is added separately to the 128 MiB non-image total.
- The potential totals R4 computed:
  - 64 KiB: about 100 million bytes, under the cap, still Complete;
  - 128 KiB, 256 KiB, 512 KiB, 1 MiB: about 410 million, 1.64 billion, 6.55 billion and 26.19 billion bytes respectively, all over the cap;
  - from 2 MiB on, the stream-count cap is hit first.
- The other approach (counting unfiltered physical bytes only once) would change the meaning of an existing limit; the student did not choose it.

### What was not verified

- **G did not construct or run the R4-PDF-10 reproduction family.**
  - R4's original inputs were not uploaded. The fix is based on the code path and structural tests: sharing instead of copying.
  - Waiting for R4 to re-verify with its own set of inputs, including the confirmation by calling `read_pdf` directly.
- The cap on length candidate values has only unit tests; there is no timing record on large inputs.
- Round 6's residual risks (hayro-internal paths), N4 and the content-quality items are unchanged; **there is no claim that `read_pdf` is fully bounded**.

### Sync to quality diagnostics (PR #158, `pdf-experiment-10`)

- #158 syncs #148's `81ddd80` (all round-7 commits; merge commit `4b3d28f`) with a merge, with no rebase and no force-push, and the quality-diagnostics feature is kept. The merged version is `pdf-experiment-10`, i.e. 9 plus the quality report.
- The new stop point for too many length candidate values has the note "the length cannot be checked before reading" and is recorded as `unsupported_structure` by round 6's classification rule. No existing sample triggers it, so there were no new quality expectations to freeze before the run.
- **run 21** (`4b3d28f`): of 158 samples, 155 PASS / 0 FAIL / 3 unverified (g25–g27). fmt, clippy, all tests, the hashes of each sample set and both isolation limits passed (`fixtures/generate.py --check` as before, passed with the font wrapper).
- **Compared with pdf-experiment-9**: `--base` uses pdf-experiment-9 built from #148 `0c72d05`; for all 158 files, the text, status, `left_out`, note and page table are all identical.
- **Compared with pdf-experiment-8** (compare-05, quality-02): 214 files (158 samples + 56 stress inputs) are fully identical in both reading results and quality reports (the reader name is not compared).
- **stress-08**: 56 / 56, no time-outs; for the 7 families, the time ratio of 8 MiB to 1 MiB is 5.0–9.0.

## Verification (developer self-tests)

| Step | commit | Result |
|---|---|---|
| Fixtures, manifest, real exports and expectations committed **first** | `32303b0` | The reader had not yet been run on any fixture |
| Validation script and robustness tests committed **first** | `5a4cdd4` | Same as above |
| run 01 (first run) | `5a4cdd4` | **48 PASS / 3 FAIL**, fmt FAIL, isolated memory check FAIL. Found and fixed 2 reader defects (image area, wide gaps between spaces); 2 expectations corrected according to their basis (e02 drawing order, the sample for the isolated memory check). Details in dev-log entry 4; raw record `evidence/runs/run-01-5a4cdd4.json` |
| run 02 | `1e53b91` | 51/51 PASS; fmt, clippy -D warnings, all tests and both isolation limit checks PASS |
| Re-run on the final commit | see the PR description | see the latest file in `evidence/runs/` |

Fixtures (all self-made and synthetic; no personal files):

- **Program-generated** (`generate.py`, with a self-written PDF writer and reportlab), 18: single-page/multi-page English; reportlab Chinese (non-embedded CID font + embedded TrueType subset); 90°/45° rotation and `/Rotate 90` pages; two columns drawn by column and by line; font mappings (Differences, Identity-H + ToUnicode, Type3 without ToUnicode, `uniXXXX` glyph names, StandardEncoding ligatures); blank page; scanned image page; mixed text and images; ruled table; formulas; annotations/forms/JavaScript/attachments; invisible OCR layer; off-page text and repeated drawing; words placed by position; 1-million-character truncation; glyph and drawing-event caps.
- **Adversarial/resource** (a01–a20, including a07a/a07b), 21: Flate bomb, nested Flate, a bomb marked as an image, total over the cap, page count, declared object count, over-deep nesting (stream dictionary and page dictionary), self-recursive form, form fan-out (125,000 calls), LZW, an image codec on a content stream, indirect filter, unknown filter, RunLength bomb, stream count, oversized input, fake `endstream`, pypdf encryption (permissions only / user password), escaped `/Encrypt`.
- **Damaged** (c01–c08), 8: truncated in half, end cut off, `startxref` misplaced, corrupt compressed data, not a PDF, random bytes, page count mismatch, empty file.
- **Real exports** (recorded separately from the program-generated ones), 4: e01 from LibreOffice 24.2.7.2 Writer (mixed Chinese and English, a table, an image, formula characters, a two-column section, characters rotated 90°) and e02 printed by Chromium 141 (HTML with the same content, CSS two columns, transform rotation); two encrypted PDFs exported by LibreOffice, one requiring a password (e03a) and one with permissions only (e03b). In total 18 + 21 + 8 + 4 = 51.
- **Sweeps**: `tests/robustness.rs` checks the contract rules on every truncation length of p01 and on 1,700 single-byte mutations across 5 files, and compares repeated reads of 4 files.

## Untested / BLOCKED

- **R4's 27 original independent inputs: G did not obtain them and has not run them.** The reproductions above were built from the descriptions; waiting for R4 to re-verify with the original inputs.
- The quality diagnostics were tested only on synthetic samples and 3 LibreOffice/Chromium exports. The following are **untested**:
  - real PDFs drawn line by line across multiple columns (for example the output of some converters);
  - Word, LaTeX and InDesign output;
  - false-positive and false-negative rates on real forms and reports.
- The column-detection thresholds (0.95, 6 and 0.5 font sizes, 3 lines) were set from the sample design and not calibrated on a real corpus. Its false-negative cases (see "Round 4") have no remedy; an empty `suspected_risk` must not be taken to mean the reading order has been checked.
- **R4 round 2's original inputs (s01–s19) and logs: G did not obtain them and has not run them.** `fixtures/defects2/` was built from the description in R4's comment; waiting for R4 to re-verify with the original inputs.
- **R4 round 3's original inputs (t05–t07, the inputs of the timing protocol) and logs: G did not obtain them and has not run them.** `fixtures/defects3/` and `fixtures/stress/` were built by G-PDF from the description in R4's comment; waiting for R4 to re-verify with the original inputs.
- **R4 round 4's original inputs (70 independent cases, the six inputs for R4-PDF-10) and logs: G did not obtain them and has not run them.** G also did not build its own R4-PDF-10 reproduction family (see "Round 7 → What was not verified").
- p03 in `fixtures/generate.py` uses a Linux font path (WenQuanYi TTC), so the hash check of the generated set fails on macOS (R4's round 3 and this session's runs 15–17 all recorded `generated_hashes_ok=false`; it passes with an external wrapper pointing to the same font); not changed.

- Round 6's developer self-tests ran on macOS arm64 (session G4, local); the earlier rounds ran in Linux containers. PDFs exported by Mac apps (Word, Pages, Preview): **untested** (the #65 UI slot was not claimed, and no GUI app was opened).
- End-to-end reading through core's Snapshot API: **BLOCKED** (traeva-core compiles only on macOS).
- OSV vulnerability query: **untested** (proxy 403); the local RustSec database was used instead.
- Not covered: vertical CJK, Type3 fonts with ToUnicode, non-Identity CID encodings without UCS2 (hayro's "unsupported font" path), real scans, LaTeX/InDesign/Word output, large real corpora, multithreaded concurrency.

## Whether to continue, and the minimum work needed for integration

**Developer judgment: worth continuing, but only as limited support.** Pure Rust, permissive licenses, no native components; on the text layer of ordinary exports (Chinese and English, font mappings, rotation, two columns drawn by column) the results are accurate, and it degrades honestly on scans, images, tables, formulas and truncation.

The main risks are that hayro is very new, its text-extraction interface is marked experimental, and the in-process resource limits are only soft limits. The recommendation is to first have Codex-R4 independently re-verify at the exact commit; when real Mac output is needed, the student decides the GUI arrangement.

If the student separately approves integration (this experiment **does not implement** it):

- **A**:
  - Dependencies and the lockfile; third-party notices (the BSD-3 fonts and CMaps).
  - Delete the contract copy, add a `.pdf` branch in `read_text`, with the reader name `pdf-1`.
  - Decide whether `[Page N]` and the page table go into the shared contract.
  - Decide whether to put reading in a separate process or worker to get hard limits, and decide on the `panic = "unwind"` requirement.
  - Run an end-to-end test on macOS with the Snapshot API.
- **B**: save derived results per reader version; respect Partial/Unsupported/Failed and `left_out`, and do not claim a complete read; align evidence offsets with page numbers; whether PDF is covered by the Memory sending permission and notification text needs a separate decision.
- **A/B**: confirm the destination of the quality diagnostics, i.e. whether, and in what form, `quality` goes into the shared contract or derived records (binding keys in "Round 4").
- **R**: independently re-verify at the exact commit after integration.
