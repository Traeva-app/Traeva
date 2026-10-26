#!/usr/bin/env python3
"""Redact frozen ZIP evidence offline without running the acquisition lab."""
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import re
import struct
import subprocess
import zipfile


ARCHIVES = ("review-fixes-evidence.zip", "source-verified-evidence.zip")
ROOT = Path(__file__).resolve().parent
PATTERNS = (
    re.compile(r"/(?:Users|home)/[^/\\\s\x00\"'<>;,()]+"),
    re.compile(r"\\/(?:Users|home)\\/[^/\\\s\x00\"'<>;,()]+"),
    re.compile(r"[A-Za-z]:[\\/]Users[\\/][^/\\\s\x00\"'<>;,()]+", re.I),
    re.compile(r"(?:%2f|/)(?:Users|home)(?:%2f|/)(?:(?!%2f)[A-Za-z0-9._~%-])+", re.I),
)
ENCODINGS = ("latin1", "utf-16-le", "utf-16-be")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def redact(data):
    count = 0
    for encoding in ENCODINGS:
        offsets = (0,) if encoding == "latin1" else (0, 1)
        for offset in offsets:
            end = len(data) if encoding == "latin1" else len(data) - (len(data) - offset) % 2
            if end <= offset:
                continue
            text = data[offset:end].decode(encoding, errors="surrogatepass")
            for pattern in PATTERNS:
                text, found = pattern.subn("<HOME>", text)
                count += found
            data = data[:offset] + text.encode(encoding, errors="surrogatepass") + data[end:]
    return data, count


def redact_extra(extra):
    output = bytearray()
    offset = 0
    count = 0
    while offset < len(extra):
        if len(extra) - offset < 4:
            raise ValueError("invalid_extra_field")
        tag, size = struct.unpack_from("<HH", extra, offset)
        end = offset + 4 + size
        if end > len(extra):
            raise ValueError("invalid_extra_field_size")
        payload, found = redact(extra[offset + 4:end])
        output.extend(struct.pack("<HH", tag, len(payload)))
        output.extend(payload)
        count += found
        offset = end
    return bytes(output), count


def local_fields(data, info):
    offset = info.header_offset
    if data[offset:offset + 4] != b"PK\x03\x04":
        raise ValueError("invalid_local_header")
    name_size, extra_size = struct.unpack_from("<HH", data, offset + 26)
    start = offset + 30
    return data[start:start + name_size], data[start + name_size:start + name_size + extra_size]


def check_envelope(data, archive):
    end = len(data) - 22 - len(archive.comment)
    if end < 0 or data[end:end + 4] != b"PK\x05\x06":
        raise ValueError("unparsed_archive_trailer")
    disk, directory_disk, disk_count, total_count, size, offset = struct.unpack_from("<HHHHII", data, end + 4)
    if disk or directory_disk or disk_count != total_count or total_count != len(archive.infolist()):
        raise ValueError("unsupported_archive_directory")
    if offset != archive.start_dir or offset + size != end:
        raise ValueError("unparsed_archive_directory_bytes")
    position = 0
    for info in sorted(archive.infolist(), key=lambda item: item.header_offset):
        if info.header_offset != position or info.flag_bits & 0x8:
            raise ValueError("unparsed_archive_local_bytes")
        name_size, extra_size = struct.unpack_from("<HH", data, position + 26)
        position += 30 + name_size + extra_size + info.compress_size
    if position != archive.start_dir:
        raise ValueError("unparsed_archive_prefix_or_gap")


def rewrite(data, label, report):
    target = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(
        target, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as output:
        if source.testzip() is not None:
            raise ValueError("archive_integrity_failed")
        check_envelope(data, source)
        output.comment, found = redact(source.comment)
        report["found"] += found
        names = set()
        for info in source.infolist():
            local_name, local_extra = local_fields(data, info)
            codec = "utf-8" if info.flag_bits & 0x800 else "cp437"
            if local_name.decode(codec) != info.filename or local_extra != info.extra:
                raise ValueError("local_central_metadata_disagreement")
            item = copy.copy(info)
            name_bytes, name_count = redact(info.filename.encode("utf-8"))
            item.filename = name_bytes.decode("utf-8")
            item.orig_filename = item.filename
            if item.filename in names:
                raise ValueError("duplicate_or_redacted_name_collision")
            names.add(item.filename)
            item.comment, comment_count = redact(info.comment)
            item.extra, extra_count = redact_extra(info.extra)
            report["found"] += name_count + comment_count + extra_count
            before = source.read(info)
            member = label + "!" + item.filename
            if zipfile.is_zipfile(io.BytesIO(before)):
                after = rewrite(before, member, report)
            else:
                if before.startswith(b"PK\x03\x04") or info.filename.lower().endswith(".zip"):
                    raise ValueError("unreadable_nested_zip")
                after, payload_count = redact(before)
                report["found"] += payload_count
                report["leaf_files"] += 1
                if before != after:
                    report["changed_members"].append({
                        "member": member, "found": payload_count,
                        "original_sha256": digest(before), "rewritten_sha256": digest(after),
                    })
                else:
                    report["unchanged_leaf_files"] += 1
            output.writestr(item, after)
    rewritten = target.getvalue()
    report["archives"].append({
        "archive": label, "original_sha256": digest(data),
        "rewritten_sha256": digest(rewritten),
    })
    return rewritten


def scan(data):
    # Scan raw headers/trailers as well as decoded archive surfaces.
    raw_count = redact(data)[1]
    logical_count = 0
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        if archive.testzip() is not None:
            raise ValueError("archive_integrity_failed")
        logical_count += redact(archive.comment)[1]
        for info in archive.infolist():
            local_name, local_extra = local_fields(data, info)
            logical_count += sum(redact(part)[1] for part in (
                info.filename.encode("utf-8"), info.comment, info.extra, local_name, local_extra
            ))
            payload = archive.read(info)
            if zipfile.is_zipfile(io.BytesIO(payload)):
                logical_count += scan(payload)
            else:
                if payload.startswith(b"PK\x03\x04") or info.filename.lower().endswith(".zip"):
                    raise ValueError("unreadable_nested_zip")
                logical_count += redact(payload)[1]
    return raw_count + logical_count


def new_report():
    return {"found": 0, "leaf_files": 0, "unchanged_leaf_files": 0,
            "changed_members": [], "archives": []}


def baseline(ref, name):
    result = subprocess.run(
        ["git", "show", ref + ":experiments/source-verified/" + name],
        cwd=ROOT, capture_output=True, check=False,
    )
    if result.returncode:
        raise ValueError("baseline_read_failed")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("rewrite", "verify", "scan"))
    parser.add_argument("--baseline-ref")
    args = parser.parse_args()
    if args.mode == "scan":
        remaining = sum(scan((ROOT / name).read_bytes()) for name in ARCHIVES)
        print(json.dumps({"remaining": remaining}))
        return int(bool(remaining))
    if not args.baseline_ref or not re.fullmatch(r"[0-9a-f]{40}", args.baseline_ref):
        raise ValueError("exact_baseline_commit_required")
    report = new_report()
    results = {}
    for name in ARCHIVES:
        results[name] = rewrite(baseline(args.baseline_ref, name), name, report)
    remaining = sum(scan(data) for data in results.values())
    if remaining:
        raise ValueError("residual_home_paths")
    report.update({"baseline_commit": args.baseline_ref, "remaining": remaining,
                   "reason": "因含本机路径而改写", "evidence_statuses": "历史状态原样保留；未重新运行实验"})
    if args.mode == "rewrite":
        for name, data in results.items():
            (ROOT / name).write_bytes(data)
        for checksum, name in (("archive-sha256.txt", ARCHIVES[1]), ("review-fixes-sha256.txt", ARCHIVES[0])):
            (ROOT / checksum).write_text(digest(results[name]) + "  " + name + "\n")
        (ROOT / "evidence-privacy-validation.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    else:
        for name, expected in results.items():
            if (ROOT / name).read_bytes() != expected:
                raise ValueError("archive_diff_exceeds_expected_redaction")
        saved = json.loads((ROOT / "evidence-privacy-validation.json").read_text())
        if saved != report:
            raise ValueError("provenance_report_mismatch")
    print(json.dumps({"found": report["found"], "replaced": True,
                      "remaining": remaining, "changed_files": len(report["changed_members"]),
                      "leaf_files": report["leaf_files"], "archives": len(report["archives"])}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, zipfile.BadZipFile):
        raise SystemExit("Evidence privacy check failed; no private diagnostic values displayed.")
