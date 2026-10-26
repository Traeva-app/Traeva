# PDF reader fixtures

Normal, synthetic PDFs for the PDF reader in the core (`src/pdf.rs`, #186).
No personal documents, sealed evaluation material or model calls were used.
They come unchanged from the reviewed experiment (`experiments/pdf-reader`,
PR #148 at `81ddd80`): `p02`, `p03` and `p08` are outputs of its generator
(not committed there; their SHA-256 equal the ones in its
`fixtures/manifest.json`), and `e03a` is committed there as it is here.

| File | From | Reading |
|---|---|---|
| `p02-multi-en.pdf` | the experiment's generator (own PDF writer) | three pages of English text: Complete |
| `p03-cjk-reportlab.pdf` | the experiment's generator (reportlab, non-embedded and embedded CJK fonts) | two pages of Chinese text: Complete |
| `p08-scanned-images.pdf` | the experiment's generator | two pages that are only images, like a scan: Unsupported, no text layer |
| `e03a-libreoffice-open-password.pdf` | LibreOffice 24.2.7.2 export with an open password | Unsupported, encrypted |

`expected.json` holds each file's sha256, the summary and the text as the
experiment read them (`pdf-experiment-9`, probe sha256 `de47dbf3…`), with the
reader renamed `pdf-1`. Status, character count, note and text also match the
experiment's frozen expectations. `src/pdf_tests.rs` checks that the core
reads each file exactly so.

The experiment keeps its other inputs (damaged, resource-limit, boundary and
scanning-cost inputs) and the tests that use them; Codex-R4 verifies with
those there. They are not copied into the core. The experiment is on PR #148's
branch (`claude/optimistic-allen-nazsq7`), not on main.

`relocate.py` writes the core's `pdf*.rs` from the experiment's source at
`81ddd80`, or with `--check` shows that the core files are exactly that move
(see `docs/features/pdf-reader-core.md`).
