#!/usr/bin/env python3
"""Synthetic checks for recursive evidence redaction, without lab execution."""
import io
import json
import struct
import unittest
import zipfile

import sanitize_evidence as privacy


FIXTURE_HOME = "/" + "Users" + "/" + "fixture-owner"


def archive(entries, comment=b""):
    target = io.BytesIO()
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as output:
        output.comment = comment
        for name, data in entries:
            output.writestr(name, data)
    return target.getvalue()


class EvidencePrivacyTests(unittest.TestCase):
    def rewrite(self, source):
        report = privacy.new_report()
        result = privacy.rewrite(source, "synthetic.zip", report)
        self.assertEqual(privacy.scan(result), 0)
        return result, report

    def test_utf8_utf16_both_alignments_and_url_encoding(self):
        samples = [
            (FIXTURE_HOME + "/evidence").encode(),
            (FIXTURE_HOME + "/evidence").replace("/", "\\/").encode(),
            (FIXTURE_HOME + "/evidence").replace("/", "%2F").encode(),
            (FIXTURE_HOME + "/evidence").replace("/", "%2f").encode(),
        ]
        for codec in ("utf-16-le", "utf-16-be"):
            samples.extend((prefix + (FIXTURE_HOME + "/evidence").encode(codec)) for prefix in (b"", b"x"))
        for sample in samples:
            with self.subTest(length=len(sample)):
                rewritten, count = privacy.redact(sample)
                self.assertEqual(count, 1)
                self.assertEqual(privacy.redact(rewritten), (rewritten, 0))

    def test_archive_member_comments_names_and_extra_fields(self):
        item = zipfile.ZipInfo(FIXTURE_HOME + "/evidence.json")
        item.comment = (FIXTURE_HOME + "/member-comment").encode()
        extra = (FIXTURE_HOME + "/extra-field").encode()
        item.extra = struct.pack("<HH", 0xCAFE, len(extra)) + extra
        source = archive([(item, b'{"result":"FAIL"}')], (FIXTURE_HOME + "/archive-comment").encode())
        rewritten, report = self.rewrite(source)
        self.assertEqual(report["found"], 4)
        with zipfile.ZipFile(io.BytesIO(rewritten)) as output:
            info = output.infolist()[0]
            self.assertEqual(info.filename, "<HOME>/evidence.json")
            self.assertEqual(output.comment, b"<HOME>/archive-comment")
            self.assertEqual(info.comment, b"<HOME>/member-comment")
            self.assertEqual(struct.unpack_from("<HH", info.extra)[1], len(b"<HOME>/extra-field"))
            self.assertEqual(privacy.local_fields(rewritten, info)[1], info.extra)

    def test_nested_failure_evidence_and_binary_payload_are_preserved(self):
        original = json.dumps({"result": "FAIL_RETAINED_wrong_object", "path": FIXTURE_HOME + "/fixture"}).encode()
        nested = archive([("original.json", original), ("opaque.bin", bytes(range(256)))])
        source = archive([("evidence/nested.zip", nested), ("other.json", b'{"result":"PASS"}')])
        rewritten, report = self.rewrite(source)
        self.assertEqual(report["found"], 1)
        self.assertEqual(report["leaf_files"], 3)
        self.assertEqual(report["unchanged_leaf_files"], 2)
        self.assertEqual(len(report["archives"]), 2)
        with zipfile.ZipFile(io.BytesIO(rewritten)) as outer:
            with zipfile.ZipFile(io.BytesIO(outer.read("evidence/nested.zip"))) as inner:
                actual = json.loads(inner.read("original.json"))
                self.assertEqual(actual["result"], "FAIL_RETAINED_wrong_object")
                self.assertEqual(actual["path"], "<HOME>/fixture")
                self.assertEqual(inner.read("opaque.bin"), bytes(range(256)))
        twice, _ = self.rewrite(rewritten)
        self.assertEqual(twice, rewritten)

    def test_raw_trailing_bytes_are_scanned(self):
        source = archive([("safe.txt", b"safe")]) + (FIXTURE_HOME + "/trailer").encode()
        self.assertGreater(privacy.scan(source), 0)
        with self.assertRaisesRegex(ValueError, "unparsed_archive_trailer"):
            self.rewrite(source)

    def test_duplicate_redacted_name_is_rejected(self):
        source = archive([(FIXTURE_HOME + "/same.txt", b"first"), ("<HOME>/same.txt", b"second")])
        with self.assertRaisesRegex(ValueError, "name_collision"):
            self.rewrite(source)

    def test_unreadable_nested_zip_is_not_reported_clean(self):
        source = archive([("nested.zip", b"PK\x03\x04broken")])
        with self.assertRaisesRegex(ValueError, "unreadable_nested_zip"):
            privacy.scan(source)
        with self.assertRaisesRegex(ValueError, "unreadable_nested_zip"):
            self.rewrite(source)

    def test_malformed_extra_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "invalid_extra_field_size"):
            privacy.redact_extra(struct.pack("<HH", 0xCAFE, 20) + b"short")


if __name__ == "__main__":
    result = unittest.TextTestRunner(stream=io.StringIO()).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(EvidencePrivacyTests)
    )
    print(json.dumps({"tests": result.testsRun, "failures": len(result.failures), "errors": len(result.errors)}))
    raise SystemExit(not result.wasSuccessful())
