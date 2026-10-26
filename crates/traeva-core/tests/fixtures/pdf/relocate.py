#!/usr/bin/env python3
"""Moves the reviewed PDF reader experiment into the core, or checks that the
core files are exactly that move (#186).

    python3 crates/traeva-core/tests/fixtures/pdf/relocate.py           # write
    python3 crates/traeva-core/tests/fixtures/pdf/relocate.py --check   # compare

Source: experiments/pdf-reader/src at SOURCE (PR #148, pdf-experiment-9,
Codex-R4 round five LIMITED PASS), read with `git show`, so the commit must
be fetched. Every change is in CHANGES below; nothing else differs:

- each file's `#[cfg(test)]` module (the last item) is left out: those tests
  stay in the experiment with its fixtures;
- module paths: the submodules hang under `pdf` with `#[path]`, and refer to
  the entry module as `super` instead of `crate`;
- the shared text contract is the core's (`memory_contracts`, `reader`), not
  the experiment's copy (`contract.rs`, not moved);
- the reader's name is `pdf-1`;
- `read_pdf` returns the text contract only; the reading with the page table
  is `read_with_pages`;
- the entry module's docs, and the hook for the core's own tests.

The output is formatted with rustfmt (edition 2024), as `cargo fmt` does.
"""

import hashlib
import json
import subprocess
import sys
from pathlib import Path

SOURCE = "81ddd8036d60b14d2d631ad05ae57fa025945625"
ROOT = Path(__file__).resolve().parents[5]
CORE = ROOT / "crates" / "traeva-core" / "src"
FILES = {
    "lib.rs": "pdf.rs",
    "document.rs": "pdf_document.rs",
    "layout.rs": "pdf_layout.rs",
    "preflight.rs": "pdf_preflight.rs",
    "scan.rs": "pdf_scan.rs",
}

CHANGES = {
    "lib.rs": [
        (
            "//! TRAEVA PDF Reader experiment (#28, Claude-G). Not registered in\n"
            "//! traeva-core, Memory, Organize or Tauri.\n",
            "//! The PDF reader for Organize (#186, organize-spec §14): the reviewed\n"
            "//! experiment `experiments/pdf-reader` (PR #148, `81ddd80`) moved into the\n"
            "//! core without changing its logic (`tests/fixtures/pdf/relocate.py`).\n"
            "//! Organize reads PDF only through `pdf_child`, in a separate process;\n"
            "//! Memory does not read PDF.\n",
        ),
        (
            "//! `read_pdf(bytes)` turns one exact set of PDF bytes into the shared\n"
            "//! Memory text contract (copied in `contract.rs`) plus a per-page table.\n",
            "//! `read_with_pages(bytes)` turns one exact set of PDF bytes into the\n"
            "//! shared text contract (`memory_contracts`) plus a per-page table;\n"
            "//! `read_pdf(bytes)` returns the text contract only.\n",
        ),
        (
            "mod contract;\nmod document;\nmod layout;\nmod preflight;\nmod scan;\n",
            '#[path = "pdf_document.rs"]\nmod document;\n'
            '#[path = "pdf_layout.rs"]\nmod layout;\n'
            '#[path = "pdf_preflight.rs"]\nmod preflight;\n'
            '#[path = "pdf_scan.rs"]\nmod scan;\n',
        ),
        (
            "pub use contract::{MAX_MEMORY_TEXT_CHARS, TextExtraction, TextStatus, TextSummary};\n",
            "use crate::memory_contracts::{TextExtraction, TextStatus, TextSummary};\n"
            "use crate::reader::MAX_MEMORY_TEXT_CHARS;\n",
        ),
        (
            "/// Reads one exact set of PDF bytes. A pure function of the bytes and\n"
            "/// `PDF_READER`.\n"
            "pub fn read_pdf(bytes: &[u8]) -> PdfReading {\n",
            "/// The text, status and note of one exact set of PDF bytes: what\n"
            "/// Organize's child process asks for. A pure function of the bytes and\n"
            "/// `PDF_READER`.\n"
            "pub(crate) fn read_pdf(bytes: &[u8]) -> TextExtraction {\n"
            "    read_with_pages(bytes).extraction\n"
            "}\n"
            "\n"
            "/// Reads one exact set of PDF bytes, with the per-page table. A pure\n"
            "/// function of the bytes and `PDF_READER`.\n"
            "pub fn read_with_pages(bytes: &[u8]) -> PdfReading {\n",
        ),
    ],
    "document.rs": [("use crate::", "use super::", 3)],
    "preflight.rs": [("use crate::", "use super::", 2)],
    "scan.rs": [("use crate::", "use super::", 1)],
    "layout.rs": [],
}

# The version notes of the experiment's numbers, replaced as one block.
READER_FROM = "/// Raise the number when the same bytes start producing different text.\n"
READER_TO = 'pub const PDF_READER: &str = "pdf-experiment-9";\n'
READER_NEW = (
    "/// Which reader, and which version of its rules, produced a text: raise\n"
    "/// the number when the same bytes start producing different text. `pdf-1`\n"
    "/// is `pdf-experiment-9` (PR #148, `81ddd80`; Codex-R4 round five, LIMITED\n"
    "/// PASS) moved into the core unchanged.\n"
    'pub const PDF_READER: &str = "pdf-1";\n'
)
TESTS_HOOK = '\n#[cfg(test)]\n#[path = "pdf_tests.rs"]\nmod tests;\n'


def sha256(data):
    return hashlib.sha256(data.encode()).hexdigest()


def source(name):
    done = subprocess.run(
        ["git", "show", f"{SOURCE}:experiments/pdf-reader/src/{name}"],
        cwd=ROOT, capture_output=True, text=True,
    )
    if done.returncode != 0:
        sys.exit(f"git show {SOURCE[:7]}:{name} failed; fetch PR #148's branch first.\n{done.stderr}")
    return done.stdout


def without_tests(text):
    lines = text.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line == "#[cfg(test)]\n"]
    assert len(starts) == 1, "expected exactly one test module"
    start = starts[0]
    assert lines[start + 1].startswith("mod ") and lines[-1] == "}\n", "the test module must be last"
    return "".join(lines[:start]).rstrip("\n") + "\n"


def replace(text, old, new, count=1):
    assert text.count(old) == count, f"{old[:50]!r} found {text.count(old)} times, expected {count}"
    return text.replace(old, new)


def moved(name):
    text = without_tests(source(name))
    for change in CHANGES[name]:
        text = replace(text, *change)
    if name == "lib.rs":
        start, end = text.index(READER_FROM), text.index(READER_TO) + len(READER_TO)
        text = text[:start] + READER_NEW + text[end:]
        text += TESTS_HOOK
    done = subprocess.run(["rustfmt", "--edition", "2024", "--emit", "stdout"],
                          input=text, capture_output=True, text=True)
    assert done.returncode == 0, done.stderr
    return done.stdout


def main():
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] not in ([], ["--check"]):
        sys.exit(__doc__)
    record = {"source_commit": SOURCE, "files": {}}
    same = True
    for name, target in FILES.items():
        text = moved(name)
        path = CORE / target
        record["files"][target] = {
            "from": f"experiments/pdf-reader/src/{name}",
            "source_sha256": sha256(source(name)),
            "core_sha256": sha256(text),
        }
        if check:
            current = path.read_text() if path.exists() else None
            record["files"][target]["identical"] = current == text
            same &= current == text
        else:
            path.write_text(text)
    print(json.dumps(record, indent=1))
    if check and not same:
        sys.exit("the core files differ from the moved experiment")


if __name__ == "__main__":
    main()
