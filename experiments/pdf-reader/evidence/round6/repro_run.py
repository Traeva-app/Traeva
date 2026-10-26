#!/usr/bin/env python3
"""Runs the N1 and N2 reproductions (evidence/round6/repro/) through the
release pdf-probe in its isolated child process, with a heap cap and a time
limit, and records evidence/round6/n1-n2-NN-<commit>.json, never
overwritten. Inputs are generated into a temporary directory and recorded
by SHA-256 only. Developer evidence only.

    python3 evidence/round6/repro_run.py
"""

import hashlib
import json
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PROBE = ROOT / "target" / "release" / "pdf-probe"
TIMEOUT_SECONDS = 60
MEMORY_MIB = 1024


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def probe(path):
    command = [str(PROBE), "--text", "--isolated", "--timeout", str(TIMEOUT_SECONDS),
               "--memory-mib", str(MEMORY_MIB), str(path)]
    done = subprocess.run(command, capture_output=True, text=True, timeout=TIMEOUT_SECONDS + 60)
    reading = json.loads(done.stdout.strip().splitlines()[-1])
    return {
        "status": reading["summary"]["status"],
        "note": reading["summary"]["note"],
        "chars": reading["summary"]["chars"],
        "isolation": reading.get("isolation"),
        "peak_heap_bytes": reading.get("peak_heap_bytes"),
    }


def main():
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "status", "--porcelain", "."], cwd=ROOT, capture_output=True,
                                text=True).stdout.strip())
    record = {
        "commit": commit,
        "worktree_dirty": dirty,
        "probe_sha256": sha256(PROBE),
        "environment": {"platform": platform.platform(), "machine": platform.machine(),
                        "python": platform.python_version()},
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "limits": {"timeout_seconds": TIMEOUT_SECONDS, "memory_mib": MEMORY_MIB, "mode": "pdf-probe --isolated"},
        "runs": {},
    }
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        subprocess.run([sys.executable, "-I", str(HERE / "repro" / "predictor_abort.py"),
                        str(tmp / "n1-predictor-abort.pdf")], check=True)
        subprocess.run([sys.executable, "-I", str(HERE / "repro" / "hayro_cost_inputs.py"), str(tmp)], check=True)
        for path in sorted(tmp.glob("*.pdf")):
            result = probe(path)
            result["sha256"] = sha256(path)
            result["bytes"] = path.stat().st_size
            record["runs"][path.name] = result
            print(path.name, result["status"], (result["isolation"] or {}).get("outcome"),
                  (result["isolation"] or {}).get("millis"), "ms", flush=True)
    numbers = [int(p.name.split("-")[2]) for p in HERE.glob("n1-n2-*.json")]
    number = max([0, *numbers]) + 1
    out = HERE / f"n1-n2-{number:02d}-{commit[:7]}{'-dirty' if dirty else ''}.json"
    out.write_text(json.dumps(record, indent=1) + "\n")
    print(f"written {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
