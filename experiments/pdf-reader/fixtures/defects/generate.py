#!/usr/bin/env python3
"""Reproduction fixtures for the defects Codex-R4 found in 07ce381 (#148),
written by Claude-G from R4's descriptions only: R4's own inputs were not
seen. All content is synthetic. Every d- file is adversarial or damaged on
purpose: offline reader only, never a desktop PDF viewer.

    python3 fixtures/defects/generate.py           # write files and the manifest
    python3 fixtures/defects/generate.py --check   # write files, compare with the manifest

The five classes (R4's numbering):
1. P1  stream location ignores strings and comments (d01-d05)
2. P2  bare byte search takes text and comments for /Encrypt, /Size, /JS,
       /EmbeddedFiles (d06-d08)
3. P2  escaped names hide real JavaScript and attachments (d09-d12)
4. P2  missing or non-stream members of /Contents are skipped (d13-d16)
5. P2  Flate without a complete zlib wrapper or a correct Adler-32 (d17-d21)
Plus cross-reference streams, object streams and /Prev chains (d22-d24),
which the structural checks must read.
"""

import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import generate as base  # noqa: E402  (the first round's PDF writer)
from generate import HELVETICA, Doc, flate, line, stream  # noqa: E402

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"
BOMB = 17 << 20  # one MiB over the 16 MiB stream limit


def raw_stream(entries, packed, between=b"\n"):
    """A stream whose data is given as is; `between` goes between `>>` and `stream`."""
    return b"<< %s /Length %d >>%sstream\n" % (entries, len(packed), between) + packed + b"\nendstream"


def page_doc(content=None, *, objects=(), contents=None, content_body=None, page_extra=b"",
             catalog_extra=b"", xobjects=b"", info=None):
    """One page. `objects` are added first, as objects 3, 4, ..., so the other
    arguments can refer to them. `contents` replaces the page's /Contents value."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    for body in objects:
        doc.add(body)
    font = doc.add(HELVETICA)
    if content_body is not None or content is not None:
        content_ref = doc.add(content_body if content_body is not None else flate(b"", content))
        contents = contents if contents is not None else b"%d 0 R" % content_ref
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                   b"/F1 %d 0 R >> /XObject << %s>> >> /Contents %s %s>>"
                   % (tree, font, xobjects, contents, page_extra))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R %s>>" % (tree, catalog_extra))
    trailer_extra = b""
    if info is not None:
        trailer_extra = b" /Info %d 0 R" % doc.add(info)
    return doc.build(catalog, trailer_extra=trailer_extra)


FORM = b"/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> %s>>"


def nested_forms(inner_extra, inner_content, between=b"\n"):
    """Page -> /Fx1 -> /Fx2. Object 3 is the font, 4 the inner form, 5 the outer."""
    inner = raw_stream(FORM % b"" + inner_extra + b" /Filter /FlateDecode",
                       zlib.compress(inner_content, 9), between)
    outer = flate(FORM % b"/XObject << /Fx2 4 0 R >> ", b"/Fx2 Do\n")
    return page_doc(line("Page text", 72, 720) + b"/Fx1 Do\n", objects=(HELVETICA, inner, outer),
                    xobjects=b"/Fx1 5 0 R ")


# --- 1. stream location -------------------------------------------------------


def d01():
    """A nested form whose dictionary holds a legal string "(>>)", and 17 MiB of content."""
    return nested_forms(b" /Memo (>>)", b" " * BOMB)


def d02():
    """A form with a comment between its dictionary and `stream`, and 17 MiB of content."""
    form = raw_stream(FORM % b"" + b" /Filter /FlateDecode", zlib.compress(b" " * BOMB, 9),
                      b"\n% a comment between the dictionary and the keyword >> <<\n")
    return page_doc(line("Page text", 72, 720) + b"/Fx1 Do\n", objects=(HELVETICA, form),
                    xobjects=b"/Fx1 4 0 R ")


def d03():
    """Control for d01: the same nesting and "(>>)" string, with ordinary content."""
    return nested_forms(b" /Memo (>>)", line("Nested form text", 72, 700))


def d04():
    """Control for d02: a page content stream with a comment before `stream`."""
    body = raw_stream(b"/Filter /FlateDecode", zlib.compress(line("Comment before stream", 72, 700), 9),
                      b"\n% comment >>\n")
    return page_doc(content_body=body)


def d05():
    """Strings in a content stream's dictionary that hold `>>`, `stream` and `endstream`."""
    body = raw_stream(b"/Note (a \\) >> stream \\( endstream) /H <3E3E> /Filter /FlateDecode",
                      zlib.compress(line("Strings in a stream dictionary", 72, 700), 9))
    return page_doc(content_body=body)


# --- 2. words in text and comments -------------------------------------------------


def d06():
    """/Encrypt in shown text (uncompressed), in a comment, and in a string that
    looks like a trailer. The file is not encrypted."""
    return page_doc(content_body=stream(b"", line("Key /Encrypt is not set", 72, 700)),
                    catalog_extra=b"% /Encrypt 9 0 R\n",
                    info=b"<< /Title (see /Encrypt) /Subject (trailer << /Encrypt 9 0 R >>) >>")


def d07():
    """/Size 999999 in shown text, a comment and a string; the real /Size is small."""
    return page_doc(content_body=stream(b"", line("Paper /Size 999999 sheets", 72, 700)),
                    catalog_extra=b"% /Size 999999\n", info=b"<< /Title (/Size 999999) >>")


def d08():
    """/JS, /JavaScript, /EmbeddedFile(s) as words only: shown text (compressed),
    a comment and a string. There is no script and no attachment."""
    return page_doc(line("Names /JS /JavaScript /EmbeddedFiles appear here", 72, 700),
                    catalog_extra=b"% /JavaScript /EmbeddedFiles\n",
                    info=b"<< /Subject (/JS and /EmbeddedFile) >>")


# --- 3. escaped names ------------------------------------------------------------------


def d09():
    """A real JavaScript open action spelled /S /Java#53cript /J#53."""
    return page_doc(line("Escaped JavaScript action", 72, 700),
                    catalog_extra=b"/OpenAction << /S /Java#53cript /J#53 (app.alert\\(1\\);) >> ")


def d10():
    """A real attachment in /Names /Embedded#46iles, its file spec using /E#46."""
    file = stream(b"/Type /Embedded#46ile", b"attached synthetic text")
    spec = b"<< /Type /Filespec /F (note.txt) /E#46 << /F 3 0 R >> >>"
    return page_doc(line("Escaped attachment", 72, 700), objects=(file, spec),
                    catalog_extra=b"/Names << /Embedded#46iles << /Names [(note.txt) 4 0 R] >> >> ")


def d11():
    """A link annotation whose action is escaped JavaScript."""
    return page_doc(line("Escaped link script", 72, 700),
                    page_extra=b"/Annots [<< /Type /Annot /Subtype /Link /Rect [72 690 300 710] /Border [0 0 0] "
                               b"/A << /S /Java#53cript /J#53 (app.alert\\(2\\);) >> >>] ")


def d12():
    """A file attachment annotation whose embedded file is spelled with escapes."""
    file = stream(b"/Type /Embedded#46ile", b"attached synthetic text")
    return page_doc(line("Attachment annotation", 72, 700), objects=(file,),
                    page_extra=b"/Annots [<< /Type /Annot /Subtype /FileAttachment /Rect [400 700 420 720] "
                               b"/Contents (attached) /FS << /Type /Filespec /F (a.txt) /E#46 << /F 3 0 R >> >> >>] ")


# --- 4. /Contents members ---------------------------------------------------------------


def contents_doc(contents_for):
    """Object 3 is a valid content stream; `contents_for(3)` is the /Contents value."""
    part = flate(b"", line("Missing content part", 72, 700))
    return page_doc(objects=(part,), contents=contents_for(3))


def d13():
    """/Contents [3 0 R 99 0 R]: object 99 does not exist."""
    return contents_doc(lambda ref: b"[%d 0 R 99 0 R]" % ref)


def d14():
    """/Contents [3 0 R (junk)]: a string in the content list."""
    return contents_doc(lambda ref: b"[%d 0 R (junk)]" % ref)


def d15():
    """/Contents 99 0 R: the only content reference points nowhere."""
    return contents_doc(lambda ref: b"99 0 R")


def d16():
    """Control: /Contents with two valid parts."""
    first = flate(b"", line("First part", 72, 720))
    second = flate(b"", line("Second part", 72, 700))
    return page_doc(objects=(first, second), contents=b"[3 0 R 4 0 R]")


# --- 5. zlib wrapper and Adler-32 --------------------------------------------------------------


def packed_text(text):
    return zlib.compress(line(text, 72, 700), 9)


def flip_checksum(packed):
    return packed[:-4] + bytes(b ^ 0xFF for b in packed[-4:])


def d17():
    """Page content whose Adler-32 is wrong."""
    body = raw_stream(b"/Filter /FlateDecode", flip_checksum(packed_text("Wrong checksum text")))
    return page_doc(content_body=body)


def d18():
    """Page content whose Adler-32 is missing (the last 4 bytes cut, /Length matches)."""
    body = raw_stream(b"/Filter /FlateDecode", packed_text("Missing checksum text")[:-4])
    return page_doc(content_body=body)


def d19():
    """Page content as raw deflate, without the zlib header and checksum."""
    squeeze = zlib.compressobj(9, zlib.DEFLATED, -15)
    raw = squeeze.compress(line("Headerless deflate text", 72, 700)) + squeeze.flush()
    looks_like_zlib = raw[0] & 0x0F == 8 and raw[0] >> 4 <= 7 and (raw[0] << 8 | raw[1]) % 31 == 0
    assert not looks_like_zlib, "pick other text: these raw bytes happen to look like a zlib header"
    return page_doc(content_body=raw_stream(b"/Filter /FlateDecode", raw))


def d20():
    """A form (not page content) whose Adler-32 is wrong."""
    form = raw_stream(FORM % b"" + b" /Filter /FlateDecode", flip_checksum(packed_text("Form text")))
    return page_doc(line("Page with a damaged form", 72, 720) + b"/Fx1 Do\n", objects=(HELVETICA, form),
                    xobjects=b"/Fx1 4 0 R ")


def d21():
    """An image whose Adler-32 is wrong: images are not read, so this is only an image."""
    pixels = bytes((x * 7 + y * 3) % 256 for y in range(20) for x in range(20))
    image = raw_stream(b"/Type /XObject /Subtype /Image /Width 20 /Height 20 /ColorSpace /DeviceGray "
                       b"/BitsPerComponent 8 /Filter /FlateDecode", flip_checksum(zlib.compress(pixels, 9)))
    return page_doc(line("Image with a damaged checksum", 72, 720) + b"q 100 0 0 100 72 500 cm /Im1 Do Q\n",
                    objects=(image,), xobjects=b"/Im1 3 0 R ")


# --- cross-reference streams, object streams, /Prev ---------------------------------------------


def xref_stream_doc(text, trailer_extra=b"", extra_objects=()):
    """Catalog, pages, page and font in an object stream; the content stream,
    any `extra_objects` and the xref stream are ordinary objects."""
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
    objstm = 6
    plain = {5: flate(b"", line(text, 72, 700)),
             objstm: flate(b"/Type /ObjStm /N %d /First %d" % (len(packed), len(head)), head + body)}
    for index, obj in enumerate(extra_objects):
        plain[7 + index] = obj
    xref = 7 + len(extra_objects)
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number in sorted(plain):
        offsets[number] = len(out)
        out += b"%d 0 obj\n" % number + plain[number] + b"\nendobj\n"
    offsets[xref] = len(out)
    size = xref + 1
    rows = b""
    for number in range(size):
        if number == 0:
            rows += struct.pack(">BIH", 0, 0, 0xFFFF)
        elif number in packed:
            rows += struct.pack(">BIH", 2, objstm, list(packed).index(number))
        else:
            rows += struct.pack(">BIH", 1, offsets[number], 0)
    out += b"%d 0 obj\n" % xref + flate(b"/Type /XRef /Size %d /W [1 4 2] /Root 1 0 R%s" % (size, trailer_extra),
                                        rows) + b"\nendobj\n"
    out += b"startxref\n%d\n%%%%EOF\n" % offsets[xref]
    return bytes(out)


def d22():
    """Control: a cross-reference stream and an object stream."""
    return xref_stream_doc("Object stream page")


def d23():
    """A cross-reference stream whose trailer entries say /Encr#79pt: encrypted."""
    encrypt = b"<< /Filter /Standard /V 1 /R 2 /O <%s> /U <%s> /P -4 >>" % (b"00" * 32, b"00" * 32)
    return xref_stream_doc("Encrypted by escaped key", trailer_extra=b" /Encr#79pt 7 0 R /ID [<00> <00>]",
                           extra_objects=(encrypt,))


def d24():
    """Control: an incremental update. The second section replaces the content
    stream and points back with /Prev."""
    first = base.build([line("Original text", 72, 700)])
    xref_at = int(first.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    content_number = 4  # simple_doc: catalog 1, pages 2, font 3, content 4, page 5
    replacement = b"%d 0 obj\n" % content_number + flate(b"", line("Updated text", 72, 700)) + b"\nendobj\n"
    out = bytearray(first)
    at = len(out)
    out += replacement
    xref = len(out)
    out += b"xref\n0 1\n0000000000 65535 f \n%d 1\n%010d 00000 n \n" % (content_number, at)
    out += b"trailer\n<< /Size 6 /Root 1 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (xref_at, xref)
    return bytes(out)


MAKERS = [
    ("d01-nested-form-string-bomb.pdf", d01), ("d02-comment-before-stream-bomb.pdf", d02),
    ("d03-nested-form-string-ok.pdf", d03), ("d04-comment-before-content-stream.pdf", d04),
    ("d05-strings-in-stream-dict.pdf", d05),
    ("d06-encrypt-as-words.pdf", d06), ("d07-size-as-words.pdf", d07), ("d08-script-names-as-words.pdf", d08),
    ("d09-escaped-javascript-action.pdf", d09), ("d10-escaped-embedded-file.pdf", d10),
    ("d11-escaped-link-javascript.pdf", d11), ("d12-escaped-attachment-annotation.pdf", d12),
    ("d13-contents-missing-member.pdf", d13), ("d14-contents-string-member.pdf", d14),
    ("d15-contents-dangling.pdf", d15), ("d16-contents-two-parts.pdf", d16),
    ("d17-flate-wrong-adler.pdf", d17), ("d18-flate-missing-adler.pdf", d18),
    ("d19-flate-no-zlib-header.pdf", d19), ("d20-form-wrong-adler.pdf", d20),
    ("d21-image-wrong-adler.pdf", d21),
    ("d22-xref-stream.pdf", d22), ("d23-xref-stream-escaped-encrypt.pdf", d23),
    ("d24-incremental-update.pdf", d24),
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
        "generator": "fixtures/defects/generate.py",
        "note": "Synthetic reproductions of R4's findings on 07ce381, from R4's descriptions only. "
                "Adversarial or damaged on purpose: offline reader only, never a desktop viewer.",
        "files": files,
    }, indent=2) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
