# F Excel save evidence and metadata-sanitized derivatives

F observed three real Microsoft Excel save/edit stages on 2026-10-11. The
original Excel outputs contained personal metadata. **They are not included in
the rewritten branch history.** The repository contains metadata-sanitized
derivatives, which are not byte-identical Excel outputs. Original-output hashes
are retained only as records of F's historical observations. GitHub may retain
old PR commits or cached access paths after a force push; a complete hosted purge
requires GitHub Support and is the student's decision.

**Sanitized package revalidation: developer FIELD_PASS. R3 privacy-tool review
of head `5446144eaa88a71ff02326abba28e27ce769f4bf` found an archive-comment
residual-check defect and returned FAIL. F has fixed it with retained regression
evidence; independent R3 revalidation of the fix is pending. R3 independent real
Excel execution was already completed separately in #161 with a LIMITED PASS.**
Previous #137 review conclusions apply to historical heads only and cannot be
carried forward as acceptance of these changed files.

## Source and original Office observations

Reader runtime and manifest are unchanged from reviewed commit
`6de78846225a65321089ec5794254407f35d632d`. The frozen protocol hash remains
`bf3807767d1fb5740118be64a2e85410d960e17d3b3dc4767b576357075f1de0`.
The earlier preparation head is a historical identifier, not the rewritten
branch's parent. The new branch is rebuilt directly from base
`eb98835e712d371dff1a3c52d8ea7e67664e1e58`.

The historical F GUI run used Excel **16.78 (23100802)** and the existing
**Automatic** calculation setting. F used its own synthetic seed, Save As,
close, reopen, visible checks, and close without another save for each stage.
Each original closed-package hash was unchanged after reopen/close. These
observations and original hashes remain in `gui-observations.json` and the
three `*-roundtrip.json` files. They do not describe a new GUI run or reopening
of the sanitized derivatives. Only Excel created the original arithmetic caches;
metadata sanitization, Reader/probe and read-only openpyxl do not calculate.

The user authorized Computer Use and the primary display for that F run.
F waited for the [slot return](https://github.com/P30g53/Traeva/issues/65#issuecomment-6114887812),
[claimed](https://github.com/P30g53/Traeva/issues/65#issuecomment-6114895205),
and [returned](https://github.com/P30g53/Traeva/issues/65#issuecomment-6114986273)
the slot. Excel was not running before F launched official `com.microsoft.Excel`;
native Open dialogs and workbook AX URLs/titles identified its own files.
Numeric PID, OS window ID and physical monitor/window bounds remain unmeasured.
This is the disclosed primary-display method, not evidence that the earlier
Dell/PID/geometry protocol was fully met. Journal timestamps are insertion
times, with a delayed baseline-close observation explicitly identified.
Screenshots were inspected in Computer Use and not separately exported here.
Recent-file metadata and the About device identifier were omitted. Recorded
home paths are now placeholders.

## Privacy cleanup and actual package hashes

`sanitize-metadata.py` normalizes creator/last-modified identity to a synthetic
value, clears Company/Manager if present, and removes the absolute-path element.
For these three packages: 3 existing identity fields normalized, 3 absent creator
fields added, 3 absolute-path elements removed, 0 Company/Manager fields present.
Only 6 package parts changed. All other part payloads are byte-identical to the
original outputs. ZIP entry order and compression methods are preserved; CRC and
sizes are valid. Recompression may change archive bytes and compressed sizes.
All members, entry metadata and introduced text evidence were scanned for known
private identity/home tokens; package absolute home paths were also checked.
Remaining private findings: **0**. Two text evidence files had home paths replaced.
See `privacy-cleanup.json` for counts, part changes and verification records.

| Stage | Original bytes | Historical original SHA-256 | Sanitized bytes | Repository SHA-256 |
|---|---:|---|---:|---|
| baseline | 11471 | `c78ea807b695b297027463160b7a9e86e9d579ef035b5ffa67c6df55d6eabcf2` | 8710 | `5ea94d7eda871ce73231a1ab775d6e660a0c27a1b80f83e267d4740dbf88b650` |
| input_edit | 11480 | `26fde6cd9cf76febafccdb8155cebce7bc54e56419705b23b5af86cf9e3b5d04` | 8717 | `5eb4915a4f6622e22bf8890d7fa51725143f6311d63adbd18dea8192299cefc4` |
| formula_edit | 11476 | `eeddbfd45da9e52493457fdbf795f1cb49d0d1c7caef6a3491708596faa366cd` | 8716 | `2acf204755f8ba50625c8c96acc60c1fb4c3f3c3084f84f62a452b54941abed0` |

The original Excel outputs were briefly published before this correction.
Rewriting this branch removes them from its reachable new commits; it does not
promise erasure from GitHub's retained old objects, other refs or private local
copies. Original-output hash records do not make a sanitized archive a raw save.

## Revalidation on sanitized packages

| Stage | F Data!B2 literal | F Data!C2 formula | Actual stored cache | Field result |
|---|---|---|---|---|
| baseline | number `5` | `B2*2` | number `10` | FIELD_PASS, 41 assertions |
| input_edit | number `7` | `B2*2` | number `14` | FIELD_PASS, 41 assertions |
| formula_edit | number `7` | `B2*3` | number `21` | FIELD_PASS, 41 assertions |

Every worksheet context record equals the historical Reader output: names/order,
part/sheet identity, sparse addresses/row/column/style, English/Chinese text,
number/boolean literals, visibility, merge `F Data!A6:C6`, formula attributes/text
and stored cache tokens. Frozen expectations were not changed. Both positional
comparisons are unchanged: input edit changes B2 literal and C2 cache; formula
edit changes C2 formula/cache. Cache freshness remains **unverified**.

Reader statuses remain **partial**, with limitations and omissions reported in
the refreshed Reader JSON. No parser/profile/resource/security change was made.
No VBA or external relationship is present. Read-only openpyxl 3.1.5 confirms the
selected fields again without saving or evaluating. Its initial supplemental
harness attempt selected incorrect boolean/text addresses and failed. That first
FAIL is retained in `privacy-checks/first-crosscheck-fail.json`; F corrected the
harness to the existing frozen protocol, not the expected values. The subsequent
cross-check passed with input hashes unchanged.

Offline locked checks passed: fmt, clippy on all targets with warnings denied,
54 existing Rust tests, probe build and 3 metadata-cleanup tests. Sanitized logs
are in `privacy-checks/`; exact commands, hashes and results are in
`privacy-cleanup.json`. Reader outputs, field reports, pair comparisons and the
file-hash inventory were regenerated on sanitized packages.

The original R3 122-input matrix and all nine initial failures/fix evidence remain
historical reference and were not rerun or recounted. Source identity remains
verified; it does not certify these new packages. The first exporter candidate's
Unsupported package/error and preparation BLOCKED/FIELD_PASS records are retained,
with text path redaction and affected evidence hashes disclosed.

## Remaining scope and future evidence

R3 must review the new pinned fix independently. Its own real Excel validation
was completed separately in [#161 X15](https://github.com/P30g53/Traeva/pull/161#issuecomment-6117727421) and the
[arithmetic supplement](https://github.com/P30g53/Traeva/pull/161#issuecomment-6117918871).
Those LIMITED PASS results retain PID/OS-window/bounds and strict UI limitations;
they do not certify this metadata tool or F original save provenance. The earlier
statement that R3 still owed real Excel execution was stale and is corrected.
No new GUI operation occurred in this cleanup. The original batched navigation
shortcut observation and post-quit `procNotFound` remain in the journal.

Hidden sheets (this protocol selected merges), advanced formulas, chart/pivot/
macro/external data understanding, formatting fidelity, other Office versions/
platforms, worker isolation and complete OOXML support remain untested or outside
profile. No formal Memory/App registration, shared DTO/schema, root manifest/lock,
Tauri/UI/cloud, resource limit or safety policy changed. Real model requests: 0.
#28 remains Post-MVP. No merge or Issue closure.

Before committing future real Office files, retain private originals outside Git,
sanitize their metadata, scan all package parts for personal identifiers/paths,
and revalidate the derivative. Keep original and derivative hashes separately.
Use relative paths or `<HOME>` in all evidence and logs. This helper handles the
specified metadata fields; it is not a general anonymizer for personal cell text.

## Archive-comment regression follow-up

R3's [original defect](https://github.com/P30g53/Traeva/pull/137#issuecomment-6118794567) and
[formal FAIL](https://github.com/P30g53/Traeva/pull/137#issuecomment-6118875317) remain historical failures.
The sanitizer copied the archive-level ZIP comment but only scanned member
payload/name/extra/comment. A known synthetic identity marker could survive in
the archive comment while the report incorrectly returned remaining=0.

The residual scan now includes the archive-level comment before returning a
derivative. A known identity/home residual raises ValueError, as for member
metadata, rather than returning bytes or a success report. No Reader/profile
limit or safety rule changed. This is a focused coverage fix, not general PII
discovery. Existing member checks remain intact.

F created its own synthetic reproduction and retained its first FAIL, output/report
hashes and test log in `privacy-checks/archive-comment-first-fail.*`. The same
frozen input now gives the expected rejection. Four metadata tests pass, and the
three existing native derivatives are byte-identical when regenerated. Each also
passes the same 41 frozen field checks with Reader stdout byte-identical. Original
package hashes, previous FAILs and GUI records are not rewritten. The original
54 Rust/fmt/clippy/build evidence is retained; no Rust source changed and no old
122-input or R3 12-scenario matrix was repeated. See the regression and field
records in `privacy-checks/`. The new fix requires R3's independent new-head review.
