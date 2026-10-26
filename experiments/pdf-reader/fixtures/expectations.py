#!/usr/bin/env python3
"""Frozen expectations for the #28 PDF Reader experiment.

Written from each fixture's own source (generate.py, export.py) and the
output rules in README.md, before the reader was first run on any fixture.
Derived numbers (character counts, page offsets, the text-limit cut) are
computed here from the expected text by those rules, never from the reader.

    python3 fixtures/expectations.py   # writes fixtures/expected.json

Keys per fixture (all optional except status):
  text               exact text (text_sha256 + text_head/text_tail for p16)
  snippets_in_order  each must occur in the text after the previous one
  note               exact note;  note_contains: substring of the note
  left_out           exact list;  left_out_kinds: substrings that must each match an entry
  left_out_absent    substrings no entry may contain
  covered_contains   substrings that must each match a covered entry
  pages              {page: {state, omissions (kinds), rotated_lines, rotated_lines_min,
                     wide_gaps, invisible_chars, start, end}}
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import generate  # noqa: E402  (for the p16 line text only)

MAX_CHARS = 1_000_000
ENCRYPTED = "The PDF is encrypted. This reader does not open encrypted PDFs, even when no password is needed."
TRUNCATED = "The file ends before its end-of-file marker; it is truncated or damaged."
NOT_ALL = "Not all of this PDF is in the text. Left out: "


def joined(pages):
    """The documented text form: "[Page N]" then one line per text line."""
    return "\n".join(f"[Page {n}]" + "".join("\n" + line for line in lines)
                     for n, lines in enumerate(pages, start=1))


def page_offsets(pages):
    """(start, end) of each page's lines in Unicode scalars, as documented."""
    offsets, chars = [], 0
    for n, lines in enumerate(pages, start=1):
        if n > 1:
            chars += 1
        chars += len(f"[Page {n}]")
        start = chars + (1 if lines else 0)
        for line in lines:
            chars += 1 + len(line)
        offsets.append((start if lines else chars, chars))
    return offsets


def text_case(lines_by_page, **extra):
    case = {"text": joined(lines_by_page)}
    case["chars"] = len(case["text"])
    page_info = extra.pop("pages", {})
    for n, (start, end) in enumerate(page_offsets(lines_by_page), start=1):
        page_info.setdefault(str(n), {}).update({"start": start, "end": end})
    case["pages"] = page_info
    case.update(extra)
    return case


def unsupported(note):
    return {"status": "unsupported", "note": note}


def failed(note):
    return {"status": "failed", "note": note}


def p16_case():
    pages = [[generate.p16_line(p, i).rstrip() for i in range(generate.P16_LINES)]
             for p in range(1, generate.P16_PAGES + 1)]
    full = joined(pages)
    offsets = page_offsets(pages)
    first_cut = next(n for n, (_, end) in enumerate(offsets, start=1) if end > MAX_CHARS)
    after = {str(n): {"state": "not_read" if start >= MAX_CHARS else "text_with_omissions",
                      "omissions": ["truncated"]}
             for n, (start, end) in enumerate(offsets, start=1) if end > MAX_CHARS}
    return {
        "status": "partial",
        # The exact text is 1,000,000 characters: kept as its hash and ends.
        "text_sha256": hashlib.sha256(full[:MAX_CHARS].encode()).hexdigest(),
        "text_head": full[:200],
        "text_tail": full[MAX_CHARS - 200:MAX_CHARS],
        "chars": MAX_CHARS,
        "left_out": [f"the last {len(full) - MAX_CHARS} characters of the text (from page {first_cut} on)"],
        "note": NOT_ALL + "text past the length limit.",
        "covered_contains": ["all 120 pages"],
        "pages": after,
    }


def expectations():
    cases = {}

    def gen(name, case, kind="generated"):
        case.setdefault("status", "complete")
        cases[name] = {"path": f"generated/{name}", "kind": kind, **case}

    def exp(name, case):
        cases[name] = {"path": f"exported/{name}", "kind": "exported", **case}

    gen("p01-single-en.pdf", text_case(
        [["Traeva PDF reader test", "The quick brown fox jumps over the lazy dog.", "Version 1, single page."]],
        left_out=[], note=None,
        covered_contains=["the text layer of the page in the order the PDF draws it"],
        pages={"1": {"state": "text", "omissions": []}}))
    gen("p02-multi-en.pdf", text_case(
        [[f"Page {w} heading", f"Body text on page {w}."] for w in ("one", "two", "three")],
        left_out=[], covered_contains=["all 3 pages"],
        pages={str(n): {"state": "text", "omissions": []} for n in (1, 2, 3)}))
    gen("p03-cjk-reportlab.pdf", text_case(
        [["版本记忆测试", "第二行：中文与English混排"], ["嵌入字体的中文段落", "Mixed 中文 and English"]],
        left_out=[]))
    gen("p04-rotated.pdf", text_case(
        [["Upright heading", "Rotated ninety degrees", "Tilted forty-five", "Upright footer"],
         ["Landscape page text"]],
        left_out=[], covered_contains=["rotated text on page 1"],
        pages={"1": {"rotated_lines": 2}, "2": {"rotated_lines": 0}}))
    left = [f"Left column line {w}" for w in ("one", "two", "three", "four")]
    right = [f"Right column line {w}" for w in ("one", "two", "three", "four")]
    gen("p05-two-column.pdf", text_case(
        [["Two column report"] + left + right,
         ["Two column report"] + [f"{l}\t{r}" for l, r in zip(left, right)]],
        left_out=[],
        # Page 2 draws each row across both columns: the documented drawing
        # order interleaves them, with a TAB at the gutter.
        pages={"1": {"wide_gaps": 0}, "2": {"wide_gaps": 4}}))
    gen("p06-font-mapping.pdf", text_case(
        [["Zebra", "Tōkyō 東京 fine", "中文", "file"]],
        status="partial", left_out=["3 characters with no Unicode mapping on page 1"],
        note=NOT_ALL + "characters without Unicode.",
        pages={"1": {"state": "text_with_omissions", "omissions": ["unmapped"]}}))
    gen("p07-blank.pdf", {**unsupported("The PDF has no text to read: its pages draw no text."),
                          "pages": {"1": {"state": "no_text", "omissions": []}}})
    gen("p08-scanned-images.pdf", {
        **unsupported("No page has a text layer that this reader can use. 2 of 2 pages are mostly images "
                      "(likely scans); reading them needs OCR, which this reader does not do."),
        "pages": {str(n): {"state": "no_text", "omissions": ["image_page"]} for n in (1, 2)}})
    gen("p09-mixed-pages.pdf", text_case(
        [["Mixed page heading", "Caption below the picture."], [], ["Final text page."]],
        status="partial",
        left_out=["page 2: no text layer; mostly an image (likely a scan) that would need OCR, "
                  "which this reader does not do",
                  "images on page 1 (1 in all; images are not read, no OCR)"],
        note=NOT_ALL + "scanned or image-only pages, images.",
        covered_contains=["pages 1, 3 (2 of 3 pages)"],
        pages={"1": {"state": "text_with_omissions", "omissions": ["images"]},
               "2": {"state": "no_text", "omissions": ["image_page"]},
               "3": {"state": "text", "omissions": []}}))
    gen("p10-ruled-table.pdf", text_case(
        [["Table of versions", "Name\tVersion\tSize", "alpha\t1\t10 KB", "beta\t2\t12 KB"]],
        status="partial",
        left_out=["ruled lines forming a table or chart grid on page 1: its text is kept in drawing order, "
                  "its rows and columns are not"],
        note=NOT_ALL + "table structure.",
        pages={"1": {"wide_gaps": 6, "omissions": ["grid"]}}))
    gen("p11-formula.pdf", text_case(
        [["Energy: E = mc2", "∑ ≤ ∞"]],
        status="partial",
        left_out=["mathematical notation on page 1: its symbols are kept as characters, its layout "
                  "(fractions, sub- and superscripts, radicals) is not"],
        note=NOT_ALL + "formula layout.",
        pages={"1": {"omissions": ["math"]}}))
    gen("p12-annotations-form.pdf", text_case(
        [["Form page"]],
        status="partial",
        left_out=["annotations on page 1 (1: comments, highlights, stamps or attachments; not read)",
                  "form fields on page 1 (1; their values are not read)",
                  "JavaScript (not run and not read)",
                  "embedded files (not opened)"],
        note=NOT_ALL + "annotations, form fields, JavaScript, embedded files.",
        pages={"1": {"omissions": ["annotations", "form_fields"]}}))
    gen("p13-invisible-ocr-layer.pdf", text_case(
        [["Scanned invoice number 42"]],
        status="partial",
        left_out=["images on page 1 (1 in all; images are not read, no OCR)"],
        note=NOT_ALL + "images.",
        covered_contains=["invisible text on page 1"],
        pages={"1": {"invisible_chars": 22, "omissions": ["images"]}}))
    gen("p14-outside-and-duplicates.pdf", text_case(
        [["Visible line", "Stroked once", "Bold twice"]],
        status="partial",
        left_out=["19 characters drawn outside the visible page area on page 1"],
        note=NOT_ALL + "text outside the page.",
        pages={"1": {"omissions": ["outside"]}}))
    gen("p15-positioned-words.pdf", text_case(
        [["Word spacing by position", "Kerning", "Wide gaps", "Tab\tstop"]],
        left_out=[], pages={"1": {"wide_gaps": 1}}))
    gen("p16-text-cap.pdf", p16_case())
    gen("p17-glyph-budget.pdf", unsupported("The PDF passes this reader's limit on characters drawn."))
    gen("p18-draw-budget.pdf", unsupported("The PDF passes this reader's limit on drawing operations."))

    beyond16 = "A stream expands beyond this reader's limit of 16 MiB."
    gen("a01-flate-bomb.pdf", unsupported(beyond16), "adversarial")
    gen("a02-nested-flate.pdf", unsupported(beyond16), "adversarial")
    gen("a03-image-label-bomb.pdf", unsupported("A stream expands beyond this reader's limit of 64 MiB."),
        "adversarial")
    gen("a04-decoded-total.pdf", unsupported(
        "The PDF's non-image streams expand to more than this reader's limit (128 MiB)."), "adversarial")
    gen("a05-too-many-pages.pdf", unsupported("The PDF has more pages than this reader's limit (500)."),
        "adversarial")
    gen("a06-declared-objects.pdf", unsupported(
        "The PDF declares more objects than this reader's limit (200000)."), "adversarial")
    gen("a07a-deep-stream-dict.pdf", unsupported(
        "A stream's dictionary could not be checked before reading."), "adversarial")
    # Assumes hayro refuses the over-deep page dictionary (its own limit is
    # 64), so no page is left: some Failed note, never text.
    gen("a07b-deep-page-dict.pdf", {"status": "failed"}, "adversarial")
    # 50 nested copies of the form draw "loop" at one place; repeated runs at
    # the same place are dropped.
    gen("a08-form-recursion.pdf", {**text_case([["loop"]]), "left_out": []}, "adversarial")
    gen("a09-form-fanout.pdf", unsupported("The PDF passes this reader's limit on characters drawn."),
        "adversarial")
    gen("a10-lzw.pdf", unsupported(
        "The PDF uses legacy LZW compression, which this experiment does not read."), "adversarial")
    gen("a11-codec-on-content.pdf", unsupported(
        "An image codec is used on a stream that is not an image."), "adversarial")
    gen("a12-indirect-filter.pdf", unsupported(
        "A stream's filter list is stored elsewhere in the file; this reader does not guess its size."),
        "adversarial")
    gen("a13-unknown-filter.pdf", unsupported("A stream uses a filter this reader does not know."),
        "adversarial")
    gen("a14-run-length-bomb.pdf", unsupported(beyond16), "adversarial")
    gen("a15-stream-count.pdf", unsupported("The PDF has more streams than this reader's limit (50000)."),
        "adversarial")
    gen("a16-big-input.pdf", unsupported("The PDF is larger than this reader's input limit (32 MiB)."),
        "adversarial")
    gen("a17-fake-endstream.pdf", unsupported(beyond16), "adversarial")
    gen("a18-encrypted-owner-only.pdf", unsupported(ENCRYPTED), "adversarial")
    gen("a19-encrypted-user-password.pdf", unsupported(ENCRYPTED), "adversarial")
    gen("a20-escaped-encrypt.pdf", unsupported(ENCRYPTED), "adversarial")

    gen("c01-truncated-half.pdf", failed(TRUNCATED), "damaged")
    gen("c02-truncated-tail.pdf", failed(TRUNCATED), "damaged")
    gen("c03-wrong-startxref.pdf", failed(
        "The PDF's cross-reference data is damaged (the offset at its end is wrong)."), "damaged")
    gen("c04-damaged-flate.pdf", failed("A compressed stream in the PDF is damaged."), "damaged")
    gen("c05-not-a-pdf.pdf", failed("The file has no PDF header."), "damaged")
    gen("c06-garbage.pdf", failed(TRUNCATED), "damaged")
    gen("c07-page-count.pdf", failed(
        "The PDF's page tree is damaged: it declares 3 pages but 2 were found."), "damaged")
    gen("c08-empty.pdf", failed("The file has no PDF header."), "damaged")

    shared = ["English paragraph for the reader.", "这是中文段落，用于版本记忆测试。",
              "Mixed 中文 and English text.", "Name", "Version", "Size", "alpha", "v1.0", "10 KB",
              "beta", "v2.0", "12 KB", "Sum ∑ x ≤ ∞ and √2"]
    columns = ["Column one first line.", "Column one second line.", "Column two first line.",
               "Column two second line.", "Before", "Rotated label", "after"]
    real_kinds = ["images on page 1", "table or chart grid on page 1", "mathematical notation on page 1"]
    absent = ["no text layer", "no Unicode mapping", "could not decode", "outside the visible page"]
    exp("e01-libreoffice.pdf", {
        "status": "partial",
        "snippets_in_order": ["[Page 1]", "Traeva export test"] + shared + ["[Page 2]", "Second page heading"]
        + columns,
        "left_out_kinds": real_kinds, "left_out_absent": absent,
        "pages": {"1": {"state": "text_with_omissions"}, "2": {"rotated_lines_min": 1}},
    })
    # Corrected after run 01 (the FAIL stays in evidence/runs/run-01-*): the
    # frozen order assumed DOM order. Chromium paints the transformed
    # inline-block in its own stacking context after the line's in-flow text
    # (CSS 2.1 Appendix E), and `pdftotext -raw` (content-stream order) shows
    # "Before after" before "Rotated label" too. The reader's documented rule
    # is drawing order, so the expectation now follows it.
    chromium_columns = columns[:4] + ["Before", "after", "Rotated label"]
    exp("e02-chromium.pdf", {
        "status": "partial",
        "snippets_in_order": ["[Page 1]", "Traeva browser export"] + shared + chromium_columns,
        "left_out_kinds": real_kinds, "left_out_absent": absent,
        "pages": {"1": {"state": "text_with_omissions", "rotated_lines_min": 1}},
    })
    exp("e03a-libreoffice-open-password.pdf", unsupported(ENCRYPTED))
    exp("e03b-libreoffice-permissions-only.pdf", unsupported(ENCRYPTED))
    return cases


def main():
    cases = expectations()
    (HERE / "expected.json").write_text(json.dumps({
        "reader": "pdf-experiment-1",
        "frozen": "Written before the reader's first run on any fixture; changes must state their basis.",
        "fixtures": cases,
    }, indent=1, ensure_ascii=False) + "\n")
    print(f"{len(cases)} expectations")


if __name__ == "__main__":
    main()
