# Synthetic images for image text recognition (#37)

Drawn by `experiments/image-ocr/make_images.py` (Pillow 11.3.0; the same bytes each run). No photos and no personal data.

| File | What it checks |
|---|---|
| `text-plain.png` | Text Vision reads (opt-in `tests/ocr_image.rs`) |
| `text-exif6.jpg` | Stored a quarter turn to the left; EXIF orientation 6 turns it upright |
| `text-meta.jpg` | Visible text, plus made-up EXIF tags (`FAKE-...`) and GPS 0,0 that must never reach the text or the note |
| `text-alpha.png` | Transparent background |
| `block-exif6.jpg`, `block-gray16.png`, `block-cmyk.jpg`, `block-two-frames.png` | A 60 x 40 white picture with a black block at the top left: orientation, 16-bit grey, CMYK, and the first frame of two |
| `white-12mp.png` | 12 million white pixels: scaled down to the 8-million budget, no text |
| `truncated.png`, `truncated.jpg` | The first half of a good file: damaged |
