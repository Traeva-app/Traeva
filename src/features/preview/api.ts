// The thin API for a saved version's local preview, whatever its type
// (contract §4.7, v23). The command reads the saved version's checked bytes
// in the core (never the user's file) and returns { snapshot, extraction },
// the shape of DocxPreview; nothing is opened, copied, stored or sent. Check
// the result against the snapshot asked for and drop a late one: a started
// read cannot be stopped.

import { invoke } from "@tauri-apps/api/core";
import type { DocxPreview } from "../docx/preview";

export type VersionPreview = DocxPreview;

export function previewVersionText(snapshotId: string): Promise<VersionPreview> {
  return invoke<VersionPreview>("preview_memory_text", { snapshot_id: snapshotId });
}
