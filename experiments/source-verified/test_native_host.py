#!/usr/bin/env python3
import json
import base64
import os
from pathlib import Path
import secrets
import struct
import subprocess
import sys
import tempfile
import unittest

import native_host

ORIGIN = "chrome-extension://" + "a" * 32 + "/"


class Tests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="traeva-c-verified-native-test-", dir="/private/tmp"))
        self.run = self.root / "run"
        self.token = secrets.token_hex(32)
        self.config = self.root / "config.json"
        self.config.write_text(json.dumps({"token": self.token, "extensionOrigin": ORIGIN,
                                          "run": str(self.run), "origins": ["https://fixture.invalid"]}))
        self.config.chmod(0o600)
        self.process = None

    def tearDown(self):
        if self.process and self.process.poll() is None:
            self.process.stdin.close()
            self.process.wait(timeout=5)
        if self.process:
            self.process.stdin.close()
            self.process.stdout.close()
            self.process.stderr.close()

    def launch(self, origin=ORIGIN, lose_completion_reply=False):
        command = [sys.executable, "-B", str(Path(__file__).with_name("native_host.py"))]
        if lose_completion_reply:
            code = "import sys,os;sys.path.insert(0,sys.argv.pop(1));import native_host;n=native_host.reply\ndef reply(v):\n if v.get('receiptCommitted'):os._exit(74)\n n(v)\nnative_host.reply=reply;native_host.main()"
            command = [sys.executable, "-B", "-c", code, str(Path(__file__).parent)]
        self.process = subprocess.Popen(command + ["--config", str(self.config), origin], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def request(self, command, payload=None, token=None):
        data = json.dumps({"rid": 1, "auth": self.token if token is None else token,
                           "command": command, "payload": payload or {}}).encode()
        self.process.stdin.write(struct.pack("=I", len(data)) + data)
        self.process.stdin.flush()
        return native_host.read(self.process.stdout)

    def authorize(self):
        return self.request("authorize", {"takeoverConsent": True,
                                           "saveLocationConsent": str(self.run / "delivery")})

    def test_origin_rejected_before_any_writer_directory(self):
        self.launch("chrome-extension://" + "b" * 32 + "/")
        self.assertEqual(self.process.wait(timeout=5), 2)
        self.assertIn(b"native_extension_origin_rejected", self.process.stderr.read())
        self.assertFalse(self.run.exists())

    def test_auth_consent_and_command_allowlist(self):
        self.launch()
        self.assertEqual(self.request("authorize", token="wrong")["error"], "native_authentication_required")
        self.assertFalse(self.run.exists())
        self.assertEqual(self.request("begin")["error"], "authorization_required")
        self.assertFalse(self.run.exists())
        self.assertFalse(self.request("authorize", {"takeoverConsent": False})["ok"])
        self.assertFalse(self.run.exists())
        self.assertFalse(self.request("shell", {"command": "ignored"})["ok"])
        self.assertTrue(self.authorize()["ok"])
        self.assertEqual(self.request("hello")["pid"], self.request("status")["pid"])
        self.assertEqual(self.request("hello")["pid"], self.process.pid)

    def test_malformed_or_oversize_native_frames_close_without_writer(self):
        for frame in [struct.pack("=I", native_host.MAX_MESSAGE + 1), struct.pack("=I", 100) + b"{}",
                      b"\x01\x00", struct.pack("=I", 2) + b"[]"]:
            with self.subTest(frame=frame[:10]):
                result = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("native_host.py")),
                                         "--config", str(self.config), ORIGIN], input=frame,
                                        capture_output=True, timeout=5)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, b"")
                self.assertFalse(self.run.exists())

    def test_eof_during_receive_persists_refusal_without_delivery(self):
        self.launch()
        self.authorize()
        payload = {"acquisitionId": "native-eof-task", "destination": "file.txt", "scenario": "native-eof",
                   "binding": {"sessionId": None, "networkId": None, "debuggerEpoch": "test-epoch", "tabId": 1, "fetchRequestId": "specific-request", "stream": "specific-stream"},
                   "response": {"url": "https://fixture.invalid/file", "status": 200,
                                "headers": [{"name": "Content-Length", "value": "100"}]}}
        self.assertTrue(self.request("begin", payload)["ok"])
        self.process.stdin.close()
        self.assertEqual(self.process.wait(timeout=5), 0)
        state = json.loads((self.run / "app-data" / "results.json").read_text())["records"][0]
        self.assertEqual(state["state"], "REFUSED")
        self.assertEqual(state["error"], "native_port_closed_during_receive")
        self.assertFalse(list((self.run / "delivery").iterdir()))
        self.assertFalse(list((self.run / "app-data").glob("*.receipt.json")))

    def test_insecure_config_rejected_before_writer(self):
        self.config.chmod(0o644)
        self.launch()
        self.assertEqual(self.process.wait(timeout=5), 2)
        self.assertIn(b"private_config_required", self.process.stderr.read())
        self.assertFalse(self.run.exists())

    def test_real_host_reply_loss_after_commit_retains_history(self):
        self.launch(lose_completion_reply=True); self.authorize()
        aid = "reply-loss-task"
        payload = {"acquisitionId": aid, "destination": "reply.txt", "scenario": "reply-loss",
                   "binding": {"tabId": 1, "sessionId": None, "networkId": None, "debuggerEpoch": "test-epoch", "fetchRequestId": "request", "stream": "stream"},
                   "response": {"url": "https://fixture.invalid/file", "status": 200, "headers": [{"name": "Content-Length", "value": "3"},
                       {"name": "Set-Cookie", "value": "SYNTHETIC_PRIVATE_COOKIE"}]}}
        self.assertTrue(self.request("begin", payload)["ok"])
        self.assertTrue(self.request("chunk", {"acquisitionId": aid, "sequence": 0, "base64": base64.b64encode(b"abc").decode()})["ok"])
        self.assertIsNone(self.request("complete", {"acquisitionId": aid, "eof": True}))
        self.assertEqual(self.process.wait(timeout=5), 74)
        import writer
        inspection = writer.inspect_restart(self.run)
        self.assertEqual(inspection["state"], "HISTORICAL_RECEIPT_ONLY")
        self.assertFalse(inspection["liveDescriptorContinuity"])
        for path in (self.run / "app-data").iterdir():
            if path.is_file(): self.assertNotIn(b"SYNTHETIC_PRIVATE_COOKIE", path.read_bytes())
        self.assertEqual((self.run / "delivery" / "reply.txt").read_bytes(), b"abc")


if __name__ == "__main__":
    unittest.main(verbosity=2)
