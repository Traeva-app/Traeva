#!/usr/bin/env python3
"""Synthetic checks for sanitize_evidence.py; no real report, no account."""
import json
import unittest

import sanitize_evidence as privacy

FIXTURE_USER = "fixture-owner"
FIXTURE_HOME = "/" + "Users" + "/" + FIXTURE_USER
SCRATCH = "/private/tmp/claude-501/-" + "Users" + "-" + FIXTURE_USER + "-Developer-Traeva/s1"


def report():
    return json.dumps(
        {
            "commit": "abc1234",
            "fixtures_generated": {
                "command": [SCRATCH + "/venv/bin/python", "fixtures/generate.py"],
                "tail": ["wrote 12 files", FIXTURE_HOME + "/Developer/Traeva-G4/x.pdf"],
            },
            "checks": [
                {"name": "fmt", "status": "FAIL", "seconds": 0.25},
                {"name": "tests", "status": "PASS", "passed": 123, "failed": 0},
            ],
            "totals": {"pass": 123, "fail": 1},
        },
        indent=2,
    )


class SanitizeTests(unittest.TestCase):
    def test_only_the_two_forms_change(self):
        original = report()
        text, homes, names = privacy.sanitize(original, FIXTURE_HOME, FIXTURE_USER)
        self.assertEqual((homes, names), (1, 1))
        expected = original.replace(FIXTURE_HOME + "/", "<HOME>/").replace(
            "-Users-" + FIXTURE_USER + "-", "-Users-<user>-"
        )
        self.assertEqual(text, expected)
        self.assertNotIn(FIXTURE_USER, text)
        before, after = json.loads(original), json.loads(text)
        self.assertEqual(before["checks"], after["checks"])
        self.assertEqual(before["totals"], after["totals"])

    def test_a_longer_name_is_not_cut(self):
        text = FIXTURE_HOME + "2/notes and " + FIXTURE_HOME + "/notes"
        with self.assertRaises(privacy.Refused):
            privacy.sanitize(text, FIXTURE_HOME, FIXTURE_USER)

    def test_the_name_elsewhere_refuses(self):
        for text in (
            "owner: " + FIXTURE_USER,
            "owner: " + FIXTURE_USER.upper(),
            "/" + "Volumes/" + FIXTURE_USER + "/x",
        ):
            with self.assertRaises(privacy.Refused):
                privacy.sanitize(text, FIXTURE_HOME, FIXTURE_USER)

    def test_already_sanitized_text_refuses(self):
        text, _, _ = privacy.sanitize(report(), FIXTURE_HOME, FIXTURE_USER)
        with self.assertRaises(privacy.Refused):
            privacy.sanitize(text, FIXTURE_HOME, FIXTURE_USER)


class RestoreTests(unittest.TestCase):
    def test_restore_proves_nothing_else_changed(self):
        original = report()
        text, homes, names = privacy.sanitize(original, FIXTURE_HOME, FIXTURE_USER)
        self.assertEqual(privacy.restore(text, original), (homes, names, 0))

    def test_any_other_change_is_found(self):
        original = report()
        text, _, _ = privacy.sanitize(original, FIXTURE_HOME, FIXTURE_USER)
        for changed in (
            text.replace('"FAIL"', '"PASS"'),
            text.replace("0.25", "0.26"),
            text.replace("123", "124", 1),
            text + "\n",
            text[:-1],
        ):
            with self.assertRaises(privacy.Refused):
                privacy.restore(changed, original)

    def test_tokens_must_stand_for_one_account(self):
        original = FIXTURE_HOME + "/a " + "/" + "Users/other/b"
        with self.assertRaises(privacy.Refused):
            privacy.restore("<HOME>/a <HOME>/b", original)
        original = FIXTURE_HOME + "/a -Users-other-b"
        with self.assertRaises(privacy.Refused):
            privacy.restore("<HOME>/a -Users-<user>-b", original)

    def test_a_name_left_beside_a_token_is_counted(self):
        original = FIXTURE_HOME + "/a " + FIXTURE_USER
        _, _, remaining = privacy.restore("<HOME>/a " + FIXTURE_USER, original)
        self.assertEqual(remaining, 1)


if __name__ == "__main__":
    unittest.main()
