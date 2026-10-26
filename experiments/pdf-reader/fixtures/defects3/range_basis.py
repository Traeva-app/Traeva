#!/usr/bin/env python3
"""Independent basis for where each content or form stream's data ends in
fixtures/defects3/generated/, written to fixtures/defects3/range_basis.json.

Two readings, neither of them this reader's code:

1. hayro 0.8.0's own rule (hayro-syntax src/object/stream.rs), written again
   here from its source. `parse_proper` (lines 299-312): after the
   dictionary, white space and comments, `stream`, then exactly one end of
   line (LF, CR LF or CR); the data is `/Length` bytes (a direct number, or
   the integer an indirect reference resolves to); then white space only
   and `endstream`. If that fails, `parse_fallback` (lines 314-338): the
   first `stream` after the dictionary, one end of line, then everything up
   to the first `endstream`, trailing white space removed. Numbers follow
   hayro's grammar (sign, digits, one point; a real is cut to an integer;
   it must fit a u32). White space is NUL, TAB, LF, FF, CR and space.
2. pypdf (non-strict) reading the same object: the length of the data it
   took, and the page's text as pypdf extracts it.

Indirect /Length values are resolved with pypdf. Only objects pypdf lists
at a file offset are read (not members of object streams).

    python3 -I fixtures/defects3/range_basis.py
"""

import hashlib
import json
import logging
import sys
from pathlib import Path

import pypdf

HERE = Path(__file__).resolve().parent
WHITE = b"\x00\t\n\x0c\r "
DELIMITERS = b"()<>[]{}/%"


def skip_white(data, i, comments=True):
    while i < len(data):
        if data[i] in WHITE:
            i += 1
        elif comments and data[i] == ord("%"):
            while i < len(data) and data[i] not in b"\r\n":
                i += 1
        else:
            break
    return i


def skip_value(data, i):
    """The index after the PDF object at i (strings, hex strings, names,
    numbers, references, arrays, dictionaries, keywords)."""
    c = data[i]
    if c == ord("("):
        depth, j = 0, i
        while True:
            b = data[j]
            if b == ord("\\"):
                j += 2
                continue
            if b == ord("("):
                depth += 1
            elif b == ord(")"):
                depth -= 1
                if depth == 0:
                    return j + 1
            j += 1
    if data.startswith(b"<<", i):
        j = skip_white(data, i + 2)
        while not data.startswith(b">>", j):
            j = skip_white(data, skip_value(data, j))
        return j + 2
    if c == ord("<"):
        return data.index(b">", i) + 1
    if c == ord("["):
        j = skip_white(data, i + 1)
        while data[j] != ord("]"):
            j = skip_white(data, skip_value(data, j))
        return j + 1
    if c == ord("/"):
        j = i + 1
        while j < len(data) and data[j] not in WHITE and data[j] not in DELIMITERS:
            j += 1
        return j
    j = i + 1
    while j < len(data) and data[j] not in WHITE and data[j] not in DELIMITERS:
        j += 1
    return j


def hayro_u32(token):
    """hayro's number grammar for `dict.get::<u32>`; None if it is not one."""
    text = token.decode("latin-1")
    sign = 1
    if text[:1] in "+-":
        sign = -1 if text[0] == "-" else 1
        text = text[1:]
    whole, _, fraction = text.partition(".")
    if not (whole + fraction).isdigit() or text.count(".") > 1:
        return None
    value = sign * int(whole or "0")
    return value if 0 <= value <= 0xFFFFFFFF else None


def dictionary(data, start):
    """(end of the dictionary, {key: raw value bytes}) for the `<<` at start."""
    values = {}
    j = skip_white(data, start + 2)
    while not data.startswith(b">>", j):
        key_end = skip_value(data, j)
        key = data[j + 1:key_end]
        value_start = skip_white(data, key_end)
        value_end = skip_value(data, value_start)
        if data[value_start:value_start + 1].isdigit():
            # `12 0 R`
            k = skip_white(data, value_end)
            k2 = skip_value(data, k) if k < len(data) else k
            r = skip_white(data, k2)
            if data[k:k2].isdigit() and data[r:r + 1] == b"R":
                value_end = r + 1
        values[key] = data[value_start:value_end]
        j = skip_white(data, value_end)
    return j + 2, values


def eol_after(data, i):
    for eol in (b"\n", b"\r\n", b"\r"):
        if data.startswith(eol, i):
            return i + len(eol)
    return None


def hayro_range(data, offset, resolve):
    """(rule, data start, data length) by hayro's rule for the object at offset."""
    i = skip_white(data, offset)
    for _ in range(2):  # object number and generation
        i = skip_white(data, skip_value(data, i))
    assert data.startswith(b"obj", i)
    start = skip_white(data, i + 3)
    end, values = dictionary(data, start)
    raw = values.get(b"Length")
    length = None
    if raw is not None:
        parts = raw.split()
        if len(parts) == 3 and parts[2] == b"R":
            length = resolve(int(parts[0]))
        else:
            length = hayro_u32(raw)
    k = skip_white(data, end)
    if length is not None and data.startswith(b"stream", k):
        begin = eol_after(data, k + 6)
        if begin is not None and begin + length <= len(data):
            after = skip_white(data, begin + length, comments=False)
            if data.startswith(b"endstream", after):
                return "proper", begin, length
    k = data.find(b"stream", end)
    begin = eol_after(data, k + 6) if k >= 0 else None
    if begin is None:
        return "none", None, None
    stop = data.find(b"endstream", begin)
    if stop < 0:
        return "none", None, None
    while stop > begin and data[stop - 1] in WHITE:
        stop -= 1
    return "fallback", begin, stop - begin


def streams_of(reader):
    """Every content and form stream the pages use, by object number."""
    numbers = []
    for page in reader.pages:
        contents = page.get_object().get("/Contents")
        refs = contents if isinstance(contents, pypdf.generic.ArrayObject) else [contents]
        for ref in refs:
            if isinstance(ref, pypdf.generic.IndirectObject):
                numbers.append(ref.idnum)
        resources = page.get_object().get("/Resources")
        xobjects = resources.get_object().get("/XObject") if resources is not None else None
        if xobjects is not None:
            for ref in xobjects.get_object().values():
                if isinstance(ref, pypdf.generic.IndirectObject):
                    numbers.append(ref.idnum)
    return numbers


def basis(path):
    data = path.read_bytes()
    messages = []

    class Keep(logging.Handler):
        def emit(self, record):
            messages.append(record.getMessage())

    handler = Keep()
    logging.getLogger("pypdf").addHandler(handler)
    try:
        reader = pypdf.PdfReader(str(path), strict=False)
        offsets = {num: off for gen in reader.xref.values() for num, off in gen.items()}

        def resolve(number):
            value = reader.get_object(number)
            return int(value) if isinstance(value, (int, pypdf.generic.NumberObject)) else None

        streams = {}
        for number in streams_of(reader):
            entry = {}
            if number in offsets:
                rule, begin, length = hayro_range(data, offsets[number], resolve)
                entry["hayro_rule"] = rule
                entry["hayro_data_start"] = begin
                entry["hayro_data_length"] = length
                if begin is not None:
                    entry["hayro_data_sha256"] = hashlib.sha256(data[begin:begin + length]).hexdigest()
            else:
                entry["hayro_rule"] = "not at a file offset (object stream member)"
            raw = reader.get_object(number)._data
            entry["pypdf_data_length"] = len(raw)
            entry["pypdf_data_sha256"] = hashlib.sha256(raw).hexdigest()
            streams[str(number)] = entry
        text = [page.extract_text() for page in reader.pages]
    finally:
        logging.getLogger("pypdf").removeHandler(handler)
    return {
        "sha256": hashlib.sha256(data).hexdigest(),
        "streams": streams,
        "pypdf_page_text": text,
        "pypdf_messages": sorted(set(messages)),
    }


def main():
    names = sys.argv[1:] or [f"g{n}" for n in range(14, 18)]
    manifest = json.loads((HERE / "manifest.json").read_text())["files"]
    out = {
        "basis": "hayro-syntax 0.8.0 src/object/stream.rs parse_proper/parse_fallback, re-implemented here; "
                 f"pypdf {pypdf.__version__} (strict=False)",
        "fixtures": {},
    }
    for name in sorted(manifest):
        if not any(name.startswith(prefix + "-") for prefix in names):
            continue
        out["fixtures"][name] = basis(HERE / "generated" / name)
    (HERE / "range_basis.json").write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n")
    for name, found in out["fixtures"].items():
        for number, entry in found["streams"].items():
            print(name, number, entry["hayro_rule"], entry.get("hayro_data_length"), entry["pypdf_data_length"])
    print("written fixtures/defects3/range_basis.json")


if __name__ == "__main__":
    main()
