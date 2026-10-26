#!/usr/bin/env python3
"""Package only lab artifacts; never copy profiles, host secrets or TLS keys."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

import writer


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome", required=True)
    parser.add_argument("--direct", required=True)
    parser.add_argument("--legacy", required=True)
    args = parser.parse_args()
    source = Path(__file__).parent.resolve()
    output = source / "evidence"
    if output.exists():
        raise RuntimeError("evidence_already_frozen_choose_new_destination_or_review_existing")
    output.mkdir()
    tmp = Path("/private/tmp")
    before = set(tmp.glob("traeva-c-verified-*"))
    tests = subprocess.run([sys.executable, "-B", "-m", "unittest", "discover", "-s", str(source),
                            "-p", "test_*.py", "-v"], capture_output=True, timeout=60)
    (output / "unit-tests.txt").write_bytes(tests.stdout + tests.stderr)
    if tests.returncode:
        raise RuntimeError("unit_tests_failed_evidence_retained")
    new_roots = sorted(set(tmp.glob("traeva-c-verified-*")) - before)
    roots = [("chrome-final", Path(args.chrome)), ("direct-final", Path(args.direct))]
    roots.extend(("unit-" + p.name, p) for p in new_roots)
    attributes = []
    for label, root in roots:
        folder = output / label
        folder.mkdir()
        for p in root.rglob("*"):
            relative = p.relative_to(root)
            # Explicit inclusion only; NativeMessaging config and browser profile are excluded.
            allowed = relative.parts[0] == "evidence" or "app-data" in relative.parts or "delivery" in relative.parts or relative.parts[0] == "chrome-downloads"
            if not allowed or not p.is_file() or p.is_symlink():
                continue
            target = folder / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(p, target)
            if "delivery" in relative.parts or relative.parts[0] == "chrome-downloads":
                fd = os.open(p, os.O_RDONLY | os.O_NOFOLLOW)
                entry = {"file": label + "/" + str(relative), "physicalObjectAtPackaging": writer.base.snapshot(fd), "attributes": {}}
                for attr in ("com.apple.quarantine", "com.apple.metadata:kMDItemWhereFroms"):
                    try:
                        entry["attributes"][attr] = writer.attribute(fd, attr).hex()
                    except OSError as error:
                        entry["attributes"][attr] = {"unavailableErrno": error.errno}
                os.close(fd)
                attributes.append(entry)
    checked = subprocess.run([sys.executable, "-B", str(source / "verify_artifacts.py"), args.chrome],
                             capture_output=True, timeout=30)
    (output / "chrome-author-consistency.txt").write_bytes(checked.stdout + checked.stderr)
    if checked.returncode:
        raise RuntimeError("chrome_artifact_checks_failed_evidence_retained")
    (output / "file-attributes.json").write_text(json.dumps(attributes, indent=2))
    (output / "unit-roots.json").write_text(json.dumps([str(p) for p in new_roots], indent=2))
    legacy = Path(args.legacy)
    expected_legacy = "b44db618a174f7c07c11be32a22c5b2702dae5111e539f394cd2bb8a84d555fd"
    if sha(legacy) != expected_legacy:
        raise RuntimeError("prior_failure_archive_changed")
    shutil.copyfile(legacy, output / "legacy-comparison-evidence.zip")
    (output / "retained-failures.json").write_text(json.dumps({
        "legacyArchiveSha256": expected_legacy,
        "legacyWrongObjectDelivery": "FAIL_RETAINED_wrong_source_object_entered_delivery_then_refused",
        "legacyMalformedChunked": "FAIL_RETAINED_missing_final_chunk_delimiter_accepted_with_receipt",
        "firstNewChromeSetup": {"root": "/private/tmp/traeva-c-verified-chrome-bCe4vu", "result": "HARNESS_FAIL_EEXIST", "fixed": "recursive_create_existing_empty_Chrome_owned_manifest_directory"},
        "firstArtifactVerifier": {"root": "/private/tmp/traeva-c-verified-chrome-TLis8Y", "result": "FAIL_same_tab_overlap_assertion", "resolution": "downgraded_claim_to_sequential_same_tab_no_overlap_PASS_claim"},
        "nativePermissionUICapture": {"result": "BLOCKED_CUA_TIMEOUT", "attempts": 2, "claim": "API_attach_observed_not_permission_UI_acceptance"}
    }, indent=2))
    first_error = Path("/private/tmp/traeva-c-verified-chrome-bCe4vu/evidence/error.json")
    if first_error.exists():
        shutil.copyfile(first_error, output / "first-chrome-setup-error.json")
    files = sorted(p for p in output.rglob("*") if p.is_file())
    manifest = {"scope": "author experimental evidence only; no Codex-R review or production VERIFIED",
                "baseCommit": "1c0ff381a0331e270e8f38c3e0e21a56c4ee478e",
                "sourceRevision": {p.name: sha(p) for p in sorted(source.iterdir()) if p.is_file() and p.suffix in (".py", ".js", ".cjs", ".html")},
                "excluded": ["Chrome Profile", "NativeMessaging config/token", "TLS private keys", "personal files"],
                "files": [{"path": str(p.relative_to(output)), "bytes": p.stat().st_size, "sha256": sha(p)} for p in files]}
    (output / "MANIFEST.json").write_text(json.dumps(manifest, indent=2))
    archive = source / "source-verified-evidence.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for p in sorted(output.rglob("*")):
            if p.is_file(): z.write(p, p.relative_to(source))
    (source / "archive-sha256.txt").write_text(sha(archive) + "  " + archive.name + "\n")
    print(json.dumps({"archive": str(archive), "sha256": sha(archive), "bytes": archive.stat().st_size,
                      "fileCount": len(files) + 1, "unitExit": tests.returncode, "artifactCheckExit": checked.returncode}))


if __name__ == "__main__":
    main()
