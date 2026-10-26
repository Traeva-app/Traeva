#!/usr/bin/env python3
"""Frozen expectations for R4's third-round reproductions (fixtures/defects3/).

Written before any fix, from each fixture's own source (generate.py), the
output rules in README.md, and these rules for R4-PDF-08:

- A stream without filters is held to the same per-stream limit as a
  decoded one (16 MiB for a stream that is not an image): over it, the
  reading stops, Unsupported. Exactly 16 MiB is allowed.
- The range of every stream hayro reads must be proven: hayro's /Length
  (direct, or an object it resolves, also inside an object stream) must be
  exactly the data hayro took, with only white space and then `endstream`
  after it. A missing /Length, or one that does not end there (hayro then
  falls back to the first `endstream` in the data), stops the reading:
  Failed, the stream's extent is unknown. A `endstream` inside the data is
  never taken as its end when /Length says otherwise.
- A dictionary that names filters hayro can decode but has no stream data
  of its own: hayro, asked for a stream there, takes the data after the
  next `stream` keyword in the file and decodes it with that dictionary's
  filters. Such a dictionary stops the reading (Unsupported). When it is an
  object stream or a cross-reference stream, hayro decodes that data while
  it opens the file, so the raw-byte check measures exactly that data
  first, under the usual limits.
- Limits that already held (compressed bombs, also with a wrong /Length,
  inside other data, or with cross-reference data that hayro rebuilds)
  still hold.

Re-frozen on 2026-10-12 (G4), before any fix, with its own basis: the
second rule above is withdrawn. The student decided that a stream's range
follows hayro 0.8.0's own rule (`parse_proper`, else `parse_fallback`, in
hayro-syntax src/object/stream.rs) and that no "wrong /Length means Failed"
rule is added. g14-g17 are therefore expected to read the data hayro's
rule gives. The basis is range_basis.py / range_basis.json: hayro's rule
written again in Python, and pypdf reading the same objects. For g14-g17
both readings agree on where the data starts, hayro's range is pypdf's
data without its trailing white space, and pypdf's page text is the text
below (none for g17, whose range holds no text, so the document has no
text: Unsupported). G's first expectations and their failures on
pdf-experiment-5 (evidence/round6/, run 15) stay in the history.

    python3 fixtures/defects3/expectations.py   # writes fixtures/defects3/expected.json
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from expectations import joined, page_offsets  # noqa: E402

BOMB = "A stream expands beyond this reader's limit of 16 MiB."
RAW_TOO_LARGE = "A stream's data is larger than this reader's limit of 16 MiB."
NO_DATA = "A dictionary in the PDF names stream filters but has no stream data."
NO_TEXT = "The PDF has no text to read: its pages draw no text."


def text(pages):
    case = {"status": "complete", "text": joined(pages)}
    case["chars"] = len(case["text"])
    case["pages"] = {str(n): {"start": s, "end": e} for n, (s, e) in enumerate(page_offsets(pages), start=1)}
    case["left_out"] = []
    case["note"] = None
    return case


def stopped(status, note):
    return {"status": status, "note": note, "text": "", "chars": 0, "left_out": []}


# The student decided (2026-10-12) that g25-g27 are marked unverified: on
# pdf-experiment-5 (run 15) none of them reaches the path it was written
# for, so they reproduce nothing. Their first expectations stay as frozen;
# validate.py records their result and checks the shared contract on it,
# but does not count them as PASS or FAIL.
UNVERIFIED = ("unverified: the construction does not reach the path it was written for "
              "(run 15 on pdf-experiment-5), so it reproduces nothing; the result is recorded, not judged")


def unverified(case):
    return {**case, "unverified": UNVERIFIED}


def cases():
    found = {
        "g01-contents-raw-under-limit.pdf": ("control", text([["Contents under the limit"]])),
        "g02-contents-raw-at-limit.pdf": ("control", text([["Contents at the limit"]])),
        "g03-contents-raw-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g04-contents-raw-17mib-early-endstream.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g05-form-raw-under-limit.pdf": ("control", text([["Page text", "Form under the limit"]])),
        "g06-form-raw-at-limit.pdf": ("control", text([["Page text", "Form at the limit"]])),
        "g07-form-raw-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g08-form-raw-17mib-early-endstream.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g09-indirect-length-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g10-indirect-length-under-limit-early-endstream.pdf": ("control", text([["Indirect length under the limit"]])),
        "g11-length-in-objstm-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g12-length-in-objstm-under-limit.pdf": ("control", text([["Length in an object stream under the limit"]])),
        "g13-matching-length-endstream-comment.pdf": ("control", text([["Matching length"]])),
        "g14-length-too-short.pdf": ("range", text([["Length too short"]])),
        "g15-length-too-long.pdf": ("range", text([["Length too long"]])),
        "g16-length-missing.pdf": ("range", text([["No length"]])),
        "g17-wrong-length-17mib-early-endstream.pdf": ("range", stopped("unsupported", NO_TEXT)),
        "g18-flate-bomb-wrong-length.pdf": ("control", stopped("unsupported", BOMB)),
        "g19-objstm-bomb-in-image-data.pdf": ("control", stopped("unsupported", BOMB)),
        "g20-objstm-raw-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g21-rebuilt-xref-raw-over-limit.pdf": ("defect", stopped("unsupported", RAW_TOO_LARGE)),
        "g22-rebuilt-xref-flate-bomb.pdf": ("control", stopped("unsupported", BOMB)),
        "g23-rebuilt-xref-control.pdf": ("control", text([["Rebuilt control"]])),
        "g24-unbroken-xref-control.pdf": ("control", text([["Rebuilt control"]])),
        "g25-dictionary-read-as-stream.pdf": ("related", unverified(stopped("unsupported", NO_DATA))),
        "g26-xref-stream-without-data.pdf": ("related", unverified(stopped("unsupported", BOMB))),
        "g27-objstm-without-data.pdf": ("related", unverified(stopped("unsupported", BOMB))),
    }
    out = {}
    for name, (role, case) in sorted(found.items()):
        out[name] = {"path": f"defects3/generated/{name}", "kind": "defect-reproduction",
                     "r4_finding": "R4-PDF-08", "role": role, **case}
    return out


def main():
    (HERE / "expected.json").write_text(json.dumps({
        "frozen": "Written before any fix; changes must state their basis. g14-g17 re-frozen "
                  "before any fix from range_basis.json (hayro 0.8.0's range rule and pypdf).",
        "source": "Codex-R4's third-round description of R4-PDF-08 (its inputs t05-t07 were not seen)",
        "roles": {
            "defect": "fails as R4 describes before the fix (a stream over 16 MiB without filters is read)",
            "control": "the rule that must hold before and after the fix",
            "range": "hayro reads the data its own rule gives (a /Length that does not end at "
                     "endstream falls back to the first endstream); re-frozen from range_basis.json",
            "related": "a dictionary without data that hayro reads as a stream: data the raw-byte check "
                       "never measured with that dictionary's filters",
        },
        "fixtures": cases(),
    }, indent=1, ensure_ascii=False) + "\n")
    print("third-round defect expectations written")


if __name__ == "__main__":
    main()
