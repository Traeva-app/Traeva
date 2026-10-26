// The desktop pet's face and bubble (#208, #248), read from the pet's view
// of the drop queue (#168; no paths or names). Pure: the core decides what
// happens to each file; the pet only shows it.

import type { PetItemView, PetStateView } from "../features/pet/api.ts";

export type Mood = "idle" | "thinking" | "question" | "wink" | "sad";
type Flash = "wink" | "sad";

/** How long a wink or a sad face shows before the steady mood. */
export const FLASH_MS = 3000;
/** How long "Needs you" shows after more files came to wait, before it
 * becomes the count. */
export const SAY_MS = 4000;
export const ACTIVE_POLL_MS = 1000;
// A drop arrives without any event, so the pet looks this often when
// nothing goes on; it answers a drop within about this long (#37, 10/14).
export const QUIET_POLL_MS = 1500;

export interface PetState {
  /** When the pet started looking (ms since 1970): on the first read, only
   * files dropped since then are news. */
  since: number;
  /** How each file had ended at the last read (null: not yet); the map is
   * null before the first read. */
  seen: Map<string, Flash | null> | null;
  /** A wink or sad face shown until `until` (ms since 1970). */
  flash: { mood: Flash; until: number } | null;
  /** How many files waited for the user at the last read. */
  waiting: number;
  /** "Needs you" shows until then (ms since 1970), after more files came
   * to wait than at the read before; null: not since the start. */
  asked: number | null;
  /** One more for each read that found a newly dropped file: the pet hops
   * once as it catches it (#37, 10/14). */
  catches: number;
}

export function startState(since: number): PetState {
  return { since, seen: null, flash: null, waiting: 0, asked: null, catches: 0 };
}

/** A file waits for the user, as the core works it out (needs_user): its
 * folder to allow, the one question, or a normal flow still open. A flow
 * the window leaves without confirming is cancelled, so it stops asking. */
export function needsUser(queue: PetStateView | null): boolean {
  return queue !== null && queue.items.some((item) => item.needs_user);
}

/** Where a click on the pet goes: nothing waits for the user, the quick
 * search; the first file that waits has a question or its usual steps to
 * finish, the popup beside the pet (#37, 10/14); it waits for something
 * else (a folder to allow, whose explanation is in Organize), Organize in
 * the main window. */
export function clickTarget(queue: PetStateView | null): "quick" | "popup" | "organize" {
  const first = queue?.items.find((item) => item.needs_user);
  if (first === undefined) return "quick";
  return first.state === "needs_answer" || first.state === "handed_over" ? "popup" : "organize";
}

/** The core works on the queue, or a file waits its turn. */
export function isWorking(queue: PetStateView | null): boolean {
  return (
    queue !== null &&
    (queue.running ||
      queue.items.some(
        (item) =>
          item.state === "waiting" ||
          item.state === "working" ||
          item.action_status === "prepared" ||
          item.action_status === "running",
      ))
  );
}

/** The mood while the queue stays as it is; the user comes first. */
export function steadyMood(queue: PetStateView | null): Mood {
  if (needsUser(queue)) return "question";
  if (isWorking(queue)) return "thinking";
  return "idle";
}

// How a file ended, as a face: moved or already in place winks, also a
// move the normal flow confirmed (its action committed); a stop, or a move
// that failed or needs the user's decision, looks sad. A cancel is the
// user's choice, and not processed follows a stop or a cancel of an earlier
// file: neither gets a face.
function ending(item: PetItemView): Flash | null {
  if (item.state === "moved" || item.state === "already_in_place") return "wink";
  if (item.action_status === "committed") return "wink";
  if (item.state === "stopped") return "sad";
  if (item.action_status === "failed_no_move" || item.action_status === "needs_user_decision") return "sad";
  return null;
}

/** Takes a new read of the queue. A file that just ended winks or looks
 * sad, also when it is new and already finished. Each ending counts once.
 * On the first read, files dropped before the pet started looking are old
 * news. A newly dropped file is caught (one hop), and more files waiting
 * for the user than before say "Needs you" for a moment. */
export function readQueue(state: PetState, queue: PetStateView, now: number): PetState {
  const seen = new Map(queue.items.map((item) => [item.drop_id, ending(item)] as const));
  let flash: Flash | null = null;
  let caught = false;
  for (const item of queue.items) {
    if (state.seen === null && item.received_at_ms < state.since) continue;
    if (!state.seen?.has(item.drop_id)) caught = true;
    const next = seen.get(item.drop_id) ?? null;
    if (next === null || state.seen?.get(item.drop_id) === next) continue;
    // A sad ending in the same read outweighs a wink.
    if (next === "sad" || flash === null) flash = next;
  }
  const waiting = needsUserCount(queue);
  return {
    since: state.since,
    seen,
    flash: flash === null ? state.flash : { mood: flash, until: now + FLASH_MS },
    waiting,
    asked: waiting > state.waiting ? now + SAY_MS : state.asked,
    catches: caught ? state.catches + 1 : state.catches,
  };
}

/** What the pet shows now: a fresh wink or sad face first, then the steady mood. */
export function moodAt(state: PetState, queue: PetStateView | null, now: number): Mood {
  if (state.flash !== null && now < state.flash.until) return state.flash.mood;
  return steadyMood(queue);
}

/** How many files wait for the user now (needs_user): the pet's bubble.
 * More than none exactly when needsUser is true. */
export function needsUserCount(queue: PetStateView | null): number {
  return queue === null ? 0 : queue.items.filter((item) => item.needs_user).length;
}

/** What the speech bubble beside the folder says: "Needs you" for a moment
 * after more files came to wait (then the count shows), three dots while
 * the core works and nothing waits for the user, else nothing. Never a
 * file's name. */
export type Speech = "needs" | "working" | null;

export function speechAt(state: PetState, queue: PetStateView | null, now: number): Speech {
  const count = needsUserCount(queue);
  if (count > 0) return state.asked !== null && now < state.asked ? "needs" : null;
  return isWorking(queue) ? "working" : null;
}

/** The bubble's text: the count, "9+" above nine, empty when none waits. */
export function bubbleText(count: number): string {
  if (count <= 0) return "";
  return count > 9 ? "9+" : String(count);
}

/** "1 file waits", "3 files wait". */
export function waitText(count: number): string {
  return count === 1 ? "1 file waits" : `${count} files wait`;
}

const MOOD_TEXT: Record<Mood, string> = {
  idle: "Traeva.",
  thinking: "Traeva is working on your files.",
  question: "Traeva needs you.",
  wink: "Traeva: done.",
  sad: "Traeva stopped on a file.",
};

const HELP = "Drop files here to organize them.";

/** The pet's label (screen readers and the tooltip): its face, how many
 * files wait for the user, what a drop does and what a click opens. */
export function petLabel(mood: Mood, count: number): string {
  let said = MOOD_TEXT[mood];
  if (count > 0) {
    said = mood === "question" ? `Traeva needs you: ${waitText(count)}.` : `${said} ${waitText(count)} for you.`;
  }
  return `${said} ${HELP} ${count > 0 ? "Click to open Traeva." : "Click to find a file."}`;
}

/** The bubble as shown: its count, and how often it has bumped. It bumps
 * (a short grow) when more files wait than before and its text shows it
 * ("9+" staying "9+" does not); it appears with its own scale-in, so the
 * bumps start over when the bubble goes. */
export interface Bubble {
  count: number;
  bumps: number;
}

export const NO_BUBBLE: Bubble = { count: 0, bumps: 0 };

export function nextBubble(bubble: Bubble, count: number): Bubble {
  if (count === bubble.count) return bubble;
  if (count <= 0) return NO_BUBBLE;
  const bump = bubble.count > 0 && count > bubble.count && bubbleText(count) !== bubbleText(bubble.count);
  return { count, bumps: bump ? bubble.bumps + 1 : bubble.bumps };
}

/** The faces on screen, bottom first; the last one is the current face.
 * When the mood changes, the new face comes in on top and every older face
 * goes back out the way it came, from wherever it is (PetApp, pet.css).
 * Each face keeps its key, and so its element and its motion, until the
 * faces settle; a face that was still coming in is not started over. */
export interface Faces {
  layers: { key: number; mood: Mood }[];
}

export function firstFaces(mood: Mood): Faces {
  return { layers: [{ key: 0, mood }] };
}

function current(faces: Faces): { key: number; mood: Mood } {
  return faces.layers[faces.layers.length - 1];
}

export function nextFaces(faces: Faces, mood: Mood): Faces {
  const top = current(faces);
  if (mood === top.mood) return faces;
  return { layers: [...faces.layers, { key: top.key + 1, mood }] };
}

/** The older faces have gone out: only the current one stays. */
export function settleFaces(faces: Faces): Faces {
  return faces.layers.length === 1 ? faces : { layers: [current(faces)] };
}

/** The faces to draw, bottom first: the older ones while they go, then the
 * current one on top. */
export function faceLayers(faces: Faces): { key: number; mood: Mood; leaving: boolean }[] {
  const last = faces.layers.length - 1;
  return faces.layers.map((layer, index) => ({ ...layer, leaving: index < last }));
}

/** About every second while the core works, a file waits, or a short face
 * shows (so the mood after it is fresh); else now and then, since a drop
 * arrives without any event. */
export function pollMs(queue: PetStateView | null, state: PetState, now: number): number {
  const active =
    isWorking(queue) ||
    (queue !== null &&
      queue.items.some((item) => item.state === "needs_source_authorization" || item.state === "needs_answer")) ||
    (state.flash !== null && now < state.flash.until);
  return active ? ACTIVE_POLL_MS : QUIET_POLL_MS;
}
