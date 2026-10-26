// The pet's popup window "ask" (#37, the student's request 10/14): its
// commands (window_commands.rs). The pet opens it; the popup sets its own
// height and hides itself.

import { invoke } from "@tauri-apps/api/core";

/** Shows the popup beside the pet, or brings it back where it was (the pet
 * calls this). */
export function showDropPopup(): Promise<void> {
  return invoke<void>("show_drop_popup");
}

/** Sets the popup's height in points, and its width when given (the
 * "Moved to" note is narrower); the core keeps both in its limits and
 * returns the height it took. */
export function resizeDropPopup(height: number, width?: number): Promise<number> {
  return invoke<number>("resize_drop_popup", width === undefined ? { height } : { height, width });
}

/** The pet: a file dropped alone on it was moved by the automatic mode.
 * Shows the popup as a small note beside the pet, without taking the
 * focus (the popup checks the file itself before it says anything). */
export function showMovedNote(dropId: string): Promise<void> {
  return invoke<void>("show_moved_note", { drop_id: dropId });
}

/** The popup: the dropped file the pet asked a note for since the last
 * call, once (null: none). */
export function takeMovedNote(): Promise<string | null> {
  return invoke<string | null>("take_moved_note");
}

export function hideDropPopup(): Promise<void> {
  return invoke<void>("hide_drop_popup");
}
