#!/usr/bin/env python3
"""Replay Core-only DOCX history in fresh app-data; never launch Word or an App."""
import argparse
import copy
import hashlib
import json
import os
import platform
from pathlib import Path
import select
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def preview_map(previews):
    indexed = {p["snapshot"]["snapshot_id"]: p for p in previews}
    if len(indexed) != len(previews):
        raise ValueError("Duplicate Snapshot ID")
    return indexed


def previews_match(previews, snapshots, packages, text_hashes):
    actual = preview_map(previews)
    wanted = {s["snapshot_id"]: s for s in snapshots}
    return (len(wanted) == len(snapshots) and set(actual) == set(wanted) and
            {p["snapshot"]["sha256"] for p in previews} == set(packages) and
            all(p["snapshot"] == wanted[sid] and "error" not in p and
                p["snapshot"]["sha256"] in text_hashes and
                p.get("preview", {}).get("text_sha256") == text_hashes[p["snapshot"]["sha256"]] and
                p.get("verified_bytes_sha256") == p["snapshot"]["sha256"] for sid, p in actual.items()))


def candidate_identity(candidate):
    return copy.deepcopy({k: candidate[k] for k in ("candidate_id", "file_id", "snapshots", "continue_from")})


def fault_matches(previews, baseline, victim_id, code):
    actual, wanted = preview_map(previews), preview_map(baseline)
    if set(actual) != set(wanted) or victim_id not in wanted:
        return False
    victim = actual[victim_id]
    return (victim["snapshot"] == wanted[victim_id]["snapshot"] and "preview" not in victim and
            victim.get("error", {}).get("code") == code and victim.get("error", {}).get("stage") == "memory" and
            all(p == wanted[sid] for sid, p in actual.items() if sid != victim_id))


def pairs_after_interruption(actual, baseline):
    if len(actual) != len(baseline):
        return False
    for current, saved in zip(actual, baseline):
        stable = lambda p: {k: v for k, v in p.items() if k not in ("gap", "gap_kinds")}
        before, after = set(saved["gap_kinds"]), set(current["gap_kinds"])
        if stable(current) != stable(saved) or after not in (before, before | {"watcher_stopped"}) or current["gap"] != bool(after):
            return False
    return True


class Driver:
    def __init__(self, binary, source, data, log, chain=None):
        args = [str(binary), "memory-e2e", str(source), str(data)]
        if chain:
            args.append(chain)
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, bufsize=1)
        self.log = log
        self.ready = self.read()

    def read(self):
        if not select.select([self.process.stdout], [], [], 20)[0]:
            self.process.kill()
            raise RuntimeError("Core driver timed out")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(self.process.stderr.read())
        value = json.loads(line)
        self.log.write(line)
        self.log.flush()
        return value

    def command(self, op, **args):
        self.process.stdin.write(json.dumps({"op": op, **args}) + "\n")
        self.process.stdin.flush()
        return self.read()["result"]

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.write('{"op":"quit"}\n')
            self.process.stdin.flush()
            _, errors = self.process.communicate(timeout=20)
            self.log.write(json.dumps({"event": "child_exit", "pid": self.process.pid, "returncode": self.process.returncode}) + "\n")
            self.log.flush()
            if self.process.returncode:
                raise RuntimeError(errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=HERE / "target/debug/docx-probe")
    parser.add_argument("--output", type=Path, help="fresh directory, outside any source root")
    args = parser.parse_args()
    base = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="traeva-d-memory-e2e-", dir="/private/tmp"))
    if args.output:
        base.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = base / "root"
    root.mkdir(mode=0o700)
    source, data = root / "lifecycle.docx", base / "app-data"
    fixture = HERE.parent / "fixtures/docx"
    first = (fixture / "word-layout-v1.docx").read_bytes()
    second = (fixture / "word-layout-v2.docx").read_bytes()
    # Reuse the accepted synthetic package generator and independently specified expected text.
    generator = HERE.parent / "word_research/make_seeds.py"
    subprocess.run(["python3", str(generator), str(base / "seeds")], check=True, capture_output=True)
    third = (base / "seeds/root/plain-history.docx").read_bytes()
    missed = (base / "seeds/root/ordinary.docx").read_bytes()
    expected = {sha(first): sha((fixture / "word-basic-v1.txt").read_bytes()),
                sha(second): sha((fixture / "word-basic-v2.txt").read_bytes()),
                sha(third): sha((base / "seeds/expected/plain-history.txt").read_bytes())}
    source.write_bytes(first)
    rows, driver = [], None
    head = subprocess.check_output(["git", "-C", str(HERE), "rev-parse", "HEAD"], text=True).strip()
    source_hashes = {str(p.relative_to(HERE.parent.parent)): sha(p.read_bytes()) for p in
                    [HERE / "memory_e2e.rs", HERE / "probe.rs", HERE / "run_memory_e2e.py", HERE / "test_memory_e2e_oracles.py", HERE.parent.parent / "src/docx.rs", HERE.parent.parent / "src/docx_cached.rs", HERE.parent.parent / "src/docx_preview.rs"]}
    report = {"scope": "Core-only synthetic replay, not a new Word GUI run",
              "base_revision": subprocess.check_output(["git", "-C", str(HERE), "merge-base", "HEAD", "origin/main"], text=True).strip(),
              "tested_tree": {"head_revision": head,
                              "worktree_dirty": bool(subprocess.check_output(["git", "-C", str(HERE), "status", "--porcelain"], text=True).strip()),
                              "source_sha256": source_hashes},
              "environment": {"os": platform.system(), "os_version": platform.mac_ver()[0], "python": platform.python_version()},
              "binary_sha256": sha(args.binary.read_bytes()),
              "expected_text_sha256_by_package": expected, "rows": rows,
              "app_flow": "BLOCKED: default dispatch/IPC/Memory entry are not connected on this main",
              "cloud_calls": 0, "historical_word_opens": 0}
    protocol = {"expected_text_sha256_by_package": expected,
                "pair_restart_expectation": "exact pair metadata and existing gaps; only watcher_stopped may be added after interruption",
                "steps": ["capture first real-Word fixture", "replace with second real-Word fixture",
                          "refuse implicit continuation", "explicitly enable candidate capture",
                          "explicit test-author continuation", "exit process without stopping Memory",
                          "replace twice while watcher absent", "restart and retain final candidate",
                          "restart with candidate still pending", "explicit continuation with watcher gap",
                          "delete synthetic source", "restart and read all versions",
                          "corrupt only second blob: snapshot_corrupt", "remove only second blob: snapshot_unavailable"],
                "app_flow": "BLOCKED", "word_gui": "existing accepted Word fixtures reused; no new GUI evidence"}
    (base / "protocol.json").write_text(json.dumps(protocol, indent=2) + "\n")
    report["protocol_sha256"] = sha((base / "protocol.json").read_bytes())

    def check(name, want, actual, passed):
        rows.append({"case": name, "expected": want, "actual": actual, "status": "PASS" if passed else "FAIL"})
        (base / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        if not passed:
            raise AssertionError(name)

    def verify(name, history, snapshots, packages):
        previews = history["previews"]
        check(name, {"snapshots": snapshots, "packages": sorted(packages), "text_hashes": expected}, previews,
              previews_match(previews, snapshots, packages, expected))

    def decide(page):
        assert len(page["items"]) == 1 and page["unread_occupant"] is None
        item = page["items"][0]
        assert item["continue_skips"] == 0 and item["continue_needs"] is None
        return {"chain_id": page["chain_id"], "page_token": page["page_token"],
                "items": [{"candidate_id": item["candidate_id"], "choice": {"kind": "continue", "parent_snapshot_id": item["continue_from"]["snapshot_id"]}}]}

    def replace(package):
        before = source.stat().st_ino
        temporary = root / "replacement.docx"
        temporary.write_bytes(package)
        os.replace(temporary, source)
        check("physical replacement", "different inode", {"before": before, "after": source.stat().st_ino}, before != source.stat().st_ino)

    try:
        with (base / "transcript.jsonl").open("x") as log:
            driver = Driver(args.binary.resolve(), source, data, log)
            initial = driver.ready["history"]
            chain = initial["chain_id"]
            family = initial["record"]["family_id"]
            snapshots = copy.deepcopy([p["snapshot"] for p in initial["previews"]])
            packages = {sha(first)}
            verify("initial actual immutable Snapshot", initial, snapshots, packages)
            check("owner-only app-data under inherited umask", "no group or other access", oct(data.stat().st_mode & 0o777),
                  data.stat().st_mode & 0o077 == 0)
            replace(second)
            driver.command("check")
            page = driver.command("pending")["page"]
            check("replacement without candidate permission", "no automatic continuation/capture", page,
                  page["unread_occupant"] is not None and not page["items"])
            verify("old Snapshot before confirmation", driver.command("history"), snapshots, packages)
            driver.command("enable_candidates")
            page = driver.command("pending")["page"]
            check("candidate is not a confirmed version", "one pending candidate, no pair", page,
                  len(page["items"]) == 1 and not driver.command("history")["pairs"])
            snapshots += copy.deepcopy(page["items"][0]["snapshots"])
            packages.add(sha(second))
            result = driver.command("decide", request=decide(page))
            check("explicit Core continuation", "watch moved; history count 2", result,
                  result.get("decision", {}).get("continued_watch") is not None and result["decision"]["record"]["line_versions"] == 2)
            history = driver.command("history")
            verify("two confirmed versions", history, snapshots, packages)
            check("continuation evidence", "user, with replacement gap", history["pairs"],
                  len(history["pairs"]) == 1 and history["pairs"][0]["evidence"] == "user" and history["pairs"][0]["gap"])
            check("confirmed pair names exact captured versions", {"before": snapshots[0], "after": snapshots[1]}, history["pairs"],
                  history["pairs"][0]["before"] == snapshots[0] and history["pairs"][0]["after"] == snapshots[1])
            first_pairs = copy.deepcopy(history["pairs"])
            driver.close()
            # A save while the watcher is absent is intentionally not captured.
            replace(missed)
            replace(third)
            driver = Driver(args.binary.resolve(), source, data, log, chain)
            driver.command("check")
            page = driver.command("pending")["page"]
            check("pending capture after process restart", "one final candidate; missed version absent", page,
                  len(page["items"]) == 1 and page["items"][0]["snapshots"][0]["sha256"] == sha(third))
            pending_identity = candidate_identity(page["items"][0])
            snapshots += copy.deepcopy(page["items"][0]["snapshots"])
            packages.add(sha(third))
            driver.close()
            driver = Driver(args.binary.resolve(), source, data, log, chain)
            page = driver.command("pending")["page"]
            check("exact pending identity across restart", pending_identity, page,
                  page["chain_id"] == chain and page["family_id"] == family and len(page["items"]) == 1 and
                  candidate_identity(page["items"][0]) == pending_identity)
            check("existing confirmed pairs survive pending restart", {"pairs": first_pairs, "permitted_gap_addition": "watcher_stopped"}, driver.ready["history"]["pairs"],
                  pairs_after_interruption(driver.ready["history"]["pairs"], first_pairs))
            driver.command("decide", request=decide(page))
            history = driver.command("history")
            verify("pending decision survives another restart", history, snapshots, packages)
            pairs = copy.deepcopy(history["pairs"])
            check("new pair names exact captured parent and child", {"before": snapshots[1], "after": snapshots[2]}, pairs,
                  len(pairs) == 2 and pairs_after_interruption(pairs[:1], first_pairs) and pairs[1]["before"] == snapshots[1] and pairs[1]["after"] == snapshots[2])
            check("historical gap kept", "watcher_stopped; no invented intermediate version", history["pairs"],
                  any("watcher_stopped" in p["gap_kinds"] for p in history["pairs"]) and all(p["snapshot"]["sha256"] != sha(missed) for p in history["previews"]))
            source.unlink()
            driver.command("check")
            history = driver.command("history")
            verify("source deletion retains all history", history, snapshots, packages)
            check("exact pairs after source deletion", pairs, history["pairs"], history["pairs"] == pairs)
            check("source loss gap", "open capture_failed gap", history["record"]["open_gaps"],
                  any(g["kind"] == "capture_failed" for g in history["record"]["open_gaps"]))
            driver.close()
            driver = Driver(args.binary.resolve(), source, data, log, chain)
            recovered = driver.command("history")
            verify("deleted-source restart recovery", recovered, snapshots, packages)
            check("exact chain family pairs and previews after recovery",
                  {"chain_id": chain, "family_id": family, "pairs": pairs, "previews": history["previews"]}, recovered,
                  recovered["chain_id"] == chain and recovered["record"]["family_id"] == family and
                  recovered["pairs"] == pairs and recovered["previews"] == history["previews"])
            victim = next(p["snapshot"] for p in history["previews"] if p["snapshot"]["sha256"] == sha(second))
            blob = data / "snapshots" / victim["sha256"][:2] / victim["sha256"]
            assert blob.resolve().is_relative_to(data.resolve()) and blob.read_bytes() == second
            blob.write_bytes(b"test-only corrupted immutable blob")
            damaged = driver.command("history")["previews"]
            check("one corrupt historical blob", {"snapshot_id": victim["snapshot_id"], "code": "snapshot_corrupt", "others": "identical full previews"}, damaged,
                  fault_matches(damaged, recovered["previews"], victim["snapshot_id"], "snapshot_corrupt"))
            blob.unlink()
            missing = driver.command("history")["previews"]
            check("one missing historical blob", {"snapshot_id": victim["snapshot_id"], "code": "snapshot_unavailable", "others": "identical full previews"}, missing,
                  fault_matches(missing, recovered["previews"], victim["snapshot_id"], "snapshot_unavailable") and not source.exists())
            driver.close()
        report["status"] = "PASS"
    except Exception as e:
        report["status"], report["failure"] = "FAIL", str(e)
        raise
    finally:
        if driver and driver.process.poll() is None:
            driver.process.kill()
            driver.process.wait(timeout=20)
        report["raw_log_sha256"] = sha((base / "transcript.jsonl").read_bytes())
        (base / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"directory": str(base), "status": report["status"], "rows": len(rows)}))


if __name__ == "__main__":
    main()
