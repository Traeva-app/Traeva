// What the pet's popup shows (#37, the student's request 10/14). The popup
// takes one dropped file that waits for the user: its question, or its
// usual steps (a suggestion or a folder chosen by hand, the preview, Move
// file). Every decision is the core's; this only picks the file and words
// what shows.

import type { DropItemView } from "../features/autodrop/types.ts";

/** States the popup takes a file in. A folder to allow is not one: its
 * explanation is in Organize, in the main window. */
const POPUP_STATES: DropItemView["state"][] = ["needs_answer", "handed_over"];

/** Whether the popup can take this file now: it waits for the user with a
 * question or its usual steps. */
export function takesFile(item: DropItemView): boolean {
  return item.drop_id !== "" && item.needs_user && POPUP_STATES.includes(item.state);
}

/** The file the popup works on: the one it has while the user is in the
 * middle of its steps (`keep`) and it is still listed; else the first file
 * the popup can take, leaving out the ones the user let go of here
 * (`skip`: cancelled in this popup, until the core's queue says so too);
 * null when none. */
export function popupDrop(
  items: DropItemView[],
  current: string | null,
  keep: boolean,
  skip: ReadonlySet<string> = new Set(),
): DropItemView | null {
  if (keep && current !== null) {
    const kept = items.find((item) => item.drop_id === current);
    if (kept !== undefined) return kept;
  }
  return items.find((item) => takesFile(item) && !skip.has(item.drop_id)) ?? null;
}

/** How many other files wait for the user besides this one, for the line
 * that says Traeva has the rest; not the ones let go of here (`skip`). */
export function othersWaiting(
  items: DropItemView[],
  dropId: string | null,
  skip: ReadonlySet<string> = new Set(),
): number {
  return items.filter((item) => item.needs_user && item.drop_id !== dropId && !skip.has(item.drop_id)).length;
}

/** The popup's height for what it shows: its content plus the window's own
 * edge; the core keeps it between 200 and 600 points. */
export function popupHeight(content: number): number {
  return Math.ceil(content) + 2;
}

/** The file a "Moved to" note is for (the pet asked for it, #37 10/17), when
 * the queue agrees: the only file of its drop (batch), moved by the
 * automatic mode, with its action. Null otherwise: nothing is said. */
export function noteItem(items: DropItemView[], dropId: string): DropItemView | null {
  const item = items.find((one) => one.drop_id === dropId);
  if (item === undefined || item.state !== "moved" || item.action_id === null) return null;
  const batch = items.filter((one) => one.batch_id === item.batch_id).length;
  return batch === 1 ? item : null;
}

/** The note's tail toward the pet, when its side is sure. The core puts the
 * note left of the pet, or right of it (at the pet's right edge plus 8 pt)
 * when there is no room on the left: only a note 440 pt or more from the
 * screen's left edge is surely left of the pet. Unsure: no tail. */
export function noteTail(windowLeft: number, screenLeft: number): "right" | null {
  return windowLeft - screenLeft >= 440 ? "right" : null;
}
