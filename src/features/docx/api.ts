// The thin API for a saved .docx version's local preview (#66). The command
// reads the saved version's checked bytes in the core (never the user's
// file) and returns a DocxPreview; nothing is opened, copied, stored or sent.
// The component checks the result against the snapshot it asked for
// (checkedPreview) and drops a late one, so the signal is not passed on: a
// started read cannot be stopped.

import { invoke } from "@tauri-apps/api/core";
import type { DocxPreview, DocxPreviewApi } from "./preview";

export const previewDocxVersion: DocxPreviewApi = (snapshotId: string) =>
  invoke<DocxPreview>("preview_memory_docx", { snapshot_id: snapshotId });
