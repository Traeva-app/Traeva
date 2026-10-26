"""Query OSV for resolved Rust dependency versions; send no workbook or user data."""
import datetime
import json
from pathlib import Path
import subprocess
import urllib.request

HERE = Path(__file__).resolve().parent


def main():
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--offline", "--format-version", "1"], cwd=HERE))
    packages = sorted((p for p in metadata["packages"] if p["source"]), key=lambda p: p["name"])
    queries = [{"package": {"name": p["name"], "ecosystem": "crates.io"}, "version": p["version"]} for p in packages]
    req = urllib.request.Request("https://api.osv.dev/v1/querybatch", data=json.dumps({"queries": queries}).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as response:
        results = json.load(response)["results"]
    assert len(results) == len(packages)
    findings = []
    for p, r in zip(packages, results):
        for v in r.get("vulns", []):
            findings.append({"package": p["name"], "version": p["version"], "id": v["id"]})
    report = {"recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source": "https://api.osv.dev/v1/querybatch", "packages": queries, "response": results,
              "status": "PASS" if not findings else "FAIL", "findings": findings,
              "scope": "known advisory match at query time, not proof of absence of vulnerabilities"}
    (HERE / "evidence").mkdir(exist_ok=True)
    (HERE / "evidence" / "dependency-advisories.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "queried_packages": len(packages), "findings": findings}))
    if findings:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
