#!/usr/bin/env python3
"""Sets up the second CAC demo folder (#37): the folder tree, and every file
in files/ copied byte for byte to the same place and checked against
SHA256SUMS.txt before and after copying. Refuses to touch a folder that
already exists. Synthetic files only.

    python3 demo/cac2/setup.py [folder]     (default: ~/Traeva-CAC-Demo-2)
"""
import hashlib
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FOLDERS = [
    "Inbox",
    "Downloads",
    "Notes",
    "Writing",
    "School/Biology",
    "School/Economics",
    "School/History",
    "School/Computer Science",
    "Clubs/Debate",
    "Clubs/Robotics",
    "Personal/Travel",
    "Personal/Recipes",
]


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def main():
    target = os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else "~/Traeva-CAC-Demo-2")
    if os.path.exists(target):
        sys.exit(f"{target} already exists; not touching it")
    expected = {}
    with open(os.path.join(HERE, "SHA256SUMS.txt"), encoding="utf-8") as sums:
        for line in sums:
            if line.strip():
                digest, name = line.rstrip("\n").split("  ", 1)
                expected[name] = digest
    files = {}
    for name, digest in expected.items():
        with open(os.path.join(HERE, "files", name), "rb") as f:
            data = f.read()
        if sha256(data) != digest:
            sys.exit(f"files/{name} does not match SHA256SUMS.txt; nothing was created")
        files[name] = data
    for folder in FOLDERS:
        os.makedirs(os.path.join(target, folder))
    for name, data in files.items():
        path = os.path.join(target, name)
        with open(path, "xb") as f:
            f.write(data)
        with open(path, "rb") as f:
            if sha256(f.read()) != expected[name]:
                sys.exit(f"{name} was not copied exactly")
        print(f"{name}  {len(data)} bytes  sha256 {expected[name]}")
    print(f"ready: {target}  ({len(files)} files)")


if __name__ == "__main__":
    main()
