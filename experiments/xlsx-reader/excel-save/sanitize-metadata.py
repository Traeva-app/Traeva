"""Create a metadata-sanitized derivative of an owned Office test package."""

import argparse
import copy
import hashlib
import io
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET

SYNTHETIC = "Traeva synthetic"
CP = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
DC = "http://purl.org/dc/elements/1.1/"
for prefix, uri in [("cp", CP), ("dc", DC),
                    ("dcterms", "http://purl.org/dc/terms/"),
                    ("dcmitype", "http://purl.org/dc/dcmitype/"),
                    ("xsi", "http://www.w3.org/2001/XMLSchema-instance")]:
    ET.register_namespace(prefix, uri)


def sanitize(original):
    changed = []
    private = {str(Path.home()), Path.home().name}
    fields = {"existing_identity_fields": 0, "added_identity_fields": 0,
              "company_manager_fields": 0, "absolute_path_elements": 0}
    output = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(original)) as source:
        records = source.infolist()
        if len({i.filename for i in records}) != len(records):
            raise ValueError("duplicate package entries")
        if source.testzip() is not None:
            raise ValueError("invalid source CRC")
        with zipfile.ZipFile(output, "w") as target:
            target.comment = source.comment
            for info in records:
                before = source.read(info)
                data = before
                if info.filename == "docProps/core.xml":
                    tree = ET.fromstring(data)
                    for tag in [f"{{{DC}}}creator", f"{{{CP}}}lastModifiedBy"]:
                        elements = list(tree.iter(tag))
                        if not elements:
                            elements = [ET.SubElement(tree, tag)]
                            fields["added_identity_fields"] += 1
                        else:
                            fields["existing_identity_fields"] += len(elements)
                        for element in elements:
                            if element.text and element.text.strip() != SYNTHETIC:
                                private.add(element.text.strip())
                            element.text = SYNTHETIC
                    data = ET.tostring(tree, encoding="utf-8", xml_declaration=True)
                elif info.filename == "docProps/app.xml":
                    tree = ET.fromstring(data)
                    elements = [e for e in tree.iter() if e.tag.rsplit("}", 1)[-1] in ["Company", "Manager"]]
                    if elements:
                        for element in elements:
                            if element.text and element.text.strip():
                                private.add(element.text.strip())
                            element.text = None
                        fields["company_manager_fields"] += len(elements)
                        data = ET.tostring(tree, encoding="utf-8", xml_declaration=True)
                elif info.filename == "xl/workbook.xml":
                    tree = ET.fromstring(data)
                    count = sum(e.tag.rsplit("}", 1)[-1] == "absPath" for e in tree.iter())
                    if count:
                        text = data.decode("utf-8")
                        pattern = r"<(?P<tag>(?:[\w.-]+:)?absPath)\b[^>]*(?:/>|>.*?</(?P=tag)\s*>)"
                        text, removed = re.subn(pattern, "", text, flags=re.DOTALL)
                        if removed != count:
                            raise ValueError("absolute-path element removal count mismatch")
                        data = text.encode("utf-8")
                        ET.fromstring(data)
                        fields["absolute_path_elements"] += removed
                if data != before:
                    changed.append(info.filename)
                target.writestr(copy.copy(info), data)
    sanitized = output.getvalue()
    with zipfile.ZipFile(io.BytesIO(sanitized)) as target:
        if target.testzip() is not None:
            raise ValueError("invalid derivative CRC")
        if [(i.filename, i.compress_type) for i in target.infolist()] != [(i.filename, i.compress_type) for i in records]:
            raise ValueError("entry order or compression method changed")
        archive_comment = target.comment.decode("utf-8", errors="replace")
        remaining = int("/Users/" in archive_comment or any(t and t in archive_comment for t in private))
        for info in target.infolist():
            payloads = [target.read(info), info.filename.encode(), info.extra, info.comment]
            for payload in payloads:
                decoded = payload.decode("utf-8", errors="replace")
                remaining += int("/Users/" in decoded or any(t and t in decoded for t in private))
        if remaining:
            raise ValueError(f"remaining private package locations: {remaining}")
    report = {
        "kind": "metadata-sanitized derivative; not a byte-identical native Excel output",
        "original_output_sha256": hashlib.sha256(original).hexdigest(),
        "original_output_bytes": len(original),
        "sanitized_sha256": hashlib.sha256(sanitized).hexdigest(),
        "sanitized_bytes": len(sanitized),
        "changed_parts": changed,
        "unchanged_parts": len(records) - len(changed),
        "field_counts": fields,
        "entry_order_preserved": True,
        "compression_methods_preserved": True,
        "crc_and_sizes_valid": True,
        "remaining_private_locations": 0,
        "formula_evaluation": False,
    }
    return sanitized, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    if args.input.resolve() == args.output.resolve():
        parser.error("use a separate output for the derivative")
    try:
        data, report = sanitize(args.input.read_bytes())
        args.output.write_bytes(data)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    except (OSError, ValueError, zipfile.BadZipFile, ET.ParseError):
        print(json.dumps({"status": "FAIL", "reason": "metadata sanitization or privacy verification failed"}))
        return 1
    print(json.dumps({"status": "PASS", "changed_parts": len(report["changed_parts"]),
                      "remaining_private_locations": 0}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
