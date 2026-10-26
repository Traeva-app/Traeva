import { invoke } from "@tauri-apps/api/core";
import type { SourceApi } from "./types";

export const sourceApi: SourceApi = {
  state: () => invoke("source_state"),
  setEnabled: (enabled) => invoke("source_set_enabled", { enabled }),
  refresh: (sourceId) => invoke("source_refresh", { source_id: sourceId }),
  chooseFile: (sourceId) => invoke("source_choose_file", { source_id: sourceId }),
  confirm: (sourceId, fileId, relation, note) =>
    invoke("source_confirm", { source_id: sourceId, file_id: fileId, relation, note }),
  reject: (sourceId, fileId) =>
    invoke("source_reject", { source_id: sourceId, file_id: fileId }),
  unlink: (sourceId, note) => invoke("source_unlink", { source_id: sourceId, note }),
  openLink: (sourceId, kind) => invoke("source_open_link", { source_id: sourceId, kind }),
  connection: () => invoke("source_connection"),
};
