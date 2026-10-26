// The thin API for files dropped on Traeva's icon (#168). Poll
// getDropQueue while a file is waiting or being worked on; after answering
// a dropped file's question (answer_clarification) or finishing one in the
// normal flow, call continueDrops.

import { invoke } from "@tauri-apps/api/core";
import type { OperationView } from "../organize/types";
import type { AutoDropRequest, AutoDropSettingsView, DropQueueView } from "./types";

export function getAutoDropSettings(): Promise<AutoDropSettingsView> {
  return invoke<AutoDropSettingsView>("get_auto_drop_settings");
}

export function setAutoDropSettings(request: AutoDropRequest): Promise<AutoDropSettingsView> {
  return invoke<AutoDropSettingsView>("set_auto_drop_settings", { request });
}

export function getDropQueue(): Promise<DropQueueView> {
  return invoke<DropQueueView>("get_drop_queue");
}

export function authorizeDropSource(dropId: string): Promise<DropQueueView> {
  return invoke<DropQueueView>("authorize_drop_source", { drop_id: dropId });
}

export function cancelDrop(dropId: string): Promise<DropQueueView> {
  return invoke<DropQueueView>("cancel_drop", { drop_id: dropId });
}

export function continueDrops(): Promise<DropQueueView> {
  return invoke<DropQueueView>("continue_drops");
}

/** The mode off: start the normal operation for a dropped file. */
export function startDropOperation(dropId: string, targetRootId: string): Promise<OperationView> {
  return invoke<OperationView>("start_drop_operation", {
    drop_id: dropId,
    target_root_id: targetRootId,
  });
}
