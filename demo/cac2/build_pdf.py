#!/usr/bin/env python3
"""Rebuilds files/Inbox/museum-visit.pdf, the demo PDF with a text layer
(reportlab 5.0.1, invariant mode: the same bytes every time). Not needed
to set up the demo: the PDF is committed and checked by SHA256SUMS.txt.

    python3 demo/cac2/build_pdf.py
"""
from pathlib import Path

from reportlab import rl_config
from reportlab.lib.pagesizes import A4
from reportlab.pdfgen import canvas

rl_config.invariant = 1

LINES = [
    (18, "Field trip: City History Museum"),
    (13, "History - Unit 4: The Industrial Revolution"),
    (11, "Date: Friday 14 November. Meet at the front gate at 8:45."),
    (11, "9:00  Leave school by coach"),
    (11, "10:00 Gallery tour: steam engines and the first textile mills"),
    (11, "12:00 Lunch in the museum garden (bring a packed lunch)"),
    (11, "13:00 Workshop: a day in the life of a child worker in 1830"),
    (11, "15:00 Coach back to school"),
    (11, "Task: take notes on two machines and one working condition"),
    (11, "for your Unit 4 essay."),
    (11, "Permission: please return the signed slip to the history"),
    (11, "department by Wednesday."),
]


def main():
    out = Path(__file__).resolve().parent / "files" / "Inbox" / "museum-visit.pdf"
    c = canvas.Canvas(str(out), pagesize=A4)
    y = A4[1] - 72
    for size, text in LINES:
        c.setFont("Helvetica-Bold" if size > 12 else "Helvetica", size)
        c.drawString(64, y, text)
        y -= size * 1.8
    c.showPage()
    c.save()
    print(f"written {out}")


if __name__ == "__main__":
    main()
