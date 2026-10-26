import copy
import errno
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

import writer
import test_writer
from test_writer import begin


class ReviewFixes(unittest.TestCase):
    setUp = test_writer.Tests.setUp
    tearDown = test_writer.Tests.tearDown
    finish = test_writer.Tests.finish
    receipt = test_writer.Tests.receipt

    def recorded(self):
        task = begin(self.lab, b"synthetic bytes")
        self.finish(task)
        return task, json.loads(self.receipt(task).read_text())

    def assert_rejected(self, path):
        result = writer.inspect_restart(self.run)
        self.assertEqual(result["state"], "READ_FAILED_UNCERTAIN")
        self.assertNotIn(path.name, result["receipts"])
        self.assertFalse(result["liveDescriptorContinuity"])

    def test_every_required_field_missing_or_null_is_rejected(self):
        task, valid = self.recorded()
        path = self.receipt(task)
        for key in sorted(valid):
            # null is deliberately valid only for the explicitly unknown Chrome ID.
            for variant in ("missing", "null"):
                if key == "chromeDownloadId" and variant == "null":
                    continue
                with self.subTest(key=key, variant=variant):
                    bad = copy.deepcopy(valid)
                    if variant == "missing": del bad[key]
                    else: bad[key] = None
                    path.write_text(json.dumps(bad))
                    self.assert_rejected(path)

    def test_types_ranges_identity_and_binding_are_rejected(self):
        task, valid = self.recorded()
        path = self.receipt(task)
        changes = [(key, value) for key in ("expectedBytes", "receivedBytes", "nextSequence", "startedAtNs", "observedAtNs", "receiptVersion")
                   for value in (True, "1", -1, 0, 1 << 64)]
        changes += [("streamSha256", "g" * 64), ("streamSha256", "a" * 63), ("streamSha256", 123),
                    ("sourceNamespace", "CHROME_DOWNLOAD"), ("acquisitionId", "different-id"),
                    ("stagingName", "other.part"), ("destination", "../other"),
                    ("currentPathRequiresRecheck", 1), ("quarantinePresent", 1), ("chromeDownloadId", 9)]
        for key, value in changes:
            with self.subTest(key=key, value=value):
                bad = copy.deepcopy(valid); bad[key] = value
                path.write_text(json.dumps(bad)); self.assert_rejected(path)
        for field in ("device", "inode", "birthTime", "size", "links", "mode", "mtimeNs", "ctimeNs"):
            for obj in ("createdObject", "targetCreatedObject", "finalObject"):
                with self.subTest(obj=obj, missing=field):
                    bad = copy.deepcopy(valid); del bad[obj][field]
                    path.write_text(json.dumps(bad)); self.assert_rejected(path)
        for change in (lambda r: r["finalObject"].update(inode=r["finalObject"]["inode"] + 1),
                       lambda r: r["finalObject"].update(birthTime=True),
                       lambda r: r["binding"].update(tabId=True),
                       lambda r: r["binding"].pop("debuggerEpoch"),
                       lambda r: r["binding"].update(kind="TRAEVA_DIRECT_HTTPS"),
                       lambda r: r["response"].update(status=True),
                       lambda r: r["response"].update(url="https://fixture.invalid/file?token=private"),
                       lambda r: r["response"]["headers"].append({"name": "set-cookie", "value": "private"})):
            bad = copy.deepcopy(valid); change(bad)
            path.write_text(json.dumps(bad)); self.assert_rejected(path)

    def test_restart_reader_rejects_duplicate_fields_symlink_fifo_and_legacy(self):
        task, valid = self.recorded(); path = self.receipt(task)
        path.write_text('{"receiptVersion":1,' + json.dumps(valid)[1:]); self.assert_rejected(path)
        path.write_text(json.dumps({"receiptKind": "HISTORICAL_ACQUISITION_OBSERVATION",
                                   "sourceNamespace": "TRAEVA_CHROME_FETCH_ACQUISITION", "state": "OBSERVATION_RECORDED"}))
        child = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("test_writer.py")), "--inspect", str(self.run)],
                               capture_output=True, check=True, timeout=5)
        self.assertEqual(json.loads(child.stdout)["state"], "READ_FAILED_UNCERTAIN")
        path.unlink(); target = self.run / "outside-receipt.json"; target.write_text(json.dumps(valid))
        path.symlink_to(target); self.assert_rejected(path)
        path.unlink(); os.mkfifo(path); self.assert_rejected(path)

    def test_post_commit_log_write_fsync_and_status_failures_remain_committed(self):
        for fault in ("event", "event_write", "event_fsync", "status"):
            with self.subTest(fault=fault):
                task = begin(self.lab, b"synthetic bytes")
                event, persist = self.lab.event, self.lab.persist
                write, fsync = os.write, os.fsync
                def inject_event(kind, value):
                    if fault == "event" and kind == "acquisition_observation_saved": raise OSError(errno.EIO, "synthetic EIO")
                    return event(kind, value)
                def inject_write(fd, value):
                    if fault == "event_write" and fd == self.lab.events_fd and task.record.get("receiptCommitted"): raise OSError(errno.EIO, "synthetic EIO")
                    return write(fd, value)
                def inject_fsync(fd):
                    if fault == "event_fsync" and fd == self.lab.events_fd and task.record.get("receiptCommitted"): raise OSError(errno.EIO, "synthetic EIO")
                    return fsync(fd)
                def inject_status():
                    if fault == "status" and task.record.get("receiptCommitted"): raise OSError(errno.EIO, "synthetic EIO")
                    return persist()
                self.lab.event, self.lab.persist = inject_event, inject_status
                try:
                    with patch("os.write", inject_write), patch("os.fsync", inject_fsync): result = self.finish(task)
                finally: self.lab.event, self.lab.persist = event, persist
                self.assertTrue(result["ok"]); self.assertTrue(result["receiptCommitted"])
                self.assertTrue(result["warnings"])
                self.assertEqual(task.record["state"], "ACQUIRED_OBSERVED_SAME_OBJECT")
                self.assertTrue(self.lab.recheck({"acquisitionId": task.record["acquisitionId"]})["ok"])
                self.assertTrue(self.finish(task)["idempotentHistoricalResult"])
                self.assertEqual(writer.inspect_restart(self.run)["state"], "HISTORICAL_RECEIPT_ONLY")

    def test_unacknowledged_receipt_commit_is_unknown_not_refusal(self):
        task = begin(self.lab, b"synthetic bytes")
        def fault(phase, path):
            if path.name.endswith(".receipt.json"): raise OSError(errno.EIO, "directory fsync not acknowledged")
        self.lab.persistence_hook = fault
        result = self.finish(task)
        self.assertFalse(result["ok"]); self.assertEqual(result["outcome"], "OUTCOME_UNKNOWN")
        self.assertTrue(result["reconciliationRequired"])
        self.assertTrue(self.receipt(task).exists())
        self.assertEqual(task.record["state"], "OUTCOME_UNKNOWN")
        self.assertEqual(self.finish(task)["outcome"], "OUTCOME_UNKNOWN")
        with self.assertRaises(writer.base.Refusal): self.lab.abort({"acquisitionId": task.record["acquisitionId"]})
        self.lab.refuse(task, ValueError("later_failure"))
        self.assertEqual(task.record["state"], "OUTCOME_UNKNOWN")
        self.assertEqual(writer.inspect_restart(self.run)["currentAssociation"], "UNKNOWN_REQUIRES_NEW_EVIDENCE")

    def test_sensitive_response_headers_and_errors_never_persist(self):
        sentinels = ["SYNTHETIC_COOKIE_SECRET", "SYNTHETIC_AUTH_SECRET", "SYNTHETIC_ERROR_SECRET"]
        task = begin(self.lab, b"synthetic bytes")
        # Native boundary strips these before any task or event is allocated.
        payload = {"acquisitionId": "private-header-test", "destination": "private.txt", "scenario": "privacy",
                   "binding": dict(task.record["binding"]), "response": copy.deepcopy(task.record["response"])}
        payload["response"]["headers"] += [{"name": "Set-Cookie", "value": sentinels[0]}, {"name": "Authorization", "value": sentinels[1]}]
        self.lab.begin(payload)
        private = self.lab.tasks[payload["acquisitionId"]]
        self.lab.refuse(private, OSError(errno.EIO, sentinels[2]))
        self.finish(task)
        for path in self.lab.app_data.iterdir():
            if path.is_file():
                for value in sentinels: self.assertNotIn(value.encode(), path.read_bytes())
        for value in sentinels: self.assertNotIn(value, json.dumps(self.lab.status()))

    def test_query_credentials_and_existing_destination_refused_during_prepare(self):
        for url in ("https://fixture.invalid/file?secret=private", "https://user:password@fixture.invalid/file", "https://fixture.invalid/file#private"):
            with self.assertRaises(writer.base.Refusal):
                self.lab.prepare({"response": {"url": url, "status": 200, "headers": [{"name": "Content-Length", "value": "1"}]}})
        (self.lab.delivery / "exists.txt").write_bytes(b"keep")
        with self.assertRaisesRegex(writer.base.Refusal, "destination_already_exists"):
            self.lab.prepare({"destination": "exists.txt", "response": {"url": "https://fixture.invalid/file", "status": 200,
                                                                       "headers": [{"name": "Content-Length", "value": "1"}]}})
        self.assertFalse(self.lab.tasks)

    def test_native_task_and_log_budgets_refuse_before_extra_side_effects(self):
        for _ in range(4): begin(self.lab, b"synthetic bytes")
        with self.assertRaisesRegex(writer.base.Refusal, "lab_active_task_budget_exceeded"):
            begin(self.lab, b"synthetic bytes")
        self.assertEqual(len(self.lab.tasks), 4)
        self.lab.event_sequence = 16384
        length = os.fstat(self.lab.events_fd).st_size
        with self.assertRaisesRegex(writer.base.Refusal, "lab_event_budget_exceeded"):
            self.lab.event("unused", {})
        self.assertEqual(os.fstat(self.lab.events_fd).st_size, length)


if __name__ == "__main__":
    unittest.main(verbosity=2)
