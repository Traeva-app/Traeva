# Phase 3: A/B-confirmed v23 output

Frozen before executing the final aligned candidate. Changes are based on A's
actual reply https://github.com/P30g53/Traeva/pull/150#issuecomment-6116720334,
not changes to make observed failures pass. Old protocols/source/FAILs remain.

Compile and invoke the actual #151 implementation at
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`; reader.rs SHA-256
`e0eb50657197eee4b343d38d55c13294a2c89635d8d8575df3dd12bef9bdca55`.
This is an unmerged dependency in F's worktree, not a change to main/R2.

Shared Reader identities remain A's xlsx-1 / xlsx-preview-1 constants. Internal
revision is f-xlsx-projection-candidate-3; original parser profile v1 is unchanged.

- No part field in labels: it stays in the typed parser Workbook. User-visible
  locations stop at declared sheet ordinal/ID/name, A1 and field/type. This is A's
  explicit decision, not a hidden relaxation of the original context request.
- Match A's exact sheet/cell syntax and actual structure_name/escape_source_line.
  Name control/delimiter/quote/backslash characters use \uXXXX. Source escaping
  retains LF-normalized content; both known and Unrecognized generated lines are
  excluded from original evidence; other Readers never recognize these labels.
- No literal Blank label. Cache Blank/missing labels are separate generated
  states, without source lines; an empty Text value retains an empty source line.
  Missing formula text is never expanded and goes into actual left_out.
- Output with zero retained original scalars is Unsupported/zero body, even when
  sheet/field labels were generated or metadata exists. Generated labels and
  escape/separator scalars cannot manufacture original evidence.
- Keep original declaration ordinals after hidden filtering. Sheet renames alter
  names and cannot be called pure reorder. Moves/deletions remain A1 evidence.
- Million-scalar truncation preserves whole labels and escape + first scalar;
  actual remainder is Partial. A deliberately inconsistent saved cache (formula
  1+1, cache999) remains 999/not verified, proving no formula recalculation.

Expected final distinct checks: 1 writer unit + 16 integration + 5 actual-helper
checks. The previous part-known target now tests the actual no-part label because
A decided to omit part from projections. No assertion changes the old FAIL log.
All generated labels in successful integration outputs must be typed-recognized
by the actual pinned helper, not merely safe Unrecognized. The old scalar typo
correction remains separately assessed in PROTOCOL-V23.md.

Final budget follow-up (before the focused binding recheck): the experiment-local
snapshot wrapper must reject input above the existing 8 MiB ceiling before extra
SHA work, returning input_size_limit with no extraction. Raw reader entry points
retain the existing Unsupported/ResourceLimit mapping. This adds no shared error
API or new limit; the exact verified_snapshot test includes this expected refusal.
