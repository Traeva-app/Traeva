#!/usr/bin/env python3
"""The frozen protocol for the scanning-cost check (R4-PDF-09), written
before any measurement. stress.py reads protocol.json and applies it.

    python3 fixtures/stress/protocol.py   # writes fixtures/stress/protocol.json

Mode: pdf-probe in direct mode (read_pdf in the probe's own process, no
--isolated), one run per input, started by stress.py as a child process
that is killed at the cut-off. A killed run is recorded as a time-out; no
completion time is made up for it. The cut-off is this check's line, not
a product time limit.

Every input is one page that shows TEXT (fixtures/stress/generate.py).
Expected readings:
- S1-S4, S6, S7: Complete, the text "[Page 1]\\nStress control text". The
  extra bytes are a string nobody shows, comments, fragments between the
  objects, or an object nobody refers to: none of them is text.
- S5: Complete while the fragments hold at most 50,000 `stream` keywords
  (the existing stream limit, MAX_STREAMS), else Unsupported with that
  limit's note. Each fragment is 21 bytes.

Revised 2026-10-12, after the runs (stress-02 to stress-05 keep the
original expectation and its four S5 failures): under the stream limit,
S5 is Unsupported with the 128 MiB non-image limit's note when the
potential non-image total under the existing rule is above that limit.
Basis: Codex-R4's independent count from the inputs' bytes and the source
(https://github.com/P30g53/Traeva/pull/148#issuecomment-6134091083), in
R4_S5_POTENTIAL below: every fragment's range is charged separately, as
the existing rule does. The rule, its limit and the counting are
unchanged; the student chose to correct the expectation (2026-10-12).

Criteria, all frozen:
1. Every input returns before the cut-off.
2. Every reading is as expected (status, note, text).
3. For each family, the time to read the 8 MiB input is at most 16 times
   the time for the 1 MiB input (growth in proportion to the size is about
   8 times; growth with its square about 64 times). Times are the probe's
   own `millis` for read_pdf.
Peak memory (the child's maximum resident set) and, when the probe reports
it, the raw-byte check's scanning work are recorded without thresholds.
"""

import json
import sys
from pathlib import Path

import importlib.util  # noqa: E402

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("stress_generate", HERE / "generate.py")
generate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(generate)

TEXT = "[Page 1]\n" + generate.TEXT
STREAM_LIMIT = 50_000
NOTE_STREAMS = f"The PDF has more streams than this reader's limit ({STREAM_LIMIT})."
UNIT_S5 = len(b"1 0 obj << >> stream\n")
DECODED_LIMIT = 128 << 20
NOTE_DECODED = "The PDF's non-image streams expand to more than this reader's limit (128 MiB)."
# Potential non-image bytes under the existing rule, by input size (Codex-R4's
# count, see the revision above). Larger inputs stop at the stream limit first.
R4_S5_POTENTIAL = {
    "64KiB": 103_070_811,
    "128KiB": 410_695_297,
    "256KiB": 1_639_604_652,
    "512KiB": 6_551_540_322,
    "1MiB": 26_192_404_869,
}


def expected(family, size):
    if family == "S5-no-endstream" and size // UNIT_S5 > STREAM_LIMIT:
        return {"status": "unsupported", "note": NOTE_STREAMS, "text": ""}
    if family == "S5-no-endstream" and R4_S5_POTENTIAL[generate.label(size)] > DECODED_LIMIT:
        return {"status": "unsupported", "note": NOTE_DECODED, "text": ""}
    return {"status": "complete", "note": None, "text": TEXT}


def main():
    inputs = {}
    for family, _ in generate.FAMILIES:
        for size in generate.SIZES:
            name = f"{family}-{generate.label(size)}.pdf"
            inputs[name] = {"family": family, "size": size, **expected(family, size)}
    (HERE / "protocol.json").write_text(json.dumps({
        "frozen": "Written before any measurement; changes must state their basis and keep earlier runs.",
        "mode": "pdf-probe direct (no --isolated), one run per input, killed at the cut-off",
        "cutoff_seconds": 5,
        "criteria": [
            "every input returns before the cut-off",
            "every reading is as expected (status, note, text)",
            "per family: millis(8 MiB) <= 16 * millis(1 MiB)",
        ],
        "ratio": {"small": "1MiB", "large": "8MiB", "max": 16},
        "recorded_without_threshold": ["peak resident memory of the child", "scan work, when reported"],
        "inputs": inputs,
    }, indent=1) + "\n")
    print(f"{len(inputs)} inputs in the protocol")


if __name__ == "__main__":
    main()
