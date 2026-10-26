# Round 6 handoff — PDF Reader defensive hardening (#28 isolated experiment)

> **Status (2026-10-12, session G4):** implemented as `pdf-experiment-7`; see README "Round 6" and `evidence/dev-log.md` entries 20-28. g14-g17 were re-frozen from hayro's rule and pypdf (`fixtures/defects3/range_basis.*`), g25-g27 are marked unverified. This note is kept as the handoff record and is not updated further.

This branch carries **unfinished round-6 work only**. No fix is implemented. It
is a handoff for the local session **G3**, who takes the round forward. Nothing
here is merged, no PR is opened, and #148 / #158 are untouched.

- **Reviewed base (unchanged):** `#148 = c3a28cf39f5dee5d83926f9730f73936bbcc41be` (reader `pdf-experiment-5`), `#158 = f47d9b3fe0767bbed2188445737c27065b3251dd` (adds the quality diagnostics on top of the same base).
- **Scope the student set for the round:** fix **PDF-08**, **PDF-09**, and fold in **N1, N2, N3** as minimal resource fixes; **N4 deferred** unless a concrete bypass is shown. PDF-09 to be made linear **without a scan budget** this round (a budget stays a documented candidate only). Keep the isolated-experiment scope. Give controllable paths a clear boundary; list hayro-internal paths honestly and do **not** claim `read_pdf` is fully bounded. R4 does the independent verdict.
- **This branch stops at:** reproductions + frozen expectations for PDF-08 (`fixtures/defects3/`), the PDF-09 stress harness (`fixtures/stress/` + `stress.py`), unfixed-v5 run records (`evidence/round6/`), repro inputs for N1/N2 (`evidence/round6/repro/`), and this note. **The fix itself is not started.**

## Measurement environment (all numbers below)

`Linux-6.18.44-x86_64, glibc 2.39`, `rustc 1.98.1`, release build, in-process
`read_pdf` via `pdf-probe` unless stated. Reader binary = `pdf-experiment-5`
built from `c3a28cf` (developer binary; sha in the evidence files). hayro
pinned at `hayro-syntax`/`hayro-interpret` **=0.8.0**, `flate2 =1.1.10`
(`rust_backend`). Times are developer self-tests on one machine, **not** a
product budget and **not** an independent Codex-R4 result. Source line numbers:
the experiment under `experiments/pdf-reader/src/`; hayro under the crates.io
checkout of `hayro-syntax-0.8.0` / `hayro-interpret-0.8.0`.

Each item below is written at one level: **invariant violated · hayro
location · measurement · coverage & whether it reproduced on v5 · fix
direction.** Construction detail lives only in the generators (`fixtures/…`,
`evidence/round6/repro/…`), never restated here.

---

## PDF-08 — real stream range and the per-stream cap

**Invariant violated.** "Every non-image stream decodes to ≤ 16 MiB
(`MAX_STREAM_BYTES`), measured in preflight before hayro parses" (README
resource table; `lib.rs:47`). An **unfiltered** stream is never compared to the
cap, and preflight never proves *where the stream ends* — only where its
dictionary and data *start*.

**Where it goes wrong (experiment).**
- `preflight.rs:266-277`: `cap` is computed but only passed into `decode_chain`; with no filters the size is `memmem::find(data, b"endstream")` (the **first** `endstream`), never compared to `cap`. It only feeds the 256/128 MiB `work`/`decoded_total` totals (`:278-293`).
- `document.rs:259-278` (`stream_check`): builds `StreamAt{dict,data}` from pointer offsets and ignores `raw_data().len()`; it checks `start ≤ len`, never `start+len`.
- `document.rs:361-388` (`content_size`): sums `check.decoded` against `MAX_PAGE_CONTENT_BYTES` (32 MiB) only; a Form XObject gets **no** size check.

**Where hayro's true range comes from (hayro 0.8.0).**
- `object/stream.rs:299-312` `parse_proper`: data starts after `stream` + exactly one EOL; end = start + `/Length` bytes, then whitespace-only (no comments), then `endstream`. The payload may itself contain the bytes `endstream`.
- `object/stream.rs:314-348` `parse_fallback` (any proper failure): first `stream`+EOL after `>>`, to the **first** `endstream`, trailing whitespace trimmed.
- `object/stream.rs:115-138`: for unencrypted data `raw_data()` is `Cow::Borrowed` of exactly the slice hayro decodes.
- `/Length` is resolved through the stream's own ctx (`object/ref.rs:95-100`); a dummy ctx never resolves it (`xref.rs:532-534`) so xref-stream / repair paths always take the fallback range; the value may live in an object stream (`xref.rs:581-597`).

**Measurement (v5, `evidence/round6/v5-defects3-c3a28cf.json`).** The true
PDF-08 defects read a 17 MiB unfiltered stream as **Complete** instead of
refusing: `g03, g04, g07, g08, g09, g11, g20, g21`. Controls under/at the limit
(`g01, g02, g05, g06, g10, g12, g13, g23, g24`) read as text; compressed bombs
(`g18, g19, g22`) already refuse.

**Coverage & reproduction.** `fixtures/defects3/` (`generate.py`,
`expectations.py` → `expected.json`, `manifest.json`). Covered: Form and page
Contents; under / at / over the cap; direct and indirect `/Length`; a `/Length`
whose value lives in an object stream; a fake `endstream` at the start of the
payload; wrong `/Length`; Flate bombs; rebuilt cross-reference data. **The 8
named fixtures reproduce the defect on v5.** See the expectation corrections at
the end — some frozen `expected.json` rows are wrong and must be re-frozen from
hayro's real behaviour before the fix.

**Fix direction.** Model hayro's range exactly: parse a **direct** `/Length`
with hayro's own grammar and accept it only when whitespace-then-`endstream`
sits at `start+L`; otherwise take the fallback range (first `endstream`, trim
trailing whitespace). Size an unfiltered stream by that range length and apply
`MAX_STREAM_BYTES` as a **hard stop** for the file's own streams. In
`document.rs` compute `end = offset(data) + data.len()`, require a preflight
record made for exactly `[start, end)`, and stop with an explicit reason when it
cannot be proven (never widen to the last `endstream` in the file — that
re-refuses legal files). Re-run the range and content checks on every reopened
`Pdf`.

---

## PDF-09 — quadratic preflight scanning on legal files

**Invariant violated.** Preflight is meant to be bounded work on the raw bytes;
in practice every candidate re-scans toward the end of the file, so cost grows
with the square of the input — on **legal** comment-heavy files, not only
adversarial ones. (There is deliberately no wall-clock limit, to keep "same
bytes → same result"; the only bound is work, and it is quadratic.)

**Where it goes wrong (experiment).** Each is an unbounded forward scan repeated
once per candidate:
- `preflight.rs:319-325` `candidate_dicts` and `:417-435` `length_objects` each call `skip_white` per `obj` hit; `skip_white` (`:935-947`) swallows a whole run of consecutive comment lines.
- `preflight.rs:203-216` runs `read_dict` per candidate; `skip_object` (`:966-990`) walks a literal string byte-by-byte to the matching `)` or EOF with an unbounded nesting counter; hex strings use `memchr` to the next `>` (`:1002`).
- On a failed dictionary, `stream_follows` (`:529-533`) and `names_decoded_early` (`:452-456`) make several `memmem` passes forward.
- `preflight.rs:276`: per-candidate `memmem` to the next `endstream` for the unfiltered size.

**Measurement (v5, `evidence/stress/stress-01-c3a28cf-dirty.json`; cut-off 5 s).**
Quadratic families (≈4× time per 2× size, then time-out):

| family (64 / 128 / 256 / 512 KiB) | ms | ms | ms | 512 KiB |
|---|---|---|---|---|
| S2 comment-stress (R4's legal case) | 186 | 867 | 3227 | time-out |
| S3 open-literal | 393 | 1480 | time-out | time-out |
| S4 open-hex | 40 | 102 | 429 | 1696 |
| S6 comment-line | 425 | 1464 | time-out | time-out |

Linear control **S1 big-string** 1 / 2 / 6 / 9 ms (8 MiB = 143 ms); **S5
no-endstream** bounded by `MAX_STREAMS`. This matches R4's reported shape (their
comment family 0.10 / 0.38 / 1.49 s; absolute times differ by machine).

**Coverage & reproduction.** `fixtures/stress/` (`generate.py`, `protocol.py` →
`protocol.json`, `manifest.json`) and `stress.py` (bounded subprocess, time-out
recorded as time-out, never a fabricated completion time). **Reproduces on v5.**

**Fix direction (student's choice: linear, no budget this round).** Precompute
once: comment-run ends; literal-string ends with an escape-aware depth pass
(sound because every scan resynchronises one byte after the starting `(`); next
`>` positions; sorted position tables for `endobj`/`stream`/`endstream`/`ObjStm`/
`XRef`. Answer each candidate in O(1)/O(log n) and charge decode work by input
consumed and every layer's output, not just the final size. Keep every
candidate hayro could decode early in scope — do not trade a skipped check for
speed. A deterministic size-proportional **scan-work budget** (c·len + k, stop →
`Unsupported`/`resource_limit`, no text) is written up as a **candidate only**,
to be confirmed by the student; it is **not** to ship as a product rule, and the
5 s review cut-off is not a product limit.

---

## N1 — predictor allocation abort (reproduced)

**Invariant violated.** Process-internal allocation is unbounded and a tiny file
can force a multi-GB allocation that aborts the process; `catch_unwind` cannot
catch an allocator abort. Preflight ignores `/DecodeParms` entirely.

**Where it goes wrong (hayro).** `filter/lzw_flate.rs:711-751`
`PredictorParams::from_params` reads `/Predictor /Columns /Colors
/BitsPerComponent` from `/DecodeParms`; `apply_predictor` allocates a row buffer
sized `columns × colors × bpc` **independent of the stream's real (small)
decoded length**. The experiment's `read_dict` (`preflight.rs:591-626`) only
reads a fixed key set and never looks at `/DecodeParms`.

**Measurement (v5).** A **737-byte** file whose only content stream carries a
huge `/Columns` → `memory allocation of 34359738368 bytes failed` in
`hayro_syntax::filter::lzw_flate::apply_predictor` → **SIGABRT (exit 134)**. The
in-process `read_pdf` aborts the whole process; only the `--isolated` subprocess
kill contains it. Record: `evidence/round6/v5-n1-n2-c3a28cf.json`; input
generator `evidence/round6/repro/predictor_abort.py`.

**Coverage & reproduction.** No frozen `g` fixture yet — reproduced from the
committed generator. Reproduces on v5.

**Fix direction.** In preflight, read `/DecodeParms` (`/Columns /Colors
/BitsPerComponent /Predictor`) for Flate/LZW streams and refuse when the row
allocation implied by those parameters exceeds the per-stream cap, before hayro
is given the file. Controllable path with a clear bound.

---

## N2 — hayro's own super-linear work on a direct `read_pdf` (partly reproduced)

**Invariant violated.** Making preflight linear does not bound `read_pdf`: hayro
itself has super-linear paths that run on a direct call. R4 asked specifically
for this to be shown, not papered over by the `--isolated` kill.

**Where it goes wrong (hayro).**
- **ObjStm members, reproduced:** `xref.rs:581-597` resolves an object-stream member by building `ObjectStream::new` **per fetch**; `ObjectStream::new` (`xref.rs:1044-1077`) parses all `/N` offset pairs each time. `objects()` (`xref.rs:422-466`) iterates every member, so the experiment's `object_facts` (two `pdf.objects()` passes, `document.rs:233`) is O(members²).
- **Repair-scan literal string, not reproduced through the reader:** `fallback_xref_map_inner` (`xref.rs:72-176`) token-walks the whole file and runs `skip_literal` (`object/string.rs:167-185`) to EOF on an unbalanced `(`.

**Measurement (v5, `evidence/round6/v5-n1-n2-c3a28cf.json`).** ObjStm via the
probe: **0.09 / 1.57 / 24.49 s** for **1k / 4k / 16k** members (≈16× per 4× ⇒
O(n²)); **64k > 120 s**. Repair-scan: measured quadratic only in a **hayro-only**
harness (direct `Pdf::new`, a 1 MiB broken-startxref input ≈ 57 s). **Through the
reader it did not reproduce:** `preflight check_end` (`preflight.rs:104-120`)
refuses a broken/zero `startxref` before hayro runs, and a preflight-passing
construction that still reaches the repair scan was not built.

**Coverage & reproduction.** ObjStm generator
`evidence/round6/repro/hayro_cost_inputs.py` (reproduces). No frozen `g` fixture
yet. Repair-scan deliberately not emitted (the construction is gated out by
`check_end`).

**Fix direction & boundary.** Controllable: bound the **members of an object
stream** in preflight (a cap on `/N`, or on total ObjStm members) so the O(n²)
iteration cannot blow up; this is the reachable path. Not controllable from the
experiment: hayro's internal repair-scan quadratic — list it honestly as a
hayro-internal residual, do **not** claim `read_pdf` is fully bounded. The
repair scan's reachability through the reader is **unconfirmed** and is the
first thing G3 should settle before claiming either way.

---

## N3 — streams hayro decodes that preflight never measured (hole real; chosen reproductions missed)

**Invariant violated.** "Every stream hayro can decode has been measured in
preflight" (`preflight.rs:7-12`, `315-318`). hayro picks streams to decode by
**role**, not by matching preflight's `obj`+`<<`+`stream` candidate rule, so
several decode paths are never measured.

**Where it goes wrong.**
- **Pseudo-streams (draw time):** `Dict::get::<Stream>` on a dictionary-only object (or an inline value) makes hayro search for the next `stream` keyword and decode *another* object's data with **this** dictionary's filters. Reached from `hayro-interpret` at draw time with no cap: XObjects (`x_object/form.rs:26`, re-decoded on every `Do`), fonts (`font/…`), `ToUnicode` (`font/mod.rs:677`), functions/shadings/patterns/ICC/soft-masks. The experiment's `object_facts` sees these as `Object::Dict`, so `stream_check` never runs on them.
- **Keyword-less / early-decode containers (open time):** a `/Type /XRef` or `/Type /ObjStm` dictionary with no `stream` keyword is decoded while hayro opens the file (`xref.rs:846-870`, `data.rs:93-110`, repair scan `xref.rs:90-137`). `section_trailer` (`preflight.rs:189-191`) accepts a keyword-less `/Type /XRef` trailer but `scan_streams` (`:218`) skips anything not followed by `stream`.

**Measurement (v5).** The hole is confirmed by code audit. **But the three
`g25–g27` fixtures built for it did not hit the target path** (see corrections
below): `g25` → Complete, `g26` → Complete, `g27` → Failed (structure). They do
**not** demonstrate a catastrophic decode on v5, because the payloads chosen
decode to ~17 MiB (annoying, not a DoS) or the container is rejected first.

**Coverage & reproduction.** `g25–g27` in `fixtures/defects3/` — **weak, do not
reproduce a danger.** Treat N3 as *not yet reproduced as a DoS*.

**Fix direction & boundary (minimal).** Controllable: make
damage/unmeasurable/image-codec findings on streams hayro decodes **at open**
(keyword-less `/Type /XRef` and `/Type /ObjStm`) a **hard stop** in preflight,
and apply the proven per-stream cap wherever preflight can see a candidate. Not
fully controllable: the draw-time pseudo-stream cross-filter decode spans every
`get::<Stream>` site in hayro-interpret; the per-stream cap bounds a *single*
such decode once PDF-08 lands, but totals/repeats are not bounded from the
experiment. List this as a residual with its boundary; do not attempt a full
mechanism. **First task for G3: build a genuine reproduction** (a payload that
actually expands to GBs or runs unbounded through one of these paths) before
freezing any N3 expectation.

---

## N4 — startxref-chain choice & encryption via repair (deferred)

**Invariant at risk.** Preflight reads `/Encrypt` and `/Size` only on the chain
it follows; hayro may follow a different chain.

**Where they differ.** hayro's `find_last_xref_pos` (`xref.rs:619-627`) uses the
**last `startxref` anywhere** in the file and allows comments before the offset;
preflight's `check_end` (`preflight.rs:104-120`) uses the last `%%EOF` in the
final 2048 bytes, then the `startxref` before it, whitespace only. A trailing
`startxref` after the last `%%EOF` could send hayro down a chain whose `/Encrypt`
/ `/Size` preflight never read. Encryption can also be discovered by hayro's
repair (`xref.rs:236-256`), which preflight refuses only on its own chain
(`:86-88`, `:221-222`).

**Status.** **Deferred per the student.** No concrete bypass of the encryption
refusal or a resource limit was reproduced in this round, so there is no
counter-example to attach. If G3 reproduces one, the student's instruction is a
**minimal gap fix** (align preflight's chain choice with hayro's; refuse
`/Encrypt` found via repair), **not** a mechanism rewrite.

---

## Frozen-expectation corrections (must re-freeze before the fix)

`fixtures/defects3/expected.json` was frozen from the audit's reading of hayro,
**before** running v5. The v5 run (`evidence/round6/v5-defects3-c3a28cf.json`)
shows some rows do not match hayro's real behaviour. They are **left as-is on
this branch** (frozen artifacts, not edited after the fact); G3 must re-freeze
them from real behaviour before implementing, or the fix would wrongly refuse
legal files — exactly what R4 warned against.

- **`g14` (Length too short), `g15` (too long), `g16` (missing):** frozen as
  `Failed / LENGTH_MISMATCH`. **Real v5 behaviour: Complete, small text** (25 /
  24 / 18 chars). When the direct `/Length` does not land on `endstream`, hayro
  falls back to the first `endstream` and reads the correct small range. The fix
  must **not** refuse these; the right expectation is the fallback read.
- **`g17` (wrong Length + early fake `endstream`, 17 MiB):** frozen as `Failed`.
  **Real v5 behaviour: `Unsupported`, "no text"** — hayro falls back to the fake
  `endstream` near offset 2 and reads ~2 bytes (a comment), so the page has no
  text. It does **not** read 17 MiB here; the 17 MiB over-read happens only when
  `/Length` is correct and large (`g03/g04`) or the fallback reaches a far
  `endstream` (`g07`).
- **`g25–g27` (the N3 class):** frozen as `Unsupported` (NO_DATA / BOMB). **Real
  v5: `g25` Complete, `g26` Complete, `g27` Failed (structure).** The chosen
  constructions do not reach the pseudo-stream / keyword-less decode as a
  danger. Rebuild these as genuine reproductions (see N3) before freezing.

The true PDF-08 defects (`g03, g04, g07, g08, g09, g11, g20, g21`) and the
controls are correct as frozen and reproduce.

---

## What is on this branch

- `fixtures/defects3/` — PDF-08 reproductions + frozen expectations + manifest (generated PDFs are git-ignored; rebuild with `python3 fixtures/defects3/generate.py`).
- `fixtures/stress/` + `stress.py` — PDF-09 stress harness and frozen protocol.
- `validate.py`, `.gitignore` — WIP edits that reference the new fixture sets.
- `evidence/round6/` — unfixed-v5 run records (`v5-defects3-*.json`, `-summary.txt`, `v5-n1-n2-*.json`) and repro generators under `repro/`.
- `evidence/stress/stress-01-*.json` — the v5 stress run above.

Not done (for G3): the fix itself (no reader change; `pdf-experiment-5` is
unchanged), the re-freeze of the corrected expectations, genuine N1/N2/N3
fixtures folded into the frozen sets, and any change to #148 / #158.
