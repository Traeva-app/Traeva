#!/usr/bin/env python3
"""Create synthetic research inputs in a fresh directory; never launch Word."""
import argparse
import hashlib
import json
from pathlib import Path
from xml.sax.saxutils import escape
from zipfile import ZIP_STORED, ZipFile, ZipInfo

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
TYPES = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>'
RELS = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'


def text(s):
    return f'<w:r><w:t xml:space="preserve">{escape(s)}</w:t></w:r>'


def marker(kind):
    return f'<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>'


def field(code, result):
    return marker("begin") + f'<w:r><w:instrText xml:space="preserve">{escape(code)}</w:instrText></w:r>' + marker("separate") + result + marker("end")


def paragraph(s):
    return f"<w:p>{s}</w:p>"


def cases():
    ordinary = text("References: ") + field("CITATION Smith2020", text("(Smith, 2020)")) + text("; ") + field('ADDIN ZOTERO_ITEM CSL_CITATION {"SECRET_ZOTERO_METADATA":"local-synthetic"}', text("(Jones, 2021)")) + text("; ") + field("ADDIN EN.CITE {SECRET_ENDNOTE_METADATA}", text("(Lee, 2022)"))
    nested = marker("begin") + "<w:r><w:instrText>REF </w:instrText></w:r>" + field("QUOTE SECRET_NESTED_CODE", text("SECRET_NESTED_CODE_RESULT")) + marker("separate") + text("Outer [") + field("REF local", text("Inner cached")) + text("]") + marker("end")
    dirty = field("CITATION ChangedMetadata", text("(Old, 1999)")).replace('fldCharType="begin"', 'fldCharType="begin" w:dirty="true"', 1)
    missing = text("Missing: ") + marker("begin") + "<w:r><w:instrText>CITATION NoResult</w:instrText></w:r>" + marker("end") + text("; stale: ") + dirty
    revisions = text("Before ") + '<w:ins w:id="1" w:author="Synthetic" w:date="2026-10-10T00:00:00Z">' + field("ADDIN ZOTERO_ITEM SECRET_INSERTED", text("(Inserted, 2022)")) + "</w:ins>" + text(" after")
    malformed = text("prefix ") + marker("begin") + "<w:r><w:instrText>ADDIN ZOTERO_ITEM SECRET_UNCLOSED</w:instrText></w:r>" + marker("separate") + text("(unclosed cache)")
    return {
        "ordinary": (paragraph(ordinary), "References: (Smith, 2020); (Jones, 2021); (Lee, 2022)"),
        "nested": (paragraph(nested), "Outer [Inner cached]"),
        "missing-stale": (paragraph(missing), "Missing: ; stale: (Old, 1999)"),
        "revisions": (paragraph(revisions), ""),
        "malformed": (paragraph(malformed), ""),
        "plain-history": (paragraph(text("Historical synthetic version.")) + paragraph(text("Original and Snapshot must remain unchanged.")), "Historical synthetic version.\nOriginal and Snapshot must remain unchanged."),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fresh_directory", type=Path)
    base = parser.parse_args().fresh_directory
    base.mkdir(mode=0o700, parents=True, exist_ok=False)
    for name in ("root", "expected", "evidence"):
        (base / name).mkdir(mode=0o700)
    manifest = []
    for name, (body, expected) in cases().items():
        path = base / "root" / f"{name}.docx"
        document = f'<w:document xmlns:w="{W}"><w:body>{body}<w:sectPr/></w:body></w:document>'
        with ZipFile(path, "x") as archive:
            for part, value in (("[Content_Types].xml", TYPES), ("_rels/.rels", RELS), ("word/document.xml", document)):
                info = ZipInfo(part, date_time=(2026, 10, 10, 21, 20, 28))
                info.compress_type = ZIP_STORED
                info.external_attr = 0o600 << 16
                archive.writestr(info, value.encode("utf-8"))
        (base / "expected" / f"{name}.txt").write_text(expected, encoding="utf-8")
        manifest.append({"name": name, "input_sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "expected_sha256": hashlib.sha256(expected.encode()).hexdigest()})
    (base / "evidence" / "seed-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"created_synthetic_inputs": len(manifest), "directory": str(base)}))


if __name__ == "__main__":
    main()
