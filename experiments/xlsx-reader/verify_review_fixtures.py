"""Recheck exact R3 synthetic inputs without copying its audit harness."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
CASES = {
    "nested-eocd-absolute.xlsx": ("a4cdec0c4881ee7e9fa2f55ff76c3ad42eeaea50f006b06d72f7d6f57f7adee5", "ambiguous_zip_end_in_comment"),
    "adjacent-attributes.xlsx": ("efe4395fe3c6922e551c0ff98c4bd1ba186150e403cd1bd4c468a2e2866966e1", "missing_xml_attribute_separator"),
    "missing-descriptor.xlsx": ("65c0309d67474e775c8a8e19f2ad59e5adb966b6bdb7cc40782564ef9d2da490", "missing_or_invalid_zip_descriptor"),
    "deflate-tail.xlsx": ("abaf58c9579dee0288fd384f6f7efe4fd475b6896c9d3b2adf4bf1a7019d471c", "zip_deflate_trailing_bytes"),
}
XML_CASES = {
    "attribute-digit-name.xlsx": ("f46d1f3ed42cfee7a6097824cba9300ddef68840d7b7a938497fc7674223e3ac", "invalid_xml_name"),
    "attribute-invalid-symbol.xlsx": ("b1fc98fa9ba064cf84bf808304cad9fa50f08327dab3b21c05e0dcfdbfc0496f", "invalid_xml_name"),
    "element-digit-name.xlsx": ("1ced698cff9f396f3e9bc014d5080cb2c92ec68ceacff20fb4754b5ec9aa5f60", "invalid_xml_name"),
    "comment-double-hyphen.xlsx": ("2ad347f9f190abe91ce72a58f26849b3fce63111b7f592ca763d5ca58ce650e3", "malformed_xml"),
    "text-cdata-close.xlsx": ("b1365930a75927e820afe443f7e3e674684a35e0811dd95d050e9fa2c44931a6", "invalid_xml_character_data"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="Directory containing the selected R3 synthetic XLSX files")
    parser.add_argument("--round", choices=("first", "xml"), default="first", help="Select first-round ZIP/separator cases or second-round XML cases")
    args = parser.parse_args()
    rows = []
    local = HERE / ("evidence/local/r3-fixes/recheck" if args.round == "first" else "evidence/local/r3-xml-fixes/recheck")
    local.mkdir(parents=True, exist_ok=True)
    for name, (expected_hash, error_code) in (CASES if args.round == "first" else XML_CASES).items():
        path = args.directory / name
        initial = hashlib.sha256(path.read_bytes()).hexdigest()
        if initial != expected_hash:
            raise RuntimeError(f"review fixture hash mismatch: {name}")
        result = subprocess.run([str(HERE / "target/debug/xlsx-probe"), str(path)],
                                capture_output=True, text=True, timeout=10)
        (local / f"{name}.stdout.json").write_text(result.stdout)
        (local / f"{name}.stderr.txt").write_text(result.stderr)
        data = json.loads(result.stdout)
        assert result.returncode == 1 and "workbook" not in data, (name, data)
        assert data["error"]["code"] == error_code, (name, data)
        assert hashlib.sha256(path.read_bytes()).hexdigest() == initial, name
        rows.append({"fixture": name, "sha256": initial, "returncode": result.returncode,
                     "error": data["error"], "has_workbook": False, "input_unchanged": True, "result": "PASS"})
    report = {"recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "scope": "F author recheck only; independent R3 acceptance on new SHA pending",
              "binary_sha256": hashlib.sha256((HERE / "target/debug/xlsx-probe").read_bytes()).hexdigest(),
              "cases": rows}
    (local / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
