"""Deterministic synthetic OPC fixtures, including raw saved formula caches."""
import hashlib
import io
import json
from pathlib import Path
import struct
import zipfile
import zlib

HERE = Path(__file__).resolve().parent
NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
P = "http://schemas.openxmlformats.org/package/2006/relationships"
T = "http://schemas.openxmlformats.org/package/2006/content-types"
PREFIX = "application/vnd.openxmlformats-officedocument.spreadsheetml."


def package(amount="20", formula="B2*2", cache="40"):
    types = f'''<Types xmlns="{T}">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="bin" ContentType="application/octet-stream"/>
<Override PartName="/xl/workbook.xml" ContentType="{PREFIX}sheet.main+xml"/>
<Override PartName="/xl/worksheets/budget.xml" ContentType="{PREFIX}worksheet+xml"/>
<Override PartName="/xl/worksheets/notes.xml" ContentType="{PREFIX}worksheet+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="{PREFIX}sharedStrings+xml"/>
</Types>'''
    workbook = f'''<workbook xmlns="{NS}" xmlns:r="{R}"><workbookPr date1904="0"/>
<sheets><sheet name="Budget" sheetId="1" r:id="budget"/><sheet name="Notes" sheetId="2" state="hidden" r:id="notes"/></sheets>
<calcPr calcId="0" calcMode="manual" fullCalcOnLoad="1"/></workbook>'''
    relationships = f'''<Relationships xmlns="{P}">
<Relationship Id="budget" Type="{R}/worksheet" Target="worksheets/budget.xml"/>
<Relationship Id="notes" Type="{R}/worksheet" Target="worksheets/notes.xml"/>
<Relationship Id="strings" Type="{R}/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>'''
    budget = f'''<worksheet xmlns="{NS}"><dimension ref="A1:G2"/><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="inlineStr"><is><t>Double</t></is></c></row>
<row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><v>{amount}</v></c>
<c r="C2"><f>{formula}</f><v>{cache}</v></c><c r="D2"><v>9007199254740993</v></c>
<c r="E2" t="b"><v>1</v></c><c r="F2" t="inlineStr"><is><t xml:space="preserve">  测试 &amp; 📊  </t></is></c>
<c r="G2"><f>SUM(B2)</f></c></row></sheetData></worksheet>'''
    return {
        "[Content_Types].xml": types,
        "_rels/.rels": f'<Relationships xmlns="{P}"><Relationship Id="main" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": workbook,
        "xl/_rels/workbook.xml.rels": relationships,
        "xl/sharedStrings.xml": f'<sst xmlns="{NS}" count="5" uniqueCount="3"><si><t>Item</t></si><si><t>USD</t></si><si><t>Hosting</t></si></sst>',
        "xl/worksheets/budget.xml": budget,
        "xl/worksheets/notes.xml": f'<worksheet xmlns="{NS}"><sheetData/></worksheet>',
    }


def archive(parts, compression=zipfile.ZIP_STORED):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as z:
        for name, content in sorted(parts.items()):
            info = zipfile.ZipInfo(name, (2000, 1, 1, 0, 0, 0))
            info.compress_type = compression
            info.external_attr = 0o100644 << 16
            z.writestr(info, content.encode("utf-8") if isinstance(content, str) else content)
    return out.getvalue()


def framed_archive(parts, deflate=True, descriptor="signed", tail=b""):
    local, central = bytearray(), bytearray()
    for name, content in sorted(parts.items()):
        name = name.encode("utf-8")
        content = content.encode("utf-8") if isinstance(content, str) else content
        compressor = zlib.compressobj(wbits=-15)
        payload = (compressor.compress(content) + compressor.flush() if deflate else content) + tail
        crc, compressed, expanded = zlib.crc32(content), len(payload), len(content)
        flag = 0x800 | (0 if descriptor == "absent" else 8)
        method, offset = 8 if deflate else 0, len(local)
        sizes = (crc, compressed, expanded) if descriptor == "absent" else (0, 0, 0)
        local += struct.pack("<4s5H3I2H", b"PK\x03\x04", 20, flag, method, 0, 0, *sizes, len(name), 0)
        local += name + payload
        if descriptor == "signed":
            local += b"PK\x07\x08"
        if descriptor in ("signed", "unsigned"):
            local += struct.pack("<3I", crc, compressed, expanded)
        central += struct.pack("<4s6H3I5H2I", b"PK\x01\x02", 20, 20, flag, method, 0, 0,
                               crc, compressed, expanded, len(name), 0, 0, 0, 0, 0, offset)
        central += name
    return bytes(local + central + struct.pack("<4s4H2IH", b"PK\x05\x06", 0, 0, len(parts), len(parts), len(central), len(local), 0))


def with_comment(data, comment):
    return data[:-2] + struct.pack("<H", len(comment)) + comment


def nested_directory_comment():
    p = package()
    p["xl/opaque.bin"] = b"x"
    outer = archive(p)
    p["xl/opaque.bin"] = b"x" * 50_000
    inner = bytearray(archive(p, zipfile.ZIP_DEFLATED))
    eocd = len(inner) - 22
    start = struct.unpack_from("<I", inner, eocd + 16)[0]
    pos = start
    while pos < eocd:
        offset = struct.unpack_from("<I", inner, pos + 42)[0]
        struct.pack_into("<I", inner, pos + 42, offset + len(outer))
        name, extra, comment = struct.unpack_from("<3H", inner, pos + 28)
        pos += 46 + name + extra + comment
    struct.pack_into("<I", inner, eocd + 16, start + len(outer))
    return with_comment(outer, inner + b"synthetic suffix")


def generate(destination=None):
    destination = Path(destination or HERE / "generated")
    destination.mkdir(parents=True, exist_ok=True)
    cases = {}

    def add(name, data, expected, note):
        (destination / name).write_bytes(data)
        cases[name] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
                       "expected": expected, "note": note}

    add("before.xlsx", archive(package()), "complete_within_profile", "Budget labels, raw decimal, formula/cache, uncached formula, hidden empty sheet")
    add("after.xlsx", archive(package("25", cache="50")), "complete_within_profile", "B2 literal change and C2 unverified saved cache change")
    add("cache-only.xlsx", archive(package(cache="999")), "complete_within_profile", "C2 cache changes without formula/literal edits")
    add("formula-only.xlsx", archive(package(formula="B2*3")), "complete_within_profile", "C2 formula changes while saved cache stays 40")
    add("deflated.xlsx", archive(package(), zipfile.ZIP_DEFLATED), "complete_within_profile", "Same synthetic workbook using Deflate")
    for deflate in (False, True):
        for descriptor in ("signed", "unsigned"):
            method = "deflate" if deflate else "stored"
            add(f"{method}-{descriptor}-descriptor.xlsx", framed_archive(package(), deflate, descriptor),
                "complete_within_profile", "Valid 32-bit ZIP data descriptor; offline read only")
    add("plain-comment.xlsx", with_comment(archive(package()), b"synthetic archive comment"),
        "complete_within_profile", "Plain archive comment accepted without rescanning a second directory")
    add("nested-directory-comment.xlsx", nested_directory_comment(), "unsupported",
        "Conflicting EOCD in comment and absolute inner offsets; no alternate directory may bypass ratio preflight")
    p = package()
    p["xl/workbook.xml"] = p["xl/workbook.xml"].replace('name="Budget" sheetId=', 'name="Budget"sheetId=')
    add("adjacent-attributes.xlsx", archive(p), "failed", "XML attribute values require separating XML whitespace")
    add("missing-descriptor.xlsx", framed_archive(package(), descriptor="missing"), "failed",
        "Bit 3 and zero local CRC/sizes require a descriptor immediately after compressed data")
    add("deflate-tail.xlsx", framed_archive(package(), descriptor="absent", tail=b"unconsumed tail"), "failed",
        "Declared compressed payload must end exactly at Deflate StreamEnd")
    for name, part, source, replacement in [
        ("attribute-digit-name.xlsx", "xl/workbook.xml", "<sheets>", '<sheets 1bad="synthetic">'),
        ("attribute-invalid-symbol.xlsx", "xl/workbook.xml", "<sheets>", '<sheets bad^name="synthetic">'),
        ("element-digit-name.xlsx", "xl/worksheets/budget.xml", "<sheetData>", "<1bad/><sheetData>"),
        ("comment-double-hyphen.xlsx", "xl/workbook.xml", "<sheets>", "<!-- synthetic -- invalid --><sheets>"),
        ("text-cdata-close.xlsx", "xl/worksheets/budget.xml", "Double", "synthetic ]]> text"),
    ]:
        p = package()
        p[part] = p[part].replace(source, replacement)
        add(name, archive(p), "failed", "Malformed XML lexical syntax; valid ZIP framing/CRC; no workbook may escape")
        cases[name]["xml_syntax_invalid_part"] = part
    p = package()
    p["xl/workbook.xml"] = p["xl/workbook.xml"].replace("<sheets>", '<sheets xmlns:测试="urn:synthetic" 测试:金额="1">')
    add("legal-unicode-name.xlsx", archive(p), "complete_within_profile", "Legal Unicode prefix and attribute local name")
    p = package()
    p["xl/workbook.xml"] = '<!-- synthetic <>& ]]> single-hyphen -->' + p["xl/workbook.xml"]
    add("legal-comment.xlsx", archive(p), "complete_within_profile", "Comment may contain markup characters and CDATA closer but no double hyphen")
    p = package()
    p["xl/worksheets/budget.xml"] = p["xl/worksheets/budget.xml"].replace("Double", "]]&gt;")
    add("escaped-cdata-close.xlsx", archive(p), "complete_within_profile", "Escaped CharData decodes to CDATA closer as literal text; no post-decode rejection")
    p = package()
    p["xl/worksheets/budget.xml"] = f'''<worksheet xmlns="{NS}"><sheetData>
<row r="2"><c r="B2"><f t="shared" si="0" ref="B2:B3">A2*2</f><v>4</v></c><c r="D2"><f t="array" ref="D2:D3">A2:A3*3</f><v>6</v></c></row>
<row r="3"><c r="B3"><f t="shared" si="0"/><v>6</v></c><c r="D3"><v>9</v></c></row>
</sheetData></worksheet>'''
    add("formula-regions.xlsx", archive(p), "partial", "Shared follower and array member retain unresolved formula context")
    p = package()
    for name in ["xl/charts/chart1.xml", "xl/pivotTables/pivot1.xml", "xl/connections.xml", "xl/externalLinks/externalLink1.xml"]:
        p[name] = "<syntheticUnsupportedFeature/>"
    p["xl/vbaProject.bin"] = b"SYNTHETIC INERT MARKER, NOT VBA"
    p["xl/worksheets/_rels/budget.xml.rels"] = f'<Relationships xmlns="{P}"><Relationship Id="outside" Type="{R}/hyperlink" Target="https://127.0.0.1:1/never-fetch" TargetMode="External"/></Relationships>'
    add("unsupported-features.xlsx", archive(p), "partial", "Inert feature stubs test reporting, not Excel compatibility of those features")
    p = package()
    p["xl/worksheets/budget.xml"] = f'<worksheet xmlns="{NS}"><dimension ref="A1:XFD1048576"/><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row></sheetData></worksheet>'
    add("sparse-extreme.xlsx", archive(p), "complete_within_profile", "Two cells across maximal dimensions; no rectangular allocation")
    add("truncated.xlsx", archive(package())[:-12], "failed", "Truncated ZIP end")
    p = package()
    p["xl/worksheets/budget.xml"] = '<!DOCTYPE worksheet [<!ENTITY x SYSTEM "file:///never-read">]>' + p["xl/worksheets/budget.xml"]
    add("doctype.xlsx", archive(p), "unsupported", "DTD prohibited")
    p = package()
    p["xl/_rels/workbook.xml.rels"] = p["xl/_rels/workbook.xml.rels"].replace('Target="worksheets/budget.xml"', 'Target="https://127.0.0.1:1/never-fetch" TargetMode="External"')
    add("external-sheet.xlsx", archive(p), "unsupported", "Required sheet cannot be external")
    p = package()
    p["xl/bomb.bin"] = "x" * 500_000
    add("compression-bomb.xlsx", archive(p, zipfile.ZIP_DEFLATED), "resource_limit", "Compression ratio cap reached before decompression")
    cfb = bytearray(512)
    cfb[:8] = bytes.fromhex("d0cf11e0a1b11ae1")
    struct.pack_into("<I", cfb, 44, 0xffffffff)
    add("hostile-cfb.xlsx", bytes(cfb), "unsupported", "CFB never enters an Office parser")
    (destination / "manifest.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2) + "\n")
    return cases


if __name__ == "__main__":
    print(json.dumps(generate(), ensure_ascii=False, indent=2))
