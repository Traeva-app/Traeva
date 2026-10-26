#!/usr/bin/env python3
"""Runs the #28 PDF Reader experiment's checks and records the evidence.

    python3 validate.py [--base OLD_PDF_PROBE]

1. Regenerates the program-generated fixtures and checks their hashes;
   checks the committed exports against their manifests.
2. cargo fmt --check, clippy -D warnings, cargo test (all --locked).
3. Runs the release pdf-probe on every fixture, in this process and again in
   its isolated child process, and compares with the frozen expectations and
   the shared contract's rules: fixtures/expected.json (round one),
   fixtures/defects/expected.json (R4's findings on 07ce381),
   fixtures/defects2/expected.json (R4's second round, on e2d836f),
   fixtures/defects3/expected.json (R4's third round, on c3a28cf),
   fixtures/quality/expected.json (the quality fixtures), and the layout
   fixtures as inputs (their reading-order expectations belong to PR #149).
   A case marked unverified (g25-g27) is checked against the contract and
   the quality rules that need no frozen data only, and counted apart. The
   scanning-cost check of the third round has its own runner, stress.py.
4. The quality rules of fixtures/quality/expected.json on every fixture:
   confirmed omissions are exactly the left_out entries, suspected risks
   exactly the frozen ones, stopped readings carry the frozen reason, a
   normal reading has no diagnostics, and the reading order is never
   called checked.
5. With --base (pdf-probe built from the reader without the quality
   report: pdf-experiment-7, PR #148): the text, status, left_out, note and
   page offsets must be the same, so the diagnostics changed nothing else.
6. Two isolation checks with fixed expected outcomes: a 16 MiB heap cap on
   p18 and a 0-second time limit on p16 must stop the child.

Each run is written to evidence/runs/run-NN-<commit>.json and never
overwritten, so first failures stay on record. Developer evidence only.
"""

import hashlib
import json
import platform
import re
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"
PROBE = HERE / "target" / "release" / "pdf-probe"
RUNS = HERE / "evidence" / "runs"
READER = "pdf-experiment-10"
# Run 04 is the layout experiment's (PR #149, another branch), so this line
# of runs went on from 05; runs 09-11 are the quality diagnostics' (PR #158),
# so it went on from 12; run 14 is PR #158's, so PR #148 went on from 15;
# runs 15-18 are PR #148's (merged here), so this branch went on from 19;
# run 20 is PR #148's (merged here), so this branch goes on from 21.
FIRST_RUN = 21


def run(command, timeout=1800):
    started = time.time()
    result = subprocess.run(command, cwd=HERE, capture_output=True, text=True, timeout=timeout)
    return {
        "command": " ".join(command),
        "exit": result.returncode,
        "seconds": round(time.time() - started, 2),
        "tail": (result.stdout + result.stderr).strip().splitlines()[-15:],
    }


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def probe(path, isolated=False, extra=(), executable=PROBE):
    command = [str(executable), "--text"]
    if isolated:
        command += ["--isolated", *extra]
    command.append(str(path))
    result = subprocess.run(command, capture_output=True, text=True, timeout=600)
    return json.loads(result.stdout.strip().splitlines()[-1])


def contract_problems(actual):
    """The shared TextExtraction rules (shared-contract.md §4.4)."""
    s, text = actual["summary"], actual.get("text", "")
    problems = []
    if s["reader"] != READER:
        problems.append(f"reader is {s['reader']}")
    if s["chars"] != len(text):
        problems.append(f"chars {s['chars']} != text length {len(text)}")
    if len(text) > 1_000_000:
        problems.append("text longer than 1,000,000 characters")
    if "\r" in text:
        problems.append("text contains CR")
    status = s["status"]
    if status == "complete" and (s["left_out"] or s["note"] is not None or not s["covered"]):
        problems.append("complete must have covered, no left_out and no note")
    if status == "complete" and not text.strip():
        problems.append("complete with empty text (the experiment never allows this)")
    if status == "partial" and not (text and s["covered"] and s["left_out"] and s["note"]):
        problems.append("partial must have text, covered, left_out and a note")
    if status in ("unsupported", "failed") and (text or s["chars"] or s["covered"] or s["left_out"]
                                               or not s["note"]):
        problems.append(f"{status} must have no text, covered or left_out, and a note")
    return problems


def expectation_problems(expected, actual):
    s, text = actual["summary"], actual.get("text", "")
    problems = []
    if s["status"] != expected["status"]:
        problems.append(f"status {s['status']} != {expected['status']}")
    if "text" in expected and text != expected["text"]:
        problems.append("text differs")
    if "text_sha256" in expected:
        if hashlib.sha256(text.encode()).hexdigest() != expected["text_sha256"]:
            problems.append("text hash differs")
        if not text.startswith(expected["text_head"]) or not text.endswith(expected["text_tail"]):
            problems.append("text head or tail differs")
    if "chars" in expected and s["chars"] != expected["chars"]:
        problems.append(f"chars {s['chars']} != {expected['chars']}")
    position = 0
    for snippet in expected.get("snippets_in_order", []):
        found = text.find(snippet, position)
        if found < 0:
            problems.append(f"snippet missing or out of order: {snippet!r}")
        else:
            position = found + len(snippet)
    if "note" in expected and s["note"] != expected["note"]:
        problems.append(f"note {s['note']!r} != {expected['note']!r}")
    if "note_contains" in expected and expected["note_contains"] not in (s["note"] or ""):
        problems.append(f"note lacks {expected['note_contains']!r}")
    if "left_out" in expected and s["left_out"] != expected["left_out"]:
        problems.append(f"left_out {s['left_out']} != {expected['left_out']}")
    for kind in expected.get("left_out_kinds", []):
        if not any(kind in entry for entry in s["left_out"]):
            problems.append(f"no left_out entry with {kind!r}")
    for kind in expected.get("left_out_absent", []):
        if any(kind in entry for entry in s["left_out"]):
            problems.append(f"unexpected left_out entry with {kind!r}")
    for part in expected.get("covered_contains", []):
        if not any(part in entry for entry in s["covered"]):
            problems.append(f"no covered entry with {part!r}")
    pages = {str(p["page"]): p for p in actual.get("pages", [])}
    for number, wanted in expected.get("pages", {}).items():
        page = pages.get(number)
        if page is None:
            problems.append(f"page {number} missing from the page table")
            continue
        for key in ("state", "rotated_lines", "wide_gaps", "invisible_chars", "start", "end"):
            if key in wanted and page[key] != wanted[key]:
                problems.append(f"page {number} {key} {page[key]} != {wanted[key]}")
        if "rotated_lines_min" in wanted and page["rotated_lines"] < wanted["rotated_lines_min"]:
            problems.append(f"page {number} rotated_lines {page['rotated_lines']} < {wanted['rotated_lines_min']}")
        if "omissions" in wanted:
            kinds = [o["kind"] for o in page["omissions"]]
            if kinds != wanted["omissions"]:
                problems.append(f"page {number} omissions {kinds} != {wanted['omissions']}")
    return problems


def pages_of(entry, page_count):
    """Pages a left_out entry names: "on page 2", "on pages 1-3, 5",
    "page 2: no text layer", or "(from page 111 on)"."""
    found = re.search(r"\(from page (\d+) on\)", entry)
    if found:
        return list(range(int(found.group(1)), page_count + 1))
    found = re.search(r"\bpages? (\d[0-9, -]*)", entry)
    if not found:
        return []
    pages = []
    for part in found.group(1).split(","):
        part = part.strip()
        if "-" in part:
            first, last = part.split("-")
            pages += range(int(first), int(last) + 1)
        elif part:
            pages.append(int(part))
    return pages


def quality_problems(name, path, actual, rules, judge=True):
    """The rules in fixtures/quality/expected.json (see its expectations.py).
    Without `judge` (an unverified case), the frozen suspected risks and stop
    reason are not compared."""
    quality = actual.get("quality")
    if quality is None:
        return ["no quality report"]
    s = actual["summary"]
    problems = []
    if quality["reader"] != READER:
        problems.append(f"quality reader {quality['reader']}")
    if quality["input_sha256"] != sha256(path):
        problems.append("quality input_sha256 is not the file's")
    if quality["reading_order"] != rules["reading_order"]:
        problems.append(f"reading_order {quality['reading_order']!r}")
    if quality["text_available"] != (s["status"] in ("complete", "partial")):
        problems.append("text_available does not match the status")
    if not quality["limitations"]:
        problems.append("no general limitations listed")
    diagnostics = quality["diagnostics"]
    for d in diagnostics:
        if not d["message"] or not d["basis"]:
            problems.append(f"{d['code']}: message or basis missing")
    by_level = {level: sorted((d["code"], tuple(d["pages"])) for d in diagnostics if d["level"] == level)
                for level in ("confirmed_omission", "suspected_risk", "stopped")}
    if len(by_level["confirmed_omission"]) + len(by_level["suspected_risk"]) + len(by_level["stopped"]) != len(
            diagnostics):
        problems.append("a diagnostic has an unknown level")
    page_count = len(actual.get("pages", []))
    wanted = []
    for entry in s["left_out"]:
        code = next((c for pattern, c in rules["left_out_codes"] if re.search(pattern, entry)), None)
        if code is None:
            problems.append(f"left_out entry of no known kind: {entry!r}")
            continue
        wanted.append((code, tuple(pages_of(entry, page_count))))
    if sorted(wanted) != by_level["confirmed_omission"]:
        problems.append(f"confirmed omissions {by_level['confirmed_omission']} != left_out {sorted(wanted)}")
    risks = sorted((code, tuple(pages)) for code, pages in rules["risks"].get(name, {}).items())
    if not judge:
        risks = by_level["suspected_risk"]
    if by_level["suspected_risk"] != risks:
        problems.append(f"suspected risks {by_level['suspected_risk']} != frozen {risks}")
    stopped = [code for code, _ in by_level["stopped"]]
    if s["status"] in ("unsupported", "failed"):
        if judge and stopped != [rules["stop_codes"].get(name)]:
            problems.append(f"stop reason {stopped} != frozen {rules['stop_codes'].get(name)}")
    elif stopped:
        problems.append(f"stop reason {stopped} on a reading with text")
    if s["status"] == "complete" and not risks and diagnostics:
        problems.append("a complete reading without risks has diagnostics")
    return problems


def base_problems(path, actual, base):
    """With --base: everything but the reader name and the quality report
    must be the same as the base reader's."""
    old = probe(path, executable=base)
    problems = []
    for key in ("status", "chars", "covered", "left_out", "note"):
        if old["summary"][key] != actual["summary"][key]:
            problems.append(f"{key} differs from the base reader's")
    if old.get("text", "") != actual.get("text", ""):
        problems.append("text differs from the base reader's")
    old_pages = [(p["page"], p["state"], p["start"], p["end"]) for p in old.get("pages", [])]
    new_pages = [(p["page"], p["state"], p["start"], p["end"]) for p in actual.get("pages", [])]
    if old_pages != new_pages:
        problems.append("page table differs from the base reader's")
    return problems


def compact(actual, keep_text):
    return {
        "summary": actual["summary"],
        "text_sha256": actual["text_sha256"],
        "text": actual.get("text") if keep_text else None,
        "pages": [{k: p[k] for k in ("page", "state", "start", "end", "lines", "glyphs", "rotated_lines",
                                     "invisible_chars", "wide_gaps")}
                  | {"omissions": [o["kind"] for o in p["omissions"]]} for p in actual["pages"][:12]],
        "page_count": len(actual["pages"]),
        "millis": actual.get("millis"),
        "quality": actual.get("quality"),
    }


def main():
    base = None
    if len(sys.argv) == 3 and sys.argv[1] == "--base":
        base = Path(sys.argv[2]).resolve()
    elif len(sys.argv) != 1:
        sys.exit(__doc__)
    RUNS.mkdir(parents=True, exist_ok=True)
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=HERE, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "status", "--porcelain", "."], cwd=HERE, capture_output=True,
                                text=True).stdout.strip())
    record = {
        "commit": commit,
        "worktree_dirty": dirty,
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "environment": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "rustc": subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip(),
        },
        "kind": "developer self-test (Claude-G); not an independent Codex-R4 result",
        "base_probe": None if base is None else {"name": base.name, "sha256": sha256(base)},
    }

    record["fixtures_generated"] = run([sys.executable, "-I", "fixtures/generate.py", "--check"])
    record["defect_fixtures_generated"] = run([sys.executable, "-I", "fixtures/defects/generate.py", "--check"])
    record["defect2_fixtures_generated"] = run([sys.executable, "-I", "fixtures/defects2/generate.py", "--check"])
    record["layout_fixtures_generated"] = run([sys.executable, "-I", "fixtures/layout/generate.py", "--check"])
    record["quality_fixtures_generated"] = run([sys.executable, "-I", "fixtures/quality/generate.py", "--check"])
    record["defect3_fixtures_generated"] = run([sys.executable, "-I", "fixtures/defects3/generate.py", "--check"])
    exported = json.loads((FIXTURES / "exported" / "manifest.json").read_text())
    layout_exported = json.loads((FIXTURES / "layout" / "exported" / "manifest.json").read_text())
    record["fixtures_exported"] = {
        name: entry["sha256"] == sha256(FIXTURES / "exported" / name)
        for name, entry in exported["files"].items()
    } | {
        "layout/" + name: entry["sha256"] == sha256(FIXTURES / "layout" / "exported" / name)
        for name, entry in layout_exported["files"].items()
    }
    record["checks"] = [
        run(["cargo", "fmt", "--check"]),
        run(["cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"]),
        run(["cargo", "test", "--locked"]),
        run(["cargo", "build", "--release", "--locked"]),
    ]

    rules = json.loads((FIXTURES / "quality" / "expected.json").read_text())
    expected = json.loads((FIXTURES / "expected.json").read_text())["fixtures"]
    expected.update(json.loads((FIXTURES / "defects" / "expected.json").read_text())["fixtures"])
    expected.update(json.loads((FIXTURES / "defects2" / "expected.json").read_text())["fixtures"])
    expected.update(json.loads((FIXTURES / "defects3" / "expected.json").read_text())["fixtures"])
    expected.update(rules["fixtures"])
    manifest = json.loads((FIXTURES / "manifest.json").read_text())["files"]
    for folder in ("defects", "defects2", "defects3", "quality", "layout"):
        manifest.update(json.loads((FIXTURES / folder / "manifest.json").read_text())["files"])
    manifest.update(layout_exported["files"])
    # The layout fixtures are inputs only here: no text or status is frozen
    # for them on this branch (the reader keeps the drawing order).
    for name in json.loads((FIXTURES / "layout" / "manifest.json").read_text())["files"]:
        expected[name] = {"path": f"layout/generated/{name}", "kind": "layout-input"}
    for name in layout_exported["files"]:
        if name.endswith(".pdf"):
            expected[name] = {"path": f"layout/exported/{name}", "kind": "layout-input"}
    results = {}
    for name, wanted in expected.items():
        path = FIXTURES / wanted["path"]
        actual = probe(path)
        isolated = probe(path, isolated=True, extra=("--timeout", "60", "--memory-mib", "1024"))
        unverified = bool(wanted.get("unverified"))
        problems = contract_problems(actual) + quality_problems(name, path, actual, rules, judge=not unverified)
        # An unverified case (fixtures/defects3: g25-g27) is not judged
        # against its frozen expectation.
        if "status" in wanted and not unverified:
            problems += expectation_problems(wanted, actual)
        if base is not None:
            problems += base_problems(path, actual, base)
        if isolated["summary"] != actual["summary"] or isolated["text_sha256"] != actual["text_sha256"]:
            problems.append("isolated result differs from in-process result")
        results[name] = {
            "r4_class": wanted.get("r4_class") or wanted.get("r4_finding"),
            "role": wanted.get("role"),
            "kind": wanted["kind"],
            "sha256": sha256(path),
            "manifest_sha256": manifest.get(name, exported["files"].get(name, {})).get("sha256"),
            "quality_note": rules["notes"].get(name),
            "verdict": "FAIL" if problems else "UNVERIFIED" if wanted.get("unverified") else "PASS",
            "problems": problems,
            "actual": compact(actual, keep_text=len(actual.get("text", "")) <= 4000),
            "isolated": {
                "outcome": isolated.get("isolation", {}).get("outcome"),
                "peak_heap_bytes": isolated.get("peak_heap_bytes"),
                "millis": isolated.get("isolation", {}).get("millis"),
            },
        }
    record["fixtures"] = results

    # Run 01 used p16 with a 32 MiB cap and the cap never triggered: p16
    # peaks at about 5 MiB (run 01). p18 peaks at about 30 MiB, so a 16 MiB
    # cap must stop it. The 0 s limit stays on p16.
    p16 = FIXTURES / "generated" / "p16-text-cap.pdf"
    p18 = FIXTURES / "generated" / "p18-draw-budget.pdf"
    memory = probe(p18, isolated=True, extra=("--memory-mib", "16"))
    timeout = probe(p16, isolated=True, extra=("--timeout", "0"))
    record["isolation_limits"] = {
        "p18 with a 16 MiB heap cap": {
            "expected": "memory_limit", "actual": memory["isolation"]["outcome"],
            "status": memory["summary"]["status"], "note": memory["summary"]["note"],
        },
        "p16 with a 0 s time limit": {
            "expected": "time_limit", "actual": timeout["isolation"]["outcome"],
            "status": timeout["summary"]["status"], "note": timeout["summary"]["note"],
        },
    }

    verdicts = [r["verdict"] for r in results.values()]
    checks_ok = all(c["exit"] == 0 for c in record["checks"])
    limits_ok = all(v["expected"] == v["actual"] for v in record["isolation_limits"].values())
    record["totals"] = {
        "fixtures": len(results),
        "fixtures_pass": verdicts.count("PASS"),
        "fixtures_fail": verdicts.count("FAIL"),
        "fixtures_unverified": verdicts.count("UNVERIFIED"),
        "checks_ok": checks_ok,
        "generated_hashes_ok": record["fixtures_generated"]["exit"] == 0,
        "defect_hashes_ok": record["defect_fixtures_generated"]["exit"] == 0,
        "defect2_hashes_ok": record["defect2_fixtures_generated"]["exit"] == 0,
        "layout_hashes_ok": record["layout_fixtures_generated"]["exit"] == 0,
        "quality_hashes_ok": record["quality_fixtures_generated"]["exit"] == 0,
        "compared_with_base": base is not None,
        "defect3_hashes_ok": record["defect3_fixtures_generated"]["exit"] == 0,
        "exported_hashes_ok": all(record["fixtures_exported"].values()),
        "isolation_limits_ok": limits_ok,
    }
    numbers = [int(path.name.split("-")[1]) for path in RUNS.glob("run-*.json")]
    number = max([FIRST_RUN - 1, *numbers]) + 1
    out = RUNS / f"run-{number:02d}-{commit[:7]}{'-dirty' if dirty else ''}.json"
    out.write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n")
    print(json.dumps(record["totals"], indent=1))
    for name, result in results.items():
        if result["problems"]:
            print(name, result["problems"])
    print(f"written {out.relative_to(HERE)}")


if __name__ == "__main__":
    main()
