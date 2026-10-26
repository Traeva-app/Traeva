# Frozen projection expectations — candidate 1

Frozen before the first candidate execution. Baseline: main
`c6a711d7f3155aee0f30b82eb44aa27cc267e523`, shared contract v22, unchanged parser
`6de78846225a65321089ec5794254407f35d632d`. A/B's v23 grammar remains a proposal.
Tests use only F-created in-memory Stored ZIP packages and typed parser results.
They are not Microsoft Excel saves or R3 independent tests.

Candidate versions: `xlsx-1` (Memory), `xlsx-preview-1` (local preview).
No registered reader, IPC, production DTO, store or model sending is added.

Expected decisions:

1. Memory includes visible worksheets only. Editing a hidden or veryHidden
   cell does not change Memory text. Preview retains those cells and labels the
   exact visibility. Omitted hidden sheets produce a counted actual `left_out`
   entry and Partial. If filtering leaves no text and omissions exist, return
   Unsupported with zero body, rather than an empty Partial.
2. Sheet labels contain declared one-based ordinal, original numeric ID, JSON
   part path and JSON name. A/B must confirm the added part and value-kind grammar.
   Declared sheet order is retained; cells are sorted by one-based row/column.
3. Literal, formula and cache are separate labels and source lines. Number `7`
   and text `7` have different type labels. Formula `B2*2` is not evaluated.
   Cache `14` always has the fixed saved/unverified label. Cache-only edits
   change cache source lines only. Absent/None cache and present Blank cache
   differ as `missing` versus `blank`; neither invents a source value.
4. Source lines beginning with U+27E6 `⟦` or backslash get one leading backslash.
   Leading spaces are retained. CRLF and CR normalize to LF. JSON quotes keep
   names containing quotes, U+27E7 `⟧` or ` · ` unambiguous. Sheet names and paths
   inside generated labels are context, not source evidence. No generated line
   can be produced by a source line, including after truncation.
5. Count Unicode scalars including labels, escapes and separators. Never exceed
   1,000,000. A truncated source prefix remains valid UTF-8, retains escaping,
   is Partial, and reports the remainder. A generated label is never cut into
   an incomplete label. Smaller caps are unit-test-only, not a production knob.
6. Reordering unchanged sheets changes the projection order/ordinal labels;
   it cannot mean whole-file equality. Row/column insertions/deletions/moves
   change A1 context, without a claim of logical-row continuity. Existing raw
   coordinate_changes is not used as a production Memory comparer.
7. Actual parser omissions are summarized by code/count, without user content
   or part names in notes. Present merged ranges, style indices or unprojected
   formula metadata are actual omissions; absent features are not listed.
   Parse Failed/Unsupported/ResourceLimit => no prefix/body, zero chars, empty
   covered/left_out, fixed reason and exact rejection code. Limits stay unchanged.
8. A supplied previously verified SnapshotRef and bytes must agree on length
   and SHA-256 before projection. Return the original nine fields unchanged.
   A mismatching pair is an error with no extraction, never an empty success.
   This does not replace core UUID/store/authorization/UsablePair validation.

Keep every first build/test failure separately, fix code when needed, and do not
change these expectations to match observed output. The original nine Reader
failures and prior Excel records are referenced, not rerun or overwritten.
