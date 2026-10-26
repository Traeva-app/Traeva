// Memory's calls to Rust (#33, #34, #35). Each function is one Tauri command
// in src-tauri/src/memory_commands.rs; a rejected Promise carries an
// AppError from the core. Argument names are snake_case, like the Rust side.

import { invoke } from "@tauri-apps/api/core";
import type {
  ChangeLinesView,
  ExportedVersion,
  ExportPreview,
  ExportRequest,
  MemoryConsentView,
  OpenedVersion,
  SearchAnswer,
  SearchView,
  VersionPairView,
} from "./types";

export function getMemoryConsent(fileId: string): Promise<MemoryConsentView> {
  return invoke<MemoryConsentView>("get_memory_consent", { file_id: fileId });
}

/** Agrees to (or withdraws) sending this file's changed lines to the model. */
export function setMemoryConsent(fileId: string, agree: boolean): Promise<MemoryConsentView> {
  return invoke<MemoryConsentView>("set_memory_consent", { file_id: fileId, agree });
}

/** Every pair of consecutive saved versions, oldest first. */
export function getMemoryChanges(fileId: string): Promise<VersionPairView[]> {
  return invoke<VersionPairView[]>("get_memory_changes", { file_id: fileId });
}

export function getChangeLines(changeId: string): Promise<ChangeLinesView> {
  return invoke<ChangeLinesView>("get_change_lines", { change_id: changeId });
}

/** Starts a search; the model is asked in the background. Poll getMemorySearch. */
export function startMemorySearch(fileId: string, question: string): Promise<SearchView> {
  return invoke<SearchView>("start_memory_search", { file_id: fileId, question });
}

export function getMemorySearch(searchId: string): Promise<SearchView> {
  return invoke<SearchView>("get_memory_search", { search_id: searchId });
}

export function answerMemorySearch(searchId: string, answer: SearchAnswer): Promise<SearchView> {
  return invoke<SearchView>("answer_memory_search", { search_id: searchId, answer });
}

export function retryMemorySearch(searchId: string): Promise<SearchView> {
  return invoke<SearchView>("retry_memory_search", { search_id: searchId });
}

export function cancelMemorySearch(searchId: string): Promise<SearchView> {
  return invoke<SearchView>("cancel_memory_search", { search_id: searchId });
}

/** Opens a new read-only copy of a saved version, never the user's file. */
export function openMemoryVersion(snapshotId: string): Promise<OpenedVersion> {
  return invoke<OpenedVersion>("open_memory_version", { snapshot_id: snapshotId });
}

/** Opens the native save panel in root `rootId` with `name` filled in;
 * null when it was closed. Nothing is written. */
export function chooseMemoryExport(snapshotId: string, rootId: string, name: string): Promise<ExportPreview | null> {
  return invoke<ExportPreview | null>("choose_memory_export", { snapshot_id: snapshotId, root_id: rootId, name });
}

export function exportMemoryVersion(request: ExportRequest): Promise<ExportedVersion> {
  return invoke<ExportedVersion>("export_memory_version", { request });
}
