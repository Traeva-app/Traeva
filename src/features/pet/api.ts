// The desktop pet's calls to Rust (#208). Each function is one Tauri command
// in src-tauri/src/window_commands.rs. The pet window may call getPetState,
// getPetSignal, showMainWindow, showQuickSearch and the popup's show
// (capabilities/pet-window.json); the main window calls takePageRequest and
// setPetVisible.

import { invoke } from "@tauri-apps/api/core";
import type { ActionStatus } from "../organize/types";
import type { DropState } from "../autodrop/types";

/** What the pet may read of the drop queue: no paths, names or reasons
 * (window_commands.rs `get_pet_state`; R1's P03 review of #208). */
export interface PetItemView {
  drop_id: string;
  state: DropState;
  received_at_ms: number;
  /** The file waits for the user now, as the core works it out. */
  needs_user: boolean;
  /** The current state of the file's action, when it has one. */
  action_status: ActionStatus | null;
}

export interface PetStateView {
  items: PetItemView[];
  /** A background pass is working on the queue. */
  running: boolean;
}

/** The pet's view of the drop queue (the only queue call the pet may make). */
export function getPetState(): Promise<PetStateView> {
  return invoke<PetStateView>("get_pet_state");
}

/** The pet's quick signal (window_commands.rs `get_pet_signal`): whether a
 * file is being dragged over it now, and how many drops it caught since
 * Traeva started. No path, name or queue state. */
export interface PetSignal {
  drag_over: boolean;
  drops: number;
}

/** Read every few tenths of a second, so the pet reacts at once. */
export function getPetSignal(): Promise<PetSignal> {
  return invoke<PetSignal>("get_pet_signal");
}

/** The pages the pet's windows may ask the main window for: "automatic"
 * is Settings with the automatic mode's card in view (the popup's "Turn on
 * automatic mode…"; agreeing to it happens there, never in the popup). */
export type MainPage = "find" | "organize" | "automatic";

/** Shows and focuses the main window, and asks it for `page`. */
export function showMainWindow(page: MainPage): Promise<void> {
  return invoke<void>("show_main_window", { page });
}

/** The page the pet asked for since the last call, once (null: none). The
 * main window calls this when it gets the focus. */
export function takePageRequest(): Promise<MainPage | null> {
  return invoke<MainPage | null>("take_page_request");
}

/** Shows or hides the pet. The main window keeps the user's choice in
 * localStorage (PET_HIDDEN_KEY) and calls this when it starts. */
export function setPetVisible(visible: boolean): Promise<void> {
  return invoke<void>("set_pet_visible", { visible });
}

/** Where the main window keeps "the user hid the pet". */
export const PET_HIDDEN_KEY = "traeva.pet.hidden";

/** Sent on the main window once setPetVisible(true) has returned. Showing
 * the pet takes the focus and set_pet_visible (window_commands.rs) gives it
 * back to the main window, but the page gets no focus event for that, so a
 * page that puts the cursor back on focus listens for this too (#259). */
export const MAIN_FOCUS_BACK = "traeva-main-focus-back";

/** Opens the quick search window next to the pet (the pet calls this when
 * nothing waits for the user). */
export function showQuickSearch(): Promise<void> {
  return invoke<void>("show_quick_search");
}

/** Sets the quick search window's height in points; the core keeps it
 * between 64 and 480 and returns the height it set. */
export function resizeQuickSearch(height: number): Promise<number> {
  return invoke<number>("resize_quick_search", { height });
}

/** Hides the quick search window (Esc; it also hides when it loses the focus). */
export function hideQuickSearch(): Promise<void> {
  return invoke<void>("hide_quick_search");
}
