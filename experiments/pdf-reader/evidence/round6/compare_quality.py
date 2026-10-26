#!/usr/bin/env python3
"""Reads the same files with two pdf-probe builds that both write the
quality report, and compares the reports (everything but the reader name).
Each file is read in the probe's isolated child process with a heap cap and
a time limit. Writes evidence/round6/quality-NN-<label>.json, never
overwritten. Developer evidence only.

    python3 evidence/round6/compare_quality.py LABEL OLD_PROBE NEW_PROBE FILE...
"""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def quality(probe, path):
    command = [str(probe), "--isolated", "--timeout", "20", "--memory-mib", "1024", str(path)]
    done = subprocess.run(command, capture_output=True, text=True, timeout=120)
    reading = json.loads(done.stdout.strip().splitlines()[-1])
    report = dict(reading.get("quality") or {})
    report.pop("reader", None)
    return report


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    label, old, new, files = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4:]
    record = {
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "old_probe_sha256": sha256(old),
        "new_probe_sha256": sha256(new),
        "files": {},
    }
    same = 0
    for name in files:
        a, b = quality(old, name), quality(new, name)
        identical = a == b
        same += identical
        record["files"][Path(name).name] = {
            "identical": identical,
            "old_codes": sorted(d["code"] for d in a.get("diagnostics", [])),
            "new_codes": sorted(d["code"] for d in b.get("diagnostics", [])),
        }
        if not identical:
            print(f"CHANGED {Path(name).name}: {record['files'][Path(name).name]['old_codes']} -> "
                  f"{record['files'][Path(name).name]['new_codes']}", flush=True)
    record["totals"] = {"files": len(files), "identical": same, "changed": len(files) - same}
    numbers = [int(p.name.split("-")[1]) for p in HERE.glob("quality-*.json")]
    out = HERE / f"quality-{max([0, *numbers]) + 1:02d}-{label}.json"
    out.write_text(json.dumps(record, indent=1) + "\n")
    print(json.dumps(record["totals"]))
    print(f"written {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
