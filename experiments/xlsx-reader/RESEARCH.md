# XLSX Reader selection, 2026-10-11

This is a standalone experiment authorized in the F chat, linked to #28. #28
remains a Post-MVP tracking issue. This experiment does not change that issue's
acceptance criteria, CAC scope, or frozen Organize specifications.

## Alternatives checked

| Option | License / interface | Safety and resource implications | Decision |
| --- | --- | --- | --- |
| [calamine 0.36.1](https://docs.rs/calamine/0.36.1/calamine/) | MIT; pure Rust; sheets, values, separate formula APIs, streaming cell API with formula metadata | `Xlsx::new` loads shared strings/styles before returning. Range APIs materialize rectangular ranges. Streaming avoids that range allocation, but is not a package-wide limit. Raw decimal spellings and unexpanded formula XML are useful for this experiment. | Reconsider for a broader reader after bounded preflight and formula/cache regression checks. Do not use its unguarded entry point here. |
| [umya-spreadsheet](https://github.com/MathNya/umya-spreadsheet) | MIT; reads and writes XLSX, lazy worksheet loading | Full mutable workbook model, styles, charts, images and writing exceed this read-only scope. Lazy loading does not establish an input, decompression, or allocation ceiling. | Not selected. |
| [zip 8.6.0](https://docs.rs/zip/8.6.0/zip/) + [quick-xml 0.42.0](https://docs.rs/quick-xml/0.42.0/quick_xml/) | MIT / MIT; package and XML primitives | More OOXML validation is our responsibility. Enables raw decimal retention, sparse cells, original formula metadata and limits before allocating a workbook model. | Initially selected. R3 found disagreement between preflight and ZipArchive directory selection plus unchecked framing; zip now remains only in dev dependencies. |
| Bounded ZIP directory parser + [flate2 1.1.10](https://docs.rs/flate2/1.1.10/flate2/) + [crc32fast 1.5.2](https://docs.rs/crc32fast/1.5.2/crc32fast/) + quick-xml 0.42.0 | flate2 MIT OR Apache-2.0; crc32fast MIT OR Apache-2.0; quick-xml MIT | Decode only the preflighted payloads, check consumed Deflate bytes and CRC directly, retain explicit framing limits. Package validation remains our responsibility. | Current runtime selection after R3's initial FAIL; independent re-review pending. |

Calamine evidence is pinned to its [0.36.1 manifest](https://github.com/tafia/calamine/blob/v0.36.1/Cargo.toml),
[constructor / range implementation](https://github.com/tafia/calamine/blob/v0.36.1/src/xlsx/mod.rs), and
[streaming formula metadata API](https://docs.rs/calamine/0.36.1/calamine/struct.XlsxCellReader.html).
The current RustSec [RUSTSEC-2026-0317](https://rustsec.org/advisories/RUSTSEC-2026-0317.html)
is recorded against **sheets-diff**, not a calamine package advisory. Its description
identifies an underlying calamine CFB/password probe allocation reachable before
ZIP validation. It explicitly says a small input limit alone does not prevent it.
We reject OLE/CFB and non-ZIP input before any ZIP/XML parser. No CFB parser is linked.

Quick-xml's [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194.html)
and [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195.html)
address attribute-check CPU and namespace allocation; both list >=0.41.0 as patched.
The experiment uses 0.42.0, with additional document, event, depth, attribute and
node caps. The [zip package advisory index](https://rustsec.org/packages/zip.html)
and [quick-xml index](https://rustsec.org/packages/quick-xml.html) were reviewed.
An OSV query of all 32 resolved Rust dependencies returned no matching known
advisories on 2026-10-11; exact versions, request and response are retained in
`evidence/dependency-advisories.json`. This dated query and source review do not
prove the absence of defects or substitute for an independent security audit.

Selected dependencies use Rust decompression only. Their disabled features exclude
encryption, alternate compression methods, networking, Office execution and image
decoding. Transitive versions and license metadata are recorded by the local
validation script. No upstream fixture corpus or AGPL code is copied.

## Reading profile

The normative layout is [SpreadsheetML structure](https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/structure-of-a-spreadsheetml-document).
[Microsoft's formula documentation](https://learn.microsoft.com/en-us/office/open-xml/spreadsheet/working-with-formulas)
describes `f` as the formula and `v` as its saved result. They are separate fields
in this output, even when `v` is missing, empty, text, boolean or an error. Every
formula's cache freshness is `unverified`; calculation properties never upgrade it.
No evaluation, recalculation, external link refresh or formula rewriting occurs.

Only UTF-8 XML, transitional SpreadsheetML, ordinary unencrypted single-disk
Stored/Deflate ZIP packages are supported. ZIP64, ambiguous paths, unsupported
compression, external sheet/shared-string targets, encryption, Strict OOXML and
malformed required parts fail explicitly. Unsupported content is detected using
content types, package relationships, part names and worksheet/workbook elements.
Cells in supported worksheets can still be returned as `partial` with omissions
listed for charts, pivot tables, macros, external links/data, objects, tables,
extensions and other metadata. No unsupported binary part is interpreted.

ZIP framing follows [PKWARE APPNOTE](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT)
sections 4.3.9.1, 4.3.9.3 and 4.3.9.4 for required bit-3 data descriptors and
signed/unsigned encodings. The bounded directory parser supplies names, method,
CRC, expanded size and exact compressed ranges directly to unpacking. There is
no second ZIP parser choosing another directory. Descriptor CRC and both sizes
must match the central record; nonzero local values must match as well. Every
local record, including any descriptor, must be contiguous from offset zero to
the central directory. Archive comments with EOCD signature bytes are rejected
as ambiguous. This deliberately excludes prefixes, padding and extra archive
records; it is not a complete ZIP format implementation. Raw Deflate decoding
uses a fixed 8 KiB output buffer, checks size caps before retaining each chunk,
and requires StreamEnd with no unread payload bytes, followed by exact size and
CRC validation. Stored payloads must have equal compressed/expanded lengths.

Quick-xml's attribute iterator accepted adjacent quoted attributes in R3's
malformed XML sample. A bounded lexical check now verifies XML attribute spacing
before namespace resolution, while retaining the existing duplicate, namespace,
entity, value and resource checks. This is targeted well-formedness enforcement,
not a claim of complete OOXML schema validation.

R3's second review confirmed the first fixes, then found five malformed XML
inputs still accepted at head `3b1ac06`. Name checks now follow XML 1.0 Fifth
Edition [productions 4 and 4a](https://www.w3.org/TR/xml/#NT-NameStartChar),
with [namespace QName](https://www.w3.org/TR/xml-names/#ns-qualnames) components
checked as NCNames before namespace resolution. Explicit Unicode ranges avoid
incorrectly substituting ASCII-only names or Unicode alphabetic categories.
Quick-xml's `check_comments` option is enabled for both parser passes, covering
internal `--` and a trailing hyphen adjoining the closing delimiter. Raw Text
events reject `]]>` as required by [CharData](https://www.w3.org/TR/xml/#NT-CharData);
escaped references and CDATA are checked in their own lexical contexts and may
legitimately produce that string after decoding. These checks reuse the existing
event, node, attribute and depth budgets and introduce no dependency changes.
The two independent FAIL reviews and before/after author evidence are retained;
new-head acceptance remains R3's decision.

## Context and future change descriptions

Records preserve workbook input SHA-256; sheet name, sheetId, part path and
visibility; explicit A1 address and one-based row/column; raw typed literal or
formula/cache; style index; declared dimensions; merge ranges; workbook date
epoch and calculation attributes. Shared formulas keep original `t`, `si`, `ref`
and text. A shared follower with no text is **unresolved**, not an empty literal
or an invented expanded formula. Array/data-table formulas retain metadata and
report unsupported interpretation. Numbers stay decimal strings, avoiding f64
rounding of identifiers; date serials are not turned into timestamps.

The coordinate diff is an experiment only. Caller chooses the two byte snapshots.
It refuses changed sheet identity/name/part/visibility or merged ranges, then
separates literal, formula, cache and style changes by address. A cache-only change
cannot be described as a formula edit or a recalculated fact. Nearby cells are
available by coordinates; headers, units and row labels require explicit cited
cells and a future student-approved policy. The reader does not infer business
meaning, family continuity, sheet renames or row insertion/deletion alignment.
For example, retained records allow citing `Budget!B2` together with `Budget!A2`
and `Budget!B1`; calling those cells a row label and unit is a separate decision.

## Resource boundaries

Default hard profile: input 8 MiB, <=256 entries, central directory <=256 KiB,
one expanded entry <=4 MiB, all expanded entries <=32 MiB, compression ratio <=200,
XML event <=64 KiB, depth <=32, <=32 attributes per tag, <=200,000 XML nodes across
the package, <=32 sheets, <=20,000 shared strings, <=10,000 cells across sheets,
one text/formula <=32 KiB, retained cell text/formula total <=4 MiB.
Array/data-table regions are capped at 64 per sheet. Explicit members retain
anchor context and unverified caches, with absent and empty caches distinct.
Caller may only tighten these caps. Exceeding a cap returns `resource_limit` and
no workbook, never a successful truncated extraction. Declared ZIP sizes are
preflighted and actual decompression is capped and checked against declarations.
Declared sheet dimensions never allocate a rectangle.

These are deterministic size/work ceilings, not an OS memory or wall-time sandbox.
Before production, use a worker process with a timeout/memory budget, fuzzing,
independent review and real Excel save/compatibility validation. The CLI is an
offline probe of explicitly supplied synthetic files. There is no Memory,
Organize, shared DTO, root manifest/lockfile or Tauri integration.
