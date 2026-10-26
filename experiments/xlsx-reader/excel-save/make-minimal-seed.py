"""Author a minimal test fixture when Artifact Tool cannot emit this profile."""

import hashlib
import json
import sys
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "fixtures"))
from generate import NS, P, PREFIX, R, T, archive

protocol = json.loads((HERE / "protocol.json").read_text())
stage = protocol["stages"]["baseline"]
overrides = [f'<Override PartName="/xl/workbook.xml" ContentType="{PREFIX}sheet.main+xml"/>']
sheets, relationships = [], []
parts = {
    "_rels/.rels": f'<Relationships xmlns="{P}"><Relationship Id="main" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>'
}
for i, name in enumerate(protocol["sheet_order"], 1):
    part = f"xl/worksheets/sheet{i}.xml"
    overrides.append(f'<Override PartName="/{part}" ContentType="{PREFIX}worksheet+xml"/>')
    sheets.append(f'<sheet name={quoteattr(name)} sheetId="{i}" r:id="sheet{i}"/>')
    relationships.append(f'<Relationship Id="sheet{i}" Type="{R}/worksheet" Target="worksheets/sheet{i}.xml"/>')
    literals = dict(protocol["fixed_literals"][name])
    if name == protocol["input_cell"]["sheet"]:
        literals[protocol["input_cell"]["address"]] = {"type": "number", "value": stage["input"]}
    cells = {}
    for address, literal in literals.items():
        typ, value = literal["type"], literal["value"]
        if typ == "text":
            content = f'<c r="{address}" t="inlineStr"><is><t xml:space="preserve">{escape(value)}</t></is></c>'
        elif typ == "boolean":
            content = f'<c r="{address}" t="b"><v>{int(value)}</v></c>'
        else:
            content = f'<c r="{address}"><v>{escape(value)}</v></c>'
        cells[address] = content
    if name == protocol["formula_cell"]["sheet"]:
        address = protocol["formula_cell"]["address"]
        cells[address] = f'<c r="{address}"><f>{escape(stage["formula"])}</f><v>{stage["expected_saved_cache"]}</v></c>'
    rows = {}
    for address, content in sorted(cells.items()):
        row = int(address.lstrip("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))
        rows.setdefault(row, []).append(content)
    sheet_data = "".join(f'<row r="{row}">{"".join(content)}</row>' for row, content in sorted(rows.items()))
    merges = protocol["merged_ranges"][name]
    merge_xml = f'<mergeCells count="{len(merges)}">' + "".join(f'<mergeCell ref="{ref}"/>' for ref in merges) + "</mergeCells>" if merges else ""
    parts[part] = f'<worksheet xmlns="{NS}"><sheetData>{sheet_data}</sheetData>{merge_xml}</worksheet>'
parts["[Content_Types].xml"] = f'<Types xmlns="{T}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{"".join(overrides)}</Types>'
parts["xl/workbook.xml"] = f'<workbook xmlns="{NS}" xmlns:r="{R}"><sheets>{"".join(sheets)}</sheets></workbook>'
parts["xl/_rels/workbook.xml.rels"] = f'<Relationships xmlns="{P}">{"".join(relationships)}</Relationships>'
data = archive(parts)
(HERE / "f-excel-seed.xlsx").write_bytes(data)
print(json.dumps({"writer": "existing synthetic fixture archive helper", "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "cache_origin": "frozen synthetic token; no formula evaluation", "excel_execution": "NOT_EXECUTED"}))
