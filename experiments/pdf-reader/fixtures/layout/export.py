#!/usr/bin/env python3
"""Real-application layout fixtures for the second round (LibreOffice Writer and
Chromium, both headless), from synthetic sources.

The text is numbered tokens (intro01, flow001, ...), so the correct reading
order is "every token in increasing order", whatever lines and column breaks
the application chose. The PDFs are not byte-reproducible, so they are
committed under fixtures/layout/exported/ with hashes and tool versions.

    python3 fixtures/layout/export.py SCRATCH_DIR

SCRATCH_DIR holds throwaway LibreOffice and Chromium profiles.
"""

import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "exported"
SRC = OUT / "src"
CHROME = "/opt/pw-browsers/chromium-1194/chrome-linux/chrome"

TABLE = [("Name", "Version", "Size"), ("alpha", "v1.0", "10 KB"), ("beta", "v2.0", "12 KB"),
         ("gamma", "v3.0", "14 KB")]


def tokens(prefix, count, digits):
    return " ".join(f"{prefix}{i:0{digits}d}" for i in range(1, count + 1))


NS = """xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\""""


def fodt(body, columns, header=""):
    master = ""
    if header:
        master = f"""<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="Page">
 <style:header><text:p text:style-name="Header">{header}</text:p></style:header>
</style:master-page></office:master-styles>"""
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<office:document {NS} office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:font-face-decls>
  <style:font-face style:name="Liberation Serif" svg:font-family="'Liberation Serif'"/>
 </office:font-face-decls>
 <office:styles>
  <style:default-style style:family="paragraph">
   <style:text-properties style:font-name="Liberation Serif" fo:font-size="11pt"/>
  </style:default-style>
  <style:style style:name="Header" style:family="paragraph"><style:paragraph-properties>
   <style:tab-stops><style:tab-stop style:position="6.5in" style:type="right"/></style:tab-stops>
  </style:paragraph-properties></style:style>
 </office:styles>
 <office:automatic-styles>
  <style:page-layout style:name="Page"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"
   fo:margin-top="0.6in" fo:margin-bottom="0.8in" fo:margin-left="1in" fo:margin-right="1in"/>
   <style:header-style><style:header-footer-properties fo:min-height="0.3in" fo:margin-bottom="0.2in"/></style:header-style>
  </style:page-layout>
  <style:style style:name="Title" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/>
   <style:text-properties fo:font-size="16pt" fo:font-weight="bold"/></style:style>
  <style:style style:name="Body" style:family="paragraph"><style:paragraph-properties fo:text-align="justify"
   fo:margin-bottom="0.08in"/></style:style>
  <style:style style:name="Columns" style:family="section"><style:section-properties>
   <style:columns fo:column-count="{columns}" fo:column-gap="0.4in"/></style:section-properties></style:style>
  <style:style style:name="Table" style:family="table"><style:table-properties style:width="5in" table:align="left"/></style:style>
  <style:style style:name="Cell" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #000000"
   fo:padding="0.04in"/></style:style>
 </office:automatic-styles>
 {master}
 <office:body><office:text>{body}</office:text></office:body>
</office:document>
"""


def odt_table():
    rows = "".join("<table:table-row>" + "".join(
        f'<table:table-cell table:style-name="Cell" office:value-type="string"><text:p>{cell}</text:p></table:table-cell>'
        for cell in row) + "</table:table-row>" for row in TABLE)
    return (f'<table:table table:name="Versions" table:style-name="Table">'
            f'<table:table-column table:number-columns-repeated="3"/>{rows}</table:table>')


def x01_source():
    body = f"""
  <text:p text:style-name="Title">Layout export test</text:p>
  <text:p text:style-name="Body">{tokens("intro", 40, 2)}</text:p>
  <text:section text:style-name="Columns" text:name="Flow">
   <text:p text:style-name="Body">{tokens("flow", 160, 3)}</text:p>
  </text:section>
  {odt_table()}
  <text:p text:style-name="Body">{tokens("outro", 20, 2)}</text:p>"""
    return fodt(body, 2)


def x03_source():
    body = f"""
  <text:p text:style-name="Title">Three column export</text:p>
  <text:section text:style-name="Columns" text:name="Flow">
   <text:p text:style-name="Body">{tokens("tri", 180, 3)}</text:p>
  </text:section>
  <text:p text:style-name="Body">{tokens("end", 10, 2)}</text:p>"""
    return fodt(body, 3, header="Layout header<text:tab/>Page <text:page-number text:select-page=\"current\">1</text:page-number>")


def x02_source():
    rows = "".join("<tr>" + "".join(f"<td>{cell}</td>" for cell in row) + "</tr>" for row in TABLE)
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><title>Layout browser export</title><style>
@page {{ size: Letter; margin: 0.75in; }}
body {{ font-family: 'Liberation Serif'; font-size: 11pt; margin: 0; }}
h1 {{ font-size: 16pt; text-align: center; }}
p {{ text-align: justify; margin: 0 0 6px 0; }}
.flow {{ column-count: 2; column-gap: 0.4in; margin: 8px 0; }}
table {{ border-collapse: collapse; margin: 8px 0; }}
td {{ border: 1px solid black; padding: 3px 10px; }}
</style></head><body>
<h1>Layout browser export</h1>
<p>{tokens("intro", 40, 2)}</p>
<div class="flow"><p>{tokens("flow", 160, 3)}</p></div>
<table>{rows}</table>
<p>{tokens("outro", 20, 2)}</p>
</body></html>
"""


def soffice(profile, source, target):
    work = profile / "convert"
    work.mkdir(parents=True, exist_ok=True)
    subprocess.run(["soffice", f"-env:UserInstallation=file://{profile}/lo-profile", "--headless",
                    "--convert-to", "pdf:writer_pdf_Export", "--outdir", str(work), str(source)],
                   check=True, capture_output=True, timeout=180)
    shutil.move(work / (source.stem + ".pdf"), target)


def chrome(profile, source, target):
    subprocess.run([CHROME, "--headless", "--no-sandbox", "--disable-gpu", "--no-first-run",
                    f"--user-data-dir={profile}/chrome-profile", "--no-pdf-header-footer",
                    f"--print-to-pdf={target}", source.as_uri()], check=True, capture_output=True, timeout=180)


def version(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=60)
    return (result.stdout or result.stderr).strip().splitlines()[0]


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    scratch = Path(sys.argv[1]).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    SRC.mkdir(parents=True, exist_ok=True)
    (SRC / "x01-libreoffice-two-columns.fodt").write_text(x01_source(), encoding="utf-8")
    (SRC / "x02-chromium-two-columns.html").write_text(x02_source(), encoding="utf-8")
    (SRC / "x03-libreoffice-three-columns.fodt").write_text(x03_source(), encoding="utf-8")
    soffice(scratch, SRC / "x01-libreoffice-two-columns.fodt", OUT / "x01-libreoffice-two-columns.pdf")
    chrome(scratch, SRC / "x02-chromium-two-columns.html", OUT / "x02-chromium-two-columns.pdf")
    soffice(scratch, SRC / "x03-libreoffice-three-columns.fodt", OUT / "x03-libreoffice-three-columns.pdf")
    files = {}
    for path in sorted(OUT.glob("*.pdf")) + sorted(SRC.iterdir()):
        data = path.read_bytes()
        files[str(path.relative_to(OUT))] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    (OUT / "manifest.json").write_text(json.dumps({
        "exporter": "fixtures/layout/export.py",
        "tools": {"libreoffice": version(["soffice", "--version"]), "chromium": version([CHROME, "--version"])},
        "note": "Synthetic token text only. Real-application exports, not byte-reproducible.",
        "files": files,
    }, indent=2) + "\n")
    print(json.dumps(files, indent=2))


if __name__ == "__main__":
    main()
