#!/usr/bin/env python3
"""Writes the program-generated PDF fixtures for the #28 PDF Reader experiment.

All content is synthetic. Output goes to fixtures/generated/ (git-ignored);
fixtures/manifest.json records each file's SHA-256 so a rerun can prove the
same bytes were tested. Adversarial files (prefix a-/c-) are for the offline
reader only: do not open them in a desktop PDF viewer.

    python3 fixtures/generate.py           # write files and the manifest
    python3 fixtures/generate.py --check   # write files, compare with the manifest

Needs reportlab (p03 only) and pypdf (a18/a19 only).
"""

import hashlib
import json
import random
import struct
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"
WQY = "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"


# --- a minimal PDF writer -------------------------------------------------


class Doc:
    """Numbered objects, a classic xref table and a trailer. Offsets are exact."""

    def __init__(self):
        self.bodies = []

    def reserve(self):
        self.bodies.append(None)
        return len(self.bodies)

    def set(self, number, body):
        self.bodies[number - 1] = body

    def add(self, body):
        number = self.reserve()
        self.set(number, body)
        return number

    def build(self, root, trailer_extra=b"", size=None, header=b"%PDF-1.7\n"):
        out = bytearray(header + b"%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for number, body in enumerate(self.bodies, start=1):
            assert body is not None, f"object {number} never set"
            offsets.append(len(out))
            out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
        xref = len(out)
        count = len(self.bodies) + 1
        out += b"xref\n0 %d\n0000000000 65535 f \n" % count
        for offset in offsets:
            out += b"%010d 00000 n \n" % offset
        out += b"trailer\n<< /Size %d /Root %d 0 R%s >>\nstartxref\n%d\n%%%%EOF\n" % (
            size or count,
            root,
            trailer_extra,
            xref,
        )
        return bytes(out)


def stream(entries, data):
    return b"<< %s /Length %d >>\nstream\n" % (entries, len(data)) + data + b"\nendstream"


def flate(entries, data):
    return stream(entries + b" /Filter /FlateDecode", zlib.compress(data, 9))


def pdf_string(text):
    raw = text.encode("latin-1") if isinstance(text, str) else text
    return b"(" + raw.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)") + b")"


def line(text, x, y, size=12, font=b"F1"):
    return b"BT /%s %s Tf %s %s Td %s Tj ET\n" % (font, num(size), num(x), num(y), pdf_string(text))


def num(value):
    return (b"%d" % value) if float(value).is_integer() else (b"%.4f" % value).rstrip(b"0")


HELVETICA = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"


def simple_doc(pages, fonts=None, catalog_extra=b"", trailer_extra=b"", count=None,
               page_extra=None, xobjects=None):
    """pages: list of content bytes. fonts: {name: body}. Returns (doc, root)."""
    doc = Doc()
    catalog = doc.reserve()
    tree = doc.reserve()
    font_refs = b"".join(
        b"/%s %d 0 R " % (name, doc.add(body)) for name, body in (fonts or {b"F1": HELVETICA}).items()
    )
    xobject_refs = b"".join(b"/%s %d 0 R " % (name, ref) for name, ref in (xobjects or {}).items())
    kids = []
    for index, content in enumerate(pages):
        content_ref = doc.add(flate(b"", content))
        extra = (page_extra or {}).get(index, b"")
        kids.append(doc.add(
            b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << %s>> /XObject << %s>> >> /Contents %d 0 R %s>>"
            % (tree, font_refs, xobject_refs, content_ref, extra)
        ))
    doc.set(tree, b"<< /Type /Pages /Kids [%s] /Count %d >>" % (
        b" ".join(b"%d 0 R" % kid for kid in kids), count if count is not None else len(kids)))
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R %s>>" % (tree, catalog_extra))
    return doc, catalog


def build(pages, **kwargs):
    trailer_extra = kwargs.pop("trailer_extra", b"")
    doc, root = simple_doc(pages, **kwargs)
    return doc.build(root, trailer_extra=trailer_extra)


def gray_image(doc, width, height):
    pixels = bytes((x * 7 + y * 3) % 256 for y in range(height) for x in range(width))
    return doc.add(flate(
        b"/Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray "
        b"/BitsPerComponent 8" % (width, height),
        pixels,
    ))


# --- program-generated text fixtures --------------------------------------


def p01():
    return build([
        line("Traeva PDF reader test", 72, 720)
        + line("The quick brown fox jumps over the lazy dog.", 72, 700)
        + line("Version 1, single page.", 72, 680)
    ])


def p02():
    words = ["one", "two", "three"]
    return build([
        line(f"Page {w} heading", 72, 720) + line(f"Body text on page {w}.", 72, 700)
        for w in words
    ])


def p03(path):
    from reportlab import rl_config
    rl_config.invariant = 1
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.cidfonts import UnicodeCIDFont
    from reportlab.pdfbase.ttfonts import TTFont
    from reportlab.pdfgen import canvas

    pdfmetrics.registerFont(UnicodeCIDFont("STSong-Light"))
    pdfmetrics.registerFont(TTFont("WQY", WQY, subfontIndex=0))
    c = canvas.Canvas(str(path), pagesize=(612, 792))
    c.setFont("STSong-Light", 14)
    c.drawString(72, 720, "版本记忆测试")
    c.drawString(72, 696, "第二行：中文与English混排")
    c.showPage()
    c.setFont("WQY", 14)
    c.drawString(72, 720, "嵌入字体的中文段落")
    c.drawString(72, 696, "Mixed 中文 and English")
    c.showPage()
    c.save()


def p04():
    page1 = (
        line("Upright heading", 72, 720)
        + b"BT /F1 12 Tf 0 1 -1 0 300 400 Tm (Rotated ninety degrees) Tj ET\n"
        + b"BT /F1 12 Tf 0.7071 0.7071 -0.7071 0.7071 100 200 Tm (Tilted forty-five) Tj ET\n"
        + line("Upright footer", 72, 100)
    )
    # Shown with /Rotate 90: text drawn upward in user space reads left to right.
    page2 = b"BT /F1 12 Tf 0 1 -1 0 300 100 Tm (Landscape page text) Tj ET\n"
    return build([page1, page2], page_extra={1: b"/Rotate 90 "})


def p05():
    title = line("Two column report", 72, 740, 14)
    left = [f"Left column line {w}" for w in ("one", "two", "three", "four")]
    right = [f"Right column line {w}" for w in ("one", "two", "three", "four")]
    ys = [700, 686, 672, 658]
    by_column = title + b"".join(line(t, 72, y, 10) for t, y in zip(left, ys)) + b"".join(
        line(t, 330, y, 10) for t, y in zip(right, ys))
    by_row = title + b"".join(
        line(l, 72, y, 10) + line(r, 330, y, 10) for l, r, y in zip(left, right, ys))
    return build([by_column, by_row])


TO_UNICODE = b"""/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
10 beginbfchar
<0001> <0054>
<0002> <014D>
<0003> <006B>
<0004> <0079>
<0005> <0020>
<0006> <6771>
<0007> <4EAC>
<0008> <00660069>
<0009> <006E>
<000A> <0065>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end"""


def p06():
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    differences = doc.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding "
                          b"<< /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /Z] >> >>")
    descriptor = doc.add(b"<< /Type /FontDescriptor /FontName /TestCID /Flags 32 /FontBBox [0 -200 1000 900] "
                         b"/ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 >>")
    cid = doc.add(b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestCID /CIDSystemInfo "
                  b"<< /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor %d 0 R "
                  b"/DW 1000 /CIDToGIDMap /Identity >>" % descriptor)
    to_unicode = doc.add(flate(b"", TO_UNICODE))
    type0 = doc.add(b"<< /Type /Font /Subtype /Type0 /BaseFont /TestCID /Encoding /Identity-H "
                    b"/DescendantFonts [%d 0 R] /ToUnicode %d 0 R >>" % (cid, to_unicode))
    procs = [doc.add(stream(b"", b"600 0 0 0 600 700 d1 0 0 600 700 re f")) for _ in range(3)]
    type3 = doc.add(b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] "
                    b"/FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /g1 %d 0 R /g2 %d 0 R /g3 %d 0 R >> "
                    b"/Encoding << /Type /Encoding /Differences [97 /g1 /g2 /g3] >> /FirstChar 97 "
                    b"/LastChar 99 /Widths [600 600 600] /Resources << >> >>" % tuple(procs))
    uni_names = doc.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding "
                        b"<< /Type /Encoding /Differences [65 /uni4E2D /uni6587] >> >>")
    builtin = doc.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    content = (
        line("Aebra", 72, 720, font=b"FD")
        + b"BT /FC 12 Tf 72 700 Td <0001000200030004000200050006000700050008000900 0A> Tj ET\n"
        + line("abc", 72, 680, font=b"F3")
        + line("AB", 72, 660, font=b"FU")
        + b"BT /FB 12 Tf 72 640 Td (\\256le) Tj ET\n"
    )
    content_ref = doc.add(flate(b"", content))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                   b"/FD %d 0 R /FC %d 0 R /F3 %d 0 R /FU %d 0 R /FB %d 0 R >> >> /Contents %d 0 R >>"
                   % (tree, differences, type0, type3, uni_names, builtin, content_ref))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


def p07():
    return build([b""])


def image_page_doc(page_specs):
    """page_specs: list of (content_without_image, image box or None)."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    image = gray_image(doc, 200, 260)
    kids = []
    for content, box in page_specs:
        if box:
            x, y, w, h = box
            content = b"q %s 0 0 %s %s %s cm /Im1 Do Q\n" % (num(w), num(h), num(x), num(y)) + content
        content_ref = doc.add(flate(b"", content))
        kids.append(doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << "
                            b"/Font << /F1 %d 0 R >> /XObject << /Im1 %d 0 R >> >> /Contents %d 0 R >>"
                            % (tree, font, image, content_ref)))
    doc.set(tree, b"<< /Type /Pages /Kids [%s] /Count %d >>" % (
        b" ".join(b"%d 0 R" % k for k in kids), len(kids)))
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


FULL_PAGE = (0, 0, 612, 792)


def p08():
    return image_page_doc([(b"", FULL_PAGE), (b"", FULL_PAGE)])


def p09():
    return image_page_doc([
        (line("Mixed page heading", 72, 720) + line("Caption below the picture.", 72, 380),
         (72, 400, 400, 300)),
        (b"", FULL_PAGE),
        (line("Final text page.", 72, 720), None),
    ])


def p10():
    grid = b"0.5 w\n" + b"".join(b"70 %d m 400 %d l S\n" % (y, y) for y in (600, 580, 560, 540)) + b"".join(
        b"%d 540 m %d 600 l S\n" % (x, x) for x in (70, 190, 310, 400))
    cells = b""
    for row, y in zip((("Name", "Version", "Size"), ("alpha", "1", "10 KB"), ("beta", "2", "12 KB")),
                      (586, 566, 546)):
        for text, x in zip(row, (75, 195, 315)):
            cells += line(text, x, y, 10)
    return build([line("Table of versions", 72, 630) + grid + cells])


def p11():
    content = (
        b"BT /F1 12 Tf 72 700 Td (Energy: E = mc) Tj 4 Ts /F1 8 Tf (2) Tj ET\n"
        + b"BT /F2 12 Tf 72 680 Td (\\345 \\243 \\245) Tj ET\n"
    )
    return build([content], fonts={
        b"F1": HELVETICA,
        b"F2": b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>",
    })


def p12():
    doc = Doc()
    catalog, tree, page = doc.reserve(), doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    appearance = doc.add(stream(
        b"/Type /XObject /Subtype /Form /BBox [0 0 228 20] /Resources << /Font << /Helv %d 0 R >> >>" % font,
        b"/Tx BMC BT /Helv 12 Tf 2 5 Td (secret value) Tj ET EMC"))
    widget = doc.add(b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (secret value) "
                     b"/Rect [72 600 300 620] /P %d 0 R /AP << /N %d 0 R >> >>" % (page, appearance))
    note = doc.add(b"<< /Type /Annot /Subtype /Text /Rect [400 700 420 720] /Contents (sticky note text) >>")
    attached = doc.add(stream(b"/Type /EmbeddedFile", b"attached synthetic text"))
    filespec = doc.add(b"<< /Type /Filespec /F (note.txt) /EF << /F %d 0 R >> >>" % attached)
    content = doc.add(flate(b"", line("Form page", 72, 720)))
    doc.set(page, b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                  b"/F1 %d 0 R >> >> /Contents %d 0 R /Annots [%d 0 R %d 0 R] >>"
            % (tree, font, content, widget, note))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R /AcroForm << /Fields [%d 0 R] /DA (/Helv 0 Tf 0 g) "
                     b"/DR << /Font << /Helv %d 0 R >> >> >> "
                     b"/OpenAction << /S /JavaScript /JS (app.alert\\(1\\);) >> "
                     b"/Names << /EmbeddedFiles << /Names [(note.txt) %d 0 R] >> >> >>"
            % (tree, widget, font, filespec))
    return doc.build(catalog)


def p13():
    return image_page_doc([
        (b"BT 3 Tr /F1 12 Tf 72 700 Td (Scanned invoice number 42) Tj ET\n", FULL_PAGE),
    ])


def p14():
    return build([
        line("Visible line", 72, 700)
        + line("Hidden outside text", -500, 700)
        + b"BT 2 Tr /F1 12 Tf 72 680 Td (Stroked once) Tj ET\n"
        + line("Bold twice", 72, 660)
        + line("Bold twice", 72.3, 660)
    ])


def p15():
    return build([
        b"BT /F1 12 Tf 72 720 Td [(Word)-333(spacing)-333(by)-333(position)] TJ ET\n"
        + b"BT /F1 12 Tf 72 700 Td [(K)20(erning)] TJ ET\n"
        + b"BT /F1 12 Tf 72 680 Td (Wide) Tj 30 0 Td (gaps) Tj ET\n"
        + b"BT /F1 12 Tf 72 660 Td (Tab) Tj 50 0 Td (stop) Tj ET\n"
    ])


P16_PAGES, P16_LINES = 120, 90


def p16_line(page, index):
    head = f"{page:03d}.{index:02d} "
    filler = "lorem ipsum dolor sit amet " * 4
    return (head + filler)[:100]


def p16():
    pages = []
    for page in range(1, P16_PAGES + 1):
        content = b"".join(
            line(p16_line(page, i), 36, 780 - 8 * i, 7) for i in range(P16_LINES))
        pages.append(content)
    return build(pages)


def p17():
    run = b"BT /F1 1 Tf 10 10 Td (" + b"a" * 1000 + b") Tj ET\n"
    return build([run * 2100])


def p18():
    return build([b"0 0 1 1 re f\n" * 1_100_000])


# --- adversarial and resource-limit fixtures ------------------------------


def with_content_stream(body):
    """One page whose content stream object body is given as is."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    content = doc.add(body)
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                   b"/F1 %d 0 R >> >> /Contents %d 0 R >>" % (tree, font, content))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc, catalog


def a01():
    doc, root = with_content_stream(flate(b"", b" " * (20 << 20)))
    return doc.build(root)


def a02():
    inner = zlib.compress(b" " * (20 << 20), 9)
    body = stream(b"/Filter [/FlateDecode /FlateDecode]", zlib.compress(inner, 9))
    doc, root = with_content_stream(body)
    return doc.build(root)


def a03():
    doc, root = with_content_stream(flate(b"", line("Image bomb file", 72, 700)))
    doc.add(flate(b"/Type /XObject /Subtype /Image /Width 9000 /Height 9000 /ColorSpace /DeviceGray "
                  b"/BitsPerComponent 8", bytes(80 << 20)))
    return doc.build(root)


def a04():
    doc, root = with_content_stream(flate(b"", line("Many large streams", 72, 700)))
    for _ in range(10):
        doc.add(flate(b"", b" " * (14 << 20)))
    return doc.build(root)


def a05():
    return build([b""] * 501)


def a06():
    doc, root = simple_doc([line("Declared size", 72, 700)])
    return doc.build(root, size=300_000)


def a07a():
    nested = b"[" * 100 + b"]" * 100
    doc, root = with_content_stream(stream(b"/X " + nested, line("Deep", 72, 700)))
    return doc.build(root)


def a07b():
    nested = b"[" * 1000 + b"]" * 1000
    return build([line("Deep page", 72, 700)], page_extra={0: b"/Deep " + nested + b" "})


def form_doc(forms, page_content):
    """forms: {name: (content, [names it uses])}. Every form sees every form."""
    doc = Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(HELVETICA)
    refs = {name: doc.reserve() for name in forms}
    xobjects = b"".join(b"/%s %d 0 R " % (name, ref) for name, ref in refs.items())
    for name, content in forms.items():
        doc.set(refs[name], flate(
            b"/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> "
            b"/XObject << %s>> >>" % (font, xobjects), content))
    content_ref = doc.add(flate(b"", page_content))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                   b"/F1 %d 0 R >> /XObject << %s>> >> /Contents %d 0 R >>" % (tree, font, xobjects, content_ref))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog)


def a08():
    return form_doc({b"Fm": line("loop", 72, 700) + b"/Fm Do\n"}, b"/Fm Do\n")


def a09():
    return form_doc({
        b"A": b"/B Do\n" * 50,
        b"B": b"/C Do\n" * 50,
        b"C": b"BT /F1 1 Tf 10 10 Td (aaaaaaaaaaaaaaaaaaaa) Tj ET\n",
    }, b"/A Do\n" * 50)


def a10():
    doc, root = with_content_stream(stream(b"/Filter /LZWDecode", b"\x80\x0b\x60\x50\x22\x0c\x0c\x85\x01"))
    return doc.build(root)


def a11():
    doc, root = with_content_stream(stream(b"/Filter /DCTDecode", b"\xff\xd8\xff\xd9"))
    return doc.build(root)


def a12():
    doc, root = with_content_stream(stream(b"/Filter 99 0 R", b"x"))
    return doc.build(root)


def a13():
    doc, root = with_content_stream(stream(b"/Filter /FooDecode", b"x"))
    return doc.build(root)


def a14():
    # Each pair (129, b" ") expands to 128 bytes: 160,000 pairs = 20 MB.
    doc, root = with_content_stream(stream(b"/Filter /RunLengthDecode", b"\x81 " * 160_000 + b"\x80"))
    return doc.build(root)


def a15():
    doc, root = with_content_stream(flate(b"", line("Many streams", 72, 700)))
    for _ in range(50_001):
        doc.add(stream(b"", b"a"))
    return doc.build(root)


def a16():
    # A valid file over the 32 MiB input limit: comment lines before the
    # cross-reference table, with startxref moved to match.
    doc, root = simple_doc([line("Big input", 72, 700)])
    pdf = doc.build(root)
    xref = pdf.rindex(b"xref\n0 ")
    padding = (b"%" + b"p" * 78 + b"\n") * ((33 << 20) // 80)
    head, _, _ = pdf.rpartition(b"startxref\n")
    body = head[:xref] + padding + head[xref:]
    return body + b"startxref\n%d\n%%%%EOF\n" % (xref + len(padding))


def a17():
    # A stored deflate block holding the bytes "endstream", then 20 MiB of
    # compressed spaces: a scanner that stops at the first "endstream" would
    # miss the bomb that /Length covers.
    stored = b"\nendstream\n"
    body = struct.pack("<BHH", 0, len(stored), len(stored) ^ 0xFFFF) + stored
    compressor = zlib.compressobj(9, zlib.DEFLATED, -15)
    zeros = b" " * (20 << 20)
    body += compressor.compress(zeros) + compressor.flush()
    adler = zlib.adler32(stored + zeros) & 0xFFFFFFFF
    data = b"\x78\xda" + body + struct.pack(">I", adler)
    assert zlib.decompress(data) == stored + zeros
    doc, root = with_content_stream(stream(b"/Filter /FlateDecode", data))
    return doc.build(root)


def a18(path):
    from pypdf import PdfReader, PdfWriter
    writer = PdfWriter(clone_from=PdfReader(OUT / "p01-single-en.pdf"))
    writer.encrypt(user_password="", owner_password="owner-only", algorithm="RC4-128")
    with open(path, "wb") as handle:
        writer.write(handle)


def a19(path):
    from pypdf import PdfReader, PdfWriter
    writer = PdfWriter(clone_from=PdfReader(OUT / "p01-single-en.pdf"))
    writer.encrypt(user_password="user-secret", owner_password="owner-secret", algorithm="AES-256")
    with open(path, "wb") as handle:
        writer.write(handle)


def a20():
    doc, root = simple_doc([line("Escaped encrypt key", 72, 700)])
    encrypt = doc.add(b"<< /Filter /Standard /V 1 /R 2 /O <%s> /U <%s> /P -4 >>" % (b"00" * 32, b"00" * 32))
    return doc.build(root, trailer_extra=b" /Encr#79pt %d 0 R /ID [<00> <00>]" % encrypt)


# --- damaged files ----------------------------------------------------------


def c01(p02_bytes):
    return p02_bytes[: len(p02_bytes) // 2]


def c02(p02_bytes):
    return p02_bytes[:-10]


def c03(p01_bytes):
    head, _, tail = p01_bytes.rpartition(b"startxref\n")
    offset = int(tail.split(b"\n")[0])
    return head + b"startxref\n%d\n%%%%EOF\n" % (offset + 7)


def c04():
    packed = zlib.compress(line("Damaged stream text", 72, 700) * 20, 9)
    doc, root = with_content_stream(stream(b"/Filter /FlateDecode", packed[: len(packed) // 2]))
    return doc.build(root)


def c06():
    rng = random.Random(7)
    return b"%PDF-1.7\n" + bytes(rng.randrange(256) for _ in range(4096)) + b"\n%%EOF\n"


def c07():
    return build([line("Page A", 72, 700), line("Page B", 72, 700)], count=3)


# --- driver -------------------------------------------------------------------


def generate():
    OUT.mkdir(exist_ok=True)
    made = {}

    def put(name, data, origin="raw-writer"):
        (OUT / name).write_bytes(data)
        made[name] = origin

    for name, maker in [
        ("p01-single-en.pdf", p01), ("p02-multi-en.pdf", p02), ("p04-rotated.pdf", p04),
        ("p05-two-column.pdf", p05), ("p06-font-mapping.pdf", p06), ("p07-blank.pdf", p07),
        ("p08-scanned-images.pdf", p08), ("p09-mixed-pages.pdf", p09), ("p10-ruled-table.pdf", p10),
        ("p11-formula.pdf", p11), ("p12-annotations-form.pdf", p12), ("p13-invisible-ocr-layer.pdf", p13),
        ("p14-outside-and-duplicates.pdf", p14), ("p15-positioned-words.pdf", p15),
        ("p16-text-cap.pdf", p16), ("p17-glyph-budget.pdf", p17), ("p18-draw-budget.pdf", p18),
        ("a01-flate-bomb.pdf", a01), ("a02-nested-flate.pdf", a02), ("a03-image-label-bomb.pdf", a03),
        ("a04-decoded-total.pdf", a04), ("a05-too-many-pages.pdf", a05),
        ("a06-declared-objects.pdf", a06), ("a07a-deep-stream-dict.pdf", a07a),
        ("a07b-deep-page-dict.pdf", a07b), ("a08-form-recursion.pdf", a08),
        ("a09-form-fanout.pdf", a09), ("a10-lzw.pdf", a10), ("a11-codec-on-content.pdf", a11),
        ("a12-indirect-filter.pdf", a12), ("a13-unknown-filter.pdf", a13),
        ("a14-run-length-bomb.pdf", a14), ("a15-stream-count.pdf", a15), ("a16-big-input.pdf", a16),
        ("a17-fake-endstream.pdf", a17), ("a20-escaped-encrypt.pdf", a20),
        ("c04-damaged-flate.pdf", c04), ("c06-garbage.pdf", c06), ("c07-page-count.pdf", c07),
    ]:
        put(name, maker())

    p03(OUT / "p03-cjk-reportlab.pdf")
    made["p03-cjk-reportlab.pdf"] = "reportlab"
    a18(OUT / "a18-encrypted-owner-only.pdf")
    made["a18-encrypted-owner-only.pdf"] = "pypdf"
    a19(OUT / "a19-encrypted-user-password.pdf")
    made["a19-encrypted-user-password.pdf"] = "pypdf"

    p01_bytes = (OUT / "p01-single-en.pdf").read_bytes()
    p02_bytes = (OUT / "p02-multi-en.pdf").read_bytes()
    put("c01-truncated-half.pdf", c01(p02_bytes))
    put("c02-truncated-tail.pdf", c02(p02_bytes))
    put("c03-wrong-startxref.pdf", c03(p01_bytes))
    put("c05-not-a-pdf.pdf", b"This is a plain text file, not a PDF.\n")
    put("c08-empty.pdf", b"")
    return made


def describe(made):
    entries = {}
    for name in sorted(made):
        data = (OUT / name).read_bytes()
        entries[name] = {
            "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data),
            "origin": made[name],
            # pypdf encryption uses random salts and IVs.
            "deterministic": made[name] != "pypdf",
        }
    return entries


def main():
    entries = describe(generate())
    if "--check" in sys.argv:
        recorded = json.loads(MANIFEST.read_text())["files"]
        bad = [
            name for name, entry in entries.items()
            if entry["deterministic"] and recorded.get(name, {}).get("sha256") != entry["sha256"]
        ]
        missing = sorted(set(recorded) ^ set(entries))
        print(json.dumps({"changed": bad, "missing_or_new": missing}, indent=2))
        sys.exit(1 if bad or missing else 0)
    MANIFEST.write_text(json.dumps({
        "generator": "fixtures/generate.py",
        "note": "Synthetic files only. a-/c- files are adversarial: offline reader only, never a desktop viewer.",
        "files": entries,
    }, indent=2, ensure_ascii=False) + "\n")
    print(f"{len(entries)} files in {OUT}")


if __name__ == "__main__":
    main()
