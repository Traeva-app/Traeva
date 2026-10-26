#!/usr/bin/env python3
"""Self-tests for release_privacy_scan.py with made-up files and a made-up
identity (never this Mac's).

    python3 -I scripts/test_release_privacy_scan.py
"""

import contextlib
import importlib.util
import io
import tempfile
import unittest
import zipfile
import zlib
from pathlib import Path

# -I does not put this folder on the import path, so load the script by file.
spec = importlib.util.spec_from_file_location(
    "release_privacy_scan", Path(__file__).with_name("release_privacy_scan.py")
)
privacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(privacy)

# Built from pieces so that scanning this repository does not find them.
ACCOUNT = "pat" + "-q"
IDENTITY = {
    "home path": "/" + "Users/" + ACCOUNT,
    "account name": ACCOUNT,
    "full name": "Pat " + "Quill",
    "git e-mail": "pat.q" + "@" + "example.org",
}
PRIVATE_KEY = "-----BEGIN " + "PRIVATE KEY-----"
TEST_KEY = "sk-" + "test" + "A" * 24
REAL_LOOKING_KEY = "sk-" + "Q7wZ" * 8


def docx(core_xml):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", "<Types/>")
        z.writestr("docProps/core.xml", core_xml)
    return out.getvalue()


def pdf(stream_text):
    body = zlib.compress(stream_text.encode())
    head = b"%PDF-1.4\n1 0 obj\n<< /Length " + str(len(body)).encode() + b" /Filter /FlateDecode >>\nstream\n"
    return head + body + b"\nendstream\nendobj\n%%EOF\n"


def run(files):
    """Writes `files` into a temporary folder, scans it, returns (exit
    code, found, printed text)."""
    with tempfile.TemporaryDirectory() as folder:
        for name, data in files.items():
            path = Path(folder, name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data if isinstance(data, bytes) else data.encode())
        result = privacy.scan(folder, IDENTITY)
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            code = result.report()
    found = {category: sum(paths.values()) for (_, category), paths in result.found.items()}
    return code, found, printed.getvalue()


class ReleasePrivacyScanTests(unittest.TestCase):
    def test_a_clean_folder_passes(self):
        code, found, printed = run({"README.md": "Traeva source", "src/main.rs": "fn main() {}"})
        self.assertEqual((code, found), (0, {}))
        self.assertIn("result: PASS", printed)

    def test_identity_in_text_is_found_in_every_form(self):
        code, found, _ = run({
            "a.json": '{"path": "' + IDENTITY["home path"].replace("/", "\\/") + '\\/x"}',
            "b.txt": ("by " + IDENTITY["full name"]).encode("utf-16-le"),
            "c.md": "mail " + IDENTITY["git e-mail"].upper(),
        })
        self.assertEqual(code, 1)
        self.assertEqual(found["home path"], 1)
        self.assertEqual(found["full name"], 1)
        self.assertEqual(found["git e-mail"], 1)

    def test_identity_inside_an_office_part_and_a_pdf_stream(self):
        code, found, _ = run({
            "w.docx": docx("<dc:creator>" + ACCOUNT + "</dc:creator>"),
            "p.pdf": pdf("/Author (" + IDENTITY["home path"] + "/notes)"),
        })
        self.assertEqual(code, 1)
        self.assertEqual(found["account name"], 2)   # the docx part, and inside the home path
        self.assertEqual(found["home path"], 1)
        self.assertEqual(found["Office author, company or manager field is set"], 1)

    def test_keys(self):
        code, found, _ = run({"k.txt": PRIVATE_KEY, "t.rs": TEST_KEY})
        self.assertEqual(code, 1)
        self.assertEqual(found, {"private-key block": 1, "test key": 1})
        code, found, _ = run({"t.rs": TEST_KEY})
        self.assertEqual(code, 0)
        code, found, _ = run({"r.ts": REAL_LOOKING_KEY})
        self.assertEqual((code, found), (1, {"key-like string that is not a test value": 1}))

    def test_other_accounts_paths_are_listed_not_failed(self):
        code, found, _ = run({
            "a.ts": "/" + "Users/someone/x and /" + "home/user/y and /" + "Users/x/z",
            "b.ts": "/" + "Users/jordan2/notes",
        })
        self.assertEqual(code, 0)
        self.assertEqual(found["other account's home path (made-up)"], 3)
        self.assertEqual(found["other account's home path (check by hand)"], 1)

    def test_a_zip_is_scanned_like_the_folder(self):
        files = {"Traeva/node_modules/a.js": "x", "Traeva/.env": "KEY=1", "Traeva/ok.txt": "fine"}
        folder_code, folder_found, _ = run(files)
        with tempfile.TemporaryDirectory() as folder:
            archive = Path(folder, "Traeva-source.zip")
            with zipfile.ZipFile(archive, "w") as z:
                for name, text in files.items():
                    z.writestr(name, text)
            result = privacy.scan(archive, IDENTITY)
            with contextlib.redirect_stdout(io.StringIO()):
                zip_code = result.report()
        zip_found = {c: sum(p.values()) for (_, c), p in result.found.items()}
        self.assertEqual((folder_code, zip_code), (1, 1))
        self.assertEqual(folder_found, {"does not belong in a source archive": 2})
        self.assertEqual(zip_found, folder_found)

    def test_the_report_never_prints_the_identity(self):
        _, _, printed = run({
            ACCOUNT + "-notes/w.docx": docx("<cp:lastModifiedBy>" + IDENTITY["full name"] + "</cp:lastModifiedBy>"),
            "c.md": IDENTITY["git e-mail"] + " " + IDENTITY["home path"],
        })
        for value in IDENTITY.values():
            self.assertNotIn(value.lower(), printed.lower())
        self.assertIn("<account name>-notes/w.docx", printed)


if __name__ == "__main__":
    unittest.main()
