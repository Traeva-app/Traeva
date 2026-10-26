#!/usr/bin/env python3
"""Exports the real-application PDF fixtures (LibreOffice Writer and Chromium,
both headless) from synthetic sources, for the #28 PDF Reader experiment.

Unlike fixtures/generate.py, these files come from real PDF producers and
are not byte-reproducible (dates, IDs), so the PDFs themselves are committed
under fixtures/exported/ with their hashes and tool versions in
fixtures/exported/manifest.json. Rerun only to make new exports.

    python3 fixtures/export.py SCRATCH_DIR

SCRATCH_DIR holds a throwaway LibreOffice profile and Chromium data
directory, so no user profile is read or changed.
"""

import base64
import hashlib
import io
import json
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
OUT = HERE / "exported"
SRC = OUT / "src"
CHROME = "/opt/pw-browsers/chromium-1194/chrome-linux/chrome"


def png():
    image = Image.new("L", (160, 100))
    image.putdata([(x * 3 + y * 2) % 256 for y in range(100) for x in range(160)])
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=False)
    return buffer.getvalue()


TABLE = [("Name", "Version", "Size"), ("alpha", "v1.0", "10 KB"), ("beta", "v2.0", "12 KB")]


def fodt(image):
    cells = "".join(
        "<table:table-row>" + "".join(
            f'<table:table-cell table:style-name="Cell" office:value-type="string"><text:p>{cell}</text:p></table:table-cell>'
            for cell in row) + "</table:table-row>"
        for row in TABLE)
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
 office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:font-face-decls>
  <style:font-face style:name="Liberation Serif" svg:font-family="'Liberation Serif'"/>
  <style:font-face style:name="WenQuanYi Zen Hei" svg:font-family="'WenQuanYi Zen Hei'"/>
  <style:font-face style:name="DejaVu Sans" svg:font-family="'DejaVu Sans'"/>
 </office:font-face-decls>
 <office:styles>
  <style:default-style style:family="paragraph">
   <style:text-properties style:font-name="Liberation Serif" fo:font-size="12pt"
    style:font-name-asian="WenQuanYi Zen Hei" style:font-size-asian="12pt"/>
  </style:default-style>
 </office:styles>
 <office:automatic-styles>
  <style:style style:name="Title" style:family="paragraph"><style:text-properties fo:font-size="18pt" fo:font-weight="bold"/></style:style>
  <style:style style:name="Math" style:family="paragraph"><style:text-properties style:font-name="DejaVu Sans"/></style:style>
  <style:style style:name="PageBreak" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style>
  <style:style style:name="ColumnBreak" style:family="paragraph"><style:paragraph-properties fo:break-before="column"/></style:style>
  <style:style style:name="Rotated" style:family="text"><style:text-properties style:text-rotation-angle="90"/></style:style>
  <style:style style:name="Columns" style:family="section"><style:section-properties><style:columns fo:column-count="2" fo:column-gap="0.5in"/></style:section-properties></style:style>
  <style:style style:name="Table" style:family="table"><style:table-properties style:width="6in" table:align="left"/></style:style>
  <style:style style:name="Cell" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #000000" fo:padding="0.05in"/></style:style>
 </office:automatic-styles>
 <office:body><office:text>
  <text:p text:style-name="Title">Traeva export test</text:p>
  <text:p>English paragraph for the reader.</text:p>
  <text:p>这是中文段落，用于版本记忆测试。</text:p>
  <text:p>Mixed 中文 and English text.</text:p>
  <table:table table:name="Versions" table:style-name="Table">
   <table:table-column table:number-columns-repeated="3"/>
   {cells}
  </table:table>
  <text:p><draw:frame draw:name="Picture" text:anchor-type="as-char" svg:width="2in" svg:height="1.25in"><draw:image><office:binary-data>{base64.b64encode(image).decode()}</office:binary-data></draw:image></draw:frame></text:p>
  <text:p text:style-name="Math">Sum ∑ x ≤ ∞ and √2</text:p>
  <text:p text:style-name="PageBreak">Second page heading</text:p>
  <text:section text:style-name="Columns" text:name="Columns">
   <text:p>Column one first line.</text:p>
   <text:p>Column one second line.</text:p>
   <text:p text:style-name="ColumnBreak">Column two first line.</text:p>
   <text:p>Column two second line.</text:p>
  </text:section>
  <text:p>Before <text:span text:style-name="Rotated">Rotated label</text:span> after</text:p>
 </office:text></office:body>
</office:document>
"""


def html(image):
    rows = "".join("<tr>" + "".join(f"<td>{cell}</td>" for cell in row) + "</tr>" for row in TABLE)
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><title>Traeva browser export</title><style>
body {{ font-family: 'Liberation Sans'; font-size: 12pt; margin: 0.75in; }}
h1 {{ font-size: 18pt; }}
p {{ margin: 6px 0; }}
.zh {{ font-family: 'WenQuanYi Zen Hei'; }}
.math {{ font-family: 'DejaVu Sans'; }}
table {{ border-collapse: collapse; }}
td {{ border: 1px solid black; padding: 4px 12px; }}
.columns {{ column-count: 2; column-gap: 0.5in; }}
.columns .second {{ break-before: column; }}
.rotated {{ display: inline-block; transform: rotate(90deg); margin: 40px; }}
</style></head><body>
<h1>Traeva browser export</h1>
<p>English paragraph for the reader.</p>
<p class="zh">这是中文段落，用于版本记忆测试。</p>
<p>Mixed <span class="zh">中文</span> and English text.</p>
<table>{rows}</table>
<p><img src="data:image/png;base64,{base64.b64encode(image).decode()}" width="192" height="120"></p>
<p class="math">Sum ∑ x ≤ ∞ and √2</p>
<div class="columns"><p>Column one first line.</p><p>Column one second line.</p><p class="second">Column two first line.</p><p>Column two second line.</p></div>
<p>Before <span class="rotated">Rotated label</span> after</p>
</body></html>
"""


SMALL_FODT = """<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:body><office:text><text:p>Encrypted synthetic document.</text:p></office:text></office:body>
</office:document>
"""


def soffice(profile, source, target, options):
    work = profile / "convert"
    work.mkdir(parents=True, exist_ok=True)
    filter_name = "pdf:writer_pdf_Export" + (":" + json.dumps(options) if options else "")
    subprocess.run([
        "soffice", f"-env:UserInstallation=file://{profile}/lo-profile", "--headless",
        "--convert-to", filter_name, "--outdir", str(work), str(source),
    ], check=True, capture_output=True, timeout=180)
    shutil.move(work / (source.stem + ".pdf"), target)


def chrome(profile, source, target):
    subprocess.run([
        CHROME, "--headless", "--no-sandbox", "--disable-gpu", "--no-first-run",
        f"--user-data-dir={profile}/chrome-profile", "--no-pdf-header-footer",
        f"--print-to-pdf={target}", source.as_uri(),
    ], check=True, capture_output=True, timeout=180)


def version(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=60)
    return (result.stdout or result.stderr).strip().splitlines()[0]


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    scratch = Path(sys.argv[1]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    SRC.mkdir(parents=True, exist_ok=True)
    image = png()
    (SRC / "e01-libreoffice.fodt").write_text(fodt(image), encoding="utf-8")
    (SRC / "e02-chromium.html").write_text(html(image), encoding="utf-8")
    (SRC / "e03-encrypted.fodt").write_text(SMALL_FODT, encoding="utf-8")

    soffice(scratch, SRC / "e01-libreoffice.fodt", OUT / "e01-libreoffice.pdf", None)
    chrome(scratch, SRC / "e02-chromium.html", OUT / "e02-chromium.pdf")
    soffice(scratch, SRC / "e03-encrypted.fodt", OUT / "e03a-libreoffice-open-password.pdf", {
        "EncryptFile": {"type": "boolean", "value": "true"},
        "DocumentOpenPassword": {"type": "string", "value": "synthetic-pass"},
    })
    soffice(scratch, SRC / "e03-encrypted.fodt", OUT / "e03b-libreoffice-permissions-only.pdf", {
        "RestrictPermissions": {"type": "boolean", "value": "true"},
        "PermissionPassword": {"type": "string", "value": "synthetic-owner"},
    })

    files = {}
    for path in sorted(OUT.glob("*.pdf")) + sorted(SRC.iterdir()):
        data = path.read_bytes()
        files[str(path.relative_to(OUT))] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    (OUT / "manifest.json").write_text(json.dumps({
        "exporter": "fixtures/export.py",
        "tools": {
            "libreoffice": version(["soffice", "--version"]),
            "chromium": version([CHROME, "--version"]),
        },
        "note": "Synthetic sources only. Real-application exports, not byte-reproducible.",
        "files": files,
    }, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(files, indent=2))


if __name__ == "__main__":
    main()
