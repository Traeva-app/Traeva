#!/usr/bin/env python3
"""Makes the synthetic images for image text recognition's tests (#37,
#186; the student's decision of 2026-10-14) in
crates/traeva-core/tests/fixtures/images/. Every image is drawn here: no
photo, no personal data. The EXIF tags of text-meta.jpg are made up on
purpose (GPS 0,0 and FAKE-... values): the tests check that none of them
reaches the recognized text or the note.

    python3 experiments/image-ocr/make_images.py

Needs Pillow (made with 11.3.0). Writes the same bytes each time with the
same Pillow.
"""

import hashlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parents[2] / "crates/traeva-core/tests/fixtures/images"

# The made-up EXIF tags of text-meta.jpg; the tests look for these words.
FAKE_TAGS = {
    0x010F: "FAKE-CAMERA-MAKE",          # Make
    0x0110: "FAKE-CAMERA-MODEL",         # Model
    0x013B: "FAKE-ARTIST-NAME",          # Artist
    0x010E: "FAKE-IMAGE-DESCRIPTION",    # ImageDescription
    0x8298: "FAKE-COPYRIGHT-HOLDER",     # Copyright
    0x0131: "FAKE-SOFTWARE-NAME",        # Software
}
GPS_IFD = 0x8825
FAKE_GPS = {
    1: "N", 2: (0.0, 0.0, 0.0),          # latitude 0
    3: "E", 4: (0.0, 0.0, 0.0),          # longitude 0
    27: b"ASCII\0\0\0FAKE-GPS-AREA",     # GPSAreaInformation
}


def text_image(lines, size=(900, 320), mode="RGB", background="white"):
    image = Image.new(mode, size, background)
    draw = ImageDraw.Draw(image)
    font = ImageFont.load_default(size=44)
    for row, line in enumerate(lines):
        draw.text((40, 40 + row * 80), line, fill="black", font=font)
    return image


def save(name, image, **options):
    path = OUT / name
    image.save(path, **options)
    return path


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    written = []

    # Text for the real recognizer.
    plain = text_image(["Field notes for the image test", "Invoice number 4471", "Total due 128.50"])
    written.append(save("text-plain.png", plain, optimize=True))

    # Upright text stored a quarter turn to the left; EXIF orientation 6
    # turns it back.
    upright = text_image(["Rotated text sample", "Orientation six"])
    exif = Image.Exif()
    exif[0x0112] = 6
    written.append(save("text-exif6.jpg", upright.transpose(Image.Transpose.ROTATE_90), quality=90, exif=exif))

    # Visible text plus made-up EXIF tags and GPS 0,0.
    meta = text_image(["Library card renewal", "Due back on Tuesday"])
    exif = Image.Exif()
    for tag, value in FAKE_TAGS.items():
        exif[tag] = value
    exif.get_ifd(GPS_IFD).update(FAKE_GPS)
    written.append(save("text-meta.jpg", meta, quality=90, exif=exif))

    # Transparent background, black text.
    alpha = text_image(["Transparent background"], size=(700, 160), mode="RGBA", background=(0, 0, 0, 0))
    written.append(save("text-alpha.png", alpha))

    # Pixel checks: a black block at the top left of a white picture.
    def block(size, mode="L", white=255, black=0):
        image = Image.new(mode, size, white)
        image.paste(black, (0, 0, 10, 10))
        return image

    # 60 x 40 upright; stored as 40 x 60 with orientation 6.
    exif = Image.Exif()
    exif[0x0112] = 6
    written.append(save("block-exif6.jpg", block((60, 40)).transpose(Image.Transpose.ROTATE_90), quality=95, exif=exif))
    written.append(save("block-gray16.png", block((60, 40), mode="I;16", white=65535)))
    written.append(save("block-cmyk.jpg", block((60, 40), mode="CMYK", white=(0, 0, 0, 0), black=(0, 0, 0, 255)), quality=95))

    # Two frames: the first white with the block, the second all black.
    first = block((60, 40), mode="RGB", white=(255, 255, 255), black=(0, 0, 0))
    second = Image.new("RGB", (60, 40), "black")
    written.append(save("block-two-frames.png", first, save_all=True, append_images=[second], duration=100, loop=0))

    # 12 million white pixels: decoded scaled down to the 8-million budget.
    written.append(save("white-12mp.png", Image.new("L", (4000, 3000), 255), optimize=True))

    # Damaged: the first half of a good file.
    for good, name in (("text-plain.png", "truncated.png"), ("text-meta.jpg", "truncated.jpg")):
        data = (OUT / good).read_bytes()
        (OUT / name).write_bytes(data[: len(data) // 2])
        written.append(OUT / name)

    for path in written:
        data = path.read_bytes()
        print(f"{hashlib.sha256(data).hexdigest()}  {len(data):7d}  {path.name}")


if __name__ == "__main__":
    main()
