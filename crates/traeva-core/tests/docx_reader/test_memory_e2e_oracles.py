"""False-positive regressions for the test driver, without running a Reader."""
import copy
import unittest
from run_memory_e2e import candidate_identity, fault_matches, pairs_after_interruption, preview_map, previews_match


def version(sid, fid, package, text):
    return {"snapshot": {"snapshot_id": sid, "file_id": fid, "sha256": package},
            "preview": {"text_sha256": text, "summary": {"status": "complete", "chars": 3}},
            "verified_bytes_sha256": package}


class Oracles(unittest.TestCase):
    def setUp(self):
        self.before = [version("v1", "f1", "pkg1", "text1"), version("v2", "f2", "pkg2", "text2")]
        self.snapshots = [p["snapshot"] for p in self.before]
        self.hashes = {"pkg1": "text1", "pkg2": "text2"}

    def matches(self, previews):
        return previews_match(previews, self.snapshots, self.hashes, self.hashes)

    def test_known_hash_does_not_hide_changed_file_or_snapshot_identity(self):
        self.assertTrue(self.matches(self.before))
        for field in ("snapshot_id", "file_id"):
            with self.subTest(field=field):
                changed = copy.deepcopy(self.before)
                changed[0]["snapshot"][field] = "recreated"
                self.assertFalse(self.matches(changed))

    def test_known_hashes_do_not_hide_a_duplicate_or_missing_package(self):
        repeated = [copy.deepcopy(self.before[0]), copy.deepcopy(self.before[0])]
        repeated[1]["snapshot"]["snapshot_id"] = "v2"
        self.assertFalse(self.matches(repeated))
        with self.assertRaises(ValueError):
            preview_map([self.before[0], self.before[0]])

    def test_pending_identity_is_frozen_before_restart(self):
        candidate = {"candidate_id": "c1", "file_id": "f2", "snapshots": [self.snapshots[1]], "continue_from": self.snapshots[0]}
        saved = candidate_identity(candidate)
        candidate["snapshots"][0]["snapshot_id"] = "recreated"
        self.assertNotEqual(candidate_identity(candidate), saved)
        candidate["snapshots"][0]["snapshot_id"] = "v2"
        candidate["candidate_id"] = "c2"
        self.assertNotEqual(candidate_identity(candidate), saved)
        self.assertEqual(saved["snapshots"][0]["snapshot_id"], "v2")

    def test_fault_requires_exact_victim_and_unchanged_other_preview(self):
        damaged = copy.deepcopy(self.before)
        damaged[1] = {"snapshot": self.snapshots[1], "error": {"code": "snapshot_corrupt", "stage": "memory"}}
        self.assertTrue(fault_matches(damaged, self.before, "v2", "snapshot_corrupt"))
        self.assertFalse(fault_matches(damaged, self.before, "v1", "snapshot_corrupt"))
        damaged[0]["preview"]["summary"]["chars"] = 99
        self.assertFalse(fault_matches(damaged, self.before, "v2", "snapshot_corrupt"))

    def test_pair_interruption_preserves_identity_and_only_adds_watcher_gap(self):
        before = [{"before": self.snapshots[0], "after": self.snapshots[1], "edge_id": "edge", "evidence": "user", "gap": True, "gap_kinds": ["replaced_uncaptured"]}]
        actual = copy.deepcopy(before)
        actual[0]["gap_kinds"].append("watcher_stopped")
        self.assertTrue(pairs_after_interruption(actual, before))
        for change in ("edge", "gap", "evidence"):
            with self.subTest(change=change):
                bad = copy.deepcopy(actual)
                if change == "edge": bad[0]["edge_id"] = "recreated"
                elif change == "gap": bad[0]["gap_kinds"] = ["watcher_stopped"]
                else: bad[0]["evidence"] = "system"
                self.assertFalse(pairs_after_interruption(bad, before))


if __name__ == "__main__":
    unittest.main()
