// Letting the model look at an image (organize-spec §14.10, the student's
// decision 10/14): before Suggest a folder, what happens about the image.
// The setting is the user's (Settings → Model); the core sends a copy only
// when it allows it, and never with the setting off.

import type { FileView, ImageSending } from "./types.ts";

/** Whether the model could look at an image of this file: the core read it
 * as one (a JPEG or PNG, or a PDF with no text layer). */
export function hasImage(file: FileView): boolean {
  return file.extraction.image_input;
}

/** What Suggest a folder does about the image: ask first (the setting asks
 * each time), send it without asking, or nothing (the setting is off, or
 * the file has no image). */
export function imageStep(mode: ImageSending, file: FileView): "ask" | "send" | "none" {
  if (!hasImage(file) || mode === "off") return "none";
  return mode === "ask_each_time" ? "ask" : "send";
}

/** The setting to keep when the user ticks "Don't ask again": the choice
 * they just made, from now on. */
export function rememberedMode(send: boolean): ImageSending {
  return send ? "without_asking" : "off";
}
