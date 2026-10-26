"""Scrub synthetic Word fixtures without changing ZIP layout or other entries."""
import argparse
import hashlib
import json
import random
import re
import struct
import zlib
from pathlib import Path


def scrub(source, destination):
    original = source.read_bytes()
    data = bytearray(original)
    end = len(data) - 22
    assert data[end:end + 4] == b"PK\x05\x06"
    assert struct.unpack_from("<H", data, end + 20)[0] == 0
    central = struct.unpack_from("<I", data, end + 16)[0]
    at = central
    while at < end:
        name_len, extra_len, comment_len = struct.unpack_from("<HHH", data, at + 28)
        name = bytes(data[at + 46:at + 46 + name_len])
        if name == b"docProps/core.xml":
            break
        at += 46 + name_len + extra_len + comment_len
    assert at < end
    flags, method = struct.unpack_from("<HH", data, at + 8)
    packed, unpacked = struct.unpack_from("<II", data, at + 20)
    local = struct.unpack_from("<I", data, at + 42)[0]
    local_name, local_extra = struct.unpack_from("<HH", data, local + 26)
    start = local + 30 + local_name + local_extra
    assert method == 8 and flags & 8 == 0
    core = zlib.decompress(data[start:start + packed], -15)
    assert len(core) == unpacked
    pattern = re.compile(rb"(<(?:dc:creator|cp:lastModifiedBy)>)(.*?)(</[^>]+>)")
    author_fields = len(pattern.findall(core))
    assert author_fields >= 1
    rng = random.Random(66)
    alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
    found = None
    compression_mode = "single_deflate_stream"
    sync_flush_split = None
    for trial in range(10000):
        def replace(match):
            value = match[2]
            replacement = bytes(rng.choice(alphabet) for _ in value)
            return match[1] + replacement + match[3]

        clean = pattern.sub(replace, core)
        assert len(clean) == len(core)
        for level in range(1, 10):
            for strategy in [zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED]:
                compressor = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
                compressed = compressor.compress(clean) + compressor.flush()
                if len(compressed) == packed:
                    found = clean, compressed
                    break
            if found:
                break
        if found:
            break
    if found is None:
        # A valid intermediate flush can match Word's less compact Deflate output.
        for split in range(1, len(clean)):
            compressor = zlib.compressobj(6, zlib.DEFLATED, -15)
            compressed = (compressor.compress(clean[:split])
                          + compressor.flush(zlib.Z_SYNC_FLUSH)
                          + compressor.compress(clean[split:])
                          + compressor.flush())
            if len(compressed) == packed:
                found = clean, compressed
                compression_mode = "intermediate_sync_flush"
                sync_flush_split = split
                break
    if found is None:
        raise RuntimeError("Cannot preserve compressed length; do not repack")
    clean, compressed = found
    assert zlib.decompress(compressed, -15) == clean
    crc = zlib.crc32(clean)
    data[start:start + packed] = compressed
    struct.pack_into("<I", data, local + 14, crc)
    struct.pack_into("<I", data, at + 16, crc)
    allowed = [(start, start + packed), (local + 14, local + 18), (at + 16, at + 20)]
    assert len(data) == len(original)
    assert all(a == b or any(lo <= i < hi for lo, hi in allowed)
               for i, (a, b) in enumerate(zip(original, data)))
    destination.write_bytes(data)
    return {"source_sha256": hashlib.sha256(original).hexdigest(),
            "fixture_sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data), "central_offset": central,
            "allowed_changed_ranges": allowed, "core_packed": packed,
            "core_unpacked": unpacked, "trial": trial,
            "author_fields_replaced": author_fields,
            "compression_mode": compression_mode,
            "sync_flush_split": sync_flush_split,
            "other_entry_bytes_and_layout_identical": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(json.dumps(scrub(args.source, args.destination), indent=2))
