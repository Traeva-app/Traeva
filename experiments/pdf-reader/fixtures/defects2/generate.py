#!/usr/bin/env python3
"""Reproduction fixtures for the two defects Codex-R4 found in its second
round (#148 at e2d836f, #158 at 7f79cd8), written by Claude-G from R4's
descriptions only: R4's own inputs and logs were not seen. All content is
synthetic. Offline reader only, never a desktop PDF viewer.

    python3 fixtures/defects2/generate.py           # write files and the manifest
    python3 fixtures/defects2/generate.py --check   # write files, compare with the manifest

R4-PDF-06  text that looks like a PDF object (`8 0 obj << ... >> stream`) in
           shown text, comments, strings or another stream's data is taken
           for a real stream (f01-f10), with controls that real streams are
           still checked (f11-f14) and a real header written with a comment
           (f24)
R4-PDF-07  /Contents that refers to an object which is null (f15-f23)
"""

import hashlib
import importlib.util
import json
import struct
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from generate import HELVETICA, Doc, flate, line, stream  # noqa: E402  (the first round's PDF writer)

_spec = importlib.util.spec_from_file_location("defects_generate", HERE.parent / "defects" / "generate.py")
_defects = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_defects)
page_doc, raw_stream, FORM, BOMB = _defects.page_doc, _defects.raw_stream, _defects.FORM, _defects.BOMB

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"

# R4's example, and its Flate form.
LISTING = "Listing 8 0 obj << /Length 0 /Filter /Missing >> stream sample endstream endobj"
LISTING_FLATE = "Listing 8 0 obj << /Length 3 /Filter /FlateDecode >> stream abc endstream endobj"
# Text that ends a stream early for a reader that looks for the first `endstream`.
ENDSTREAM_TEXT = "endstream endobj 9 0 obj << /Length 1 /Filter /Missing >> stream x endstream"
OBJSTM_LISTING = "Listing 5 0 obj << /Type /ObjStm /N 1 /First 4 /Filter /Missing >> stream 1 0 x endstream endobj"
OBJSTM_LZW = "Listing 5 0 obj << /Type /ObjStm /N 1 /First 4 /Filter /LZWDecode >> stream 1 0 x endstream endobj"


def shown(text):
    """An uncompressed content stream that shows `text`: its bytes are in the file as is."""
    return stream(b"", line(text, 40, 700, 9))


def with_comment_after_header(doc, root, comment):
    """Doc.build, with a comment line after the header (offsets stay exact)."""
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n" + comment + b"\n")
    offsets = []
    for number, body in enumerate(doc.bodies, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
    xref = len(out)
    count = len(doc.bodies) + 1
    out += b"xref\n0 %d\n0000000000 65535 f \n" % count
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (count, root, xref)
    return bytes(out)


# --- R4-PDF-06: text that looks like an object ------------------------------------


def f01():
    """R4's example as shown text, in an uncompressed content stream."""
    return page_doc(content_body=shown(LISTING))


def f02():
    """The same example in a comment between the file's objects."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    content = doc.add(flate(b"", line("Comment control", 72, 700)))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return with_comment_after_header(doc, catalog, b"% " + LISTING.encode())


def f03():
    """The example with /FlateDecode and `stream abc`, as shown text."""
    return page_doc(content_body=shown(LISTING_FLATE))


def f04():
    """The example in a comment inside an uncompressed content stream."""
    content = b"% " + LISTING.encode() + b"\n" + line("Content comment control", 72, 700)
    return page_doc(content_body=stream(b"", content))


def f05():
    """The Flate example in a literal string of the document information
    dictionary, and a hex string next to it."""
    hexed = LISTING_FLATE.encode().hex().upper().encode()
    return page_doc(line("String control", 72, 700),
                    info=b"<< /Subject (" + LISTING_FLATE.encode() + b") /Keywords <" + hexed + b"> >>")


def f06():
    """An object listing with /LZWDecode inside an uncompressed image's pixel data."""
    payload = (b"\x00\x01\n9 0 obj\n<< /Length 4 /Filter /LZWDecode >>\nstream\nabcd\nendstream\nendobj\n"
               + bytes(range(256)))
    image = raw_stream(b"/Type /XObject /Subtype /Image /Width %d /Height 1 /ColorSpace /DeviceGray "
                       b"/BitsPerComponent 8" % len(payload), payload)
    return page_doc(line("Image payload control", 72, 720) + b"q 200 0 0 20 72 600 cm /Im1 Do Q\n",
                    objects=(image,), xobjects=b"/Im1 3 0 R ")


def f07():
    """Shown text with `endstream`, `endobj` and an object listing, in a content
    stream whose /Length is right."""
    return page_doc(content_body=shown(ENDSTREAM_TEXT))


def f08():
    """As f07, with /Length an indirect object written after the stream."""
    data = line(ENDSTREAM_TEXT, 40, 700, 9)
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    content, length = doc.reserve(), doc.reserve()
    doc.set(content, b"<< /Length %d 0 R >>\nstream\n" % length + data + b"\nendstream")
    doc.set(length, b"%d" % len(data))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


def f09():
    """An object-stream listing with an unknown filter, as shown text."""
    return page_doc(content_body=shown(OBJSTM_LISTING))


def f10():
    """An object-stream listing with /LZWDecode, as shown text."""
    return page_doc(content_body=shown(OBJSTM_LZW))


# --- real streams are still checked --------------------------------------------------


def f11():
    """The example as shown text, and a real form of 17 MiB."""
    form = raw_stream(FORM % b"" + b" /Filter /FlateDecode", zlib.compress(b" " * BOMB, 9))
    return page_doc(content_body=stream(b"", line(LISTING, 40, 700, 9) + b"/Fx1 Do\n"),
                    objects=(HELVETICA, form), xobjects=b"/Fx1 4 0 R ")


def f12():
    """A damaged content part, then a part with the example as shown text."""
    packed = zlib.compress(line("Damaged part", 72, 720), 9)
    damaged = raw_stream(b"/Filter /FlateDecode", packed[:-4] + bytes(b ^ 0xFF for b in packed[-4:]))
    return page_doc(objects=(damaged, shown(LISTING)), contents=b"[3 0 R 4 0 R]")


def f13():
    """A real content stream with a filter nobody knows."""
    return page_doc(content_body=raw_stream(b"/Filter /Missing", b"sample"))


def f14():
    """An object listing inside an image's pixel data whose own data is a
    Flate stream of 17 MiB."""
    bomb = zlib.compress(b" " * BOMB, 9)
    payload = (b"\x00\n9 0 obj\n<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(bomb) + bomb
               + b"\nendstream\nendobj\n")
    image = raw_stream(b"/Type /XObject /Subtype /Image /Width %d /Height 1 /ColorSpace /DeviceGray "
                       b"/BitsPerComponent 8" % len(payload), payload)
    return page_doc(line("Hidden limit control", 72, 720) + b"q 200 0 0 20 72 600 cm /Im1 Do Q\n",
                    objects=(image,), xobjects=b"/Im1 3 0 R ")


# --- R4-PDF-07: /Contents and null ------------------------------------------------------


def f15():
    """/Contents null, written directly."""
    return page_doc(contents=b"null")


def f16():
    """/Contents 3 0 R, and object 3 is null."""
    return page_doc(objects=(b"null",), contents=b"3 0 R")


def two_pages(second_contents, objects=()):
    """Page 1 shows "First page text"; page 2's /Contents is `second_contents`.
    Objects 3, 4, ... are `objects`."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    for body in objects:
        doc.add(body)
    font = doc.add(HELVETICA)
    first = doc.add(flate(b"", line("First page text", 72, 700)))
    resources = b"/Resources << /Font << /F1 %d 0 R >> >>" % font
    pages = [doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] %s /Contents %s >>"
                     % (tree, resources, contents)) for contents in (b"%d 0 R" % first, second_contents)]
    doc.set(tree, b"<< /Type /Pages /Kids [%s] /Count 2 >>" % b" ".join(b"%d 0 R" % p for p in pages))
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


def f17():
    """A text page, then a page whose /Contents refers to a null object."""
    return two_pages(b"3 0 R", objects=(b"null",))


def f18():
    """A text page, then a page whose /Contents refers to an object that does not exist."""
    return two_pages(b"99 0 R")


def f19():
    """/Contents refers to a number."""
    return page_doc(objects=(b"42",), contents=b"3 0 R")


def f20():
    """/Contents refers to a dictionary that is not a stream."""
    return page_doc(objects=(b"<< /Note (not a stream) >>",), contents=b"3 0 R")


def f21():
    """/Contents [3 0 R 4 0 R]: 3 is a content stream, 4 is null."""
    return page_doc(objects=(flate(b"", line("Array part", 72, 700)), b"null"), contents=b"[3 0 R 4 0 R]")


def f22():
    """/Contents refers to a null that is stored in an object stream (with a
    cross-reference stream)."""
    packed = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> "
           b"/Contents 5 0 R >>",
        4: HELVETICA,
        5: b"null",
    }
    head, body = b"", b""
    for number, obj in packed.items():
        head += b"%d %d " % (number, len(body))
        body += obj + b"\n"
    objstm, xref = 6, 7
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    objstm_at = len(out)
    out += b"%d 0 obj\n" % objstm + flate(b"/Type /ObjStm /N %d /First %d" % (len(packed), len(head)),
                                          head + body) + b"\nendobj\n"
    xref_at = len(out)
    rows = struct.pack(">BIH", 0, 0, 0xFFFF)
    for number in range(1, xref + 1):
        if number in packed:
            rows += struct.pack(">BIH", 2, objstm, list(packed).index(number))
        else:
            rows += struct.pack(">BIH", 1, objstm_at if number == objstm else xref_at, 0)
    out += b"%d 0 obj\n" % xref + flate(b"/Type /XRef /Size %d /W [1 4 2] /Root 1 0 R" % (xref + 1), rows)
    out += b"\nendobj\nstartxref\n%d\n%%%%EOF\n" % xref_at
    return bytes(out)


def f23():
    """A text page, then a page with /Contents null written directly."""
    return two_pages(b"null")


def f24():
    """A real content stream whose object header has a comment between its
    numbers and `obj`, which PDF allows wherever white space is."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    content = doc.add(flate(b"", line("Header comment control", 72, 700)))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    built = doc.build(catalog)
    # "4 0 obj\n<<" becomes "4%\n0 obj<<": a comment (one white space) in place
    # of the space, the same length, so every offset stays exact.
    old = b"%d 0 obj\n<<" % content
    new = b"%d%%\n0 obj<<" % content
    assert len(old) == len(new) and built.count(old) == 1
    return built.replace(old, new)


MAKERS = [
    ("f01-listing-in-text.pdf", f01), ("f02-listing-in-file-comment.pdf", f02),
    ("f03-flate-listing-in-text.pdf", f03), ("f04-listing-in-content-comment.pdf", f04),
    ("f05-listing-in-strings.pdf", f05), ("f06-lzw-listing-in-image-data.pdf", f06),
    ("f07-endstream-in-text.pdf", f07), ("f08-endstream-in-text-indirect-length.pdf", f08),
    ("f09-objstm-listing-in-text.pdf", f09), ("f10-objstm-lzw-listing-in-text.pdf", f10),
    ("f11-listing-and-real-bomb.pdf", f11), ("f12-damaged-part-and-listing.pdf", f12),
    ("f13-real-unknown-filter.pdf", f13), ("f14-bomb-listing-in-image-data.pdf", f14),
    ("f15-contents-direct-null.pdf", f15), ("f16-contents-indirect-null.pdf", f16),
    ("f17-text-and-indirect-null-page.pdf", f17), ("f18-text-and-missing-reference-page.pdf", f18),
    ("f19-contents-number.pdf", f19), ("f20-contents-dictionary.pdf", f20),
    ("f21-contents-array-null-member.pdf", f21), ("f22-contents-null-in-object-stream.pdf", f22),
    ("f23-text-and-direct-null-page.pdf", f23), ("f24-header-with-comment.pdf", f24),
]


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
        "generator": "fixtures/defects2/generate.py",
        "note": "Synthetic reproductions of R4's second-round findings (R4-PDF-06, R4-PDF-07), from R4's "
                "descriptions only. Offline reader only, never a desktop viewer.",
        "files": files,
    }, indent=2) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
