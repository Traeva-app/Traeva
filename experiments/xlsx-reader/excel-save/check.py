"""Compare a retained package to frozen fields; never certify Excel provenance."""

import argparse
import hashlib
import json
import subprocess
from decimal import Decimal
from pathlib import Path

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def equivalent(actual, expected):
    if not isinstance(actual, dict) or actual.get("type") != expected["type"]:
        return False
    if expected["type"] == "number":
        return Decimal(actual["value"]) == Decimal(expected["value"])
    return actual.get("value") == expected["value"]


def check(workbook, protocol, stage):
    checks = []

    def expect(label, passed):
        checks.append({"field": label, "status": "PASS" if passed else "FAIL"})

    expect("sheet_order", [s["name"] for s in workbook["sheets"]] == protocol["sheet_order"])
    by_name = {s["name"]: s for s in workbook["sheets"]}
    for name in protocol["sheet_order"]:
        sheet = by_name.get(name, {})
        cells = {c["address"]: c for c in sheet.get("cells", [])}
        expect(f"{name}:visibility", sheet.get("visibility") == protocol["visibility"])
        expect(f"{name}:merged_ranges", sheet.get("merged_ranges") == protocol["merged_ranges"][name])
        expected = dict(protocol["fixed_literals"][name])
        if name == protocol["input_cell"]["sheet"]:
            expected[protocol["input_cell"]["address"]] = {"type": "number", "value": stage["input"]}
        for address, value in expected.items():
            cell = cells.get(address, {})
            expect(f"{name}!{address}:literal", equivalent(cell.get("literal"), value))
            expect(f"{name}!{address}:no_formula_cache", cell.get("formula") is None and cell.get("cached_result") is None)
        if name == protocol["formula_cell"]["sheet"]:
            cell = cells.get(protocol["formula_cell"]["address"], {})
            formula = cell.get("formula") or {}
            cache = cell.get("cached_result") or {}
            expect(f"{name}!C2:formula_text", formula.get("text") == stage["formula"])
            expect(f"{name}!C2:formula_literal_separate", cell.get("literal") is None)
            expect(f"{name}!C2:cache_token", equivalent(cache.get("value"), {"type": "number", "value": stage["expected_saved_cache"]}))
            expect(f"{name}!C2:cache_freshness", cache.get("freshness") == "unverified")
    return checks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path)
    parser.add_argument("stage", choices=["seed", "baseline", "input_edit", "formula_edit"])
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    protocol = json.loads((HERE / "protocol.json").read_text())
    stage = protocol["stages"]["baseline" if args.stage == "seed" else args.stage]
    input_hash = digest(args.input)
    args.output.mkdir(parents=True, exist_ok=True)
    result = subprocess.run([str(args.probe.resolve()), str(args.input.resolve())], capture_output=True, timeout=10)
    (args.output / "reader.json").write_bytes(result.stdout)
    (args.output / "reader.stderr").write_bytes(result.stderr)
    parsed = json.loads(result.stdout)
    workbook = parsed.get("workbook")
    checks = check(workbook, protocol, stage) if workbook else []
    unchanged = input_hash == digest(args.input)
    successful = result.returncode == 0 and workbook is not None and unchanged
    successful = successful and workbook["input_sha256"] == input_hash and all(c["status"] == "PASS" for c in checks)
    report = {
        "status": "FIELD_PASS" if successful else "FAIL",
        "stage": args.stage, "protocol_sha256": digest(HERE / "protocol.json"),
        "input_sha256": input_hash, "input_sha256_after": digest(args.input),
        "reader_output_sha256": digest(args.output / "reader.json"), "probe_sha256": digest(args.probe),
        "exit_code": result.returncode, "input_unchanged": unchanged, "checks": checks,
        "excel_provenance": "UNVERIFIED_BY_THIS_CHECKER",
        "cache_freshness": "unverified",
        "scope": "Frozen field comparisons only. GUI save/close/reopen evidence must be reviewed separately.",
        "reader_error": parsed.get("error"),
        "reader_status": workbook.get("status") if workbook else None,
        "omissions": workbook.get("omissions") if workbook else None,
        "limitations": workbook.get("limitations") if workbook else None
    }
    (args.output / "fields.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "checks": len(checks), "excel_provenance": report["excel_provenance"]}))
    return 0 if successful else 1


if __name__ == "__main__":
    raise SystemExit(main())
