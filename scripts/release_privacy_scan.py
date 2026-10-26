#!/usr/bin/env python3
"""Privacy scan of the source archive before it is handed in (#37,
docs/development/final-release.md steps 2 and 4).

    python3 -I scripts/release_privacy_scan.py Traeva-source.zip
    python3 -I scripts/release_privacy_scan.py <folder the zip was unpacked into>

It reads this Mac's identity when it runs: the account name, the full name,
the home folder and the git e-mail. It never prints them; it prints counts,
paths inside the archive and categories only.

What it looks in: every file as bytes (UTF-8 and UTF-16, plus the escaped
forms of the home path); inside every zip container (zip, docx, pptx, xlsx,
nested ones too: names, comments and every part); inside every PDF stream
that can be decompressed.

Exit code 1 (FAIL) when it finds:
- this Mac's identity (home path, account name, full name, git e-mail);
- a private-key block, or an API-key-like string that does not look like a
  test value;
- something that does not belong in a source archive (node_modules/, target/,
  .git/, .env, .DS_Store, key or certificate files).
Listed only, not a failure: other accounts' home paths (tests use made-up
ones), test keys, Office author fields that are set, and encrypted PDFs
(their metadata cannot be read).

A zip is scanned both as raw bytes and member by member, so a match in a
member stored without compression is counted twice.
"""

import getpass
import io
import os
import pwd
import re
import struct
import subprocess
import sys
import zipfile
import zlib
from collections import Counter, defaultdict
from pathlib import Path

# Made-up account names that tests and earlier cloud runs use (the cloud
# container's account is "user"). One-character names and "..." are
# placeholders too. A home path with one of these is listed as "made-up";
# any other one as "check by hand". Neither is a failure: only this Mac's
# own identity is.
MADE_UP_ACCOUNTS = {
    "user", "test", "tester", "someone", "me", "example", "alice", "bob",
    "fixture", "fixture-owner", "runner", "ubuntu", "shared", "student", "demo",
}
# A key-like string with one of these words counts as a test value.
TEST_WORDS = re.compile(rb"test|fake|dummy|example|sample|placeholder|redacted|xxxx|0000|1234", re.I)
KEY_PATTERNS = [
    re.compile(rb"sk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}"),
    re.compile(rb"gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}"),
    re.compile(rb"AKIA[0-9A-Z]{16}"),
    re.compile(rb"AIza[0-9A-Za-z_-]{35}"),
    re.compile(rb"xox[abprs]-[A-Za-z0-9-]{10,}"),
]
PRIVATE_KEY = re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----")
OTHER_HOME = re.compile(rb"/(?:Users|home)/([A-Za-z0-9._-]+)")
NOT_IN_SOURCE_DIRS = {"node_modules", "target", ".git"}
NOT_IN_SOURCE_SUFFIXES = {".pem", ".p12", ".pfx", ".key", ".keychain"}
MAX_DEPTH = 4                 # zip inside zip inside ...
MAX_MEMBER = 256 << 20        # larger zip members are noted, not read

FAIL = "FAIL"
LIST = "LIST"
OTHER_FAILURES = [
    "private-key block",
    "key-like string that is not a test value",
    "does not belong in a source archive",
]


def read_identity():
    """This Mac's identity, read now. Never printed."""
    home = str(Path.home())
    account = getpass.getuser()
    full_name = pwd.getpwuid(os.getuid()).pw_gecos.split(",")[0].strip()
    try:
        email = subprocess.run(
            ["git", "config", "--get", "user.email"], capture_output=True, text=True
        ).stdout.strip()
    except OSError:
        email = ""
    return {"home path": home, "account name": account, "full name": full_name, "git e-mail": email}


class Scan:
    def __init__(self, identity):
        self.identity = {k: v for k, v in identity.items() if v}
        self.found = defaultdict(Counter)   # (FAIL/LIST, category) -> path -> count
        self.notes = []
        self.files = 0
        self.patterns = {}
        for category, value in self.identity.items():
            if len(value) < 3:
                self.notes.append(f"{category} is shorter than 3 characters; not scanned")
                continue
            forms = {value}
            if category == "home path":
                forms |= {value.replace("/", "\\/"), value.replace("/", "%2F")}
            encoded = [f.encode(codec) for f in forms for codec in ("utf-8", "utf-16-le", "utf-16-be")]
            self.patterns[category] = re.compile(b"|".join(re.escape(e) for e in encoded), re.I)

    def add(self, kind, category, path, count=1):
        if count:
            self.found[(kind, category)][self.mask(path)] += count

    def mask(self, text):
        """Replaces any identity in a printed path with its category."""
        for category, value in self.identity.items():
            if len(value) >= 3:
                text = re.sub(re.escape(value), f"<{category}>", text, flags=re.I)
        return text

    # --- one file -----------------------------------------------------------
    def scan_file(self, path, data):
        self.files += 1
        parts = path.split("/")
        name = parts[-1]
        if NOT_IN_SOURCE_DIRS & set(parts[:-1]):
            self.add(FAIL, "does not belong in a source archive", path)
        elif name in (".env", ".DS_Store") or name.startswith(".env."):
            self.add(FAIL, "does not belong in a source archive", path)
        elif Path(name).suffix.lower() in NOT_IN_SOURCE_SUFFIXES or name.startswith("id_rsa"):
            self.add(FAIL, "does not belong in a source archive", path)
        self.scan_bytes(path + " <file name>", path.encode(), MAX_DEPTH)
        self.scan_bytes(path, data, depth=0)

    def scan_bytes(self, path, data, depth):
        for category, pattern in self.patterns.items():
            self.add(FAIL, category, path, len(pattern.findall(data)))
        self.add(FAIL, "private-key block", path, len(PRIVATE_KEY.findall(data)))
        for pattern in KEY_PATTERNS:
            for match in pattern.finditer(data):
                if TEST_WORDS.search(match.group(0)):
                    self.add(LIST, "test key", path)
                else:
                    self.add(FAIL, "key-like string that is not a test value", path)
        for match in OTHER_HOME.finditer(data):
            account = match.group(1).decode("ascii").lower()
            if any(account == v.lower() for v in self.identity.values()):
                continue  # this Mac's own account: counted above
            if account in MADE_UP_ACCOUNTS or len(account) == 1 or set(account) == {"."}:
                self.add(LIST, "other account's home path (made-up)", path)
            else:
                self.add(LIST, "other account's home path (check by hand)", path)
        if data[:4] == b"PK\x03\x04" and depth < MAX_DEPTH:
            self.scan_zip(path, data, depth)
        elif data[:5] == b"%PDF-" and depth < MAX_DEPTH:
            self.scan_pdf(path, data, depth)

    # --- containers ---------------------------------------------------------
    def scan_zip(self, path, data, depth):
        try:
            archive = zipfile.ZipFile(io.BytesIO(data))
        except zipfile.BadZipFile:
            self.notes.append(self.mask(f"{path}: not a readable zip; scanned as bytes only"))
            return
        self.scan_bytes(path + "!<zip comment>", archive.comment, MAX_DEPTH)
        for info in archive.infolist():
            member = f"{path}!{info.filename}"
            self.scan_bytes(member + "<name, extra>", info.filename.encode() + info.extra + info.comment, MAX_DEPTH)
            if info.is_dir():
                continue
            if info.file_size > MAX_MEMBER:
                self.notes.append(self.mask(f"{member}: larger than 256 MiB, not read"))
                continue
            try:
                content = archive.read(info)
            except Exception as error:  # a deliberately damaged test file
                content = raw_member(data, info)
                self.notes.append(self.mask(f"{member}: {type(error).__name__}; scanned the raw bytes"))
            if info.filename == "docProps/core.xml" or info.filename == "docProps/app.xml":
                self.list_office_fields(path, content)
            self.scan_bytes(member, content, depth + 1)

    def list_office_fields(self, path, xml):
        for tag in (b"dc:creator", b"cp:lastModifiedBy", b"Company", b"Manager"):
            for value in re.findall(b"<" + tag + rb"(?:\s[^>]*)?>(.*?)</" + tag + b">", xml, re.S):
                if value.strip():
                    self.add(LIST, "Office author, company or manager field is set", path)

    def scan_pdf(self, path, data, depth):
        if b"/Encrypt" in data:
            self.add(LIST, "encrypted PDF (metadata cannot be read)", path)
        for value in re.findall(rb"/(?:Author|Creator|Producer|Title|Subject|Keywords)\s*<([0-9A-Fa-f\s]+)>", data):
            try:
                self.scan_bytes(path + "!<metadata>", bytes.fromhex(value.decode()), MAX_DEPTH)
            except ValueError:
                pass
        for match in re.finditer(rb"stream\r?\n", data):
            end = data.find(b"endstream", match.end())
            if end < 0:
                continue
            try:
                content = zlib.decompressobj().decompress(data[match.end():end], 64 << 20)
            except zlib.error:
                continue
            self.scan_bytes(path + "!<stream>", content, depth + 1)

    # --- report -------------------------------------------------------------
    def report(self):
        failed = False
        print(f"files: {self.files}")
        for category in list(self.patterns) + OTHER_FAILURES:
            if (FAIL, category) not in self.found:
                print(f"OK    {category}: 0")
        for (kind, category), paths in sorted(self.found.items()):
            failed |= kind == FAIL
            print(f"{kind}  {category}: {sum(paths.values())} in {len(paths)} place(s)")
            for path, count in sorted(paths.items()):
                print(f"        {count:4d}  {path}")
        for note in self.notes:
            print(f"note  {note}")
        print("result: " + ("FAIL" if failed else "PASS"))
        return 1 if failed else 0


def raw_member(data, info):
    """A zip member's bytes without the CRC check."""
    offset = info.header_offset
    if data[offset:offset + 4] != b"PK\x03\x04":
        return b""
    name_length, extra_length = struct.unpack("<HH", data[offset + 26:offset + 30])
    start = offset + 30 + name_length + extra_length
    raw = data[start:start + info.compress_size]
    if info.compress_type == zipfile.ZIP_STORED:
        return raw
    if info.compress_type == zipfile.ZIP_DEFLATED:
        try:
            return zlib.decompressobj(-15).decompress(raw, MAX_MEMBER)
        except zlib.error:
            return b""
    return b""


def scan(target, identity):
    """Scans a folder or a zip file. Returns the Scan."""
    result = Scan(identity)
    target = Path(target)
    if target.is_dir():
        for folder, _, names in os.walk(target):
            for name in sorted(names):
                path = Path(folder, name)
                result.scan_file(path.relative_to(target).as_posix(), path.read_bytes())
    else:
        with zipfile.ZipFile(target) as archive:
            result.scan_bytes("<zip comment>", archive.comment, MAX_DEPTH)
            for info in archive.infolist():
                if not info.is_dir():
                    result.scan_file(info.filename, archive.read(info))
    return result


def main():
    if len(sys.argv) != 2:
        print(__doc__.split("\n\n")[1])
        return 2
    return scan(sys.argv[1], read_identity()).report()


if __name__ == "__main__":
    sys.exit(main())
