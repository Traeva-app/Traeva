#!/usr/bin/env python3
"""Real TLS fixture for Download with TRAEVA; independent of Chrome events."""
import base64
import concurrent.futures
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading
import uuid

from writer import Writer, base, inspect_restart

BODY = b"direct HTTPS fixture\n" * 16384
OTHER = b"second HTTPS fixture\n" * 16384


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        content = OTHER if self.path.endswith("/other") else BODY
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Connection", "close")
        if self.path == "/malformed-chunked":
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            self.wfile.write(b"3\r\nabc\r\n0\r\n")
        else:
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content[:65536] if self.path == "/truncated" else content)
        self.close_connection = True


def acquire(lab, origin, ca, endpoint, scenario):
    from urllib.parse import urlsplit
    parsed = urlsplit(origin)
    aid = str(uuid.uuid4())
    connection = http.client.HTTPSConnection(parsed.hostname, parsed.port,
                                              context=ssl.create_default_context(cafile=str(ca)), timeout=10)
    task = None
    try:
        # This call and its returned response remain local to this task.
        connection.request("GET", endpoint)
        response = connection.getresponse()
        payload = {"acquisitionId": aid, "destination": aid + ".txt", "scenario": scenario,
                   "binding": {"kind": "TRAEVA_DIRECT_HTTPS", "requestId": aid,
                               "responseHandle": str(uuid.uuid4())},
                   "response": {"url": origin + endpoint, "status": response.status,
                                "headers": [{"name": k, "value": v} for k, v in response.getheaders()]}}
        lab.prepare(payload)
        lab.begin(payload)
        task = lab.tasks[aid]
        sequence = 0
        while True:
            data = response.read(32768)
            if not data:
                break
            lab.chunk({"acquisitionId": aid, "sequence": sequence,
                       "base64": base64.b64encode(data).decode()})
            sequence += 1
        outcome = lab.complete({"acquisitionId": aid, "eof": True})
        expected = OTHER if endpoint.endswith("/other") else BODY
        target = lab.delivery / task.record["destination"]
        assert target.read_bytes() == expected
        assert target.stat().st_ino == os.fstat(task.target_fd).st_ino
        assert outcome["chromeDownloadId"] is None
        return {"scenario": scenario, "result": "PASS_DIRECT_OBSERVED", "record": outcome}
    except Exception as error:
        if task:
            lab.refuse(task, error)
            assert not (lab.delivery / task.record["destination"]).exists()
        return {"scenario": scenario, "result": "REFUSED", "error": str(error),
                "record": dict(task.record) if task else None}
    finally:
        connection.close()


def main():
    root = Path(tempfile.mkdtemp(prefix="traeva-c-verified-direct-", dir="/private/tmp"))
    evidence = root / "evidence"
    evidence.mkdir()
    cert, key = root / "cert.pem", root / "key.pem"
    subprocess.run(["/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                    "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
                    "-keyout", str(key), "-out", str(cert)], check=True, capture_output=True)
    key.chmod(0o600)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(str(cert), str(key))
    server.socket = tls.wrap_socket(server.socket, server_side=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    origin = "https://127.0.0.1:" + str(server.server_port)
    lab = Writer(root / "native", [origin], namespace="TRAEVA_DIRECT_HTTPS_ACQUISITION")
    lab.authorize({"takeoverConsent": True, "saveLocationConsent": str(lab.delivery)})
    results = []
    try:
        results.append(acquire(lab, origin, cert, "/normal", "direct-normal"))
        results.append(acquire(lab, origin, cert, "/truncated", "direct-truncated"))
        results.append(acquire(lab, origin, cert, "/malformed-chunked", "direct-malformed-chunked"))
        stimuli = []
        def exchange():
            tasks = [t for t in lab.tasks.values() if t.record["scenario"] == "direct-parallel-swap"]
            paths = [lab.staging / t.record["stagingName"] for t in tasks]
            assert len(paths) == 2
            before = [p.stat().st_ino for p in paths]
            tmp = lab.staging / "actor-swapped"
            paths[0].rename(tmp); paths[1].rename(paths[0]); tmp.rename(paths[1])
            after = [p.stat().st_ino for p in paths]
            assert before == list(reversed(after))
            stimuli.append({"action": "actual_concurrent_source_names_swapped", "before": before, "after": after})
        barrier = threading.Barrier(2, action=exchange, timeout=10)
        lab.hook = lambda phase, task: barrier.wait() if phase == "before_target_create" else None
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(acquire, lab, origin, cert, endpoint, "direct-parallel-swap")
                       for endpoint in ("/normal", "/other")]
            results.extend(future.result(timeout=20) for future in futures)
        assert [r["result"] for r in results] == ["PASS_DIRECT_OBSERVED", "REFUSED", "REFUSED",
                                                  "PASS_DIRECT_OBSERVED", "PASS_DIRECT_OBSERVED"]
        assert results[2]["record"] is None
        assert results[1]["record"]["receivedBytes"] < results[1]["record"]["expectedBytes"]
        (evidence / "results.json").write_text(json.dumps(results, indent=2))
        (evidence / "stimuli.json").write_text(json.dumps(stimuli, indent=2))
        (evidence / "environment.json").write_text(json.dumps({"origin": origin, "tlsScope": "generated fixture CA only; hostname verification enabled",
            "bodySha256": hashlib.sha256(BODY).hexdigest(), "otherSha256": hashlib.sha256(OTHER).hexdigest(),
            "restartInspection": inspect_restart(lab.run)}, indent=2))
        for name in ("writer.py", "base_receiver.py", "direct_probe.py"):
            shutil.copy2(Path(__file__).with_name(name), evidence / name)
    finally:
        lab.close()
        server.shutdown(); server.server_close(); thread.join(timeout=5)
    print(json.dumps({"root": str(root), "results": [r["result"] for r in results], "productionAcceptance": False}))


if __name__ == "__main__":
    main()
