# Frozen minimal Excel save protocol (F, Refs #28)

`protocol.json` was frozen before generating the seed or reading new outputs.
Two worksheets in order: `F Data`, `F 中文`. The fixed addresses, text, numbers,
both booleans and merge `F Data!A6:C6` are recorded there. Merge context is the
one selected existing profile feature; hidden sheets are untested in this run.
The seed uses the experiment's existing synthetic ZIP fixture helper. Its cache
is the frozen synthetic token 10, not a calculation or an Excel save. The first
Artifact Tool candidate is retained separately with its original hash/error:
the exporter uses a default workbook content type instead of the profile's
required explicit override and adds a feature property bag. Its public API/help
exposed no minimal-profile export control. The fallback authors a new minimal
fixture from the same frozen cells; it does not rewrite the rejected candidate
or relax Reader checks. Both files remain unverified for native Excel opening.

## GUI prerequisite and execution

1. Read the latest #65 claims/returns. Claim a free slot with the pinned Reader
   commit, own worktree/build/evidence/input directories and no Traeva App or
   model requests. Do not proceed while another line owns the slot.
2. Verify the Office PID, executable, exact workbook path/window ID and ownership.
   Verify the Dell display identity and bounds and place every test window and
   dialog fully there before interaction. Retain timestamped evidence. Never
   activate an unidentified Office instance or close somebody else's workbook.
   If those facts cannot be verified or system permission/human intervention is
   required, record BLOCKED and stop GUI. Do not retry permission workarounds.
3. Record the **running** Excel version/build. Open only a private working copy
   of this synthetic seed. Check there are no macros/external relationships.
   Leave application calculation/security settings unchanged. Record the actual
   calculation mode. A manual mode or absent cache is evidence to report, not
   permission to change global settings or patch saved caches.
4. Save As `baseline.xlsx` in the private evidence directory with B2=5 and
   C2=`=B2*2`. Save, close only this workbook, reopen it, verify path/window and
   visible fields, then close it. Record save/close/reopen timestamps and hash
   the closed package. Reopen without another save, close, and check that hash
   is unchanged. Preserve the original package. Normal Excel arithmetic
   computation, if observed, belongs to Excel. Expected saved cache: 10.
5. Reopen that own baseline, change only `F Data!B2` to 7. Save As
   `input_edit.xlsx`, save/close/reopen/verify/close and hash as above. Expected
   formula still `B2*2`, saved cache 14. Retain baseline unchanged.
6. Reopen that own input-edit file, change only `F Data!C2` to `=B2*3`. Save As
   `formula_edit.xlsx`, repeat the same save/close/reopen/hash sequence. Expected
   input 7, formula `B2*3`, saved cache 21. Retain both earlier packages.
7. Verify each saved package with the pinned Reader and `check.py`. Preserve
   raw Reader stdout/stderr/exit code, before/after input hash and output hash.
   Record workbook status, sheet identities/order, visibility, merge ranges,
   address/row/column/style, literals, formula text/attributes/interpretation and
   cache token/freshness. Do not strip omitted features to get a passing input.
8. Run the existing probe with two packages for baseline→input_edit and
   input_edit→formula_edit. Compare the separately frozen literal/formula/cache
   flags in `protocol.json`; retain style flags and any identity mismatch error.
   A sheet/part identity mismatch refuses positional diff. Record that limit
   rather than inventing row identity or semantic alignment.
9. A read-only openpyxl formula/data-only check may cross-check the same packages.
   It must never save, evaluate formulas or substitute for Excel provenance.
   Hash inputs again. Release only the slot claimed by this run.

For every stage, retain the input source/hash, Excel output/hash, process/window/
display evidence, actual save/close/reopen sequence, running version, probe hash
and exact source commit. A package hash or field comparison alone cannot prove
Office execution. `check.py` deliberately returns only FIELD_PASS and always
leaves Office provenance unverified; a reviewer judges that separate evidence.
Numeric comparisons normalize decimal spelling against fixed expectations while
the raw tokens remain in `reader.json`. The checker never evaluates a formula.

## Run the preparation check

From `experiments/xlsx-reader/`, using a private ignored build directory:

```sh
CARGO_TARGET_DIR=excel-save/local/build cargo build --offline --bin xlsx-probe
python3 excel-save/check.py excel-save/local/build/debug/xlsx-probe seed \
  excel-save/f-excel-seed.xlsx excel-save/local/seed
```

For an actual saved file, substitute its frozen stage name and own package path.
Outputs are written only to the specified evidence directory. The checker does
not mutate XLSX or read personal workbooks. Dependencies and the local lock are
ignored; the repository root manifests and Cargo.lock remain untouched.
`make-minimal-seed.py` reproduces the final seed without evaluating a formula.
`build-seed.mjs` reproduces only the initial Artifact Tool candidate using the
bundled runtime. Neither generates substitute baseline/edit Excel outputs.

## Review and failure boundary

Reader/formula/cache semantics and ZIP/XML/resource limits remain unchanged.
Cache freshness stays `unverified` even when Excel saved the expected number.
An Excel compatibility PASS requires three actual Office stages plus field and
coordinate checks, not merely the present preparation results.

If a valid in-profile Excel package fails, retain its original bytes/hash/error
before diagnosis. Fix only this experiment, with no ZIP/XML check removal or
budget increase. Re-run related normal inputs and all nine original R3 failures
at the new runtime SHA, then hand off to R3. Any profile/security change requires
a user decision first. Preserve all historical FAILs.

Charts, pivots, macros, external/embedded data and links are not exercised or
interpreted. Styling, formatting/date conversion, hidden sheets in this run,
advanced formulas, OS worker isolation, other Excel versions/platforms and
formal Memory/App/cloud integration are untested. Existing omissions and static
limitations are retained even for a FIELD_PASS. `partial` never means full
workbook fidelity. No formula engine, link refresh or business interpretation is
added. R3 must prepare its own materials and rebuild at the new commit rather
than inherit the developer's results as independent PASS.

## Privacy handling before publication

Keep raw Office saves in a private ignored directory. Before committing any real
Office test package, use `sanitize-metadata.py INPUT OUTPUT REPORT` to create a
separate metadata-sanitized derivative, scan every part for personal identities
and home paths, and run the frozen Reader/field checks on that derivative. Keep
original-output and derivative hashes separately; only the sanitized derivative
belongs in Git. Replace evidence/log home paths with `<HOME>` or relative paths.
This policy does not change frozen cell expectations or Reader security rules.
Historical GUI records describe original saves, not a new sanitized-file GUI run.
