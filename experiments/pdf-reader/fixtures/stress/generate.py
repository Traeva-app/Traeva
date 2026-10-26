#!/usr/bin/env python3
"""Inputs for the scanning-cost check of R4-PDF-09 (#148 at c3a28cf): the
raw-byte check scanned towards the end of the file again for every
candidate, so its cost grew with the square of the file size, also on
legal files. Written by Claude-G from R4's description; R4's own inputs
were not seen. All content is synthetic. Offline reader only.

    python3 fixtures/stress/generate.py           # write files and the manifest
    python3 fixtures/stress/generate.py --check   # write files, compare with the manifest

Every file is one page that shows a short line of text, plus a body of
`size` bytes made of one pattern (protocol.json has the sizes, the cut-off
and the frozen criteria):

S1 big-string        a legal literal string of `size` bytes in the
                     document information dictionary (R4's normal control)
S2 comment-stress    a legal uncompressed content stream: the text, then
                     comment lines `% 1 0 obj << /A (` (R4's legal family)
S3 open-literal      the same fragments between the objects, outside any
                     comment: unclosed literal strings (malformed)
S4 open-hex          fragments `1 0 obj << /A <` between the objects:
                     unclosed hex strings (malformed)
S5 no-endstream      `1 0 obj << >> stream` between the objects, never
                     closed (R4's no-endstream family)
S6 comment-line      one comment line of `obj%` repeated (a single very
                     long line, legal)
S7 nested-dicts      a legal unreferenced object: 60 dictionaries nested
                     through keys named /obj, the innermost holding
                     `size` bytes of entries
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from generate import HELVETICA, Doc, line, stream  # noqa: E402  (the first round's PDF writer)

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"
SIZES = [64 << 10, 128 << 10, 256 << 10, 512 << 10, 1 << 20, 2 << 20, 4 << 20, 8 << 20]
TEXT = "Stress control text"


def repeat(unit, size):
    """Whole copies of `unit`, at most `size` bytes."""
    return unit * (size // len(unit))


def document(content, info=None, before=b"", extra=()):
    """One page showing `content` (uncompressed); `before` goes between the
    header and the first object; `extra` are more objects nobody refers to."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    body = doc.add(stream(b"", content))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, body))
    for obj in extra:
        doc.add(obj)
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    trailer = b""
    if info is not None:
        trailer = b" /Info %d 0 R" % doc.add(info)
    built = doc.build(catalog, trailer_extra=trailer)
    if not before:
        return built
    # Put `before` after the header lines and shift every offset.
    header_end = built.index(b"\n", built.index(b"\n") + 1) + 1
    shifted = bytearray(built[:header_end] + before + built[header_end:])
    xref_at = int(built.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    new_xref = xref_at + len(before)
    table = shifted[new_xref:]
    lines = table.split(b"\n")
    for k, row in enumerate(lines):
        if len(row) == 19 and row.endswith(b" n "):
            lines[k] = b"%010d" % (int(row[:10]) + len(before)) + row[10:]
    table = b"\n".join(lines)
    tail = b"startxref\n%d\n%%%%EOF\n" % xref_at
    assert table.endswith(tail)
    table = table[: -len(tail)] + b"startxref\n%d\n%%%%EOF\n" % new_xref
    return bytes(shifted[:new_xref]) + table


def s1(size):
    string = repeat(b"Normal text in a big string. ", size)
    return document(line(TEXT, 72, 700), info=b"<< /Subject (" + string + b") >>")


def s2(size):
    return document(line(TEXT, 72, 700) + repeat(b"% 1 0 obj << /A (\n", size))


def s3(size):
    return document(line(TEXT, 72, 700), before=repeat(b"1 0 obj << /A (\n", size))


def s4(size):
    return document(line(TEXT, 72, 700), before=repeat(b"1 0 obj << /A <\n", size))


def s5(size):
    return document(line(TEXT, 72, 700), before=repeat(b"1 0 obj << >> stream\n", size))


def s6(size):
    return document(line(TEXT, 72, 700), before=b"%" + repeat(b"obj%", size) + b"\n")


def s7(size):
    depth = 60
    inner = repeat(b"/a 1 ", size)
    nested = b"<< /obj" * depth + b"<< " + inner + b">>" + b" >>" * depth
    return document(line(TEXT, 72, 700), extra=(nested,))


FAMILIES = [("S1-big-string", s1), ("S2-comment-stress", s2), ("S3-open-literal", s3), ("S4-open-hex", s4),
            ("S5-no-endstream", s5), ("S6-comment-line", s6), ("S7-nested-dicts", s7)]


def label(size):
    return f"{size >> 20}MiB" if size >= 1 << 20 else f"{size >> 10}KiB"


def main():
    OUT.mkdir(exist_ok=True)
    files = {}
    for family, maker in FAMILIES:
        for size in SIZES:
            name = f"{family}-{label(size)}.pdf"
            data = maker(size)
            (OUT / name).write_bytes(data)
            files[name] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    if "--check" in sys.argv:
        recorded = json.loads(MANIFEST.read_text())["files"]
        changed = sorted(n for n in files if recorded.get(n, {}).get("sha256") != files[n]["sha256"])
        missing = sorted(set(recorded) ^ set(files))
        print(json.dumps({"changed": changed, "missing_or_new": missing}, indent=2))
        sys.exit(1 if changed or missing else 0)
    MANIFEST.write_text(json.dumps({
        "generator": "fixtures/stress/generate.py",
        "note": "Synthetic scanning-cost inputs for R4-PDF-09. Offline reader only.",
        "files": files,
    }, indent=2) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
