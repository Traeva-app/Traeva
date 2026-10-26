#!/usr/bin/env python3
"""Reads the same files with two pdf-probe builds and compares their results:
status, characters, covered, left_out, note, text hash and the full page
table (the reader name is left out). Each file is read in the probe's
isolated child process with a heap cap and a time limit. Writes
evidence/round6/compare-NN-<label>.json, never overwritten. Developer
evidence only.

    python3 evidence/round6/compare_readers.py LABEL OLD_PROBE NEW_PROBE FILE...
"""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TIMEOUT_SECONDS = 20
MEMORY_MIB = 1024


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(probe, path):
    command = [str(probe), "--text", "--isolated", "--timeout", str(TIMEOUT_SECONDS),
               "--memory-mib", str(MEMORY_MIB), str(path)]
    done = subprocess.run(command, capture_output=True, text=True, timeout=TIMEOUT_SECONDS + 60)
    reading = json.loads(done.stdout.strip().splitlines()[-1])
    summary = dict(reading["summary"])
    summary.pop("reader", None)
    return {
        "summary": summary,
        "text_sha256": reading["text_sha256"],
        "pages": reading.get("pages", []),
        "isolation": (reading.get("isolation") or {}).get("outcome"),
        "millis": (reading.get("isolation") or {}).get("millis"),
    }


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    label, old_probe, new_probe, files = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4:]
    record = {
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "old_probe_sha256": sha256(old_probe),
        "new_probe_sha256": sha256(new_probe),
        "limits": {"timeout_seconds": TIMEOUT_SECONDS, "memory_mib": MEMORY_MIB},
        "files": {},
    }
    same = changed = 0
    for name in files:
        path = Path(name)
        old, new = read(old_probe, path), read(new_probe, path)
        keys = ("summary", "text_sha256", "pages", "isolation")
        identical = all(old[k] == new[k] for k in keys)
        same += identical
        changed += not identical
        entry = {"sha256": sha256(path), "identical": identical,
                 "old": {k: old[k] for k in ("isolation", "millis")} | {"status": old["summary"]["status"],
                                                                           "note": old["summary"]["note"]},
                 "new": {k: new[k] for k in ("isolation", "millis")} | {"status": new["summary"]["status"],
                                                                           "note": new["summary"]["note"]}}
        record["files"][str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else path.name] = entry
        if not identical:
            print(f"CHANGED {path.name}: {old['summary']['status']} -> {new['summary']['status']} "
                  f"({new['summary']['note']})", flush=True)
    record["totals"] = {"files": len(files), "identical": same, "changed": changed}
    numbers = [int(p.name.split("-")[1]) for p in HERE.glob("compare-*.json")]
    number = max([0, *numbers]) + 1
    out = HERE / f"compare-{number:02d}-{label}.json"
    out.write_text(json.dumps(record, indent=1) + "\n")
    print(json.dumps(record["totals"]))
    print(f"written {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
