#!/usr/bin/env python3
"""Reproduction fixtures for R4's third-round finding R4-PDF-08 (#148 at
c3a28cf, #158 at f47d9b3), written by Claude-G from R4's description only:
R4's own inputs (t05-t07) were not seen. All content is synthetic. Offline
reader only, never a desktop PDF viewer.

    python3 fixtures/defects3/generate.py           # write files and the manifest
    python3 fixtures/defects3/generate.py --check   # write files, compare with the manifest

R4-PDF-08: a stream without filters is not held to the 16 MiB per-stream
limit, and the raw-byte check sizes it up to the first `endstream` in its
data, which can be far shorter than what hayro reads. Matching where the
data starts does not prove where it ends.

g01-g08   unfiltered page contents and forms just under, at and over 16 MiB,
          with `endstream` in a comment at the start of the data (R4's t05-t07)
g09-g12   the same with /Length in an indirect object, also one stored in an
          object stream (where the raw-byte check cannot read it)
g13-g17   /Length that matches, is too short, too long, missing, or wrong over
          17 MiB of data with `endstream` near its start
g18-g24   compressed bombs (wrong /Length, inside another stream's data, in an
          object stream), an unfiltered object stream over the limit, and
          cross-reference data that hayro has to rebuild
g25-g27   a dictionary without data, used as a form, as the cross-reference
          stream, or as an object stream, followed by other stream data:
          hayro reads the next stream's data for it (the last two while it
          opens the file)
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
page_doc, raw_stream = _defects.page_doc, _defects.raw_stream

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"
LIMIT = 16 << 20  # MAX_STREAM_BYTES
BIG = 17 << 20  # R4's size
FORM = b"/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >>"


def padded(text, size, lead=b""):
    """Content that shows `text` and is exactly `size` bytes: `lead`, the
    text operation, then spaces."""
    body = lead + line(text, 72, 700)
    assert len(body) <= size
    return body + b" " * (size - len(body))


def raw(entries, data, length=None):
    """An unfiltered stream; /Length is `length` (bytes) or the data's length."""
    value = length if length is not None else b"%d" % len(data)
    return b"<< %s /Length %s >>\nstream\n" % (entries, value) + data + b"\nendstream"


def contents(data, length=None):
    return page_doc(content_body=raw(b"", data, length))


def form(data, length=None):
    body = raw(FORM, data, length)
    return page_doc(line("Page text", 72, 740) + b"/Fx1 Do\n", objects=(HELVETICA, body),
                    xobjects=b"/Fx1 4 0 R ")


ENDSTREAM_COMMENT = b"% endstream endobj\n"


# --- g01-g08: unfiltered contents and forms around the limit ------------------------


def g01():
    return contents(padded("Contents under the limit", LIMIT - 1))


def g02():
    return contents(padded("Contents at the limit", LIMIT))


def g03():
    return contents(padded("Contents over the limit", LIMIT + 1))


def g04():
    """R4's t07 for contents: 17 MiB, `endstream` in a comment at offset 2."""
    return contents(padded("Contents with an early endstream", BIG, ENDSTREAM_COMMENT))


def g05():
    return form(padded("Form under the limit", LIMIT - 1))


def g06():
    return form(padded("Form at the limit", LIMIT))


def g07():
    return form(padded("Form over the limit", LIMIT + 1))


def g08():
    return form(padded("Form with an early endstream", BIG, ENDSTREAM_COMMENT))


# --- g09-g12: indirect /Length ------------------------------------------------------------


def indirect_length(data):
    """Page content whose /Length is an object written after the stream."""
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


def g09():
    return indirect_length(padded("Indirect length over the limit", LIMIT + 1))


def g10():
    return indirect_length(padded("Indirect length under the limit", LIMIT - 1, ENDSTREAM_COMMENT))


def length_in_object_stream(data):
    """Catalog, pages, page, font and the content's /Length integer in an
    object stream (with a cross-reference stream); the content stream is an
    ordinary object whose /Length refers into the object stream."""
    packed = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> "
           b"/Contents 5 0 R >>",
        4: HELVETICA,
        6: b"%d" % len(data),
    }
    head, body = b"", b""
    for number, obj in packed.items():
        head += b"%d %d " % (number, len(body))
        body += obj + b"\n"
    objstm, xref = 7, 8
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    offsets[5] = len(out)
    out += b"5 0 obj\n<< /Length 6 0 R >>\nstream\n" + data + b"\nendstream\nendobj\n"
    offsets[objstm] = len(out)
    out += b"%d 0 obj\n" % objstm + flate(b"/Type /ObjStm /N %d /First %d" % (len(packed), len(head)),
                                          head + body) + b"\nendobj\n"
    offsets[xref] = len(out)
    rows = struct.pack(">BIH", 0, 0, 0xFFFF)
    for number in range(1, xref + 1):
        if number in packed:
            rows += struct.pack(">BIH", 2, objstm, list(packed).index(number))
        else:
            rows += struct.pack(">BIH", 1, offsets[number], 0)
    out += b"%d 0 obj\n" % xref + flate(b"/Type /XRef /Size %d /W [1 4 2] /Root 1 0 R" % (xref + 1), rows)
    out += b"\nendobj\nstartxref\n%d\n%%%%EOF\n" % offsets[xref]
    return bytes(out)


def g11():
    """Over the limit; /Length in an object stream; `endstream` at offset 2."""
    return length_in_object_stream(padded("Length in an object stream over the limit", LIMIT + 1, ENDSTREAM_COMMENT))


def g12():
    """Under the limit; /Length in an object stream; `endstream` at offset 2."""
    return length_in_object_stream(padded("Length in an object stream under the limit", LIMIT - 1, ENDSTREAM_COMMENT))


# --- g13-g17: /Length that matches or not -----------------------------------------------------


def g13():
    """Control: /Length matches; the data has `endstream` in a comment."""
    return contents(ENDSTREAM_COMMENT + line("Matching length", 72, 700))


def g14():
    """/Length 10, shorter than the data; nothing but data follows those 10 bytes."""
    return contents(line("Length too short", 72, 700), length=b"10")


def g15():
    """/Length larger than the data: it runs past `endstream`."""
    data = line("Length too long", 72, 700)
    return contents(data, length=b"%d" % (len(data) + 40))


def g16():
    """No /Length at all."""
    data = line("No length", 72, 700)
    return page_doc(content_body=b"<< >>\nstream\n" + data + b"\nendstream")


def g17():
    """17 MiB of data with `endstream` at offset 2 and /Length 5: hayro falls
    back to the first `endstream` and reads two bytes."""
    return contents(padded("Wrong length over an early endstream", BIG, ENDSTREAM_COMMENT), length=b"5")


# --- g18-g24: compressed bombs, object streams, rebuilt cross-reference data ----------------


def bomb():
    return zlib.compress(b" " * BIG, 9)


def g18():
    """A form that is a 17 MiB Flate bomb, with a wrong (short) /Length."""
    body = b"<< %s /Filter /FlateDecode /Length 7 >>\nstream\n" % FORM + bomb() + b"\nendstream"
    return page_doc(line("Page text", 72, 740) + b"/Fx1 Do\n", objects=(HELVETICA, body), xobjects=b"/Fx1 4 0 R ")


def g19():
    """An object-stream listing whose data is a 17 MiB Flate bomb, inside an
    uncompressed image's data."""
    packed = bomb()
    payload = (b"\x00\n9 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length %d >>\nstream\n"
               % len(packed) + packed + b"\nendstream\nendobj\n")
    image = raw(b"/Type /XObject /Subtype /Image /Width %d /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8"
                % len(payload), payload)
    return page_doc(line("Object stream bomb in image data", 72, 720) + b"q 200 0 0 20 72 600 cm /Im1 Do Q\n",
                    objects=(image,), xobjects=b"/Im1 3 0 R ")


def g20():
    """An unfiltered object stream of 17 MiB (its objects, then spaces)."""
    packed = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> "
           b"/Contents 5 0 R >>",
        4: HELVETICA,
    }
    head, body = b"", b""
    for number, obj in packed.items():
        head += b"%d %d " % (number, len(body))
        body += obj + b"\n"
    data = head + body
    data += b" " * (BIG - len(data))
    objstm, xref = 6, 7
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {5: len(out)}
    out += b"5 0 obj\n" + flate(b"", line("Big object stream", 72, 700)) + b"\nendobj\n"
    offsets[objstm] = len(out)
    out += b"%d 0 obj\n" % objstm + raw(b"/Type /ObjStm /N %d /First %d" % (len(packed), len(head)), data)
    out += b"\nendobj\n"
    offsets[xref] = len(out)
    rows = struct.pack(">BIH", 0, 0, 0xFFFF)
    for number in range(1, xref + 1):
        if number in packed:
            rows += struct.pack(">BIH", 2, objstm, list(packed).index(number))
        else:
            rows += struct.pack(">BIH", 1, offsets[number], 0)
    out += b"%d 0 obj\n" % xref + flate(b"/Type /XRef /Size %d /W [1 4 2] /Root 1 0 R" % (xref + 1), rows)
    out += b"\nendobj\nstartxref\n%d\n%%%%EOF\n" % offsets[xref]
    return bytes(out)


def broken_xref(built, number):
    """`built` with object `number`'s cross-reference offset moved by one
    byte, so hayro has to rebuild its cross-reference data."""
    xref_at = int(built.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    entry_at = xref_at + built[xref_at:].index(b"0000000000 65535 f \n") + 20 * number
    offset = int(built[entry_at:entry_at + 10])
    patched = b"%010d" % (offset + 1)
    return built[:entry_at] + patched + built[entry_at + 10:]


def g21():
    """Rebuilt cross-reference data, and unfiltered contents of 17 MiB."""
    return broken_xref(contents(padded("Rebuilt and over the limit", BIG)), 3)


def g22():
    """Rebuilt cross-reference data, and a 17 MiB Flate bomb form."""
    body = raw(FORM + b" /Filter /FlateDecode", bomb())
    built = page_doc(line("Page text", 72, 740) + b"/Fx1 Do\n", objects=(HELVETICA, body), xobjects=b"/Fx1 4 0 R ")
    return broken_xref(built, 3)


def g23():
    """Control: rebuilt cross-reference data, small ordinary contents."""
    return broken_xref(contents(line("Rebuilt control", 72, 700)), 3)


def g24():
    """Control: the same contents without the broken entry."""
    return contents(line("Rebuilt control", 72, 700))


# --- g25: a dictionary without data, used as a stream ---------------------------------------


def g25():
    """The page draws /Fx1, object 4, a form dictionary with /FlateDecode but
    no stream data. hayro, asked for a stream there, takes the data after
    the next `stream` keyword in the file: object 5, an image never drawn,
    whose unfiltered data is a zlib bomb of 17 MiB."""
    dictionary = FORM + b" /Filter /FlateDecode"
    packed = bomb()
    image = raw(b"/Type /XObject /Subtype /Image /Width %d /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8"
                % len(packed), packed)
    return page_doc(line("Page text", 72, 740) + b"/Fx1 Do\n",
                    objects=(HELVETICA, b"<< " + dictionary + b" >>", image), xobjects=b"/Fx1 4 0 R ")


def bomb_image():
    """An image nobody draws whose unfiltered data is a zlib bomb of 17 MiB."""
    packed = bomb()
    return raw(b"/Type /XObject /Subtype /Image /Width %d /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8"
               % len(packed), packed)


def g26():
    """`startxref` points at a cross-reference stream dictionary with
    /FlateDecode and no data; the next object is the bomb image. hayro reads
    the cross-reference stream while it opens the file."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    content = doc.add(flate(b"", line("Cross-reference stream without data", 72, 700)))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> "
                   b"/Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    for number, body in enumerate(doc.bodies, start=1):
        out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
    xref_at = len(out)
    size = len(doc.bodies) + 3
    out += b"%d 0 obj\n<< /Type /XRef /Size %d /W [1 4 2] /Root %d 0 R /Filter /FlateDecode >>\nendobj\n" % (
        size - 2, size, catalog)
    out += b"%d 0 obj\n" % (size - 1) + bomb_image() + b"\nendobj\n"
    out += b"startxref\n%d\n%%%%EOF\n" % xref_at
    return bytes(out)


def g27():
    """The page tree is compressed in object 6, an object stream dictionary
    with /FlateDecode and no data; the next object is the bomb image. hayro
    reads the object stream while it opens the file."""
    packed = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> "
           b"/Contents 5 0 R >>",
        4: HELVETICA,
    }
    objstm, image, xref = 6, 7, 8
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {5: len(out)}
    out += b"5 0 obj\n" + flate(b"", line("Object stream without data", 72, 700)) + b"\nendobj\n"
    offsets[objstm] = len(out)
    out += b"%d 0 obj\n<< /Type /ObjStm /N %d /First 20 /Filter /FlateDecode >>\nendobj\n" % (objstm, len(packed))
    offsets[image] = len(out)
    out += b"%d 0 obj\n" % image + bomb_image() + b"\nendobj\n"
    offsets[xref] = len(out)
    rows = struct.pack(">BIH", 0, 0, 0xFFFF)
    for number in range(1, xref + 1):
        if number in packed:
            rows += struct.pack(">BIH", 2, objstm, list(packed).index(number))
        else:
            rows += struct.pack(">BIH", 1, offsets[number], 0)
    out += b"%d 0 obj\n" % xref + flate(b"/Type /XRef /Size %d /W [1 4 2] /Root 1 0 R" % (xref + 1), rows)
    out += b"\nendobj\nstartxref\n%d\n%%%%EOF\n" % offsets[xref]
    return bytes(out)


MAKERS = [(f"g{n:02d}-{label}.pdf", maker) for n, label, maker in [
    (1, "contents-raw-under-limit", g01), (2, "contents-raw-at-limit", g02),
    (3, "contents-raw-over-limit", g03), (4, "contents-raw-17mib-early-endstream", g04),
    (5, "form-raw-under-limit", g05), (6, "form-raw-at-limit", g06),
    (7, "form-raw-over-limit", g07), (8, "form-raw-17mib-early-endstream", g08),
    (9, "indirect-length-over-limit", g09), (10, "indirect-length-under-limit-early-endstream", g10),
    (11, "length-in-objstm-over-limit", g11), (12, "length-in-objstm-under-limit", g12),
    (13, "matching-length-endstream-comment", g13), (14, "length-too-short", g14),
    (15, "length-too-long", g15), (16, "length-missing", g16),
    (17, "wrong-length-17mib-early-endstream", g17),
    (18, "flate-bomb-wrong-length", g18), (19, "objstm-bomb-in-image-data", g19),
    (20, "objstm-raw-over-limit", g20), (21, "rebuilt-xref-raw-over-limit", g21),
    (22, "rebuilt-xref-flate-bomb", g22), (23, "rebuilt-xref-control", g23),
    (24, "unbroken-xref-control", g24), (25, "dictionary-read-as-stream", g25),
    (26, "xref-stream-without-data", g26), (27, "objstm-without-data", g27),
]]


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
        "generator": "fixtures/defects3/generate.py",
        "note": "Synthetic reproductions of R4's third-round finding R4-PDF-08, from R4's description only. "
                "Adversarial or damaged on purpose: offline reader only, never a desktop viewer.",
        "files": files,
    }, indent=2) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
