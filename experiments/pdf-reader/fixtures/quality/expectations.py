#!/usr/bin/env python3
"""Frozen expectations for the quality diagnostics (pdf-experiment-4).

Written before the diagnostics existed, from each fixture's own design (its
generator or export source) and the rules below, never from a reader run.
The diagnostics never change the text: every fixture keeps the text,
status and left_out frozen in its own set (fixtures/expected.json,
fixtures/defects/expected.json, fixtures/defects2/expected.json, and the
files here); its text must equal that of the same reader without the
quality report (pdf-experiment-3, then pdf-experiment-5).

The second-round reproductions (f-) were added after the merge with
pdf-experiment-5 and before the first run of the merged reader; their stop
reasons come from each fixture's frozen note, the same mapping as above.
The third-round reproductions (g-) were added the same way after the merge
with pdf-experiment-7; g25-g27 are marked unverified in their own set, so no
stop reason is frozen for them and only the rules that need no frozen data
apply.

Rules the validator checks on every fixture of every set:
- `confirmed_omission` diagnostics are exactly the `left_out` entries: the
  same kinds (LEFT_OUT_CODES) on the same pages, nothing more.
- `suspected_risk` diagnostics are exactly RISKS[name] (none elsewhere).
- An unsupported or failed reading has one `stopped` diagnostic with
  STOP_CODES[name].
- A complete reading without risks has no diagnostics at all.
- `reading_order` is always "drawing_order_unverified": no diagnostic, and
  no lack of one, claims the reading order was checked.

    python3 fixtures/quality/expectations.py   # writes fixtures/quality/expected.json
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from expectations import NOT_ALL, joined, page_offsets  # noqa: E402

import importlib.util  # noqa: E402

_spec = importlib.util.spec_from_file_location("quality_generate", HERE / "generate.py")
_quality = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_quality)
BODY, LABELS, LEFT, RIGHT = _quality.BODY, _quality.LABELS, _quality.LEFT, _quality.RIGHT

GRID = ("ruled lines forming a table or chart grid on page 1: its text is kept in drawing order, "
        "its rows and columns are not")

# Which kind of finding each left_out entry is (a regular expression on the entry).
LEFT_OUT_CODES = [
    [r"^pages? [0-9, -]+: no text layer", "image_page"],
    [r"^images on ", "images"],
    [r"characters with no Unicode mapping on ", "unmapped_characters"],
    [r"^text in fonts this reader could not decode on ", "undecoded_fonts"],
    [r"^ruled lines forming a table or chart grid on ", "table_grid"],
    [r"^vector drawings ", "drawings"],
    [r"^mathematical notation on ", "math_layout"],
    [r"^annotations on ", "annotations"],
    [r"^form fields on ", "form_fields"],
    [r"^a form with \d+ fields", "form_fields"],
    [r"^an XFA form", "xfa_form"],
    [r"^JavaScript", "javascript"],
    [r"^embedded files", "embedded_files"],
    [r"^content in optional layers", "hidden_layers"],
    [r"^content stored in external files", "external_content"],
    [r"characters drawn outside the visible page area on ", "outside_page"],
    [r"^the last \d+ characters of the text", "truncated_text"],
]

# Why reading stopped, by fixture (from each fixture's design and its frozen note).
STOP_CODES = {
    # round one
    "p07-blank.pdf": "no_text",
    "p08-scanned-images.pdf": "no_text_layer",
    "p17-glyph-budget.pdf": "resource_limit",
    "p18-draw-budget.pdf": "resource_limit",
    "a01-flate-bomb.pdf": "resource_limit",
    "a02-nested-flate.pdf": "resource_limit",
    "a03-image-label-bomb.pdf": "resource_limit",
    "a04-decoded-total.pdf": "resource_limit",
    "a05-too-many-pages.pdf": "resource_limit",
    "a06-declared-objects.pdf": "resource_limit",
    "a07a-deep-stream-dict.pdf": "unsupported_structure",
    "a07b-deep-page-dict.pdf": "damaged",
    "a09-form-fanout.pdf": "resource_limit",
    "a10-lzw.pdf": "unsupported_feature",
    "a11-codec-on-content.pdf": "unsupported_feature",
    "a12-indirect-filter.pdf": "unsupported_feature",
    "a13-unknown-filter.pdf": "unsupported_feature",
    "a14-run-length-bomb.pdf": "resource_limit",
    "a15-stream-count.pdf": "resource_limit",
    "a16-big-input.pdf": "resource_limit",
    "a17-fake-endstream.pdf": "resource_limit",
    "a18-encrypted-owner-only.pdf": "encrypted",
    "a19-encrypted-user-password.pdf": "encrypted",
    "a20-escaped-encrypt.pdf": "encrypted",
    "c01-truncated-half.pdf": "truncated",
    "c02-truncated-tail.pdf": "truncated",
    "c03-wrong-startxref.pdf": "damaged",
    "c04-damaged-flate.pdf": "damaged",
    "c05-not-a-pdf.pdf": "not_a_pdf",
    "c06-garbage.pdf": "truncated",
    "c07-page-count.pdf": "damaged",
    "c08-empty.pdf": "not_a_pdf",
    "e03a-libreoffice-open-password.pdf": "encrypted",
    "e03b-libreoffice-permissions-only.pdf": "encrypted",
    # R4 reproductions
    "d01-nested-form-string-bomb.pdf": "resource_limit",
    "d02-comment-before-stream-bomb.pdf": "resource_limit",
    "d13-contents-missing-member.pdf": "damaged",
    "d14-contents-string-member.pdf": "damaged",
    "d15-contents-dangling.pdf": "damaged",
    "d17-flate-wrong-adler.pdf": "damaged",
    "d18-flate-missing-adler.pdf": "damaged",
    "d19-flate-no-zlib-header.pdf": "damaged",
    "d20-form-wrong-adler.pdf": "damaged",
    "d23-xref-stream-escaped-encrypt.pdf": "encrypted",
    # R4 second-round reproductions
    "f10-objstm-lzw-listing-in-text.pdf": "unsupported_feature",
    "f11-listing-and-real-bomb.pdf": "resource_limit",
    "f12-damaged-part-and-listing.pdf": "damaged",
    "f13-real-unknown-filter.pdf": "unsupported_feature",
    "f14-bomb-listing-in-image-data.pdf": "resource_limit",
    "f15-contents-direct-null.pdf": "no_text",
    "f16-contents-indirect-null.pdf": "no_text",
    "f18-text-and-missing-reference-page.pdf": "damaged",
    "f19-contents-number.pdf": "damaged",
    "f20-contents-dictionary.pdf": "damaged",
    "f21-contents-array-null-member.pdf": "damaged",
    "f22-contents-null-in-object-stream.pdf": "no_text",
    # R4 third-round reproductions (g25-g27 unverified: none frozen)
    "g03-contents-raw-over-limit.pdf": "resource_limit",
    "g04-contents-raw-17mib-early-endstream.pdf": "resource_limit",
    "g07-form-raw-over-limit.pdf": "resource_limit",
    "g08-form-raw-17mib-early-endstream.pdf": "resource_limit",
    "g09-indirect-length-over-limit.pdf": "resource_limit",
    "g11-length-in-objstm-over-limit.pdf": "resource_limit",
    "g17-wrong-length-17mib-early-endstream.pdf": "no_text",
    "g18-flate-bomb-wrong-length.pdf": "resource_limit",
    "g19-objstm-bomb-in-image-data.pdf": "resource_limit",
    "g20-objstm-raw-over-limit.pdf": "resource_limit",
    "g21-rebuilt-xref-raw-over-limit.pdf": "resource_limit",
    "g22-rebuilt-xref-flate-bomb.pdf": "resource_limit",
    # this set
    "q01-no-unicode-only.pdf": "no_unicode",
}

# Suspected risks, by fixture and page. Every other fixture must have none.
INTERLEAVED = "interleaved_columns"
RISKS = {
    # Columns drawn line by line across (each row: left line, then right line).
    "p05-two-column.pdf": {INTERLEAVED: [2]},
    "l04-two-columns-by-row.pdf": {INTERLEAVED: [1]},
    "l07-mixed-out-of-order.pdf": {INTERLEAVED: [1]},
    "l09-three-columns.pdf": {INTERLEAVED: [1]},
    "l10-unequal-columns-header-footer.pdf": {INTERLEAVED: [1]},
    "l11-staggered-columns.pdf": {INTERLEAVED: [1]},
    "l17-rotated-in-gutter.pdf": {INTERLEAVED: [1]},
    "l18-cjk-two-columns.pdf": {INTERLEAVED: [1]},
    "l20-table-then-columns.pdf": {INTERLEAVED: [1]},
    "l21-narrow-gutter-justified.pdf": {INTERLEAVED: [1]},
    "l24-justified-wide-gaps-per-line.pdf": {INTERLEAVED: [1]},
    # Known false positives: read correctly, warned anyway (see NOTES).
    "q03-unruled-long-cells.pdf": {INTERLEAVED: [1]},
    "q04-label-block.pdf": {INTERLEAVED: [1]},
    # An invisible text layer (an OCR layer over an image).
    "p13-invisible-ocr-layer.pdf": {"invisible_text": [1]},
}

NOTES = {
    "l05-two-columns-reversed.pdf": "known miss: both columns drawn bottom-up, one after the other; "
                                    "no row is split, so nothing shows the order is wrong",
    "l06-two-columns-word-order.pdf": "known miss: every word drawn as its own object, last word first; "
                                      "no long piece sits on either side of the gutter",
    "l19-short-columns.pdf": "known miss: two rows only, under the three-row threshold",
    "l25-justified-wide-gaps-per-word.pdf": "known miss: every word is its own text operation, so the pieces "
                                            "next to the gutter are single words",
    "l02-single-column-shuffled.pdf": "out of scope: one column drawn out of order is not a column risk",
    "l14-aligned-table.pdf": "known miss: an unruled table drawn column by column; its rows are not "
                             "recovered and nothing warns about it",
    "l16-label-rows.pdf": "known miss on page 1: label pairs drawn column by column read as two lists",
    "q03-unruled-long-cells.pdf": "known false positive: an unruled table of long cells, read row by row "
                                  "(correctly), looks like two columns drawn across",
    "q04-label-block.pdf": "known false positive: a form-like block of label pairs, read row by row "
                           "(correctly), looks like two columns drawn across",
    "q02-ruled-table-long-cells.pdf": "the same rows inside a ruled grid: the gap holds a vertical line, so it "
                                      "is a cell border, not a gutter, and there is no column warning",
}


def text_case(lines, status="complete", left_out=(), kinds=(), note=None):
    case = {"status": status, "text": joined([lines]) if lines else ""}
    case["chars"] = len(case["text"])
    if lines:
        start, end = page_offsets([lines])[0]
        case["pages"] = {"1": {"start": start, "end": end}}
    case["left_out"] = list(left_out)
    case["note"] = note if note is not None else ((NOT_ALL + ", ".join(kinds) + ".") if left_out else None)
    return case


def fixtures():
    rows = [f"{left}\t{right}" for left, right in zip(LEFT, RIGHT)]
    labels = [f"{left}\t{right}" for left, right in LABELS]
    cases = {
        "q01-no-unicode-only.pdf": text_case(
            [], "unsupported", note="The PDF's characters have no Unicode mapping, so it has no reliable text."),
        "q02-ruled-table-long-cells.pdf": text_case(rows, "partial", [GRID], ["table structure"]),
        "q03-unruled-long-cells.pdf": text_case(rows),
        "q04-label-block.pdf": text_case(labels + [BODY]),
    }
    return {name: {"path": f"quality/generated/{name}", "kind": "quality", **case} for name, case in cases.items()}


def main():
    (HERE / "expected.json").write_text(json.dumps({
        "frozen": "Written before the diagnostics existed; changes must state their basis.",
        "reading_order": "drawing_order_unverified",
        "left_out_codes": LEFT_OUT_CODES,
        "stop_codes": STOP_CODES,
        "risks": RISKS,
        "notes": NOTES,
        "fixtures": fixtures(),
    }, indent=1, ensure_ascii=False) + "\n")
    print("quality expectations written")


if __name__ == "__main__":
    main()
