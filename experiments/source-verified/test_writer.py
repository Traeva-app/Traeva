#!/usr/bin/env python3
import argparse
import base64
import errno
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
import uuid

import writer

ORIGIN = "https://fixture.invalid"
BODY = (b"synthetic response bytes\n" * 12000)[:262144]


def begin(lab, body=BODY, name=None):
    aid = str(uuid.uuid4())
    lab.begin({"acquisitionId": aid, "destination": name or aid + ".bin", "scenario": "offline-held-object",
               "binding": {"sessionId": None, "networkId": None, "debuggerEpoch": "test-epoch", "tabId": 1, "fetchRequestId": aid, "stream": "specific-stream-" + aid},
               "response": {"url": ORIGIN + "/file", "status": 200,
                            "headers": [{"name": "Content-Length", "value": str(len(body))}]}})
    for sequence, offset in enumerate(range(0, len(body), 32768)):
        lab.chunk({"acquisitionId": aid, "sequence": sequence,
                   "base64": base64.b64encode(body[offset:offset + 32768]).decode()})
    return lab.tasks[aid]


class Tests(unittest.TestCase):
    def setUp(self):
        self.run = Path(tempfile.mkdtemp(prefix="traeva-c-verified-writer-", dir="/private/tmp"))
        self.lab = writer.Writer(self.run, [ORIGIN])
        self.lab.authorize({"takeoverConsent": True, "saveLocationConsent": str(self.lab.delivery)})

    def tearDown(self):
        self.lab.close()

    def finish(self, task):
        return self.lab.complete({"acquisitionId": task.record["acquisitionId"], "eof": True})

    def receipt(self, task):
        return self.lab.app_data / (task.record["acquisitionId"] + ".receipt.json")

    def assert_delivery(self, task, body):
        path = self.lab.delivery / task.record["destination"]
        self.assertEqual(path.read_bytes(), body)
        self.assertEqual(path.stat().st_ino, os.fstat(task.target_fd).st_ino)
        self.assertEqual(path.stat().st_ino, task.record["targetCreatedObject"]["inode"])
        self.assertNotEqual(os.fstat(task.fd).st_ino, path.stat().st_ino)
        self.assertTrue(writer.attribute(task.target_fd, "com.apple.quarantine").startswith(b"0001;"))
        self.assertTrue(self.receipt(task).exists())

    def test_normal_and_idempotent_completion(self):
        task = begin(self.lab)
        self.finish(task)
        self.assert_delivery(task, BODY)
        self.assertTrue(self.finish(task)["idempotentHistoricalResult"])
        self.assertEqual(len(list(self.lab.delivery.iterdir())), 1)
        self.assertEqual(len(list(self.lab.app_data.glob("*.receipt.json"))), 1)

    def test_same_content_stage_replaced_before_delivery(self):
        task = begin(self.lab)
        original_inode = os.fstat(task.fd).st_ino
        def replace(phase, value):
            if phase == "before_target_create":
                path = self.lab.staging / value.record["stagingName"]
                path.rename(path.with_suffix(".original"))
                path.write_bytes(BODY)
                self.assertNotEqual(path.stat().st_ino, original_inode)
        self.lab.hook = replace
        self.finish(task)
        self.assertEqual(os.fstat(task.fd).st_ino, original_inode)
        self.assert_delivery(task, BODY)
        self.assertNotEqual((self.lab.delivery / task.record["destination"]).stat().st_ino,
                            (self.lab.staging / task.record["stagingName"]).stat().st_ino)

    def test_concurrent_path_exchange_different_streams(self):
        bodies = [BODY, b"Z" * len(BODY)]
        tasks = [begin(self.lab, body) for body in bodies]
        def exchange():
            a, b = [self.lab.staging / t.record["stagingName"] for t in tasks]
            temp = self.lab.staging / "actor-exchange"
            a.rename(temp); b.rename(a); temp.rename(b)
        barrier = threading.Barrier(2, action=exchange, timeout=10)
        self.lab.hook = lambda phase, task: barrier.wait() if phase == "before_target_create" else None
        outcomes = []
        def finish(task):
            try:
                outcomes.append(self.finish(task))
            except Exception as error:
                outcomes.append({"error": str(error)})
        threads = [threading.Thread(target=finish, args=(task,)) for task in tasks]
        for thread in threads: thread.start()
        for thread in threads: thread.join(15)
        self.assertEqual(len(outcomes), 2)
        self.assertTrue(all(x.get("ok") for x in outcomes))
        for task, body in zip(tasks, bodies): self.assert_delivery(task, body)
        self.assertEqual(len({t.record["targetCreatedObject"]["inode"] for t in tasks}), 2)

    def test_existing_target_and_symlink_never_overwritten(self):
        for kind in ("file", "symlink"):
            with self.subTest(kind=kind):
                task = begin(self.lab, name=kind + ".bin")
                path = self.lab.delivery / task.record["destination"]
                if kind == "file": path.write_bytes(b"prior content")
                else: path.symlink_to(self.run / "absent")
                identity = path.lstat().st_ino
                with self.assertRaises(FileExistsError): self.finish(task)
                self.assertEqual(identity, path.lstat().st_ino)
                self.assertFalse(self.receipt(task).exists())
                if kind == "file": self.assertEqual(path.read_bytes(), b"prior content")
                else: self.assertFalse((self.run / "absent").exists())

    def test_same_object_modified_before_snapshot_refused(self):
        task = begin(self.lab)
        os.pwrite(task.fd, b"X", 0)
        with self.assertRaisesRegex(writer.base.Refusal, "held_object_differs"): self.finish(task)
        self.assertFalse(list(self.lab.delivery.iterdir()))
        self.assertFalse(self.receipt(task).exists())

    def test_destination_replaced_during_write_no_write_to_fake(self):
        task = begin(self.lab)
        fake = b"actor replacement must stay untouched"
        def replace(phase, task):
            if phase == "after_target_create":
                path = self.lab.delivery / task.record["destination"]
                path.rename(path.with_suffix(".created-original"))
                path.write_bytes(fake)
        self.lab.hook = replace
        with self.assertRaisesRegex(writer.base.Refusal, "destination_is_different_object"): self.finish(task)
        self.assertEqual((self.lab.delivery / task.record["destination"]).read_bytes(), fake)
        self.assertEqual((self.lab.delivery / task.record["destination"]).with_suffix(".created-original").read_bytes(), BODY)
        self.assertEqual(task.record["state"], "DELIVERY_UNCERTAIN")
        self.assertFalse(self.receipt(task).exists())

    def test_injected_write_failure_keeps_partial_uncertain(self):
        task = begin(self.lab)
        def fault(phase, task):
            if phase == "after_target_chunk": raise OSError(errno.ENOSPC, "injected_write_failure")
        self.lab.hook = fault
        with self.assertRaises(OSError): self.finish(task)
        self.assertEqual((self.lab.delivery / task.record["destination"]).read_bytes(), BODY[:65536])
        self.assertEqual(task.record["state"], "DELIVERY_UNCERTAIN")
        self.assertFalse(self.receipt(task).exists())

    def test_receipt_failure_does_not_claim_success(self):
        task = begin(self.lab)
        original = self.lab.atomic_json
        def fault(path, value):
            if str(path).endswith(".receipt.json"): raise OSError(errno.EIO, "injected_receipt_save_failure")
            return original(path, value)
        self.lab.atomic_json = fault
        with self.assertRaises(OSError): self.finish(task)
        self.assertEqual((self.lab.delivery / task.record["destination"]).read_bytes(), BODY)
        self.assertEqual(task.record["state"], "DELIVERY_UNCERTAIN")
        self.assertFalse(self.receipt(task).exists())

    def test_quarantine_failure_before_any_target_body_write(self):
        task = begin(self.lab)
        original = writer.set_attribute
        writer.set_attribute = lambda *args: (_ for _ in ()).throw(OSError(errno.ENOTSUP, "injected_xattr_failure"))
        try:
            with self.assertRaises(OSError): self.finish(task)
        finally:
            writer.set_attribute = original
        self.assertEqual((self.lab.delivery / task.record["destination"]).read_bytes(), b"")
        self.assertEqual(task.record["state"], "DELIVERY_UNCERTAIN")
        self.assertFalse(self.receipt(task).exists())

    def test_delivery_directory_exchange_refused_before_create(self):
        task = begin(self.lab)
        preserved = self.run / "actor-preserved-delivery"
        def swap(phase, task):
            if phase == "before_target_create":
                self.lab.delivery.rename(preserved)
                self.lab.delivery.mkdir()
        self.lab.hook = swap
        with self.assertRaisesRegex(writer.base.Refusal, "directory_path_object_changed"): self.finish(task)
        self.assertFalse(list(self.lab.delivery.iterdir()))
        self.assertFalse(list(preserved.iterdir()))
        self.assertFalse(self.receipt(task).exists())

    def test_intent_failure_has_zero_delivery_writes(self):
        task = begin(self.lab)
        self.lab.persist = lambda: (_ for _ in ()).throw(OSError(errno.EIO, "injected_intent_failure"))
        with self.assertRaises(OSError): self.finish(task)
        self.assertFalse(list(self.lab.delivery.iterdir()))

    def test_truncated_response_refused_before_destination(self):
        aid = str(uuid.uuid4())
        self.lab.begin({"acquisitionId": aid, "destination": "short.bin", "scenario": "short",
                       "binding": {"sessionId": None, "networkId": None, "debuggerEpoch": "test-epoch", "tabId": 1, "fetchRequestId": aid, "stream": aid},
                       "response": {"url": ORIGIN + "/short", "status": 200,
                                    "headers": [{"name": "Content-Length", "value": "100"}]}})
        self.lab.chunk({"acquisitionId": aid, "sequence": 0, "base64": base64.b64encode(b"short").decode()})
        with self.assertRaisesRegex(writer.base.Refusal, "incomplete_fixed_length"):
            self.lab.complete({"acquisitionId": aid, "eof": True})
        self.assertFalse(list(self.lab.delivery.iterdir()))

    def test_chunked_ambiguous_http_compressed_unsupported_before_take(self):
        for url, headers in [(ORIGIN, [{"name": "Transfer-Encoding", "value": "chunked"}]),
                             (ORIGIN, [{"name": "Content-Length", "value": "3"}, {"name": "Transfer-Encoding", "value": "chunked"}]),
                             (ORIGIN, [{"name": "Content-Length", "value": "3"}, {"name": "Content-Encoding", "value": "gzip"}]),
                             (ORIGIN, [{"name": "Content-Length", "value": "3"}, {"name": "Content-Length", "value": "3"}]),
                             ("http://fixture.invalid", [{"name": "Content-Length", "value": "3"}])]:
            with self.subTest(url=url, headers=headers):
                with self.assertRaises(writer.base.Refusal):
                    self.lab.prepare({"response": {"url": url, "status": 200, "headers": headers}})
        self.assertFalse(self.lab.tasks)
        self.assertFalse(list(self.lab.delivery.iterdir()))

    def test_post_completion_replacement_never_inherits_live_binding(self):
        task = begin(self.lab)
        self.finish(task)
        path = self.lab.delivery / task.record["destination"]
        fake = self.run / "same-content-new-object"
        fake.write_bytes(BODY)
        os.replace(str(fake), str(path))
        self.assertFalse(self.lab.recheck({"acquisitionId": task.record["acquisitionId"]})["ok"])
        self.assertTrue(self.receipt(task).exists())

    def test_replacement_after_observation_is_historical_not_current_claim(self):
        task = begin(self.lab)
        replaced_at = []
        def replace(phase, task):
            if phase == "before_receipt":
                path = self.lab.delivery / task.record["destination"]
                fake = self.run / "same-content-late-replacement"
                fake.write_bytes(BODY)
                os.replace(str(fake), str(path))
                replaced_at.append(writer.time.time_ns())
        self.lab.hook = replace
        self.finish(task)
        self.assertLess(task.record["observedAtNs"], replaced_at[0])
        self.assertTrue(task.record["currentPathRequiresRecheck"])
        self.assertFalse(self.lab.recheck({"acquisitionId": task.record["acquisitionId"]})["ok"])
        self.assertTrue(self.receipt(task).exists())

    def test_real_process_exit_and_fresh_read_only_inspection(self):
        for phase, expected in [("after_target_chunk", BODY[:65536]), ("after_target_write", BODY)]:
            run = Path(tempfile.mkdtemp(prefix="traeva-c-verified-crash-", dir="/private/tmp"))
            child = subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), "--child", phase, "--run", str(run)],
                                   capture_output=True, timeout=20)
            self.assertEqual(child.returncode, 73)
            fresh = subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), "--inspect", str(run)],
                                   capture_output=True, check=True, timeout=10)
            inspection = json.loads(fresh.stdout)
            self.assertFalse(inspection["liveDescriptorContinuity"])
            self.assertFalse(inspection["automaticResume"])
            self.assertEqual(inspection["receipts"], [])
            files = list((run / "delivery").iterdir())
            self.assertEqual(len(files), 1)
            self.assertEqual(files[0].read_bytes(), expected)

    def test_restart_reads_history_but_never_restores_descriptor_continuity(self):
        task = begin(self.lab)
        self.finish(task)
        fresh = subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), "--inspect", str(self.run)],
                               capture_output=True, check=True, timeout=10)
        inspection = json.loads(fresh.stdout)
        self.assertEqual(inspection["state"], "HISTORICAL_RECEIPT_ONLY")
        self.assertEqual(len(inspection["receipts"]), 1)
        self.assertFalse(inspection["liveDescriptorContinuity"])
        self.assertEqual(inspection["currentAssociation"], "UNKNOWN_REQUIRES_NEW_EVIDENCE")
        self.receipt(task).write_text("{broken")
        inspection = writer.inspect_restart(self.run)
        self.assertEqual(inspection["state"], "READ_FAILED_UNCERTAIN")
        self.assertEqual(inspection["receipts"], [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--child")
    parser.add_argument("--run")
    parser.add_argument("--inspect")
    args = parser.parse_args()
    if args.inspect:
        print(json.dumps(writer.inspect_restart(args.inspect)))
    elif args.child:
        def crash(phase, task):
            if phase == args.child: os._exit(73)
        lab = writer.Writer(Path(args.run), [ORIGIN], crash)
        lab.authorize({"takeoverConsent": True, "saveLocationConsent": str(lab.delivery)})
        task = begin(lab)
        lab.complete({"acquisitionId": task.record["acquisitionId"], "eof": True})
    else:
        unittest.main(argv=[sys.argv[0]], verbosity=2)
