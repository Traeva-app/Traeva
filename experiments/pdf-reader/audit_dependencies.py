#!/usr/bin/env python3
"""Dependency audit for the #28 PDF Reader experiment.

    python3 audit_dependencies.py                      # licenses only (offline)
    python3 audit_dependencies.py --rustsec DIR        # also a local RustSec advisory-db clone
    python3 audit_dependencies.py --osv                # also ask OSV for known advisories

Licenses come from `cargo metadata --locked` for every package the library
and probe use at run or build time. `--osv` sends only public crate names
and versions (from Cargo.lock) to https://api.osv.dev; nothing else leaves
the machine. Writes evidence/dependencies.json.
"""

import json
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
PERMISSIVE = {"MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "Zlib", "Unicode-3.0", "0BSD",
              "Unlicense", "ISC", "CC0-1.0", "BSL-1.0"}


def runtime_packages():
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=HERE, capture_output=True, text=True, check=True).stdout)
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    seen, stack = set(), [metadata["resolve"]["root"]]
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    packages = []
    for package in metadata["packages"]:
        if package["id"] in seen and package["source"]:
            license_text = package["license"] or "(none)"
            names = set(license_text.replace("(", " ").replace(")", " ").replace("/", " OR ")
                        .replace(" AND ", " OR ").split(" OR "))
            names = {n.strip() for n in names if n.strip()}
            packages.append({
                "name": package["name"],
                "version": package["version"],
                "license": license_text,
                "all_permissive": names <= PERMISSIVE,
                "repository": package.get("repository"),
                "build_script": any("custom-build" in t["kind"] for t in package["targets"]),
            })
    return sorted(packages, key=lambda p: (p["name"], p["version"]))


def osv(packages):
    queries = [{"package": {"name": p["name"], "ecosystem": "crates.io"}, "version": p["version"]}
               for p in packages]
    request = urllib.request.Request(
        "https://api.osv.dev/v1/querybatch",
        data=json.dumps({"queries": queries}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        results = json.loads(response.read())["results"]
    return {f"{p['name']} {p['version']}": [v["id"] for v in r.get("vulns", [])]
            for p, r in zip(packages, results)}


def parse_version(text):
    parts = re.match(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?", text.strip())
    return tuple(int(p or 0) for p in parts.groups()), sum(1 for p in parts.groups() if p is not None)


def matches(version, requirement):
    """Cargo-style requirement ("^1.2", ">= 0.4.1, < 0.5", "=1.0.3")."""
    v = parse_version(version)[0]
    for comparator in requirement.split(","):
        comparator = comparator.strip()
        op = re.match(r"(>=|<=|>|<|=|\^|~)?", comparator).group(1) or "^"
        bound, given = parse_version(comparator[len(op):] if comparator.startswith(op) else comparator)
        if op == ">=" and not v >= bound or op == ">" and not v > bound:
            return False
        if op == "<=" and not v <= bound or op == "<" and not v < bound:
            return False
        if op == "=" and v[:given] != bound[:given]:
            return False
        if op in ("^", "~"):
            if v < bound:
                return False
            if op == "~":
                upper = (bound[0] + 1, 0, 0) if given == 1 else (bound[0], bound[1] + 1, 0)
            elif bound[0] > 0 or given == 1:
                upper = (bound[0] + 1, 0, 0)
            elif bound[1] > 0 or given == 2:
                upper = (0, bound[1] + 1, 0)
            else:
                upper = (0, 0, bound[2] + 1)
            if not v < upper:
                return False
    return True


def rustsec(packages, root):
    """Advisories in a local advisory-db clone that cover our exact versions."""
    import tomllib
    hits, informational = {}, {}
    for package in packages:
        folder = Path(root) / "crates" / package["name"]
        for advisory in sorted(folder.glob("*.md")) if folder.is_dir() else []:
            text = advisory.read_text()
            block = re.search(r"```toml\n(.*?)\n```", text, re.S)
            data = tomllib.loads(block.group(1))
            info = data.get("advisory", {})
            if info.get("withdrawn"):
                continue
            versions = data.get("versions", {})
            safe = versions.get("patched", []) + versions.get("unaffected", [])
            if any(matches(package["version"], r) for r in safe):
                continue
            key = f"{package['name']} {package['version']}"
            target = informational if info.get("informational") else hits
            target.setdefault(key, []).append(info.get("id"))
    return hits, informational


def main():
    packages = runtime_packages()
    record = {
        "checked_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "scope": "packages used at run or build time by the library and pdf-probe (Cargo.lock)",
        "packages": packages,
        "not_permissive": [f"{p['name']} {p['version']}: {p['license']}" for p in packages
                           if not p["all_permissive"]],
    }
    if "--rustsec" in sys.argv:
        root = sys.argv[sys.argv.index("--rustsec") + 1]
        commit = subprocess.run(["git", "-C", root, "log", "-1", "--format=%H %cI"],
                                capture_output=True, text=True).stdout.strip()
        hits, informational = rustsec(packages, root)
        record["rustsec"] = {"advisory_db": commit, "affecting": hits, "informational": informational}
    if "--osv" in sys.argv:
        found = osv(packages)
        record["osv"] = {"queried": len(found), "with_advisories": {k: v for k, v in found.items() if v}}
    (HERE / "evidence" / "dependencies.json").write_text(json.dumps(record, indent=1) + "\n")
    print(json.dumps({k: v for k, v in record.items() if k != "packages"}, indent=1))


if __name__ == "__main__":
    main()
