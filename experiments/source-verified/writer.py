#!/usr/bin/env python3
"""Opt-in synthetic lab writer. Never writes to the production Source database."""

import ctypes
import hashlib
import json
import os
from pathlib import Path
import plistlib
import time
import uuid
import receipt as receipt_schema

import base_receiver as base

MAX_FILE = 20 * 1024 * 1024
base.MAX_FILE = MAX_FILE
LIBC = ctypes.CDLL(None, use_errno=True)
SET_XATTR = LIBC.fsetxattr
SET_XATTR.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_uint32, ctypes.c_int]
SET_XATTR.restype = ctypes.c_int
GET_XATTR = LIBC.fgetxattr
GET_XATTR.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_uint32, ctypes.c_int]
GET_XATTR.restype = ctypes.c_ssize_t


def set_attribute(fd, name, value):
    if SET_XATTR(fd, name.encode(), value, len(value), 0, 0):
        raise OSError(ctypes.get_errno(), "fsetxattr_failed")


def attribute(fd, name):
    length = GET_XATTR(fd, name.encode(), None, 0, 0, 0)
    if length < 0:
        raise OSError(ctypes.get_errno(), "fgetxattr_failed")
    value = ctypes.create_string_buffer(length)
    if GET_XATTR(fd, name.encode(), value, length, 0, 0) != length:
        raise base.Refusal("attribute_changed_during_read")
    return value.raw


def supported(response, origins):
    from urllib.parse import urlsplit
    if not isinstance(response, dict):
        raise base.Refusal("response_required")
    parsed = urlsplit(response.get("url", ""))
    origin = parsed.scheme + "://" + parsed.netloc
    if (parsed.scheme != "https" or parsed.username or parsed.password or origin not in origins or
            parsed.query or parsed.fragment or len(response.get("url", "")) > 2048):
        raise base.Refusal("https_origin_not_supported")
    if type(response.get("status")) is not int or response["status"] != 200:
        raise base.Refusal("only_status_200_supported")
    headers = response.get("headers")
    if not isinstance(headers, list) or len(headers) > 100:
        raise base.Refusal("invalid_headers")
    values = {}
    for item in headers:
        if not isinstance(item, dict) or not isinstance(item.get("name"), str) or not isinstance(item.get("value"), str):
            raise base.Refusal("invalid_header")
        values.setdefault(item["name"].lower(), []).append(item["value"].strip())
    lengths = values.get("content-length", [])
    if len(lengths) != 1 or not lengths[0].isascii() or not lengths[0].isdigit():
        raise base.Refusal("single_content_length_required")
    if not 0 < int(lengths[0]) <= MAX_FILE:
        raise base.Refusal("unsupported_length")
    if "transfer-encoding" in values or any(x.lower() != "identity" for x in values.get("content-encoding", [])):
        raise base.Refusal("unsupported_transfer_format")
    if any(len(v) != 1 or len(v[0].encode()) > 256 for k, v in values.items() if k in receipt_schema.HEADERS):
        raise base.Refusal("unsupported_header_shape")
    return int(lengths[0])


class Writer(base.Receiver):
    def __init__(self, run, origins, hook=None, namespace="TRAEVA_CHROME_FETCH_ACQUISITION"):
        run = Path(run)
        if not run.is_absolute() or not str(run).startswith("/private/tmp/traeva-c-verified-") or ".." in run.parts:
            raise base.Refusal("synthetic_lab_directory_required")
        self.origins = origins
        if namespace not in ("TRAEVA_CHROME_FETCH_ACQUISITION", "TRAEVA_DIRECT_HTTPS_ACQUISITION"):
            raise base.Refusal("invalid_acquisition_namespace")
        self.namespace = namespace
        self.hook = hook or (lambda phase, task: None)
        self.authorized = False
        self.event_bytes = 0
        super().__init__(run)

    def event(self, kind, data):
        with self.storage_lock:
            allowed = {"acquisitionId", "state", "pid", "sequence", "bytes", "endpoint",
                       "heldObject", "sourceHeldObject", "targetCreatedObject"}
            event = {**{k: v for k, v in data.items() if k in allowed},
                     "eventSequence": self.event_sequence + 1, "atNs": time.time_ns(), "event": kind}
            encoded = (json.dumps(event, sort_keys=True) + "\n").encode()
            if self.event_sequence >= 16384 or self.event_bytes + len(encoded) > 32 * 1024 * 1024:
                raise base.Refusal("lab_event_budget_exceeded")
            base.write_all(self.events_fd, encoded)
            self.event_sequence += 1
            self.event_bytes += len(encoded)
            os.fsync(self.events_fd)

    def authorize(self, payload):
        if payload.get("takeoverConsent") is not True or payload.get("saveLocationConsent") != str(self.delivery):
            raise base.Refusal("explicit_mode_and_location_consent_required")
        self.authorized = True
        self.event("explicit_lab_authorization", {"delivery": str(self.delivery)})
        return {"ok": True, "delivery": str(self.delivery), "supportedOrigins": self.origins}

    def prepare(self, payload):
        if not self.authorized:
            raise base.Refusal("takeover_not_authorized")
        supported(payload.get("response"), self.origins)
        if "destination" in payload:
            base.component(payload["destination"], "destination")
            if payload["destination"].startswith("."):
                raise base.Refusal("hidden_destination_unsupported")
            try:
                os.stat(payload["destination"], dir_fd=self.dest_fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                raise base.Refusal("destination_already_exists")
        with self.tasks_lock:
            if sum(t.record["state"] in ("RECEIVING", "DELIVERY_INTENT") for t in self.tasks.values()) >= 4:
                raise base.Refusal("lab_active_task_budget_exceeded")
            if len(self.tasks) >= 32 or sum(t.record["expectedBytes"] for t in self.tasks.values()) + supported(payload["response"], self.origins) > 80 * 1024 * 1024:
                raise base.Refusal("lab_task_budget_exceeded")
        self.directories_current()
        return {"ok": True, "allowed": True, "delivery": str(self.delivery)}

    def begin(self, payload):
        # Preserve persist's storage -> tasks order while serializing quota allocation.
        with self.storage_lock:
            return self._begin(payload)

    def _begin(self, payload):
        self.prepare(payload)
        binding = payload.get("binding")
        if not isinstance(binding, dict):
            raise base.Refusal("missing_response_binding")
        direct = binding.get("kind") == "TRAEVA_DIRECT_HTTPS"
        if direct != (self.namespace == "TRAEVA_DIRECT_HTTPS_ACQUISITION"):
            raise base.Refusal("binding_namespace_mismatch")
        if direct:
            keys = {"kind", "requestId", "responseHandle"}
            text_fields = ("requestId", "responseHandle")
        else:
            keys = {"tabId", "sessionId", "debuggerEpoch", "fetchRequestId", "stream", "networkId"}
            text_fields = ("debuggerEpoch", "fetchRequestId", "stream")
            if type(binding.get("tabId")) is not int or binding["tabId"] < 0:
                raise base.Refusal("missing_tab_binding")
            if any(binding.get(k) is not None and (not isinstance(binding[k], str) or not 0 < len(binding[k]) <= 512) for k in ("sessionId", "networkId")):
                raise base.Refusal("invalid_optional_binding")
        if set(binding) != keys or any(not isinstance(binding.get(k), str) or not 0 < len(binding[k]) <= 512 for k in text_fields):
            raise base.Refusal("incomplete_response_binding")
        response = payload["response"]
        expected = supported(response, self.origins)
        clean_response = {"url": response["url"], "status": response["status"], "headers": [
            {"name": h["name"].lower(), "value": str(expected) if h["name"].lower() == "content-length" else h["value"].strip()}
            for h in response["headers"] if h["name"].lower() in receipt_schema.HEADERS]}
        payload = {**payload, "response": clean_response}
        result = super().begin(payload)
        task = self.get_task(payload)
        task.target_fd = None
        task.record["sourceNamespace"] = self.namespace
        task.record["chromeDownloadId"] = None
        self.persist()
        return result

    def refuse(self, task, error):
        if task.record["state"] in ("ACQUIRED_OBSERVED_SAME_OBJECT", "OUTCOME_UNKNOWN"):
            return
        task.record["state"] = "DELIVERY_UNCERTAIN" if task.record.get("deliveryAttempted") else "REFUSED"
        task.record["error"] = base.safe_error(error)
        task.record["refusedAtNs"] = time.time_ns()
        try:
            self.event("acquisition_not_successful", {"acquisitionId": task.record["acquisitionId"],
                                                     "state": task.record["state"], "error": base.safe_error(error)})
            self.persist()
        except Exception as storage_error:
            task.record["persistenceError"] = base.safe_error(storage_error)

    def complete(self, payload):
        task = self.get_task(payload)
        with task.lock:
            if task.record["state"] == "ACQUIRED_OBSERVED_SAME_OBJECT":
                return {"ok": True, "idempotentHistoricalResult": True, **dict(task.record)}
            if task.record["state"] == "OUTCOME_UNKNOWN":
                return {"ok": False, "outcome": "OUTCOME_UNKNOWN", "error": "receipt_commit_unacknowledged",
                        "acquisitionId": task.record["acquisitionId"], "reconciliationRequired": True}
            if task.record["state"] != "RECEIVING":
                raise base.Refusal("task_not_receiving")
            try:
                if payload.get("eof") is not True or task.record["receivedBytes"] != task.record["expectedBytes"]:
                    raise base.Refusal("incomplete_fixed_length_response")
                os.fsync(task.fd)
                before = base.snapshot(task.fd)
                base.file_kind(before)
                # A frozen in-memory snapshot cannot be swapped via any disk pathname.
                content = os.pread(task.fd, task.record["expectedBytes"] + 1, 0)
                after = base.snapshot(task.fd)
                digest = hashlib.sha256(content).hexdigest()
                if before != after or len(content) != task.record["expectedBytes"] or digest != task.hash.hexdigest():
                    raise base.Refusal("held_object_differs_from_received_stream")
                self.directories_current()
                task.record["state"] = "DELIVERY_INTENT"
                self.event("delivery_intent", {"acquisitionId": task.record["acquisitionId"],
                                               "sourceHeldObject": after, "destination": task.record["destination"]})
                self.persist()
                self.hook("before_target_create", task)
                self.directories_current()
                # The create call returns the actual destination handle at birth.
                task.target_fd = os.open(task.record["destination"], base.OPEN_FILE, 0o600, dir_fd=self.dest_fd)
                task.record["deliveryAttempted"] = True
                task.record["targetCreatedObject"] = base.snapshot(task.target_fd)
                self.event("target_handle_created", {"acquisitionId": task.record["acquisitionId"],
                                                     "targetCreatedObject": task.record["targetCreatedObject"]})
                quarantine = ("0001;%x;Traeva-Lab;" % int(time.time())).encode()
                set_attribute(task.target_fd, "com.apple.quarantine", quarantine)
                where_from = plistlib.dumps([task.record["response"]["url"]], fmt=plistlib.FMT_BINARY)
                set_attribute(task.target_fd, "com.apple.metadata:kMDItemWhereFroms", where_from)
                self.hook("after_target_create", task)
                for offset in range(0, len(content), 65536):
                    base.write_all(task.target_fd, content[offset:offset + 65536])
                    self.hook("after_target_chunk", task)
                os.fsync(task.target_fd)
                os.fsync(self.dest_fd)
                self.hook("after_target_write", task)
                self.directories_current()
                current_fd = os.open(task.record["destination"], os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=self.dest_fd)
                try:
                    current = base.snapshot(current_fd)
                    observed_digest, held = base.digest_fd(task.target_fd)
                    if not base.same_object(current, held) or not base.same_object(held, task.record["targetCreatedObject"]):
                        raise base.Refusal("destination_is_different_object")
                    if current != held or observed_digest != digest or held["size"] != len(content):
                        raise base.Refusal("delivered_content_changed")
                    if attribute(task.target_fd, "com.apple.quarantine") != quarantine:
                        raise base.Refusal("download_quarantine_missing_or_changed")
                    if attribute(task.target_fd, "com.apple.metadata:kMDItemWhereFroms") != where_from:
                        raise base.Refusal("download_origin_attribute_missing_or_changed")
                    final = base.snapshot(current_fd)
                    if final != current or base.snapshot(task.target_fd) != held:
                        raise base.Refusal("delivered_object_unstable")
                finally:
                    os.close(current_fd)
                observed_at = time.time_ns()
                self.hook("before_receipt", task)
                task.record.update({"state": "OBSERVATION_RECORDED", "finalObject": held,
                                    "streamSha256": digest, "observedAtNs": observed_at,
                                    "deliveryMethod": "frozen_held_fd_snapshot_to_exclusive_target_fd",
                                    "quarantinePresent": True, "currentPathRequiresRecheck": True})
                receipt = receipt_schema.make(task.record)
                try:
                    self.atomic_json(self.app_data / (task.record["acquisitionId"] + ".receipt.json"), receipt)
                except base.CommitUnknown:
                    task.record["state"] = "OUTCOME_UNKNOWN"
                    return {"ok": False, "outcome": "OUTCOME_UNKNOWN", "error": "receipt_commit_unacknowledged",
                            "acquisitionId": task.record["acquisitionId"], "reconciliationRequired": True}
                # File fsync + replacement + directory fsync is the authority boundary.
                task.record["state"] = "ACQUIRED_OBSERVED_SAME_OBJECT"
                task.record["receiptCommitted"] = True
                warnings = []
                try:
                    self.event("acquisition_observation_saved", {"acquisitionId": task.record["acquisitionId"]})
                except Exception:
                    warnings.append("post_commit_event_failed")
                try:
                    self.persist()
                except Exception:
                    warnings.append("post_commit_status_failed")
                if warnings:
                    task.record["warnings"] = warnings
                return {"ok": True, **dict(task.record)}
            except Exception as error:
                self.refuse(task, error)
                raise

    def recheck(self, payload):
        task = self.get_task(payload)
        if task.record["state"] != "ACQUIRED_OBSERVED_SAME_OBJECT":
            raise base.Refusal("no_completed_live_acquisition")
        try:
            self.directories_current()
            fd = os.open(task.record["destination"], os.O_RDONLY | os.O_NOFOLLOW, dir_fd=self.dest_fd)
            try:
                named = base.snapshot(fd)
                digest, held = base.digest_fd(task.target_fd)
                if not base.same_object(named, held) or not base.same_object(held, task.record["targetCreatedObject"]):
                    raise base.Refusal("destination_is_different_object")
                if named != held or digest != task.record["streamSha256"]:
                    raise base.Refusal("delivered_content_changed")
            finally:
                os.close(fd)
            return {"ok": True, "currentObjectMatchesLiveHandle": True}
        except Exception as error:
            return {"ok": False, "error": base.safe_error(error), "historicalReceiptRetained": True}

    def close(self):
        for task in self.tasks.values():
            if getattr(task, "target_fd", None) is not None:
                os.close(task.target_fd)
                task.target_fd = None
        super().close()


def inspect_restart(run):
    app_data = Path(run) / "app-data"
    receipts, failures = [], []
    for path in app_data.glob("*.receipt.json"):
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
            try:
                before = base.snapshot(fd)
                base.file_kind(before)
                if not 0 < before["size"] <= 128 * 1024:
                    raise ValueError("invalid_receipt_size")
                raw = os.pread(fd, 128 * 1024 + 1, 0)
                if base.snapshot(fd) != before or len(raw) != before["size"]:
                    raise ValueError("receipt_changed_during_read")
            finally:
                os.close(fd)
            record = json.loads(raw, object_pairs_hook=receipt_schema.unique_object)
            receipt_schema.validate(record, path.name)
            receipts.append(path.name)
        except (OSError, ValueError, AttributeError, KeyError, TypeError, base.Refusal):
            failures.append({"file": path.name, "error": "invalid_or_unreadable_historical_receipt"})
    result = {"receipts": receipts, "readFailures": failures, "liveDescriptorContinuity": False,
              "automaticResume": False, "currentAssociation": "UNKNOWN_REQUIRES_NEW_EVIDENCE"}
    result["state"] = "READ_FAILED_UNCERTAIN" if failures else "HISTORICAL_RECEIPT_ONLY" if receipts else "NO_RECEIPT_INCOMPLETE_OR_UNCERTAIN"
    return result
