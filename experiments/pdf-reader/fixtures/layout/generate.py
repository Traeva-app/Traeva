#!/usr/bin/env python3
"""Program-generated fixtures for the second round (reading order and simple
tables, LAYOUT.md). All content is synthetic.

Writes the PDFs to fixtures/layout/generated/ (git-ignored), their SHA-256 to
fixtures/layout/manifest.json, and each fixture's reading-order truth to
fixtures/layout/truth.json: the lines every page should read as under
LAYOUT.md, written from the fixture's own layout (where each line is placed),
never from the reader's output.

    python3 fixtures/layout/generate.py           # write files, truth and manifest
    python3 fixtures/layout/generate.py --check   # write files, compare hashes with the manifest

Needs reportlab (only for Helvetica text widths).
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import generate as base  # noqa: E402  (the first round's PDF writer)

from reportlab.pdfbase.pdfmetrics import stringWidth  # noqa: E402

OUT = HERE / "generated"
MANIFEST = HERE / "manifest.json"
TRUTH = HERE / "truth.json"

VOCAB = ("memory keeps every saved version of a local file and shows what changed "
         "between two snapshots so a reader can follow the history in order").split()


def width(text, size=10):
    return stringWidth(text, "Helvetica", size)


def fill(head, limit, size=10, start=0):
    """`head` plus vocabulary words while the line stays within `limit` points."""
    words, i = [head], start
    while width(" ".join(words + [VOCAB[i % len(VOCAB)]]), size) <= limit:
        words.append(VOCAB[i % len(VOCAB)])
        i += 1
    return " ".join(words)


def put(text, x, y, size=10):
    return base.line(text, x, y, size)


def centered(text, y, size=14):
    return (text, 306 - width(text, size) / 2, y, size)


def draw(items, order=None):
    order = range(len(items)) if order is None else order
    return b"".join(put(*items[i]) for i in order)


def words_backwards(text, x, y, size=10):
    """Each word as its own text object, the last word first."""
    placed, offset = [], 0.0
    for word in text.split(" "):
        placed.append((word, x + offset))
        offset += width(word + " ", size)
    return b"".join(put(word, at, y, size) for word, at in reversed(placed))


# --- single column ----------------------------------------------------------


def single_column():
    title = ("Single column layout", 72, 740, 14)
    body = [(fill(f"B{i + 1:02d}", 460, 10, start=i * 2), 72, 700 - 14 * i, 10) for i in range(10)]
    return title, body, [[title[0]] + [b[0] for b in body]]


def l01():
    title, body, truth = single_column()
    return base.build([draw([title] + body)]), truth


def l02():
    """Same page as l01, drawn in a shuffled order; line 5 drawn second half first."""
    title, body, truth = single_column()
    parts = []
    for i in [7, 2, 9, 0, 5, 3, 8, 1, 6, 4]:
        text, x, y, size = body[i]
        if i == 4:
            words = text.split(" ")
            first = " ".join(words[: len(words) // 2]) + " "
            second = " ".join(words[len(words) // 2:])
            parts.append(put(second, x + width(first, size), y, size))
            parts.append(put(first, x, y, size))
        else:
            parts.append(put(text, x, y, size))
    parts.append(put(*title))
    return base.build([b"".join(parts)]), truth


# --- two columns --------------------------------------------------------------


def two_columns(lines=8, top=700):
    title = centered("Two column layout", 740)
    left = [(fill(f"L{i + 1:02d}", 210, 10, start=i * 3), 72, top - 14 * i, 10) for i in range(lines)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, start=i * 3 + 5), 324, top - 14 * i, 10) for i in range(lines)]
    return title, left, right, [[title[0]] + [l[0] for l in left] + [r[0] for r in right]]


def l03():
    title, left, right, truth = two_columns()
    return base.build([draw([title] + left + right)]), truth


def l04():
    """Same page as l03, drawn row by row across both columns."""
    title, left, right, truth = two_columns()
    parts = []
    for i, (l, r) in enumerate(zip(left, right)):
        parts += [put(*l), put(*r)]
        if i == 0:
            parts.append(put(*title))
    return base.build([b"".join(parts)]), truth


def l05():
    """Same page as l03: right column bottom-up, then left bottom-up, title last."""
    title, left, right, truth = two_columns()
    return base.build([draw(right[::-1] + left[::-1] + [title])]), truth


def l06():
    """Same page as l03: row by row, right line first, every word drawn last word first."""
    title, left, right, truth = two_columns()
    parts = [words_backwards(*r) + words_backwards(*l) for l, r in zip(left, right)]
    parts.append(words_backwards(*title))
    return base.build([b"".join(parts)]), truth


# --- single and double columns on one page --------------------------------------


def mixed():
    title = centered("Mixed layout report", 740)
    intro = [(fill("Intro01", 460, 10, 0), 72, 700, 10), (fill("Intro02", 460, 10, 7), 72, 686, 10),
             ("Intro03 ends the introduction.", 72, 672, 10)]
    ys = [658 - 14 * i for i in range(6)]
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, y, 10) for i, y in enumerate(ys)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, y, 10) for i, y in enumerate(ys)]
    close = [(fill("Close01", 460, 10, 3), 72, 574, 10), (fill("Close02", 460, 10, 9), 72, 560, 10)]
    truth = [[title[0]] + [t[0] for t in intro + left + right + close]]
    return title, intro, left, right, close, truth


def l07():
    """Closing lines first, the columns row by row, the introduction, the title last."""
    title, intro, left, right, close, truth = mixed()
    rows = [item for pair in zip(left, right) for item in pair]
    return base.build([draw(close + rows + intro + [title])]), truth


def l08():
    """Same page as l07, drawn in reading order."""
    title, intro, left, right, close, truth = mixed()
    return base.build([draw([title] + intro + left + right + close)]), truth


def l09():
    """Three columns, drawn row by row."""
    title = centered("Three column layout", 740)
    xs = [72, 236, 400]
    columns = [[(fill(f"C{c + 1}-{i + 1:02d}", 130, 10, start=c * 7 + i * 2), x, 700 - 14 * i, 10)
                for i in range(6)] for c, x in enumerate(xs)]
    parts = []
    for i in range(6):
        parts += [put(*columns[c][i]) for c in range(3)]
        if i == 2:
            parts.append(put(*title))
    truth = [[title[0]] + [line[0] for column in columns for line in column]]
    return base.build([b"".join(parts)]), truth


def l10():
    """A header row, unequal columns (8 and 4 lines), a centred footer."""
    header = [("Layout report header", 72, 760, 10), ("Page 7", 500, 760, 10)]
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, 700 - 14 * i, 10) for i in range(8)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, 700 - 14 * i, 10) for i in range(4)]
    footer = centered("Footer text for the layout report", 60, 10)
    rows = [item for pair in zip(left[:4], right) for item in pair]
    content = draw([footer] + rows + left[4:] + header[::-1])
    truth = [["Layout report header\tPage 7"] + [l[0] for l in left] + [r[0] for r in right] + [footer[0]]]
    return base.build([content]), truth


def l11():
    """Columns whose lines do not line up: left leading 14, right 17 under a 12 pt heading."""
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, 700 - 14 * i, 10) for i in range(8)]
    heading = ("Right column heading", 324, 700, 12)
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, 683 - 17 * i, 10) for i in range(6)]
    items = left + [heading] + right
    order = sorted(range(len(items)), key=lambda i: (-items[i][2], items[i][1]))
    truth = [[l[0] for l in left] + [heading[0]] + [r[0] for r in right]]
    return base.build([draw(items, order)]), truth


# --- tables -----------------------------------------------------------------------


def grid(xs, ys):
    lines = b"0.5 w\n" + b"".join(b"%d %d m %d %d l S\n" % (xs[0], y, xs[-1], y) for y in ys)
    return lines + b"".join(b"%d %d m %d %d l S\n" % (x, ys[-1], x, ys[0]) for x in xs)


VERSIONS = [("Name", "Version", "Size"), ("alpha", "v1.0", "10 KB"), ("beta", "v2.0", "12 KB"),
            ("gamma", "v3.0", "14 KB")]


def ruled_table():
    xs, ys = [72, 192, 312, 432], [690, 670, 650, 630, 610]
    above = (fill("Above01", 460, 10, 2), 72, 720, 10)
    below = (fill("Below01", 460, 10, 5), 72, 580, 10)
    cells = [[(text, xs[c] + 5, ys[r] - 14, 10) for c, text in enumerate(row)] for r, row in enumerate(VERSIONS)]
    truth = [[above[0]] + ["\t".join(row) for row in VERSIONS] + [below[0]]]
    return grid(xs, ys), above, below, cells, truth


def l12():
    """A ruled table drawn column by column, between two paragraph lines."""
    lines, above, below, cells, truth = ruled_table()
    by_column = [cells[r][c] for c in range(3) for r in range(4)]
    return base.build([draw([below] + by_column) + lines + draw([above])]), truth


def l23():
    """Same page as l12, the table drawn row by row."""
    lines, above, below, cells, truth = ruled_table()
    by_row = [cell for row in cells for cell in row]
    return base.build([draw([above]) + lines + draw(by_row + [below])]), truth


def l13():
    """A ruled table with a two-line cell, drawn row by row."""
    xs, ys = [72, 192, 312, 432], [690, 670, 650, 616, 596]
    above = (fill("Above01", 460, 10, 4), 72, 720, 10)
    items = [above,
             ("Item", 77, 676, 10), ("Note", 197, 676, 10), ("Size", 317, 676, 10),
             ("alpha", 77, 656, 10), ("Short note", 197, 656, 10), ("10 KB", 317, 656, 10),
             ("beta", 77, 636, 10), ("Long note that", 197, 636, 10), ("wraps in the cell", 197, 622, 10),
             ("12 KB", 317, 636, 10),
             ("gamma", 77, 602, 10), ("Last note", 197, 602, 10), ("14 KB", 317, 602, 10)]
    truth = [[above[0], "Item\tNote\tSize", "alpha\tShort note\t10 KB",
              "beta\tLong note that wraps in the cell\t12 KB", "gamma\tLast note\t14 KB"]]
    return base.build([grid(xs, ys) + draw(items)]), truth


STOCK = [("Item", "Count", "Status"), ("apples", "12", "ready"), ("pears", "7", "waiting"),
         ("plums", "30", "ready"), ("grapes", "5", "late")]


def l14():
    """A table without lines (clear column gaps), drawn column by column."""
    above = (fill("Above01", 460, 10, 6), 72, 720, 10)
    below = (fill("Below01", 460, 10, 8), 72, 600, 10)
    xs, ys = [72, 200, 330], [690, 674, 658, 642, 626]
    cells = [(STOCK[r][c], xs[c], ys[r], 10) for c in range(3) for r in range(5)]
    truth = [[above[0]] + ["\t".join(row) for row in STOCK] + [below[0]]]
    return base.build([draw([above] + cells + [below])]), truth


def l20():
    """A table without lines above two columns; columns drawn first, row by row."""
    xs, ys = [72, 200, 330], [720, 704, 688, 672]
    cells = [(STOCK[r][c], xs[c], ys[r], 10) for c in range(3) for r in range(4)]
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, 640 - 14 * i, 10) for i in range(6)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, 640 - 14 * i, 10) for i in range(6)]
    rows = [item for pair in zip(left, right) for item in pair]
    truth = [["\t".join(row) for row in STOCK[:4]] + [l[0] for l in left] + [r[0] for r in right]]
    return base.build([draw(rows + cells)]), truth


# --- things that must not be reordered ------------------------------------------------


def l15():
    """Wide gaps inside single-column text: label rows, a stretched line, a tab stop."""
    para = [fill(f"P{i:02d}", 460, 10, start=i * 4) for i in range(1, 5)]
    words = para[1].split(" ")
    first = " ".join(words[: len(words) // 2]) + " "
    second = " ".join(words[len(words) // 2:])
    content = (put("Name:", 72, 720) + put("Traeva", 300, 720) + put(para[0], 72, 706)
               + put(first, 72, 692) + put(second, 72 + width(first) + 15, 692)
               + put(para[2], 72, 678) + put("Total", 72, 664) + put("42", 500, 664)
               + put(para[3], 72, 650) + put("Tab", 72, 636) + put("stop", 72 + width("Tab") + 29.3, 636))
    truth = [["Name:\tTraeva", para[0], para[1], para[2], "Total\t42", para[3], "Tab\tstop"]]
    return base.build([content]), truth


def l16():
    """Two label rows: wide labels drawn column-wise (page 1), short labels (page 2)."""
    above, below = fill("Above01", 460, 10, 1), fill("Below01", 460, 10, 3)
    page1 = draw([(above, 72, 720, 10), ("Project name:", 72, 700, 10), ("Release date:", 72, 686, 10),
                  ("Traeva layout experiment", 250, 700, 10), ("2026-10-12", 250, 686, 10),
                  (below, 72, 660, 10)])
    page2 = draw([(above, 72, 720, 10), ("Name:", 72, 700, 10), ("Date:", 72, 686, 10),
                  ("Traeva", 160, 700, 10), ("2026-10-12", 160, 686, 10), (below, 72, 660, 10)])
    truth = [
        # Not recognized: kept in drawing order (LAYOUT.md 3.4c).
        [above, "Project name:", "Release date:", "Traeva layout experiment", "2026-10-12", below],
        [above, "Name:\tTraeva", "Date:\t2026-10-12", below],
    ]
    return base.build([page1, page2]), truth


def l17():
    """Two columns (row by row) with a rotated label in the gutter and a rotated margin note."""
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, 700 - 14 * i, 10) for i in range(6)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, 700 - 14 * i, 10) for i in range(6)]
    gutter = b"BT /F1 10 Tf 0 1 -1 0 310 620 Tm (Gutter label) Tj ET\n"
    margin = b"BT /F1 10 Tf 0 1 -1 0 40 680 Tm (Margin note text) Tj ET\n"
    rows = [put(*item) for pair in zip(left, right) for item in pair]
    content = gutter + b"".join(rows[:6]) + margin + b"".join(rows[6:])
    # Rotated lines go after the upright text, by the top of their box: the
    # margin note reaches higher on the page than the gutter label.
    truth = [[l[0] for l in left] + [r[0] for r in right] + ["Margin note text", "Gutter label"]]
    return base.build([content]), truth


CJK_POOL = ("版本记忆会保存每一次真实的快照然后再异步分析两次之间的变化并且记录来源与时间以便之后检索"
            "历史内容和比较不同版本之间的差异以及每个文件现在所在的位置和它曾经用过的名字")


def cjk_lines(side, count):
    marks = "一二三四五六七八九十"
    lines = []
    for k in range(count):
        start = (k + (0 if side == "左" else count)) * 14
        body = (CJK_POOL * 2)[start:start + 14]
        lines.append(f"{side}栏第{marks[k]}行{body}")
    return lines


def cjk_font(doc, chars):
    """A non-embedded Identity-H font whose ToUnicode maps code n to chars[n-1]."""
    blocks = []
    for at in range(0, len(chars), 100):
        part = chars[at:at + 100]
        blocks.append(b"%d beginbfchar\n" % len(part) + b"".join(
            b"<%04X> <%04X>\n" % (at + i + 1, ord(c)) for i, c in enumerate(part)) + b"endbfchar\n")
    cmap = (b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
            b"/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n"
            b"/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n"
            b"1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n" + b"".join(blocks)
            + b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend")
    descriptor = doc.add(b"<< /Type /FontDescriptor /FontName /TestCJK /Flags 4 /FontBBox [0 -120 1000 880] "
                         b"/ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>")
    cid = doc.add(b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestCJK /CIDSystemInfo "
                  b"<< /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor %d 0 R "
                  b"/DW 1000 /CIDToGIDMap /Identity >>" % descriptor)
    to_unicode = doc.add(base.flate(b"", cmap))
    return doc.add(b"<< /Type /Font /Subtype /Type0 /BaseFont /TestCJK /Encoding /Identity-H "
                   b"/DescendantFonts [%d 0 R] /ToUnicode %d 0 R >>" % (cid, to_unicode))


def l18():
    """Chinese two columns, drawn row by row."""
    left, right = cjk_lines("左", 5), cjk_lines("右", 5)
    chars = sorted(set("".join(left + right)))
    code = {c: i + 1 for i, c in enumerate(chars)}

    def show(text, x, y):
        hexes = b"".join(b"%04X" % code[c] for c in text)
        return b"BT /F2 10 Tf %d %d Td <%s> Tj ET\n" % (x, y, hexes)

    doc = base.Doc()
    catalog, tree = doc.reserve(), doc.reserve()
    font = doc.add(base.HELVETICA)
    cjk = cjk_font(doc, chars)
    content = b"".join(show(l, 72, 700 - 14 * i) + show(r, 324, 700 - 14 * i)
                       for i, (l, r) in enumerate(zip(left, right)))
    content_ref = doc.add(base.flate(b"", content))
    page = doc.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << "
                   b"/F1 %d 0 R /F2 %d 0 R >> >> /Contents %d 0 R >>" % (tree, font, cjk, content_ref))
    doc.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    doc.set(catalog, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return doc.build(catalog), [left + right]


def l19():
    """Two columns of only two lines each, drawn row by row: cannot be told from a table."""
    title = centered("Short column section", 740)
    left = [(fill(f"L{i + 1:02d}", 210, 10, i * 3), 72, 700 - 14 * i, 10) for i in range(2)]
    right = [(fill(f"R{i + 1:02d}", 210, 10, i * 3 + 5), 324, 700 - 14 * i, 10) for i in range(2)]
    below = (fill("Below01", 460, 10, 2), 72, 660, 10)
    rows = [item for pair in zip(left, right) for item in pair]
    # Not recognized: drawing order, rows joined with a TAB at the gutter.
    truth = [[title[0]] + [f"{l[0]}\t{r[0]}" for l, r in zip(left, right)] + [below[0]]]
    return base.build([draw([title] + rows + [below])]), truth


def l21():
    """Justified two columns with a 10 pt gutter at 10 pt (LaTeX-like), drawn row by row."""
    def justified(head, start, x, y):
        text = fill(head, 222, 10, start)
        spaces = text.count(" ")
        tw = (229 - width(text)) / spaces
        return b"BT /F1 10 Tf %.4f Tw %d %d Td %s Tj ET\n" % (tw, x, y, base.pdf_string(text)), text

    content, left, right = b"", [], []
    for i in range(8):
        l_bytes, l_text = justified(f"L{i + 1:02d}", i * 3, 72, 700 - 14 * i)
        r_bytes, r_text = justified(f"R{i + 1:02d}", i * 3 + 5, 311, 700 - 14 * i)
        content += l_bytes + r_bytes
        left.append(l_text)
        right.append(r_text)
    return base.build([content]), [left + right]


def l22():
    """A numbered list whose second item wraps."""
    above = fill("Above01", 460, 10, 7)
    items = [(above, 72, 740, 10), ("1.", 72, 720, 10), ("First item text", 92, 720, 10),
             ("2.", 72, 706, 10), ("Second item that is", 92, 706, 10), ("longer and wraps", 92, 692, 10),
             ("3.", 72, 678, 10), ("Third item", 92, 678, 10)]
    truth = [[above, "1. First item text", "2. Second item that is", "longer and wraps", "3. Third item"]]
    return base.build([draw(items)]), truth


def justified_columns(per_word):
    """Two narrow justified columns of equal-width words, gaps about 1.3 em,
    drawn row by row. One text operation per line (Tw), or one per word."""
    def words(side, row):
        return [f"{side}{row:02d}w{k}" for k in range(1, 6)]

    content, left, right = b"", [], []
    for i in range(8):
        y = 700 - 14 * i
        for side, x, lines in (("L", 72, left), ("R", 324, right)):
            row = words(side, i + 1)
            gap = (200 - sum(width(w) for w in row)) / 4
            lines.append(" ".join(row))
            if per_word:
                at = x
                for w in row:
                    content += put(w, at, y)
                    at += width(w) + gap
            else:
                tw = gap - width(" ")
                content += b"BT /F1 10 Tf %.4f Tw %d %d Td %s Tj ET\n" % (tw, x, y, base.pdf_string(" ".join(row)))
    return base.build([content]), [left + right]


def l24():
    return justified_columns(per_word=False)


def l25():
    return justified_columns(per_word=True)


MAKERS = [
    ("l01-single-column.pdf", l01), ("l02-single-column-shuffled.pdf", l02),
    ("l03-two-columns-by-column.pdf", l03), ("l04-two-columns-by-row.pdf", l04),
    ("l05-two-columns-reversed.pdf", l05), ("l06-two-columns-word-order.pdf", l06),
    ("l07-mixed-out-of-order.pdf", l07), ("l08-mixed-in-order.pdf", l08),
    ("l09-three-columns.pdf", l09), ("l10-unequal-columns-header-footer.pdf", l10),
    ("l11-staggered-columns.pdf", l11), ("l12-ruled-table-by-column.pdf", l12),
    ("l13-ruled-table-two-line-cell.pdf", l13), ("l14-aligned-table.pdf", l14),
    ("l15-wide-gaps-single-column.pdf", l15), ("l16-label-rows.pdf", l16),
    ("l17-rotated-in-gutter.pdf", l17), ("l18-cjk-two-columns.pdf", l18),
    ("l19-short-columns.pdf", l19), ("l20-table-then-columns.pdf", l20),
    ("l21-narrow-gutter-justified.pdf", l21), ("l22-wrapped-list.pdf", l22),
    ("l23-ruled-table-by-row.pdf", l23), ("l24-justified-wide-gaps-per-line.pdf", l24),
    ("l25-justified-wide-gaps-per-word.pdf", l25),
]

# Fixtures that look the same on the page and differ only in drawing order:
# each group must read the same.
SAME_PAGE = [
    ["l01-single-column.pdf", "l02-single-column-shuffled.pdf"],
    ["l03-two-columns-by-column.pdf", "l04-two-columns-by-row.pdf", "l05-two-columns-reversed.pdf",
     "l06-two-columns-word-order.pdf"],
    ["l07-mixed-out-of-order.pdf", "l08-mixed-in-order.pdf"],
    ["l12-ruled-table-by-column.pdf", "l23-ruled-table-by-row.pdf"],
    ["l24-justified-wide-gaps-per-line.pdf", "l25-justified-wide-gaps-per-word.pdf"],
]


def main():
    OUT.mkdir(exist_ok=True)
    files, truth = {}, {}
    for name, maker in MAKERS:
        data, pages = maker()
        (OUT / name).write_bytes(data)
        files[name] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
        truth[name] = pages
    if "--check" in sys.argv:
        recorded = json.loads(MANIFEST.read_text())["files"]
        changed = sorted(n for n in files if recorded.get(n, {}).get("sha256") != files[n]["sha256"])
        missing = sorted(set(recorded) ^ set(files))
        print(json.dumps({"changed": changed, "missing_or_new": missing}, indent=2))
        sys.exit(1 if changed or missing else 0)
    MANIFEST.write_text(json.dumps({
        "generator": "fixtures/layout/generate.py",
        "note": "Synthetic files only; second round (reading order and simple tables).",
        "files": files,
    }, indent=2) + "\n")
    TRUTH.write_text(json.dumps({
        "rules": "LAYOUT.md",
        "note": "Lines each page should read as, from each fixture's own layout.",
        "same_page": SAME_PAGE,
        "pages": truth,
    }, indent=1, ensure_ascii=False) + "\n")
    print(f"{len(files)} files in {OUT}")


if __name__ == "__main__":
    main()
