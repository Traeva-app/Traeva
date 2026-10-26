#!/usr/bin/env python3
"""Sets up the CAC demo folder (#18): the folder tree and the three demo
files in Inbox, copied byte for byte and checked against SHA256SUMS.txt.
Refuses to touch a folder that already exists. Synthetic files only.

    python3 demo/cac/setup.py [folder]     (default: ~/Traeva-CAC-Demo)
"""
import hashlib
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FOLDERS = [
    "School/Chemistry",
    "School/English",
    "School/History",
    "Book Club",
    "Robotics Club",
    "Personal/Health",
    "Inbox",
]


def main():
    target = os.path.expanduser(sys.argv[1] if len(sys.argv) > 1 else "~/Traeva-CAC-Demo")
    if os.path.exists(target):
        sys.exit(f"{target} already exists; not touching it")
    expected = {}
    with open(os.path.join(HERE, "SHA256SUMS.txt")) as sums:
        for line in sums:
            digest, name = line.split()
            expected[name] = digest
    for folder in FOLDERS:
        os.makedirs(os.path.join(target, folder))
    for name, digest in expected.items():
        with open(os.path.join(HERE, name), "rb") as f:
            data = f.read()
        if hashlib.sha256(data).hexdigest() != digest:
            sys.exit(f"{name} does not match SHA256SUMS.txt")
        with open(os.path.join(target, "Inbox", name), "xb") as f:
            f.write(data)
        print(f"Inbox/{name}  {len(data)} bytes  sha256 {digest}")
    print(f"ready: {target}")


if __name__ == "__main__":
    main()
