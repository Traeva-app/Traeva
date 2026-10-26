#!/usr/bin/env python3
"""Fixtures for the quality diagnostics (pdf-experiment-4) that the other
sets do not have. All content is synthetic.

    python3 fixtures/quality/generate.py           # write files and the manifest
    python3 fixtures/quality/generate.py --check   # write files, compare with the manifest

q01  every character lacks a Unicode mapping (no usable text at all)
q02  a ruled two-column table of long cells, drawn row by row
q03  the same cells without lines (looks like two columns read across)
q04  a form-like block of label pairs, drawn row by row
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import generate as base  # noqa: E402  (the first round's PDF writer)
from generate import HELVETICA, Doc, flate, line, stream  # noqa: E402

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"

LEFT = ["Version history keeps every save", "Each snapshot records the file", "Changes are listed per version",
        "Readers can follow the history"]
RIGHT = ["so a reader can compare any two", "with its size and its hash value", "and shown next to the old text",
         "in the order the files changed"]
LABELS = [("Prepared by: Alice Example", "Date: 2026-10-12 morning"),
          ("Reviewed by: Bob Example", "Version: 3 of the release"),
          ("Approved by: Carol Example", "Status: ready for archive")]
BODY = "The release notes below describe what changed in this version."


def q01():
    """A page whose only font is a Type 3 font without any Unicode mapping."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    procs = [doc.add(stream(b"", b"600 0 0 0 600 700 d1 0 0 600 700 re f")) for _ in range(3)]
    font = doc.add(b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] "
                   b"/CharProcs << /g1 %d 0 R /g2 %d 0 R /g3 %d 0 R >> "
                   b"/Encoding << /Type /Encoding /Differences [97 /g1 /g2 /g3] >> /FirstChar 97 /LastChar 99 "
                   b"/Widths [600 600 600] /Resources << >> >>" % tuple(procs))
    content = doc.add(flate(b"", line("abcabc", 72, 700, font=b"F3")))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F3 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


def two_column_rows():
    """Each row: the left cell, then the right cell, as separate text objects."""
    return b"".join(line(left, 75, 686 - 20 * i, 10) + line(right, 305, 686 - 20 * i, 10)
                    for i, (left, right) in enumerate(zip(LEFT, RIGHT)))


def q02():
    xs, ys = [70, 300, 530], [700, 680, 660, 640, 620]
    grid = b"0.5 w\n" + b"".join(b"%d %d m %d %d l S\n" % (xs[0], y, xs[-1], y) for y in ys) + b"".join(
        b"%d %d m %d %d l S\n" % (x, ys[-1], x, ys[0]) for x in xs)
    return base.build([grid + two_column_rows()])


def q03():
    return base.build([two_column_rows()])


def q04():
    rows = b"".join(line(left, 72, 700 - 16 * i, 10) + line(right, 330, 700 - 16 * i, 10)
                    for i, (left, right) in enumerate(LABELS))
    return base.build([rows + line(BODY, 72, 640, 10)])


MAKERS = [("q01-no-unicode-only.pdf", q01), ("q02-ruled-table-long-cells.pdf", q02),
          ("q03-unruled-long-cells.pdf", q03), ("q04-label-block.pdf", q04)]


def main():
    OUT.mkdir(exist_ok=True)
    files = {}
    for name, maker in MAKERS:
        data = maker()
        (OUT / name).write_bytes(data)
        files[name] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    if "--check" in sys.argv:
        recorded = json.loads(MANIFEST.read_text())["files"]
        changed = sorted(n for n in files if recorded.get(n, {}).get("sha256") != files[n]["sha256"])
        missing = sorted(set(recorded) ^ set(files))
        print(json.dumps({"changed": changed, "missing_or_new": missing}, indent=2))
        sys.exit(1 if changed or missing else 0)
    MANIFEST.write_text(json.dumps({
        "generator": "fixtures/quality/generate.py",
        "note": "Synthetic files only; fixtures for the quality diagnostics.",
        "files": files,
    }, indent=2) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
