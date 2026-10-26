// "Moved to …" after one file dropped on the pet (#37, the student's request
// 10/17): when a single file dropped on the pet is moved by the automatic
// mode, the pet asks its popup to say where it went, with Undo. Nothing for
// several files dropped at once, or for files dropped on the Dock icon.
//
// The pet sees no paths or names (get_pet_state, #208): it only knows that
// a drop on it was caught (get_pet_signal's count) and which new drop IDs
// the next reading of the queue holds. One drop and one new file: dropped
// alone. Unsure (the counts do not match): no note. The popup checks again
// against the full queue (one file in its batch, moved) before it shows.

import type { PetStateView } from "../features/pet/api.ts";

export interface NoteState {
  /** The drop IDs at the last reading; null before the first one. */
  known: ReadonlySet<string> | null;
  /** The new drop IDs of the last reading: a drop the signal counts just
   * after it may be one of them (the reading came between the file joining
   * the queue and the signal's count). */
  lastFresh: readonly string[];
  /** Drops on the pet the signal counted that no reading matched yet, and
   * how many readings found nothing new for them. */
  pending: number;
  waited: number;
  /** Files dropped alone on the pet, not ended yet. */
  single: readonly string[];
}

export const NO_NOTES: NoteState = { known: null, lastFresh: [], pending: 0, waited: 0, single: [] };

/** The signal counted `added` more drops on the pet. */
export function dropsCaught(state: NoteState, added: number): NoteState {
  return added > 0 ? { ...state, pending: state.pending + added } : state;
}

// A file dropped alone that ends in one of these did not move by itself:
// no note for it. While it waits, is worked on or asks a question, it may
// still move.
const ENDED_UNMOVED = ["already_in_place", "stopped", "not_processed", "cancelled", "handed_over"];

/** Takes a reading of the queue: which new files were dropped alone on the
 * pet, and the one to tell about now (moved by the automatic mode), if any.
 *
 * One counted drop and one new file in this reading, or none new here and
 * one in the reading just before: a file dropped alone. None new in either:
 * the reading may have been sent before the drop, so the next one decides;
 * after that the drop is let go. Anything else (two drops, two files) is
 * not told about. The first reading only learns what is there. */
export function readNotes(state: NoteState, queue: PetStateView): { state: NoteState; note: string | null } {
  const ids = queue.items.map((item) => item.drop_id).filter((id) => id !== "");
  const fresh = state.known === null ? [] : ids.filter((id) => !state.known?.has(id));
  let single = state.single;
  let pending = state.known === null ? 0 : state.pending;
  let waited = 0;
  if (pending === 1 && fresh.length === 1) {
    single = [...single, fresh[0]];
    pending = 0;
  } else if (pending === 1 && fresh.length === 0 && state.lastFresh.length === 1) {
    single = [...single, state.lastFresh[0]];
    pending = 0;
  } else if (pending > 0 && fresh.length === 0 && state.waited < 1) {
    waited = state.waited + 1;
  } else {
    pending = 0;
  }
  let note: string | null = null;
  const left: string[] = [];
  for (const id of single) {
    const item = queue.items.find((one) => one.drop_id === id);
    if (item === undefined || ENDED_UNMOVED.includes(item.state)) continue;
    if (item.state === "moved") {
      if (note === null) note = id;
      continue;
    }
    left.push(id);
  }
  return { state: { known: new Set(ids), lastFresh: fresh, pending, waited, single: left }, note };
}
