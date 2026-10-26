#!/usr/bin/env python3
"""Runs the scanning-cost check (R4-PDF-09) by fixtures/stress/protocol.json
and records it in evidence/stress/stress-NN-<commit>.json, never
overwritten. Developer evidence only.

    python3 stress.py [--probe PDF_PROBE]

Each input is read once by pdf-probe in direct mode (no --isolated), in a
child process killed at the protocol's cut-off: a killed run is recorded
as a time-out, with no time. A small wrapper process starts the probe so
the probe's peak resident memory can be read back.
"""

import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
STRESS = HERE / "fixtures" / "stress"
OUT = HERE / "evidence" / "stress"
PROBE = HERE / "target" / "release" / "pdf-probe"

# Runs the probe with a time limit; prints one JSON line.
WRAPPER = r"""
import json, resource, subprocess, sys, time
cutoff, command = float(sys.argv[1]), sys.argv[2:]
started = time.monotonic()
try:
    done = subprocess.run(command, capture_output=True, text=True, timeout=cutoff)
except subprocess.TimeoutExpired:
    print(json.dumps({"outcome": "time-out"}))
    sys.exit(0)
wall = time.monotonic() - started
peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(json.dumps({"outcome": "returned", "exit": done.returncode, "wall_seconds": round(wall, 4),
                  "peak_rss_kib": peak, "stdout": done.stdout}))
"""


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    probe = PROBE
    if len(sys.argv) == 3 and sys.argv[1] == "--probe":
        probe = Path(sys.argv[2]).resolve()
    elif len(sys.argv) != 1:
        sys.exit(__doc__)
    protocol = json.loads((STRESS / "protocol.json").read_text())
    manifest = json.loads((STRESS / "manifest.json").read_text())["files"]
    cutoff = protocol["cutoff_seconds"]
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=HERE, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "status", "--porcelain", "."], cwd=HERE, capture_output=True,
                                text=True).stdout.strip())
    record = {
        "commit": commit,
        "worktree_dirty": dirty,
        "probe": {"path": str(probe.relative_to(HERE)) if probe.is_relative_to(HERE) else probe.name,
                  "sha256": sha256(probe)},
        "environment": {"platform": platform.platform(), "machine": platform.machine(),
                        "python": platform.python_version()},
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "protocol_sha256": sha256(STRESS / "protocol.json"),
        "cutoff_seconds": cutoff,
        "runs": {},
    }
    generated = subprocess.run([sys.executable, "-I", "fixtures/stress/generate.py", "--check"], cwd=HERE,
                               capture_output=True, text=True)
    record["inputs_generated_ok"] = generated.returncode == 0
    for name, wanted in protocol["inputs"].items():
        path = STRESS / "generated" / name
        done = subprocess.run([sys.executable, "-I", "-c", WRAPPER, str(cutoff), str(probe), "--text", str(path)],
                              capture_output=True, text=True)
        result = json.loads(done.stdout)
        entry = {"family": wanted["family"], "size": wanted["size"], "sha256": sha256(path),
                 "manifest_sha256": manifest[name]["sha256"], "outcome": result["outcome"]}
        problems = []
        if result["outcome"] == "returned":
            reading = json.loads(result["stdout"])
            summary = reading["summary"]
            entry.update({"wall_seconds": result["wall_seconds"], "millis": reading.get("millis"),
                          "peak_rss_kib": result["peak_rss_kib"], "status": summary["status"],
                          "note": summary["note"], "scan_work": reading.get("scan_work")})
            for key in ("status", "note"):
                if summary[key] != wanted[key]:
                    problems.append(f"{key} {summary[key]!r} != {wanted[key]!r}")
            if reading.get("text", "") != wanted["text"]:
                problems.append("text differs")
        else:
            problems.append(f"no result within {cutoff} s")
        entry["problems"] = problems
        entry["verdict"] = "PASS" if not problems else "FAIL"
        record["runs"][name] = entry
        shown = entry.get("millis", "-")
        print(f"{name}: {entry['outcome']} {shown} ms {entry.get('status', '')} {entry['verdict']}", flush=True)

    ratios = {}
    small, large, limit = protocol["ratio"]["small"], protocol["ratio"]["large"], protocol["ratio"]["max"]
    for family in sorted({r["family"] for r in record["runs"].values()}):
        a = record["runs"].get(f"{family}-{small}.pdf", {})
        b = record["runs"].get(f"{family}-{large}.pdf", {})
        if a.get("millis") is None or b.get("millis") is None:
            ratios[family] = {"ratio": None, "verdict": "FAIL", "why": "a run did not return"}
            continue
        ratio = b["millis"] / max(a["millis"], 1)
        ratios[family] = {"millis_small": a["millis"], "millis_large": b["millis"], "ratio": round(ratio, 2),
                          "verdict": "PASS" if ratio <= limit else "FAIL"}
    record["ratios"] = ratios
    verdicts = [r["verdict"] for r in record["runs"].values()]
    record["totals"] = {
        "inputs": len(verdicts),
        "pass": verdicts.count("PASS"),
        "fail": verdicts.count("FAIL"),
        "time_outs": sum(r["outcome"] == "time-out" for r in record["runs"].values()),
        "ratios_pass": sum(r["verdict"] == "PASS" for r in ratios.values()),
        "ratios_fail": sum(r["verdict"] == "FAIL" for r in ratios.values()),
        "inputs_generated_ok": record["inputs_generated_ok"],
    }
    OUT.mkdir(parents=True, exist_ok=True)
    numbers = [int(p.name.split("-")[1]) for p in OUT.glob("stress-*.json")]
    number = max([0, *numbers]) + 1
    out = OUT / f"stress-{number:02d}-{commit[:7]}{'-dirty' if dirty else ''}.json"
    out.write_text(json.dumps(record, indent=1) + "\n")
    print(json.dumps(record["totals"], indent=1))
    print(json.dumps(ratios, indent=1))
    print(f"written {out.relative_to(HERE)}")


if __name__ == "__main__":
    main()
