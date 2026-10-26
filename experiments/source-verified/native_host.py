#!/usr/bin/env python3
import argparse
import hmac
import json
import os
from pathlib import Path
import struct
import sys

from writer import Writer
from base_receiver import safe_error

MAX_MESSAGE = 2 * 1024 * 1024


def exact(stream, count):
    parts = []
    remaining = count
    while remaining:
        part = stream.read(remaining)
        if not part:
            if remaining == count and count == 4:
                return None
            raise ValueError("incomplete_native_message")
        parts.append(part)
        remaining -= len(part)
    return b"".join(parts)


def read(stream):
    header = exact(stream, 4)
    if header is None:
        return None
    size = struct.unpack("=I", header)[0]
    if not 0 < size <= MAX_MESSAGE:
        raise ValueError("native_message_size_invalid")
    message = json.loads(exact(stream, size))
    if not isinstance(message, dict):
        raise ValueError("native_json_object_required")
    return message


def reply(value):
    payload = json.dumps(value, separators=(",", ":")).encode()
    if len(payload) > 1024 * 1024:
        raise ValueError("native_reply_too_large")
    sys.stdout.buffer.write(struct.pack("=I", len(payload)) + payload)
    sys.stdout.buffer.flush()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    args, launch_args = parser.parse_known_args()
    config_path = Path(args.config)
    config = json.loads(config_path.read_text())
    if config_path.stat().st_mode & 0o077:
        raise ValueError("private_config_required")
    origins = [x for x in launch_args if x.startswith("chrome-extension://")]
    if origins != [config["extensionOrigin"]]:
        raise ValueError("native_extension_origin_rejected")
    lab = None
    try:
        while True:
            message = read(sys.stdin.buffer)
            if message is None:
                break
            rid = message.get("rid")
            token = message.get("auth")
            if not isinstance(token, str) or not hmac.compare_digest(token, config["token"]):
                reply({"rid": rid, "ok": False, "error": "native_authentication_required"})
                continue
            try:
                command, payload = message.get("command"), message.get("payload", {})
                if not isinstance(payload, dict):
                    raise ValueError("payload_object_required")
                if command == "hello":
                    result = {"ok": True, "pid": os.getpid(), "authorized": bool(lab and lab.authorized),
                              "delivery": str(Path(config["run"]) / "delivery")}
                elif command == "authorize":
                    if payload.get("takeoverConsent") is not True or payload.get("saveLocationConsent") != str(Path(config["run"]) / "delivery"):
                        raise ValueError("explicit_mode_and_location_consent_required")
                    if lab is None:
                        lab = Writer(Path(config["run"]), config["origins"])
                    result = lab.authorize(payload)
                elif command in ("prepare", "begin", "chunk", "complete", "abort", "recheck", "status"):
                    if lab is None:
                        raise ValueError("authorization_required")
                    result = lab.status() if command == "status" else getattr(lab, command)(payload)
                else:
                    raise ValueError("native_command_not_allowed")
                reply({"rid": rid, **result})
            except Exception as error:
                reply({"rid": rid, "ok": False, "error": safe_error(error)})
    finally:
        if lab is not None:
            for task in lab.tasks.values():
                if task.record["state"] == "RECEIVING":
                    lab.refuse(task, ValueError("native_port_closed_during_receive"))
            lab.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(safe_error(error), file=sys.stderr)
        raise SystemExit(2)
