# PDF Reader dependency selection, licensing and deployment (Claude-G, Refs #28)

Checked on 2026-10-11/12 (UTC). The crates.io metadata, source code and license files were all actually downloaded and checked this time; anything not run is marked "untested".

## Conclusion

Choose **hayro** (`hayro-syntax` + `hayro-interpret` 0.8.0, Apache-2.0 OR MIT, pure Rust). It handles PDF structure parsing, decompression, fonts/encodings/CMaps and interpreting page drawing instructions; this experiment only implements a "drawing device" (collecting each glyph's Unicode and position, image areas, line shapes), plus resource preflight, line joining and status rules; it does not implement the PDF format from scratch.

Main limitations (the student needs to know these):

1. hayro is very new (first released 2025-06, 0.x, single maintainer LaurenzV); its docs describe it as "still very much in development", and `Glyph::as_unicode()` (the interface used for text extraction) is marked experimental. The version is pinned exactly at `=0.8.0`; any later upgrade must re-run all fixtures.
2. hayro's decompression has no size limit (`flate2`'s `read_to_end`), so this experiment measures every stream itself with capped decoders before hayro (see README "resource limits"). This can only constrain the "decompressed size", not all of hayro's internal memory and time; hard limits rely on a separate process (`pdf-probe --isolated` has been demonstrated and tested).
3. PDFs that do not embed their fonts (for example Helvetica, STSong-Light) must enable hayro's `embed-fonts`, otherwise these glyphs **silently disappear entirely** (spike measurement: without it, neither the English nor the Chinese CID font produced any output). It is enabled; in addition, "the page has text-showing instructions but not a single glyph came out" is disclosed separately.

## Candidate comparison

| Candidate | License | Maintenance and scale (crates.io) | Conclusion |
|---|---|---|---|
| **hayro-syntax / hayro-interpret 0.8.0** | Apache-2.0 OR MIT | First released 2025-06, 0.8.0 released on 2026-10-04; about 4.2 million downloads each; `hayro-interpret` uses `#![forbid(unsafe_code)]`; has built-in limits such as object nesting 64, interpretation nesting 50, xref chain 256; page-tree traversal has cycle protection | **Adopted**. Gives each glyph's position and direction (needed for rotation, two columns and page boundaries), decodes images only on demand, and annotations can be turned off and not drawn |
| pdf-extract 0.12.1 (based on lopdf 0.42) | MIT | Since 2018, about 6.1 million downloads | Not adopted: depends on an old lopdf (current is 0.45); about 101 `unwrap/expect/panic` in `lib.rs`; built-in "automatic decryption with an empty password"; less position information |
| lopdf 0.45 on its own | MIT | Since 2016, actively maintained | Not adopted: parsing only, no handling of fonts/encodings/CMaps to Unicode, which would mean implementing the PDF text layer ourselves |
| pdf (pdf-rs) 0.10 | MIT | Few releases | Not adopted: no ready-made text extraction |
| pdf_oxide 0.3.78 | MIT OR Apache-2.0 | First released 2025-11, 78 versions in 11 months | Not adopted: large dependency surface (env_logger, regex, image, taffy, office_oxide and others, about 40), too new |
| pdfium-render 0.9.4 + PDFium | Bindings MIT/Apache; PDFium BSD-3/Apache | Mature (Chrome engine) | Not adopted this round: requires loading a C++ native library in-process (parsing untrusted PDFs would need a separate sandbox), and macOS requires packaging a signed dylib; in this environment, downloading the GitHub release was refused by the proxy (403), so it could not be tested. If hayro's quality is not good enough, this is a later candidate |
| mupdf (Rust bindings) | **AGPL-3.0** | — | **Excluded** (AGPL) |
| Poppler | GPL | — | Not used as a dependency. Only during validation, the system's `pdftotext -raw` was used once as an external reference tool to check drawing order (e02); not linked or copied |
| oxidize-pdf 5.4.2 | MIT | First released 2025-07, few downloads | Not examined in depth |

## Actual dependencies (runtime and build)

`audit_dependencies.py` lists, from `Cargo.lock` + `cargo metadata`, every package this library and `pdf-probe` use at runtime or build time (results in [evidence/dependencies.json](evidence/dependencies.json)):

- All licenses are permissive: MIT, Apache-2.0, BSD-3-Clause, Zlib, Unicode-3.0, 0BSD, Unlicense. `not_permissive` is empty. No GPL/AGPL.
- No build script compiles C/C++, and no native components are downloaded. The packages with build scripts only do Rust version/feature detection (crc32fast, libc, num-traits, proc-macro2, quote, serde, serde_core, serde_json, zmij).
- Known vulnerabilities: this environment's proxy refuses api.osv.dev (403, untested). A local clone of the RustSec advisory-db (commit `7eebec69`, 2026-10-09) was used instead: among our dependencies, once_cell, sha2, smallvec and zlib-rs have historical advisories, 8 in total, all fixed in the versions we use; no advisory affects the current versions, and there are no informational (unmaintained/unsound) entries.

### Data distributed with the binary (must go into the third-party notices at integration)

| Data | Source and license | Use |
|---|---|---|
| 14 Foxit `.pfb` fonts (about 240 KB) | PDFium, BSD-3-Clause (`LICENSE_FOXIT`) | Substitute fonts when fonts are not embedded |
| 61 predefined CMaps (brotli-compressed blocks) | Adobe cmap-resources, BSD-3-Clause | CJK predefined encodings (such as UniGB-UCS2-H) |
| `CGATS001Compat-v2-micro.icc` | CC0 | Color (not used for text extraction, but compiled in) |
| `LAB.icc` (484 bytes, generated by lcms) | No separate license file; released under the crate license | Same as above; A should confirm this during the notices review |

BSD-3 requires the copyright notice to accompany binary distributions; brotli, alloc-no-stdlib, moxcms and pxfm also have a BSD-3 option. This is notices work for A's future integration; this experiment does not change shared files.

## macOS arm64 and offline build

| Item | Result |
|---|---|
| `cargo check --release --locked --target aarch64-apple-darwin --all-targets` (cross type-check on Linux, no linking) | **PASS** |
| Linking, running and running fixtures on macOS arm64 | **BLOCKED**: this session is a Linux cloud container, with no Mac |
| `cargo build --offline --locked --release` (new target directory, only the local cargo cache) | **PASS** |
| Fully offline machine | Needs `cargo fetch` or `cargo vendor` first; no other downloads. Decided by A |
| Binary size | `pdf-probe` (Linux x86_64 release, including tool code such as serde_json) 7.7 MB; the library's own increment is untested |
| Native components / code signing | No native library; no extra signed dylib needed |

## Approaches not adopted

- Do not bring in AGPL/GPL implementations by default; do not copy any upstream code.
- No OCR, no page rendering, no JavaScript execution (hayro has no JS engine; the source of hayro's three crates was checked: apart from tests, there is no `std::fs`, `std::net` or `Command`).
