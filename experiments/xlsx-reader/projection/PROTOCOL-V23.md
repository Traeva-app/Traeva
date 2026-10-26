# Phase 2: actual A helper compatibility

Frozen before executing the revised candidate. The specification change came
from the student's #150/#151 follow-up, not from changing an answer after output.
Main/R2 remains `c6a711d`; F's own branch is now stacked on A's unmerged #151
`68d4e58f7a0bf297d9fa06ed1e1d41a3536fa9a8`. Imports compile the actual helper in
that commit. Its entire reader.rs hash is
`0b14e82a1f9b0e948da06dfa4fd657c64a50630994694fc05470ae38eb635338`.

Internal revision moves to `f-xlsx-projection-candidate-2`; shared identities are
the actual A constants `xlsx-1` / `xlsx-preview-1`, parser profile remains v1.
Phase-1 source/expectations, 14-pass evidence and first build failure stay retained.

Changed expectations authorized by v23: `cell` prefix; `cache · TYPE · not
verified`; `very hidden`; date kind `date`; canonical numeric sheet ID; use actual
A `structure_name` and `escape_source_line`. Source content remains roundtrippable
after LF normalization and source escaping, with output-based scalar coordinates.

Part and Blank/missing states remain proposals in F's comment
https://github.com/P30g53/Traeva/pull/150#issuecomment-6116636392 . They must not be
treated as approved shared syntax. The current A helper returns Unrecognized for
these labels, which is still a generated line and cannot become original evidence.
The known-Sheet compatibility target for a part-bearing label remains a required
pending check; do not call a safely Unrecognized label a recognized Sheet PASS.

First actual #151 compatibility run: 3 PASS / 2 FAIL. One was the old incompatible
part-bearing sheet grammar. The other was F's scalar assertion typo: `\⟦🙂⟧`
contains U+005C, U+27E6, U+1F642, U+27E7, exactly four scalars (not five).
The untouched first log is retained; correcting that oracle is explicitly an
assertion correction, not a Reader fix or a retroactive PASS. The revised part
proposal puts name before part, as requested in the #150 comment; the expected
result is still a known Sheet only after A/B confirm and support that grammar.
