# XLSX projection grammar aligned with A/B v23

This unregistered candidate imports the actual A helpers from #151
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`. A/B decided the remaining details in
[#150](https://github.com/P30g53/Traeva/pull/150#issuecomment-6116720334).
The dependencies remain unmerged; this is not a published production contract.

The shared identities are A's `xlsx-1` / `xlsx-preview-1` constants. The parser
profile remains `traeva-xlsx-experiment-v1`; F's internal revision is separately
`f-xlsx-projection-candidate-3`. Phase-1/2 outputs are retained history, not the
current syntax. Later published output changes require the applicable Reader
identity to change, so B does not reuse an older comparison/quote cache.

```text
⟦sheet 1 · id 1 · name "Data"⟧
⟦cell B2 · literal · number⟧
7
⟦cell C2 · formula⟧
B2*2
⟦cell C2 · cache · number · not verified⟧
14
```

Generated labels start at column zero with `⟦` (U+27E6), end with `⟧` (U+27E7),
and occupy a whole line. The separator is exactly ` · `, with one space on each
side of U+00B7. A's structure_line does not trim. Both known and Unrecognized
structure are generated context, never original evidence. The four Reader gate
is mandatory: TXT/MD/DOCX user lines starting with these characters remain source.

A's structure_name returns a JSON string with quotes, backslashes, control
characters, `⟦`, `⟧`, and `·` escaped as `\uXXXX`; non-BMP source roundtrips.
Neither a field separator nor a closing bracket can be injected by a sheet name.
Part remains in the internal typed Workbook, by A's explicit decision. Public
text locations stop at sheet ordinal/ID/name, A1 and field/type; they are not ZIP
byte or XML node offsets. The ordinal is the original declaration position, so
Memory can have gaps after hidden filtering. IDs normalize to the validated
positive integer, and are package identity, not global or permanent identity.

Preview sheet labels append ` · hidden` or ` · very hidden`. Memory filters those
sheets before text and counts the actual omission in left_out; it includes no
hidden names/parts/cell values in its filtering explanation. A valid preview
containing original hidden data can still be read. If there are zero retained
original scalars (including an otherwise empty visible sheet), return Unsupported
with zero body/covered/left_out and a fixed note. Structure cannot manufacture
source text to satisfy Partial.

Cells sort by numeric row/column. Fields appear in literal/formula/cache order.
Literal/cache types are number, text, boolean, error, date; raw number/date tokens
remain strings, and booleans project as true/false. Blank literal emits no field.
Formula uses saved text with no added equals sign or evaluation. Missing shared
formula text is not expanded; original attributes stay in Workbook and actual
omissions propagate. The formula label can have no source text in that case.

Cache always ends in ` · not verified`, meaning freshness=unverified regardless
of saved calculation properties. Blank cache uses type blank, absent cache uses
missing, neither followed by a source line. An empty saved string uses text and
has an empty source line. A deliberately inconsistent cache is preserved rather
than recomputed. Cache-only change is not a formula edit or verified calculation.

A's escape_source_line is applied to each LF-normalized source line: lines
beginning with `⟦` or `\` gain exactly one `\`; spaces at the start remain.
Original CRLF/CR normalize to LF. An original trailing LF retains its final empty
source line, followed by the projection separator LF. A/B can undo the added
escape for display, while QuoteAt uses **output** scalar offsets including the
escape. Generated label content (even the original name embedded in a label)
cannot supply an original-text quote or evidence word.

The 1,000,000-scalar bound includes labels/escapes/LFs. Labels append atomically;
an escaped source prefix cannot be split before its first source scalar. A
truncated body is a valid UTF-8 prefix and Partial with actual remainder noted.
Parser budgets are unchanged. Original scalar presence is tracked separately
from generated labels, escapes and separator LFs.

Actual Reader findings are aggregated by code/count, without source content in
notes. Present merges/styles/formula attributes are reported as omitted by this
projection, not interpreted. Absent unsupported features are not fabricated as
left_out. Failed/Unsupported/ResourceLimit discard all body and preserve a fixed
rejection code. The local snapshot wrapper rechecks supplied length/hash and
returns the original nine fields; its mismatch strings are not new IPC errors. It rejects input over the existing
8 MiB ceiling before extra hash work (local input_size_limit error).
A must call read_snapshot first and preserve the existing store/AppError boundary.

B owns comparison: ordinal-only reorder must be assessed by package IDs, with
names/content checked. Name change is not pure reorder. A1 insert/delete/move
has no inferred logical-row identity. F adds no differ, semantic/format engine,
neighbor-cell UI, prompt, consent or sending pathway. All UsablePair and newer
file-specific consent checks remain necessary for future approved wiring.
