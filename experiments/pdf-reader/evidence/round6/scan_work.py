#!/usr/bin/env python3
"""Records the raw-byte check's work (pdf-probe's scan_work) per file, for
the scan-work budget that is only a candidate: how much a budget of the
form c * size + k would need to allow so that no current file is refused.
Each file is read in the probe's isolated child process with a heap cap and
a time limit. Writes evidence/round6/scan-work-NN-<commit>.json, never
overwritten. Developer evidence only.

    python3 evidence/round6/scan_work.py FILE...
"""

import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PROBE = ROOT / "target" / "release" / "pdf-probe"


def main():
    files = sys.argv[1:]
    if not files:
        sys.exit(__doc__)
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    record = {
        "commit": commit,
        "probe_sha256": hashlib.sha256(PROBE.read_bytes()).hexdigest(),
        "environment": {"platform": platform.platform(), "machine": platform.machine()},
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "files": {},
    }
    for name in files:
        path = Path(name)
        done = subprocess.run([str(PROBE), "--isolated", "--timeout", "20", "--memory-mib", "1024", str(path)],
                              capture_output=True, text=True, timeout=120)
        reading = json.loads(done.stdout.strip().splitlines()[-1])
        work = reading.get("scan_work")
        size = path.stat().st_size
        entry = {"bytes": size, "status": reading["summary"]["status"], "scan_work": work}
        if work:
            entry["elements_per_byte"] = round(work["elements"] / max(size, 1), 4)
            entry["decode_bytes_per_byte"] = round(work["decode_bytes"] / max(size, 1), 4)
        record["files"][str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else path.name] = entry
    measured = [e for e in record["files"].values() if e["scan_work"]]
    record["totals"] = {
        "files": len(files),
        "check_passed": len(measured),
        "max_elements_per_byte": max((e["elements_per_byte"] for e in measured), default=None),
        "max_elements": max((e["scan_work"]["elements"] for e in measured), default=None),
        "max_decode_bytes_per_byte": max((e["decode_bytes_per_byte"] for e in measured), default=None),
        "max_decode_bytes": max((e["scan_work"]["decode_bytes"] for e in measured), default=None),
    }
    numbers = [int(p.name.split("-")[2]) for p in HERE.glob("scan-work-*.json")]
    number = max([0, *numbers]) + 1
    out = HERE / f"scan-work-{number:02d}-{commit[:7]}.json"
    out.write_text(json.dumps(record, indent=1) + "\n")
    print(json.dumps(record["totals"], indent=1))
    print(f"written {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
