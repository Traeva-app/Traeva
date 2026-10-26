#!/usr/bin/env python3
"""Read frozen Chrome/native evidence and actual synthetic outputs independently."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import sys

import writer


def verify(root):
    root = Path(root)
    evidence = root / "evidence"
    extension = json.loads((evidence / "extension-evidence.json").read_text())
    environment = json.loads((evidence / "environment.json").read_text())
    scenarios = json.loads((evidence / "scenario-results.json").read_text())
    events = [json.loads(x) for x in (root / "native/app-data/events.jsonl").read_text().splitlines()]
    records = json.loads((root / "native/app-data/results.json").read_text())["records"]
    stimuli = json.loads((evidence / "stimuli.json").read_text())
    checks = 0
    limitations = []

    def check(condition, label):
        nonlocal checks
        if not condition:
            raise AssertionError(label)
        checks += 1

    check(not (evidence / "error.json").exists(), "run has no hidden harness error")
    check(environment["headed"] is True, "headful browser configured")
    check("not normal public CA" in environment["tlsScope"], "TLS limitation explicit")
    check([x["eventSequence"] for x in events] == list(range(1, len(events) + 1)), "durable event sequence continuous")
    check(scenarios[0]["actual"]["error"] == "takeover_disabled", "default takeover refused")
    check(json.loads(scenarios[1]["state"])["error"] == "explicit_consent_required", "unchecked consent refused")
    acquisitions = extension["acquisitions"]
    check(len({x["acquisitionId"] for x in acquisitions}) == len(acquisitions), "unique task IDs")
    for task in acquisitions:
        aid = task["acquisitionId"]
        native = next(x for x in records if x["acquisitionId"] == aid)
        check(native["binding"] == task["binding"], aid + " exact native/extension stream binding")
        chunks = [x for x in events if x["event"] == "chunk_written_to_held_fd" and x["acquisitionId"] == aid]
        check([x["sequence"] for x in chunks] == list(range(len(chunks))), aid + " chunk sequence")
        check(sum(x["bytes"] for x in chunks) == native["receivedBytes"], aid + " actual received byte count")
        for chunk in chunks:
            check(writer.base.same_object(chunk["heldObject"], native["createdObject"]), aid + " retained receive object")
        receipts = list((root / "native/app-data").glob(aid + ".receipt.json"))
        target = root / "native/delivery" / native["destination"]
        if task["state"] == "HISTORICAL_ACQUISITION_OBSERVED":
            check(native["state"] == "ACQUIRED_OBSERVED_SAME_OBJECT", aid + " completed native state")
            check(len(receipts) == 1, aid + " single durable receipt")
            receipt = json.loads(receipts[0].read_text())
            check(receipt["sourceNamespace"] == "TRAEVA_CHROME_FETCH_ACQUISITION" and receipt["chromeDownloadId"] is None,
                  aid + " no impersonated Chrome DownloadItem ID")
            check(receipt["currentPathRequiresRecheck"] is True, aid + " no permanent current-path claim")
            fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                observed = writer.base.snapshot(fd)
                check(writer.base.same_object(observed, receipt["targetCreatedObject"]), aid + " output object created by writer")
                check(not writer.base.same_object(observed, receipt["createdObject"]), aid + " explicit new destination object")
                check(observed["size"] == native["expectedBytes"] == native["receivedBytes"], aid + " exact fixed-length output")
                expected = environment["fixture"]["distinctSha256" if task["response"]["url"].endswith("/distinct") else "sha256"]
                check(hashlib.sha256(target.read_bytes()).hexdigest() == receipt["streamSha256"] == expected,
                      aid + " stream bytes match independent server fixture")
                check(writer.attribute(fd, "com.apple.quarantine").startswith(b"0001;"), aid + " lab quarantine readback only")
                check(plistlib.loads(writer.attribute(fd, "com.apple.metadata:kMDItemWhereFroms")) == [task["response"]["url"]],
                      aid + " actual response URL metadata")
            finally:
                os.close(fd)
        else:
            check(task["state"] == "FAILED_AFTER_TAKE", aid + " failure explicit after take")
            check(not receipts, aid + " no false success receipt")
            if task["scenario"] != "target-exists":
                check(not target.exists(), aid + " receive failure had no delivery write")
    for scenario in scenarios[2:]:
        name = scenario["scenario"]
        tasks = scenario["acquisitions"]
        if name in ("chunked", "compressed", "unsupported", "http", "too-long", "private-query"):
            check(not tasks, name + " no takeover")
            if name == "private-query":
                check(scenario["downloadItemDelta"] == 0, "private query excluded from extension export")
            else:
                check(scenario["downloadItemDelta"] == 1, name + " Chrome fallback created one item")
                matches = [x for x in extension["downloadItems"] if x["url"].endswith("/" + name)]
                check(len(matches) == 1 and matches[0]["state"] == "complete", name + " real Chrome fallback completed")
            if name in ("too-long", "private-query"):
                browser_events = json.loads((evidence / "browser-download-events.json").read_text())
                check(any(x["scenario"] == name and x["event"] == "Browser.downloadProgress" and x["data"]["state"] == "completed" for x in browser_events), name + " actual browser completion event")
            if name != "http":
                check(any(x["event"] == "continued_before_take" and x.get("scenario") == name for x in extension["events"]),
                      name + " continuation before taking stream recorded")
        else:
            check(scenario["downloadItemDelta"] == 0, name + " no substitute Chrome DownloadItem")
        if name in ("parallel", "same-url", "stage-swap", "same-tab"):
            check(len(tasks) == 2, name + " two independent acquisition records")
            check(tasks[0]["binding"] != tasks[1]["binding"], name + " distinct full browser task bindings")
            bound = [[x["atNs"] for x in events if x.get("acquisitionId") == t["acquisitionId"]
                      and x["event"] == "chunk_written_to_held_fd"] for t in tasks]
            overlap = max(min(x) for x in bound) < min(max(x) for x in bound)
            if name == "same-tab" and not overlap:
                limitations.append("NOT_ESTABLISHED_SAME_TAB_OVERLAP: two sequential acquisitions only")
            else:
                check(overlap, name + " actual overlapping receive intervals")
        if name == "redirect":
            check([x["status"] for x in tasks[0]["redirectChain"]] == [302, 307, 200], "direct redirect request chain")
    for stimulus in stimuli:
        if stimulus["scenario"] == "target-exists":
            path = Path(stimulus["path"])
            check(path.stat().st_ino == stimulus["before"]["inode"] and
                  hashlib.sha256(path.read_bytes()).hexdigest() == stimulus["before"]["sha256"], "existing target untouched")
        elif stimulus["scenario"] == "fake-stage":
            check(stimulus["original"]["inode"] != stimulus["fake"]["inode"], "same-content fake was a different object")
        elif stimulus["scenario"] == "stage-swap":
            check([x["inode"] for x in stimulus["before"]] == [x["inode"] for x in reversed(stimulus["after"])],
                  "two staging object path names actually exchanged")
    if (evidence / "receipt.py").exists():
        inspection = writer.inspect_restart(root / "native")
        check(not inspection["readFailures"], "new strict receipt schema readback")
        for parent in (evidence, root / "native/app-data"):
            for file in parent.iterdir():
                if file.suffix in (".json", ".jsonl"):
                    check(not any(marker in file.read_bytes() for marker in (b"SYNTHETIC_QUERY_SECRET", b"SYNTHETIC_RESPONSE_COOKIE_SECRET")), file.name + " private fixture markers absent from runtime evidence")
    return {"result": "PASS_FROZEN_LAB_CONSISTENCY_ONLY", "checks": checks,
            "acquisitions": len(acquisitions), "receipts": len(list((root / "native/app-data").glob("*.receipt.json"))),
            "downloadScenarios": len(scenarios) - 2, "limitations": limitations, "productionAcceptance": False}


if __name__ == "__main__":
    print(json.dumps(verify(sys.argv[1]), indent=2))
