#!/usr/bin/env python3
"""Removes the local account from the committed run reports (#37, R3's
privacy scan): the home-directory prefix becomes <HOME>, and the account
name inside the scratch-directory path (`-Users-NAME-`) becomes <user>.
Nothing else changes. The identifiers come from the running account and
are never printed; only counts and SHA-256 values are.

    python3 sanitize_evidence.py rewrite --baseline-ref REF
    python3 sanitize_evidence.py verify --baseline-ref REF
    python3 sanitize_evidence.py scan

`rewrite` refuses when a report already differs from REF, already holds a
token, or still holds the account name anywhere else after the two
replacements (it writes nothing then). `verify` needs no identifiers: it
rebuilds each REF report from the committed copy by putting one fixed
home path and one fixed name back where the tokens are, and checks that
the result is byte-for-byte the REF report.
"""
import argparse
import getpass
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
VALIDATION = HERE / "evidence-privacy-validation.json"
REPORTS = [
    "experiments/pdf-reader/evidence/runs/run-15-368aa5e.json",
    "experiments/pdf-reader/evidence/runs/run-16-dba679a.json",
    "experiments/pdf-reader/evidence/runs/run-17-dd70a72.json",
    "experiments/pdf-reader/evidence/runs/run-18-c10d23a.json",
    "experiments/pdf-reader/evidence/runs/run-19-17a1616.json",
    "experiments/pdf-reader/evidence/runs/run-20-0c72d05.json",
    "experiments/pdf-reader/evidence/runs/run-21-4b3d28f.json",
]
HOME_TOKEN = "<HOME>"
USER_TOKEN = "<user>"
# A name or path segment ends at a character that cannot be part of it.
NAME = r"[A-Za-z0-9._-]+"
SEGMENT_END = r"(?![A-Za-z0-9._-])"
# Home paths a scan does not count: the cloud container's generic account
# in the earlier runs.
GENERIC_HOMES = {"/home/user"}
HOME_SHAPE = re.compile(r"/(?:Users|home)/" + NAME)


class Refused(Exception):
    pass


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def git_blob(data):
    """The id git gives these bytes as a blob (`git hash-object`)."""
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def sanitize(text, home, user):
    """Returns (text, home paths replaced, names replaced in the scratch
    path). Raises Refused if the name is left anywhere else."""
    if not home.endswith("/" + user):
        raise Refused("the home directory does not end with the account name")
    if HOME_TOKEN in text or USER_TOKEN in text:
        raise Refused("already holds a token")
    text, homes = re.subn(re.escape(home) + SEGMENT_END, HOME_TOKEN, text)
    text, names = re.subn(
        r"(?<=-Users-)" + re.escape(user) + r"(?=-)", USER_TOKEN, text
    )
    left = len(re.findall(re.escape(user), text, re.I))
    if left:
        raise Refused(f"{left} other occurrence(s) of the account name")
    return text, homes, names


def restore(sanitized, original):
    """Checks without identifiers that `sanitized` is `original` with one
    home path replaced by <HOME> and one name by <user>, and nothing else.
    Returns (home paths, names, remaining name occurrences)."""
    pieces = re.split("(" + re.escape(HOME_TOKEN) + "|" + re.escape(USER_TOKEN) + ")", sanitized)
    position, found = 0, {HOME_TOKEN: set(), USER_TOKEN: set()}
    counts = {HOME_TOKEN: 0, USER_TOKEN: 0}
    for index, piece in enumerate(pieces):
        if piece in found:
            following = pieces[index + 1] if index + 1 < len(pieces) else ""
            if not following and index + 1 < len(pieces) - 1:
                raise Refused("two tokens next to each other")
            end = original.find(following, position) if following else len(original)
            value = original[position:end]
            shape = HOME_SHAPE.pattern if piece == HOME_TOKEN else NAME
            if end < 0 or not re.fullmatch(shape, value):
                raise Refused("a token does not stand for a home path or a name")
            found[piece].add(value)
            counts[piece] += 1
            position = end
        else:
            if not original.startswith(piece, position):
                raise Refused("a byte outside the tokens changed")
            position += len(piece)
    if position != len(original):
        raise Refused("the original is longer")
    if len(found[HOME_TOKEN]) > 1 or len(found[USER_TOKEN]) > 1:
        raise Refused("more than one home path or name behind the tokens")
    names = found[USER_TOKEN] | {h.rsplit("/", 1)[1] for h in found[HOME_TOKEN]}
    if len(names) > 1:
        raise Refused("the home path and the name do not belong together")
    remaining = sum(len(re.findall(re.escape(n), sanitized, re.I)) for n in names)
    return counts[HOME_TOKEN], counts[USER_TOKEN], remaining


def baseline(ref, path):
    return subprocess.run(
        ["git", "-C", str(REPO), "show", f"{ref}:{path}"],
        check=True,
        capture_output=True,
    ).stdout


def rewrite(ref):
    home, user = str(Path.home()), getpass.getuser()
    records, results = [], []
    for path in REPORTS:
        current = (REPO / path).read_bytes()
        original = baseline(ref, path)
        if current != original:
            raise Refused(f"{path} differs from {ref}; nothing was written")
        text, homes, names = sanitize(original.decode("utf-8"), home, user)
        data = text.encode("utf-8")
        json.loads(data)
        results.append((path, data))
        records.append(
            {
                "path": path,
                "original_sha256": sha256(original),
                "sanitized_sha256": sha256(data),
                "original_git_blob": git_blob(original),
                "sanitized_git_blob": git_blob(data),
                "home_paths_replaced": homes,
                "names_replaced_outside_home_paths": names,
                "account_name_found": homes + names,
                "account_name_remaining": 0,
            }
        )
    for path, data in results:
        (REPO / path).write_bytes(data)
    VALIDATION.write_text(
        json.dumps(
            {
                "baseline_ref": ref,
                "tokens": {"home": HOME_TOKEN, "account_name": USER_TOKEN},
                "note": "The committed reports are sanitized copies; every other byte is unchanged.",
                "reports": records,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return records


def verify(ref):
    recorded = json.loads(VALIDATION.read_text(encoding="utf-8"))
    if [r["path"] for r in recorded["reports"]] != REPORTS:
        raise Refused("the validation record lists other reports")
    totals = [0, 0, 0]
    for record in recorded["reports"]:
        original = baseline(ref, record["path"])
        current = (REPO / record["path"]).read_bytes()
        if (sha256(original), git_blob(original)) != (
            record["original_sha256"],
            record["original_git_blob"],
        ):
            raise Refused(f"{record['path']}: the baseline is not the recorded original")
        if (sha256(current), git_blob(current)) != (
            record["sanitized_sha256"],
            record["sanitized_git_blob"],
        ):
            raise Refused(f"{record['path']}: the committed copy is not the recorded one")
        homes, names, remaining = restore(current.decode("utf-8"), original.decode("utf-8"))
        if (homes, names) != (record["home_paths_replaced"], record["names_replaced_outside_home_paths"]):
            raise Refused(f"{record['path']}: the counts differ from the record")
        if remaining:
            raise Refused(f"{record['path']}: the name is still there")
        if original.count(b"FAIL") != current.count(b"FAIL"):
            raise Refused(f"{record['path']}: a FAIL marker changed")
        json.loads(current)
        totals[0] += homes
        totals[1] += names
        totals[2] += original.count(b"FAIL")
    return totals


def scan():
    """Counts what is left under experiments/pdf-reader/: the account name
    (any case), its home path, and home paths of any other account."""
    home, user = str(Path.home()), getpass.getuser()
    counts = {"account_name": 0, "home_path": 0, "other_home_paths": 0}
    for path in sorted(HERE.rglob("*")):
        if not path.is_file() or "target" in path.relative_to(HERE).parts:
            continue
        text = path.read_bytes().decode("utf-8", errors="replace")
        counts["account_name"] += len(re.findall(re.escape(user), text, re.I))
        counts["home_path"] += text.count(home)
        counts["other_home_paths"] += sum(
            1 for m in HOME_SHAPE.findall(text) if m not in GENERIC_HOMES and m != home
        )
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("rewrite", "verify"):
        command = commands.add_parser(name)
        command.add_argument("--baseline-ref", required=True)
    commands.add_parser("scan")
    args = parser.parse_args()
    try:
        if args.command == "rewrite":
            records = rewrite(args.baseline_ref)
            found = sum(r["account_name_found"] for r in records)
            homes = sum(r["home_paths_replaced"] for r in records)
            print(f"account name: found {found} / replaced {found} / remaining 0")
            print(f"home paths: found {homes} / replaced {homes} / remaining 0")
        elif args.command == "verify":
            homes, names, fails = verify(args.baseline_ref)
            print(f"verify OK: {len(REPORTS)} reports; home paths {homes}, names outside home paths {names};")
            print(f"every other byte equals {args.baseline_ref}; {fails} FAIL markers kept")
        else:
            counts = scan()
            print(" ".join(f"{k}={v}" for k, v in counts.items()))
            if any(counts.values()):
                return 1
    except Refused as refused:
        print(f"refused: {refused}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
