# F developer evidence: Excel compatibility preparation, BLOCKED

This is developer evidence. It does **not** establish native Excel compatibility
or independent R3 PASS. All three actual Excel save/close/reopen stages and both
edit comparisons remain **BLOCKED**. No Office GUI was launched or activated.

## Baseline and reused evidence

Checked main `eb98835e712d371dff1a3c52d8ea7e67664e1e58`, the #134 merge, contains
reviewed `6de78846225a65321089ec5794254407f35d632d`. The ancestor check succeeds;
Reader source and its Cargo manifest have no diff against that reviewed commit.
Exact source/build hashes and dependency resolution are in `evidence/run.json`.

[R3 attempt3](https://github.com/P30g53/Traeva/pull/134#issuecomment-6113535039)
reported LIMITED PASS, 122/122 independent inputs, including rejection of all nine
original same-hash failures. That independent result is reused, not relabelled
as this developer's new test. The original [attempt1 FAIL](https://github.com/P30g53/Traeva/pull/134#issuecomment-6112932629)
and [attempt2 FAIL](https://github.com/P30g53/Traeva/pull/134#issuecomment-6113387377)
remain intact. No runtime fix occurred this round, so those nine were not rerun
or rewritten. R3 report/retest file hashes are recorded for reference.

No new valid native Excel save evidence was found in the current experiment,
R3's #134 evidence, PR #134 comments or the scoped Developer filename search for
Excel save/roundtrip/compatibility evidence. R3 attempt3 explicitly retained that
gap. This is a search of those locations, not a claim about all files on disk.

## Actual results

| Check | Result | Evidence boundary |
|---|---|---|
| Reviewed Reader included in merged main | PASS | ancestor and unchanged runtime/manifest |
| Isolated offline CLI build | PASS | own ignored target/lock; dependency versions retained |
| Initial Artifact Tool candidate | UNSUPPORTED | exact original package/hash/error retained |
| Minimal synthetic seed fields | FIELD_PASS | 41 checks, no Excel execution |
| openpyxl cross-check of seed | PASS | supplementary read only, no save/evaluation |
| Wrong input-edit expectation on unchanged seed | expected FAIL | input/cache mismatch detected; does not create an edit sample |
| #65 GUI coordination | BLOCKED | D/C claims seen; F explicitly did not claim a slot |
| Excel save/close/reopen baseline | BLOCKED | no native saved output/hash |
| Input edit and formula edit in Excel | BLOCKED | no native saved outputs or coordinate comparisons |

`protocol.json` was frozen before any new output. Seed package
`f-excel-seed.xlsx` is 3708 bytes, SHA-256
`0eaf948b7fc0a4755383e91e91c26ca476fd4209508bd4d399744bede97d0d60`.
Reader output hash
`2dde91bc2c91a2e20195a2f8f82b44156ea42620adaa4ee1ec3c75bcaf019fa6`.
Input hash was unchanged after the Reader and openpyxl reads. Raw values,
formula/cache/context and static limitations are retained in `evidence/seed-reader.json`.
The seed's cache 10 is a frozen synthetic token. Neither its generation nor the
Reader calculates a formula. Cache freshness is `unverified`.

The first candidate `artifact-candidate.xlsx` is 4937 bytes, SHA-256
`3eeefa63737e2f9713361f77aa7fe9c9f7f79f2cb4df1f2953d757b9ba9b67f4`.
Reader returned exit 1 / `unsupported` / `not_plain_xlsx_workbook_content_type`,
without a workbook. The exporter used the workbook type as the XML default;
the existing profile requires the explicit workbook override. It also emitted
feature-property-bag metadata. A bounded public export-help query returned no
matching control. The original bytes and rejection are preserved, with no
Reader/profile/security changes. A new minimal fixture was therefore authored
from the same frozen cells with the experiment's own ZIP fixture helper. No
rejected candidate was repaired into a supposed Excel save. The Artifact Tool
candidate's arithmetic cache was calculated by Artifact Tool, not Excel/Reader.
Both packages' native Excel opening is untested.

## GUI prerequisite failure

Installed application metadata: Microsoft Excel **16.78**, build
**16.78.23100802**. Running/saving Excel version is **unknown**.
The read-only process query was denied with `operation not permitted: ps`.
The enabled UI inventory reported Excel not running, Word and PowerPoint
running, but exposed no verified Office PID, exact workbook window or Dell
display geometry. That inventory cannot establish safe ownership or placement.
No permission workaround, repeated launch attempt or security change was made.

Latest claims read before preparation were D's
[Word claim](https://github.com/P30g53/Traeva/issues/65#issuecomment-6113960372)
and C's [Source claim](https://github.com/P30g53/Traeva/issues/65#issuecomment-6113976866).
F's [coordination record](https://github.com/P30g53/Traeva/issues/65#issuecomment-6114010664)
explicitly declines a slot and records the prerequisite failure. This run did
not open a native test/preview window. Artifact previews were rendered and
inspected headlessly, with no Office interaction. Nothing was moved onto or
tested in an unverified display. No personal workbook was read or closed.

## Handoff and stopping scope

R3 receives the pinned follow-up commit, this developer evidence and the frozen
[protocol](PROTOCOL.md). It must independently prepare its materials/build and
verify results, including Office provenance after #65/PID/workbook/Dell gates.
Developer FIELD_PASS/openpyxl and the reused parser LIMITED PASS cannot become
R3 real Excel PASS. R3 can audit the protocol/preparation while GUI is blocked.

Any future valid in-profile Excel rejection must retain original package/hash/
error before a narrow experimental fix and a new R3 runtime-SHA review. Profile
or safety changes need a user decision. The present PR changes no Rust Reader,
dependency manifest, lockfile, budget, production registration, DTO/schema,
Tauri, Memory UI or cloud behavior. No real model was called.

Hidden sheets, all native save/edit stages, advanced formulas, charts, pivots,
macros, external data, other Office versions/platforms, OS worker isolation and
formal Memory/App integration remain untested. #28 stays Post-MVP. This run
stops after the bounded preparation/blocked report and independent handoff.
