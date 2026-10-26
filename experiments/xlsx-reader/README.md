# TRAEVA XLSX Reader experiment (F, Refs #28)

> Historical standalone experiment. The production-integration statements below describe the original experiment, not the current app. XLSX was subsequently connected to Memory; current support is described in the [README](../../README.md#supported-content) and [shared contract](../../docs/development/shared-contract.md). The experiment and its recorded results remain separate from product acceptance.

Read-only, offline Rust library and `xlsx-probe` CLI. The implementation lives in
its own Cargo workspace under `experiments/`. It is not linked to traeva-core or
Tauri and does not register a Reader in production. The root Cargo manifests,
Cargo.lock, shared DTOs, Memory and Organize are unchanged.

[RESEARCH.md](RESEARCH.md) records alternatives, licenses, known advisories,
resource caps and context requirements. This work was explicitly authorized in
the F chat on 2026-10-11 as an independent experiment. It does not make XLSX a
CAC requirement or satisfy/close the broader #28 tracking issue.

## Run

From this directory:

```sh
python3 fixtures/generate.py
cargo test --offline
cargo run --offline --bin xlsx-probe -- fixtures/generated/before.xlsx
cargo run --offline --bin xlsx-probe -- fixtures/generated/before.xlsx fixtures/generated/after.xlsx
python3 validate.py
```

Dependencies must already be cached for `--offline`. Direct dependencies are
exactly pinned. Cargo creates a local ignored lockfile; no lockfile is committed
or changed in the parent workspace. The tested dependency versions, licenses and
known-advisory query are retained in `evidence/`. Reruns must review resolution
changes because there is no committed experiment lockfile.

`validate.py` regenerates only `fixtures/generated/`, runs formatting, Clippy,
tests and CLI build, then records expected/actual states, fixture hashes, unchanged
input hashes and three coordinate comparisons. It uses Python's ZIP/XML readers
for fixture structure checks. If installed, openpyxl performs a read-only
cross-reader check of the baseline workbook. That check does not recalculate.
The online advisory check is separate:

```sh
python3 audit_dependencies.py
```

It sends only public crate names/versions to OSV. The Reader and its tests remain
offline. `validate.py` uses a ten-second subprocess timeout for each CLI probe;
this is a test harness bound, not an in-library wall-time guarantee.

## Output

`read_xlsx(bytes, limits)` takes already-captured bytes. It never opens a path,
writes a document, runs a macro/formula or follows a link. The CLI opens the
explicitly supplied files read-only and emits JSON. This probe is not a production
authorized-root boundary; production must supply verified snapshot bytes through
the existing owner of that interface.

Each sheet retains name, sheetId, part path, visibility, declared dimension and
merge ranges. Each explicit cell retains address, row/column and style index:

```json
{
  "address": "C2",
  "row": 2,
  "column": 3,
  "style_index": null,
  "literal": null,
  "formula": {
    "text": "B2*2",
    "attributes": {},
    "interpretation": "literal_formula_text"
  },
  "cached_result": {
    "value": { "type": "number", "value": "40" },
    "freshness": "unverified"
  }
}
```

A literal number has `literal`, with `formula` and `cached_result` null. Numbers
retain raw decimal strings. Missing cache and empty `<v/>` are distinguished.
Shared followers keep their index with null text and an unresolved marker.
Explicit cells within array/data-table ranges keep the original anchor/ref and
their value as an unverified cache. No formula expansion/evaluation occurs.
Formatting and date serial conversion are outside this profile; `t=d` tokens
are preserved with a limitation. Excel `_xHHHH_` text escapes are preserved raw
with a limitation rather than silently interpreted.

`complete_within_profile` means the declared narrow cell profile completed;
`partial` includes detected omissions. Both return exit 0 and a workbook with
static limitations. Invalid required data, unsupported containers and exceeded
limits return exit 1, a structured `failed` / `unsupported` / `resource_limit`
error and no workbook. There is no successful truncation or fallback to a live
source path. This is a limited parser, not a full OOXML schema validator.

## Change evidence

The optional second file produces coordinate changes with separate
`literal_changed`, `formula_changed`, `cache_changed` and `style_changed` flags,
plus before/after cell records. In the synthetic example, `Budget!B2` changes
from 20 to 25, and `Budget!C2`'s saved cache changes from 40 to 50 while its formula
stays `B2*2`. `Budget!A2` is `Hosting`, and `Budget!B1` is `USD`; those retained
cells provide citable context for a later description.

The comparison refuses changed sheet identity/name/part/visibility, merged
ranges or date epoch. It is positional evidence and does not infer row identity,
insertions, renamed sheets, units, lineage, semantic equivalence of numeric
spellings, or changes to unsupported parts/style definitions. Equal coordinate
records do not mean two workbooks are semantically or visually identical.

## Validation boundary

The evidence is author-run experiment evidence. Tests use only generated files,
including inert markers for unsupported features. Unsupported-feature stubs are
not claims that Excel can open or render those features. No personal spreadsheet
was read. No Office window was opened during the initial parser tests; those ran
in the background. The later native Excel run is recorded separately below,
including the user's subsequent primary-display authorization.

R3 independently reviewed initial head `96a3981` and reported **FAIL** on three
findings (four malicious synthetic files): directory selection disagreement,
missing XML attribute separators, and unchecked ZIP descriptors/Deflate tails.
The original review is [retained on PR #134](https://github.com/P30g53/Traeva/pull/134#issuecomment-6112932629).
`evidence/r3-fixes.json` preserves the author's reproduction of all four initial
failures and the subsequent rejection results. That revised head still failed
R3's second review, as recorded below; author tests did not upgrade it to PASS.

[R3's second review of `3b1ac06`](https://github.com/P30g53/Traeva/pull/134#issuecomment-6113387377)
confirmed those four fixes but remained **FAIL**: invalid XML element/attribute
names, comment double hyphens and a literal CDATA closer in character data were
accepted. The current fix validates XML 1.0 Fifth Edition NCName character ranges
and QName components before namespace resolution, enables comment checking on
both parser passes, and rejects `]]>` only in raw CharData. Legal escaped text,
CDATA, Unicode names and ordinary comments retain their contents. The five old
failures and the author's post-fix recheck are separately retained in
`evidence/r3-xml-fixes.json`. [R3's third review of `6de7884`](https://github.com/P30g53/Traeva/pull/134#issuecomment-6113535039)
subsequently returned **LIMITED PASS**, 122 independent input checks, including
all nine original same-hash failures now rejected. Both older FAIL reports remain
retained. PR #134 was merged as `eb98835`; its Reader code matches the reviewed
head. This acceptance covers the independently tested parser profile only.

The runtime now decodes the exact payload ranges returned by its bounded ZIP
preflight, using flate2 and crc32fast. `zip` is a dev dependency only, used for
synthetic writers and positive cross-checks. Both signed and unsigned 32-bit data
descriptors are checked immediately after their payload; every local record must
abut its neighbor and the central directory. Deflate must reach StreamEnd after
consuming exactly the declared compressed payload. Ordinary archive comments
remain supported; a comment containing an EOCD signature is explicitly unsupported.
Prefixes, gaps/padding, ZIP64 descriptors and extra archive records are outside
this narrow package profile. XML attributes require ASCII XML whitespace between
attributes, including namespace declarations and the XML declaration.

To recheck the four review inputs when R3's synthetic corpus is available:

```sh
cargo build --offline --bin xlsx-probe
python3 verify_review_fixtures.py /path/to/R3/attempt1
python3 verify_review_fixtures.py /path/to/R3/attempt2/fix-boundaries --round xml
```

The script checks exact input hashes, rejection with no workbook, and unchanged
inputs. Review code is not imported or copied into this experiment.

The [Excel preparation report](excel-save/REPORT.md) retains its original GUI
BLOCKED and synthetic FIELD_PASS. The subsequent [native Excel developer evidence](excel-save/native-excel/REPORT.md)
contains three actual Excel 16.78 save/close/reopen packages and two frozen
coordinate comparisons. It records the changed GUI method, unmeasured PID/window
geometry, actual omissions and cache freshness `unverified`. R3 gave LIMITED PASS in [#161 independent real Excel verification](https://github.com/P30g53/Traeva/pull/161#issuecomment-6117727421)
and the [arithmetic cache follow-up test](https://github.com/P30g53/Traeva/pull/161#issuecomment-6117918871);
the original PID/OS window/bounds limitations remain, and that result does not certify F's original save provenance. OS-enforced worker isolation,
coverage-guided fuzzing and production integration remain unrun. Existing
repository CI does not discover this nested workspace automatically; run its
commands explicitly. Do not interpret unrelated product CI as XLSX acceptance.
# Canonical source relocation

The active parser and projections are now the unregistered `crates/traeva-core/src/xlsx*.rs` modules. This experiment crate is a compatibility export; research, original FAILs and Excel evidence below remain historical records. See [core relocation notes](../../docs/features/xlsx-reader-core.md). The compatibility bridge links existing core types/helpers and does not register XLSX in production.


### Privacy correction for Excel evidence

The three retained native-stage packages are metadata-sanitized derivatives, not
byte-identical Excel outputs. Original outputs are excluded from the rewritten
#137 history; their hashes remain historical F provenance records. See
[cleanup and revalidation](excel-save/native-excel/REPORT.md) and
[cleanup evidence](excel-save/native-excel/privacy-cleanup.json). R3 gave **LIMITED PASS** in [its independent re-verification of `7d60860`](https://github.com/P30g53/Traeva/pull/137#issuecomment-6119271973),
covering the archive comment fix and the bounded check of the committed material; the earlier FAIL is kept.
This sync with main only resolves a conflict in the notes; R3 needs to check that conflict resolution separately, and it does not widen the earlier accepted scope.
R3 independent real Excel validation already completed separately in #161 with
its LIMITED PASS scope and GUI limitations. The original archive-comment FAIL
and focused correction remain recorded in the evidence report. Future real Office packages must be sanitized and scanned before Git
publication; all evidence home paths use placeholders.
