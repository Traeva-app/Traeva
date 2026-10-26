#!/usr/bin/env python3
"""N2 reproduction inputs: hayro's own super-linear ObjStm work, reached by a
direct read_pdf (the probe runs pdf.objects() twice in object_facts).

objstm-<n>.pdf holds one object stream with <n> trivial members. hayro rebuilds
the object-stream offset table on every member fetch, so iterating all members
is quadratic in <n>. The separate repair-scan literal-string quadratic is NOT
emitted here: the construction that triggers it (a broken startxref) is refused
by preflight check_end before hayro runs; see HANDOFF-round6.md.

    python3 evidence/round6/repro/hayro_cost_inputs.py <out-dir>
    <probe> --text <out-dir>/objstm-16000.pdf     # ~seconds; grows with n^2
"""
import pathlib, struct, sys
ROOT = pathlib.Path(__file__).resolve().parents[3]

def objstm(n):
    members = [b"<< /M %d >>" % i for i in range(n)]
    offs, body = [], b""
    for m in members:
        offs.append(len(body)); body += m + b"\n"
    head = b" ".join(b"%d %d" % (8 + i, offs[i]) for i in range(n)) + b"\n"
    data = head + body
    out = bytearray(b"%PDF-1.7\n")
    pos = {}
    def add(num, content):
        pos[num] = len(out); out.extend(b"%d 0 obj\n" % num + content + b"\nendobj\n")
    add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
           b"/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>")
    c = b"BT /F1 12 Tf 72 700 Td (ObjStm cost) Tj ET"
    add(4, b"<< /Length %d >>\nstream\n" % len(c) + c + b"\nendstream")
    add(5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    add(6, b"<< /Type /ObjStm /N %d /First %d /Length %d >>\nstream\n" % (n, len(head), len(data))
         + data + b"\nendstream")
    size = 8 + n
    rows = b""
    for num in range(size):
        if num == 0: rows += struct.pack(">BIH", 0, 0, 65535)
        elif num in pos: rows += struct.pack(">BIH", 1, pos[num], 0)
        elif num == 7: rows += struct.pack(">BIH", 1, len(out), 0)
        else: rows += struct.pack(">BIH", 2, 6, num - 8)
    xref_at = len(out)
    out.extend(b"7 0 obj\n<< /Type /XRef /Size %d /W [1 4 2] /Root 1 0 R /Length %d >>\nstream\n"
               % (size, len(rows)) + rows + b"\nendstream\nendobj\n")
    out.extend(b"startxref\n%d\n%%%%EOF\n" % xref_at)
    return bytes(out)

if __name__ == "__main__":
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out.mkdir(parents=True, exist_ok=True)
    for n in (1000, 4000, 16000, 64000):
        (out / f"objstm-{n}.pdf").write_bytes(objstm(n))
    print(f"wrote ObjStm cost inputs to {out}")
