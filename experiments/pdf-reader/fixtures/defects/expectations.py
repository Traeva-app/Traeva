#!/usr/bin/env python3
"""Frozen expectations for the defect reproductions (fixtures/defects/).

Written before any fix, from each fixture's own source (generate.py) and the
output rules in README.md: they say what a correct reader returns. The code
at 07ce381 is expected to fail the defect cases (that is the reproduction)
and to pass the controls.

    python3 fixtures/defects/expectations.py   # writes fixtures/defects/expected.json
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from expectations import ENCRYPTED, NOT_ALL, joined, page_offsets  # noqa: E402

BOMB = "A stream expands beyond this reader's limit of 16 MiB."
DAMAGED = "A compressed stream in the PDF is damaged."
CONTENT_DAMAGED = ("A page's content list is damaged: it refers to an object that is missing "
                   "or is not a content stream.")
JAVASCRIPT = "JavaScript (not run and not read)"
EMBEDDED = "embedded files (not opened)"


def text(lines, status="complete", left_out=(), kinds=(), **extra):
    case = {"status": status, "text": joined([lines])}
    case["chars"] = len(case["text"])
    start, end = page_offsets([lines])[0]
    case["pages"] = {"1": {"start": start, "end": end}}
    case["left_out"] = list(left_out)
    case["note"] = (NOT_ALL + ", ".join(kinds) + ".") if left_out else None
    case.update(extra)
    return case


def stopped(status, note):
    return {"status": status, "note": note, "text": "", "chars": 0, "left_out": []}


def cases():
    defect = {
        "d01-nested-form-string-bomb.pdf": (1, "defect", stopped("unsupported", BOMB)),
        "d02-comment-before-stream-bomb.pdf": (1, "defect", stopped("unsupported", BOMB)),
        "d03-nested-form-string-ok.pdf": (1, "control", text(["Page text", "Nested form text"])),
        "d04-comment-before-content-stream.pdf": (1, "control", text(["Comment before stream"])),
        "d05-strings-in-stream-dict.pdf": (1, "defect", text(["Strings in a stream dictionary"])),
        "d06-encrypt-as-words.pdf": (2, "defect", text(["Key /Encrypt is not set"])),
        "d07-size-as-words.pdf": (2, "defect", text(["Paper /Size 999999 sheets"])),
        "d08-script-names-as-words.pdf": (2, "defect", text(["Names /JS /JavaScript /EmbeddedFiles appear here"])),
        "d09-escaped-javascript-action.pdf": (3, "defect", text(
            ["Escaped JavaScript action"], "partial", [JAVASCRIPT], ["JavaScript"])),
        "d10-escaped-embedded-file.pdf": (3, "defect", text(
            ["Escaped attachment"], "partial", [EMBEDDED], ["embedded files"])),
        "d11-escaped-link-javascript.pdf": (3, "defect", text(
            ["Escaped link script"], "partial", [JAVASCRIPT], ["JavaScript"])),
        "d12-escaped-attachment-annotation.pdf": (3, "defect", text(
            ["Attachment annotation"], "partial",
            ["annotations on page 1 (1: comments, highlights, stamps or attachments; not read)", EMBEDDED],
            ["annotations", "embedded files"])),
        "d13-contents-missing-member.pdf": (4, "defect", stopped("failed", CONTENT_DAMAGED)),
        "d14-contents-string-member.pdf": (4, "defect", stopped("failed", CONTENT_DAMAGED)),
        "d15-contents-dangling.pdf": (4, "defect", stopped("failed", CONTENT_DAMAGED)),
        "d16-contents-two-parts.pdf": (4, "control", text(["First part", "Second part"])),
        "d17-flate-wrong-adler.pdf": (5, "defect", stopped("failed", DAMAGED)),
        "d18-flate-missing-adler.pdf": (5, "defect", stopped("failed", DAMAGED)),
        "d19-flate-no-zlib-header.pdf": (5, "defect", stopped("failed", DAMAGED)),
        "d20-form-wrong-adler.pdf": (5, "defect", stopped("failed", DAMAGED)),
        # Images are not read (README): a damaged image is reported as an image,
        # the same as an intact one; only its decoded size is checked.
        "d21-image-wrong-adler.pdf": (5, "control", text(
            ["Image with a damaged checksum"], "partial",
            ["images on page 1 (1 in all; images are not read, no OCR)"], ["images"])),
        "d22-xref-stream.pdf": (0, "control", text(["Object stream page"])),
        "d23-xref-stream-escaped-encrypt.pdf": (0, "control", stopped("unsupported", ENCRYPTED)),
        "d24-incremental-update.pdf": (0, "control", text(["Updated text"])),
    }
    out = {}
    for name, (r4_class, role, case) in defect.items():
        out[name] = {"path": f"defects/generated/{name}", "kind": "defect-reproduction",
                     "r4_class": r4_class, "role": role, **case}
    return out


def main():
    (HERE / "expected.json").write_text(json.dumps({
        "frozen": "Written before any fix; changes must state their basis.",
        "source": "Codex-R4's descriptions of its findings on 07ce381 (its inputs were not seen)",
        "fixtures": cases(),
    }, indent=1, ensure_ascii=False) + "\n")
    print("defect expectations written")


if __name__ == "__main__":
    main()
