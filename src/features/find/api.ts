// Find's calls to Rust (#198). Each function is one Tauri command in
// src-tauri/src/find_commands.rs; a rejected Promise carries an AppError
// from the core. Argument names are snake_case, like the Rust side.

import { invoke } from "@tauri-apps/api/core";
import type { FindSetupView, FindView } from "./types";

export function getFindSetup(): Promise<FindSetupView> {
  return invoke<FindSetupView>("get_find_setup");
}

/** Agrees to (or withdraws) Find sending the typed question to the model. */
export function setFindConsent(agree: boolean): Promise<FindSetupView> {
  return invoke<FindSetupView>("set_find_consent", { agree });
}

/** Starts a search (any earlier one still working is cancelled). Poll getFind. */
export function startFind(question: string): Promise<FindView> {
  return invoke<FindView>("start_find", { question });
}

export function getFind(findId: string): Promise<FindView> {
  return invoke<FindView>("get_find", { find_id: findId });
}

/** The files opened in the last 30 days, newest first, at most 20 (#198):
 * a finished search (state "done", no words), so its results open and show
 * in Finder with openFindResult / revealFindResult. No polling. */
export function getRecentFiles(): Promise<FindView> {
  return invoke<FindView>("get_recent_files");
}

export function cancelFind(findId: string): Promise<FindView> {
  return invoke<FindView>("cancel_find", { find_id: findId });
}

/** Shows the result in Finder, only if it is still the file the search saw. */
export function revealFindResult(findId: string, resultId: string): Promise<void> {
  return invoke<void>("reveal_find_result", { find_id: findId, result_id: resultId });
}

/** Opens the result in its usual app, only if it is still the file the search saw. */
export function openFindResult(findId: string, resultId: string): Promise<void> {
  return invoke<void>("open_find_result", { find_id: findId, result_id: resultId });
}
