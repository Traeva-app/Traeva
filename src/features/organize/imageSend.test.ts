// Tests for imageSend.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { hasImage, imageStep, rememberedMode } from "./imageSend.ts";
import type { FileView } from "./types.ts";

function file(path: string, imageInput = false): FileView {
  return {
    file_id: "f",
    file_revision: 1,
    root_id: "r",
    relative_path: path,
    display_path: `/R/${path}`,
    byte_length: 1,
    sha256: "00",
    extraction: {
      status: imageInput ? "partial" : "ok",
      total_chars: 10,
      truncated: false,
      blank: false,
      usable_text: true,
      note: null,
      recognized: imageInput,
      image_input: imageInput,
    },
  };
}

test("a file has an image when the core read it as one", () => {
  assert.equal(hasImage(file("Inbox/board.JPG", true)), true);
  assert.equal(hasImage(file("Inbox/scan.pdf", true)), true);
  // The name alone does not count: the core decides what it read as an image.
  assert.equal(hasImage(file("Inbox/receipt.png")), false);
  assert.equal(hasImage(file("Inbox/notes.md")), false);
});

test("Suggest a folder asks, sends or does nothing about the image, by the setting", () => {
  const photo = file("Inbox/board.jpg", true);
  assert.equal(imageStep("ask_each_time", photo), "ask");
  assert.equal(imageStep("without_asking", photo), "send");
  assert.equal(imageStep("off", photo), "none");
  // No image: nothing to ask, whatever the setting.
  assert.equal(imageStep("ask_each_time", file("Inbox/notes.md")), "none");
  assert.equal(imageStep("without_asking", file("Inbox/notes.md")), "none");
});

test("Don't ask again keeps the choice just made", () => {
  assert.equal(rememberedMode(true), "without_asking");
  assert.equal(rememberedMode(false), "off");
});
