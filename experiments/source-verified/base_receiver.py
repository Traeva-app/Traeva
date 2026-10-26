#!/usr/bin/env python3
"""C-owned lab receive primitives; path-based delivery and HTTP server removed."""

import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import secrets
import re
import stat
import threading
import time
from urllib.parse import urlsplit


MAX_FILE = 20 * 1024 * 1024
OPEN_FILE = os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
OPEN_DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


class Refusal(Exception):
    pass


class CommitUnknown(OSError):
    """Replacement is visible, but directory durability was not acknowledged."""
    pass


def safe_error(error):
    if isinstance(error, (Refusal, ValueError)) and re.fullmatch(r"[a-z0-9_]{1,80}", str(error)):
        return str(error)
    if isinstance(error, OSError):
        return "io_error_" + str(error.errno if type(error.errno) is int else "unknown")
    return "operation_failed"


def write_all(fd, data):
    view = memoryview(data)
    while view:
        count = os.write(fd, view)
        if count <= 0:
            raise OSError("short_write")
        view = view[count:]


def component(value, label):
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > 180:
        raise Refusal("invalid_" + label)
    if value in (".", "..") or "/" in value or "\\" in value or "\x00" in value:
        raise Refusal("invalid_" + label)
    return value


def snapshot(fd):
    value = os.fstat(fd)
    return {
        "device": value.st_dev, "inode": value.st_ino,
        "birthTime": getattr(value, "st_birthtime", None),
        "size": value.st_size, "links": value.st_nlink,
        "mode": value.st_mode, "mtimeNs": value.st_mtime_ns,
        "ctimeNs": value.st_ctime_ns,
    }


def same_object(left, right):
    return (left["device"], left["inode"]) == (right["device"], right["inode"])


def file_kind(value):
    if not stat.S_ISREG(value["mode"]) or value["links"] != 1:
        raise Refusal("object_not_single_link_regular_file")


def digest_fd(fd):
    before = snapshot(fd)
    file_kind(before)
    digest = hashlib.sha256()
    offset = 0
    while True:
        part = os.pread(fd, 65536, offset)
        if not part:
            break
        offset += len(part)
        digest.update(part)
    after = snapshot(fd)
    if before != after or offset != before["size"]:
        raise Refusal("object_changed_during_readback")
    return digest.hexdigest(), after


class Task:
    def __init__(self, acquisition_id, binding, response, scenario, destination, expected):
        self.lock = threading.RLock()
        self.fd = None
        self.hash = hashlib.sha256()
        self.sequence = 0
        self.record = {
            "acquisitionId": acquisition_id,
            "binding": binding, "response": response, "scenario": scenario,
            "destination": destination, "stagingName": acquisition_id + ".part",
            "expectedBytes": expected, "receivedBytes": 0, "nextSequence": 0,
            "state": "INTENT", "startedAtNs": time.time_ns(), "rechecks": [],
            "claimScope": "Historical response-to-held-object observation; not product VERIFIED",
        }


class Receiver:
    def __init__(self, run):
        self.run = run
        self.app_data = run / "app-data"
        self.staging = self.app_data / "staging"
        self.delivery = run / "delivery"
        for path in (run, self.app_data, self.staging, self.delivery):
            path.mkdir(mode=0o700, parents=True, exist_ok=True)
            if path.is_symlink() or not path.is_dir():
                raise Refusal("unsafe_directory: " + str(path))
        self.stage_fd = os.open(str(self.staging), OPEN_DIR)
        self.dest_fd = os.open(str(self.delivery), OPEN_DIR)
        self.stage_identity = snapshot(self.stage_fd)
        self.dest_identity = snapshot(self.dest_fd)
        self.token = secrets.token_urlsafe(32)
        self.tasks = {}
        self.tasks_lock = threading.RLock()
        self.storage_lock = threading.RLock()
        self.events_fd = os.open(str(self.app_data / "events.jsonl"), OPEN_FILE, 0o600)
        self.event_sequence = 0
        self.event("receiver_started", {"pid": os.getpid(), "run": str(run)})

    def event(self, kind, data):
        with self.storage_lock:
            self.event_sequence += 1
            event = {"sequence": self.event_sequence, "atNs": time.time_ns(), "event": kind, **data}
            write_all(self.events_fd, (json.dumps(event, sort_keys=True) + "\n").encode())
            os.fsync(self.events_fd)

    def atomic_json(self, path, value):
        temp = self.app_data / (".pending-" + secrets.token_hex(12))
        fd = os.open(str(temp), OPEN_FILE, 0o600)
        try:
            write_all(fd, (json.dumps(value, sort_keys=True, indent=2) + "\n").encode())
            os.fsync(fd)
        finally:
            os.close(fd)
        os.replace(str(temp), str(path))
        try:
            hook = getattr(self, "persistence_hook", lambda phase, path: None)
            hook("after_replace", path)
            directory = os.open(str(self.app_data), OPEN_DIR)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        except Exception as error:
            raise CommitUnknown("replacement_visible_durability_unknown") from error

    def persist(self):
        # Each assignment replaces scalar/dict entries. A copied snapshot is diagnostic;
        # the append-only events identify the individual durable state transitions.
        with self.storage_lock:
            with self.tasks_lock:
                records = [dict(task.record) for task in self.tasks.values()]
            self.atomic_json(self.app_data / "results.json", {"records": records})

    def directories_current(self):
        for path, held, original in ((self.staging, self.stage_fd, self.stage_identity),
                                     (self.delivery, self.dest_fd, self.dest_identity)):
            try:
                current_fd = os.open(str(path), OPEN_DIR)
            except OSError as error:
                raise Refusal("directory_path_unavailable: " + str(error.errno))
            try:
                current = snapshot(current_fd)
                if not same_object(current, snapshot(held)) or not same_object(current, original):
                    raise Refusal("directory_path_object_changed")
            finally:
                os.close(current_fd)

    def begin(self, payload):
        acquisition_id = component(payload.get("acquisitionId"), "acquisition_id")
        destination = component(payload.get("destination"), "destination")
        binding = payload.get("binding")
        if not isinstance(binding, dict):
            raise Refusal("missing_response_binding")
        if binding.get("kind") == "TRAEVA_DIRECT_HTTPS":
            fields = ("requestId", "responseHandle")
        else:
            if not isinstance(binding.get("tabId"), int):
                raise Refusal("missing_tab_binding")
            fields = ("fetchRequestId", "stream")
        if not all(isinstance(binding.get(key), str) and binding[key] for key in fields):
            raise Refusal("missing_response_stream_binding")
        response = payload.get("response")
        if not isinstance(response, dict) or response.get("status") != 200:
            raise Refusal("only_status_200_supported")
        url = response.get("url")
        parsed = urlsplit(url) if isinstance(url, str) else None
        if not parsed or parsed.scheme not in ("http", "https") or parsed.username or parsed.password:
            raise Refusal("invalid_response_url")
        headers = response.get("headers")
        if not isinstance(headers, list) or len(headers) > 100:
            raise Refusal("invalid_response_headers")
        lengths, encodings, transfers = [], [], []
        for header in headers:
            if not isinstance(header, dict) or not isinstance(header.get("name"), str) or not isinstance(header.get("value"), str):
                raise Refusal("invalid_response_header")
            name, value = header["name"].lower(), header["value"].strip()
            if name == "content-length":
                lengths.append(value)
            if name == "content-encoding":
                encodings.append(value.lower())
            if name == "transfer-encoding":
                transfers.append(value.lower())
        if len(lengths) != 1 or not lengths[0].isascii() or not lengths[0].isdigit():
            raise Refusal("single_content_length_required")
        if any(value != "identity" for value in encodings) or transfers:
            raise Refusal("encoded_or_transfer_framing_unsupported")
        expected = int(lengths[0])
        if expected > MAX_FILE:
            raise Refusal("lab_size_limit")
        scenario = payload.get("scenario")
        if not isinstance(scenario, str) or len(scenario) > 120:
            raise Refusal("invalid_scenario")
        task = Task(acquisition_id, binding, response, scenario, destination, expected)
        with self.tasks_lock:
            if acquisition_id in self.tasks:
                raise Refusal("acquisition_id_already_used")
            self.tasks[acquisition_id] = task
        with task.lock:
            try:
                self.event("acquisition_intent", dict(task.record))
                self.directories_current()
                try:
                    task.fd = os.open(task.record["stagingName"], OPEN_FILE, 0o600, dir_fd=self.stage_fd)
                except FileExistsError:
                    raise Refusal("staging_name_already_exists")
                task.record["createdObject"] = snapshot(task.fd)
                task.record["heldDescriptor"] = task.fd
                file_kind(task.record["createdObject"])
                task.record["state"] = "RECEIVING"
                self.event("response_stream_held_fd_bound", dict(task.record))
                self.persist()
                return {"ok": True, **dict(task.record)}
            except Exception as error:
                self.refuse(task, error)
                raise

    def get_task(self, payload):
        acquisition_id = payload.get("acquisitionId")
        with self.tasks_lock:
            task = self.tasks.get(acquisition_id) if isinstance(acquisition_id, str) else None
        if task is None:
            raise Refusal("unknown_acquisition")
        return task

    def refuse(self, task, error):
        task.record["state"] = "REFUSED"
        task.record["error"] = safe_error(error)
        task.record["refusedAtNs"] = time.time_ns()
        try:
            self.event("acquisition_refused", {"acquisitionId": task.record["acquisitionId"], "error": safe_error(error)})
            self.persist()
        except Exception as storage_error:
            task.record["persistenceError"] = safe_error(storage_error)

    def chunk(self, payload):
        task = self.get_task(payload)
        with task.lock:
            if task.record["state"] != "RECEIVING":
                self.event("protocol_rejected", {"acquisitionId": task.record["acquisitionId"],
                                                "endpoint": "chunk", "state": task.record["state"]})
                raise Refusal("task_not_receiving")
            try:
                sequence = payload.get("sequence")
                if type(sequence) is not int or sequence != task.sequence:
                    raise Refusal("unexpected_sequence")
                encoded = payload.get("base64")
                if not isinstance(encoded, str) or not encoded:
                    raise Refusal("empty_or_invalid_chunk")
                try:
                    data = base64.b64decode(encoded, validate=True)
                except (ValueError, binascii.Error):
                    raise Refusal("invalid_base64")
                if not data or len(data) > 1024 * 1024:
                    raise Refusal("invalid_chunk_size")
                if task.record["receivedBytes"] + len(data) > task.record["expectedBytes"]:
                    raise Refusal("body_exceeds_content_length")
                write_all(task.fd, data)
                task.hash.update(data)
                task.sequence += 1
                task.record["receivedBytes"] += len(data)
                task.record["nextSequence"] = task.sequence
                self.event("chunk_written_to_held_fd", {
                    "acquisitionId": task.record["acquisitionId"], "sequence": sequence,
                    "bytes": len(data), "heldObject": snapshot(task.fd),
                })
                return {"ok": True, "nextSequence": task.sequence, "receivedBytes": task.record["receivedBytes"]}
            except Exception as error:
                self.refuse(task, error)
                raise

    def abort(self, payload):
        task = self.get_task(payload)
        with task.lock:
            if task.record["state"] in ("ACQUIRED_OBSERVED_SAME_OBJECT", "OUTCOME_UNKNOWN"):
                raise Refusal("historical_acquisition_already_recorded")
            reason = payload.get("reason", "caller_aborted")
            if not isinstance(reason, str):
                raise Refusal("invalid_abort_reason")
            self.refuse(task, Refusal("caller_aborted"))
            return {"ok": True, "state": task.record["state"]}

    def status(self):
        with self.tasks_lock:
            records = [dict(task.record) for task in self.tasks.values()]
        return {"ok": True, "records": records, "pid": os.getpid()}

    def close(self):
        for task in self.tasks.values():
            if task.fd is not None:
                os.close(task.fd)
                task.fd = None
        for fd in (self.events_fd, self.stage_fd, self.dest_fd):
            os.close(fd)
