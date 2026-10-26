#!/usr/bin/env python3
"""Frozen expectations for R4's second-round reproductions (fixtures/defects2/).

Written before any fix, from each fixture's own source (generate.py), the
output rules in README.md, and these decisions:

- R4-PDF-06: bytes inside a string, a hex string, a comment or another
  stream's data are not objects, so they never decide the status. Real
  streams are still checked: limits, damage and unknown filters still stop
  the reading. The size limits also hold for anything that merely looks like
  a stream (f14): they are never loosened to avoid a refusal.
  Known limitation, frozen as such (f10): text that looks like an object
  stream (/Type /ObjStm) with a filter this reader cannot measure is still
  refused, because the PDF library decodes object streams it finds while it
  rebuilds a broken cross-reference table, before any check can run.
- R4-PDF-07: /Contents that resolves to null (written directly, or through a
  reference to an object that is null, also one inside an object stream) is
  a page without content. A reference to an object that does not exist, a
  value of another type, and any member of a /Contents array that is not a
  stream (null included: the rule that null means "absent" is for
  dictionary values, not array members) still make the reading Failed.
  Blank pages follow the existing rule: a file with no text is Unsupported.
- f24: a comment between an object's numbers and `obj` is white space (ISO
  32000 7.2.4); pypdf's strict mode refuses it, hayro reads it.

    python3 fixtures/defects2/expectations.py   # writes fixtures/defects2/expected.json
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from expectations import NOT_ALL, joined, page_offsets  # noqa: E402

import importlib.util  # noqa: E402

_spec = importlib.util.spec_from_file_location("defects2_generate", HERE / "generate.py")
_gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_gen)

BOMB = "A stream expands beyond this reader's limit of 16 MiB."
DAMAGED = "A compressed stream in the PDF is damaged."
CONTENT_DAMAGED = ("A page's content list is damaged: it refers to an object that is missing "
                   "or is not a content stream.")
UNKNOWN_FILTER = "A stream uses a filter this reader does not know."
LZW = "The PDF uses legacy LZW compression, which this experiment does not read."
NO_TEXT = "The PDF has no text to read: its pages draw no text."
IMAGES = "images on page 1 (1 in all; images are not read, no OCR)"


def text(pages, left_out=(), kinds=()):
    case = {"status": "partial" if left_out else "complete", "text": joined(pages)}
    case["chars"] = len(case["text"])
    case["pages"] = {str(n): {"start": s, "end": e} for n, (s, e) in enumerate(page_offsets(pages), start=1)}
    case["left_out"] = list(left_out)
    case["note"] = (NOT_ALL + ", ".join(kinds) + ".") if left_out else None
    return case


def stopped(status, note):
    return {"status": status, "note": note, "text": "", "chars": 0, "left_out": []}


def cases():
    found = {
        # R4-PDF-06
        "f01-listing-in-text.pdf": ("R4-PDF-06", "defect", text([[_gen.LISTING]])),
        "f02-listing-in-file-comment.pdf": ("R4-PDF-06", "defect", text([["Comment control"]])),
        "f03-flate-listing-in-text.pdf": ("R4-PDF-06", "defect", text([[_gen.LISTING_FLATE]])),
        "f04-listing-in-content-comment.pdf": ("R4-PDF-06", "defect", text([["Content comment control"]])),
        "f05-listing-in-strings.pdf": ("R4-PDF-06", "defect", text([["String control"]])),
        "f06-lzw-listing-in-image-data.pdf": ("R4-PDF-06", "defect", text(
            [["Image payload control"]], [IMAGES], ["images"])),
        "f07-endstream-in-text.pdf": ("R4-PDF-06", "defect", text([[_gen.ENDSTREAM_TEXT]])),
        "f08-endstream-in-text-indirect-length.pdf": ("R4-PDF-06", "defect", text([[_gen.ENDSTREAM_TEXT]])),
        "f09-objstm-listing-in-text.pdf": ("R4-PDF-06", "defect", text([[_gen.OBJSTM_LISTING]])),
        "f10-objstm-lzw-listing-in-text.pdf": ("R4-PDF-06", "known-limitation", stopped("unsupported", LZW)),
        "f11-listing-and-real-bomb.pdf": ("R4-PDF-06", "control", stopped("unsupported", BOMB)),
        "f12-damaged-part-and-listing.pdf": ("R4-PDF-06", "control", stopped("failed", DAMAGED)),
        "f13-real-unknown-filter.pdf": ("R4-PDF-06", "control", stopped("unsupported", UNKNOWN_FILTER)),
        "f14-bomb-listing-in-image-data.pdf": ("R4-PDF-06", "limit-control", stopped("unsupported", BOMB)),
        "f24-header-with-comment.pdf": ("R4-PDF-06", "related", text([["Header comment control"]])),
        # R4-PDF-07
        "f15-contents-direct-null.pdf": ("R4-PDF-07", "control", stopped("unsupported", NO_TEXT)),
        "f16-contents-indirect-null.pdf": ("R4-PDF-07", "defect", stopped("unsupported", NO_TEXT)),
        "f17-text-and-indirect-null-page.pdf": ("R4-PDF-07", "defect", text([["First page text"], []])),
        "f18-text-and-missing-reference-page.pdf": ("R4-PDF-07", "control", stopped("failed", CONTENT_DAMAGED)),
        "f19-contents-number.pdf": ("R4-PDF-07", "control", stopped("failed", CONTENT_DAMAGED)),
        "f20-contents-dictionary.pdf": ("R4-PDF-07", "control", stopped("failed", CONTENT_DAMAGED)),
        "f21-contents-array-null-member.pdf": ("R4-PDF-07", "control", stopped("failed", CONTENT_DAMAGED)),
        "f22-contents-null-in-object-stream.pdf": ("R4-PDF-07", "defect", stopped("unsupported", NO_TEXT)),
        "f23-text-and-direct-null-page.pdf": ("R4-PDF-07", "control", text([["First page text"], []])),
    }
    out = {}
    for name, (finding, role, case) in sorted(found.items()):
        out[name] = {"path": f"defects2/generated/{name}", "kind": "defect-reproduction",
                     "r4_finding": finding, "role": role, **case}
    return out


def main():
    (HERE / "expected.json").write_text(json.dumps({
        "frozen": "Written before any fix; changes must state their basis.",
        "source": "Codex-R4's second-round descriptions of R4-PDF-06 and R4-PDF-07 (its inputs were not seen)",
        "roles": {
            "defect": "fails as R4 describes before the fix",
            "control": "the rule that must hold before and after the fix",
            "limit-control": "a size limit that still holds for bytes that only look like a stream",
            "known-limitation": "still refused after the fix, on purpose (see the module docstring)",
            "related": "the same gap seen from the other side: a real header written in a legal way "
                       "that the raw-byte check does not find",
        },
        "fixtures": cases(),
    }, indent=1, ensure_ascii=False) + "\n")
    print("second-round defect expectations written")


if __name__ == "__main__":
    main()
