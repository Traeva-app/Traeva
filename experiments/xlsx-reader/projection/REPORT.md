# F aligned projection verification — 2026-10-11 America/Los_Angeles

**Developer PASS: 22 final distinct checks, offline locked Clippy and formatting.**
Not independent review or production acceptance. Main/R2 baseline is
`c6a711d7f3155aee0f30b82eb44aa27cc267e523`. F's branch is stacked on unmerged A
#151 `3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`; the PR pins the review head,
tested adapter source commit is `72673954ea7d6c8b4dcfdad383d1213f5e4926f8`;
verification.json pins source/lock/protocol/helper hashes. F's diff changes only
this experiment directory. Parser src/manifest remain identical to `6de7884`.

## Retained expectations and first results

| Stage | Observed result / disposition |
| --- | --- |
| Phase 1 first build | FAIL before any test: sha2 0.11 digest array lacks LowerHex. Original first-build-FAIL.txt retained; digest-byte encoding fixed without oracle/dependency changes. |
| Initial formatting check | FAIL; original ignored fmt-attempt-1.log retained and hashed; final PASS. |
| Phase 1 internal candidate | 14 focused developer checks PASS, without using the then-pending shared helper. Earlier output/protocols stay retained, not presented as final v23. |
| Initial nested dependency resolution | Six indirect versions differed from root. Original ignored lock retained; final standalone registry versions all match unchanged baseline root lock. |
| First actual #151 68d4e58 compatibility | 3 PASS / 2 FAIL: old part-bearing Sheet was Unrecognized; F's scalar oracle typo expected 5 for `\⟦🙂⟧`, actually U+005C/U+27E6/U+1F642/U+27E7 = 4. Raw first-A151-compat-FAIL.txt retained; oracle assessment explicitly corrected, not a Reader fix. |
| Phase 2 after common grammar alignment | 14 adapter + 4 helper checks PASS, 1 part-known compatibility FAIL; raw v23-compat-FAIL.txt retained. No Unrecognized label claimed as known Sheet. |
| A/B actual decision | A explicitly omitted part, skipped blank literal, specified Blank/missing caches without source lines and Unsupported for structure-only output; #151 added cache recognition. Protocol-ALIGNED.md frozen before execution of the revised implementation. |
| Final actual #151 3d1a4a9 | 1 writer + 16 integration + 5 actual-helper checks PASS (22 distinct), no prior runs counted again; aligned-tests.txt retained. |
| Binding budget follow-up | Added early 8 MiB rejection before extra SHA work; affected binding test PASS, same count (binding-budget-recheck.txt). |
| Final Clippy / fmt / source whitespace | Offline --locked --all-targets -D warnings, fmt --check and source/docs diff --check PASS. Raw Cargo output retains its original EOF blank lines; whole-diff whitespace reports those two logs, which are not rewritten. |

Semantic changes between phases were driven by the student's #150/#151 follow-up
and A's actual reply, not changing answers to fit an output. Earlier source copies,
complete synthetic packages/body, first formatting log and initial resolved lock
remain in ignored evidence/local/candidate-1, candidate-2 and related run paths.
Original frozen protocol and initial test hash, phase-2 protocol hash and final
aligned protocol/test hashes are retained separately.

## Scope of 22 checks

Hidden/veryHidden filtering before text, local preview visibility and original
ordinals with gaps; exact literal/formula/cache and numeric-vs-text types; saved
cache-only editing, missing/Blank/empty-text/boolean/error caches; unresolved
formula text and actual merge/style/attribute omissions; generated-looking
multiline source, special JSON names and non-BMP Unicode; atomic labels/escape
prefix and million-scalar truncation; source-empty versus structure-only output;
sheet reorder/name changes and A1 row/column move/deletion; content-free notes,
parse/resource rejection with no body; nine-field Snapshot/hash/length binding.

The actual A helper was compiled and called at its fixed SHA. Tests cover its
four-Reader gate and TXT/MD/DOCX exclusion, exact known/Unrecognized handling,
source escape roundtrip, delimiter/bracket/quote/backslash/control/non-BMP name
roundtrip, no whitespace trimming, preview-only hidden markers and the required
not-verified cache qualifier. Every generated label in successful integration
outputs must be recognized by that real helper, not by a replica. Formula 1+1
with saved cache999 remains 999/not verified, never recalculated by the adapter.

Twenty-four unique synthetic packages were observed across the grouped checks.
Actual F synthetic input/output hashes and scalar counts are recorded in
synthetic-observed-hashes.json. These are unique packages across grouped tests,
not independent test counts or Microsoft Excel saves. Input/body files were
mechanically checked against every recorded hash/count. example-memory.txt is
an observed output for B2 number7 / C2 formula B2*2 / saved cache14. No personal,
sealed, model, network-service or Office data was used.

## Remaining boundaries

A/B's detailed interface decisions are implemented, but #150/#151 remain unmerged
dependencies; production shared exports/dispatch/preview/permissions, core source
layout and B's comparison/prompts/UI/notice v4 are not F changes. Formal wiring
waits for R2 #26. Part is deliberately internal; client locations do not expose it.
No logical-row/format/business inference, image recognition, formula engine,
macro execution or link refresh. Size/work ceilings are not process isolation.

R3 independently reviews the new adapter SHA. #137 latest LIMITED PASS covers
only source/build/evidence scope; independent Excel X15 is still BLOCKED. F did
not repeat Office saves or reuse developer results as R3 PASS. Original nine
Reader FAILs/fixes, the 122-input audit and #137 evidence are unchanged references.
Actual App/Snapshot/analysis/retrieval/history evidence and sending/revocation
require later approved exact-commit independent checks. No merge/Issue closure.
