// Tests for mood.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  clickTarget,
  ACTIVE_POLL_MS,
  FLASH_MS,
  NO_BUBBLE,
  QUIET_POLL_MS,
  SAY_MS,
  bubbleText,
  faceLayers,
  firstFaces,
  moodAt,
  needsUser,
  needsUserCount,
  nextBubble,
  nextFaces,
  petLabel,
  pollMs,
  readQueue,
  settleFaces,
  speechAt,
  startState,
  steadyMood,
  waitText,
} from "./mood.ts";
import type { Mood, PetState } from "./mood.ts";
import type { DropState } from "../features/autodrop/types.ts";
import type { PetItemView, PetStateView } from "../features/pet/api.ts";

// As the core reports it: these states wait for the user unless a test
// says otherwise (a confirmed normal flow does not).
const WAITING: DropState[] = ["needs_answer", "needs_source_authorization", "handed_over"];

function item(id: string, state: DropState, extra: Partial<PetItemView> = {}): PetItemView {
  return {
    drop_id: id,
    received_at_ms: 0,
    state,
    needs_user: WAITING.includes(state),
    action_status: null,
    ...extra,
  };
}

function queue(items: PetItemView[], running = false): PetStateView {
  return { items, running };
}

/** Reads each queue in turn, one second apart, from a first empty read. */
function reads(...queues: PetStateView[]): PetState {
  let state = readQueue(startState(0), queue([]), 0);
  queues.forEach((next, index) => {
    state = readQueue(state, next, (index + 1) * 1000);
  });
  return state;
}

test("an empty or unread queue is idle", () => {
  assert.equal(steadyMood(null), "idle");
  assert.equal(steadyMood(queue([])), "idle");
  assert.equal(moodAt(startState(0), null, 0), "idle");
});

test("the core at work, or a file waiting its turn, is thinking", () => {
  assert.equal(steadyMood(queue([], true)), "thinking");
  assert.equal(steadyMood(queue([item("a", "waiting")])), "thinking");
  assert.equal(steadyMood(queue([item("a", "working")])), "thinking");
});

test("a file that waits for the user is a question, before any work", () => {
  for (const state of ["needs_answer", "needs_source_authorization"] as DropState[]) {
    const waiting = queue([item("a", state), item("b", "working")], true);
    assert.equal(steadyMood(waiting), "question");
    assert.equal(needsUser(waiting), true);
  }
});

test("a handed-over file asks while the core says it waits, not once confirmed", () => {
  const handed = queue([item("a", "handed_over")]);
  assert.equal(needsUser(handed), true);
  assert.equal(steadyMood(handed), "question");
  const confirmed = queue([item("a", "handed_over", { needs_user: false, action_status: "committed" })]);
  assert.equal(needsUser(confirmed), false);
  assert.equal(steadyMood(confirmed), "idle");
});

test("the question follows the core: a flow the window left is not asked about", () => {
  // Already there, given up or skipped: the window cancelled the operation.
  const left = queue([item("a", "handed_over", { needs_user: false })]);
  assert.equal(needsUser(left), false);
  assert.equal(steadyMood(left), "idle");
});

test("a move the normal flow confirmed winks; one that failed or needs a decision looks sad", () => {
  const open = queue([item("a", "handed_over")]);
  const confirmed = (action_status: PetItemView["action_status"]) =>
    queue([item("a", "handed_over", { needs_user: false, action_status })]);
  assert.equal(steadyMood(confirmed("running")), "thinking");
  assert.equal(moodAt(reads(open, confirmed("committed")), null, 2500), "wink");
  assert.equal(moodAt(reads(open, confirmed("failed_no_move")), null, 2500), "sad");
  assert.equal(moodAt(reads(open, confirmed("needs_user_decision")), null, 2500), "sad");
  // An Undo afterwards is the user's choice: no new face.
  assert.equal(moodAt(reads(confirmed("committed"), confirmed("undone")), null, 4500), "idle");
});

test("a stopped file does not ask, even when its move had started", () => {
  const stopped = queue([item("a", "stopped", { action_status: "failed_no_move" })]);
  assert.equal(needsUser(stopped), false);
  assert.equal(steadyMood(stopped), "idle");
});

test("finished files alone are idle", () => {
  const done: DropState[] = ["moved", "already_in_place", "stopped", "not_processed", "cancelled"];
  assert.equal(steadyMood(queue(done.map((state, index) => item(`d${index}`, state)))), "idle");
  assert.equal(needsUser(null), false);
});

test("a file that becomes moved or already in place winks for a few seconds", () => {
  for (const end of ["moved", "already_in_place"] as DropState[]) {
    const state = reads(queue([item("a", "working")], true), queue([item("a", end)]));
    assert.equal(moodAt(state, queue([item("a", end)]), 2000), "wink");
    assert.equal(moodAt(state, queue([item("a", end)]), 2000 + FLASH_MS - 1), "wink");
    assert.equal(moodAt(state, queue([item("a", end)]), 2000 + FLASH_MS), "idle");
  }
});

test("a file that becomes stopped looks sad; a cancelled one does not", () => {
  const stopped = reads(queue([item("a", "working")], true), queue([item("a", "stopped")]));
  assert.equal(moodAt(stopped, queue([item("a", "stopped")]), 2500), "sad");
  const cancelled = reads(queue([item("a", "needs_answer")]), queue([item("a", "cancelled")]));
  assert.equal(moodAt(cancelled, queue([item("a", "cancelled")]), 2500), "idle");
  const notProcessed = reads(queue([item("a", "waiting")]), queue([item("a", "not_processed")]));
  assert.equal(moodAt(notProcessed, queue([item("a", "not_processed")]), 2500), "idle");
});

test("a new file already finished when first seen counts, but only once", () => {
  const moved = queue([item("a", "moved")]);
  const first = reads(moved);
  assert.equal(moodAt(first, moved, 1500), "wink");
  // Read again later: the same state is no news.
  const again = readQueue(first, moved, 1000 + FLASH_MS + 500);
  assert.equal(moodAt(again, moved, 1000 + FLASH_MS + 500), "idle");
  // A folder dropped is stopped on arrival.
  const folder = queue([item("b", "stopped")]);
  assert.equal(moodAt(reads(folder), folder, 1500), "sad");
});

test("on the first read, only files dropped since the pet started are news", () => {
  const old = queue([item("a", "moved"), item("b", "stopped")]);
  const state = readQueue(startState(1000), old, 1200);
  assert.equal(state.flash, null);
  assert.equal(moodAt(state, old, 1200), "idle");
  // Dropped after the pet started, stopped before its first read.
  const fresh = queue([item("a", "moved"), item("c", "stopped", { received_at_ms: 1100 })]);
  assert.equal(moodAt(readQueue(startState(1000), fresh, 1200), fresh, 1200), "sad");
});

test("the short face shows first, then the steady mood", () => {
  const after = queue([item("a", "stopped"), item("b", "needs_answer")]);
  const state = reads(queue([item("a", "working"), item("b", "waiting")], true), after);
  assert.equal(moodAt(state, after, 2000), "sad");
  assert.equal(moodAt(state, after, 2000 + FLASH_MS), "question");
  const working = queue([item("a", "moved"), item("b", "working")], true);
  const busy = reads(queue([item("a", "working"), item("b", "waiting")], true), working);
  assert.equal(moodAt(busy, working, 2000), "wink");
  assert.equal(moodAt(busy, working, 2000 + FLASH_MS), "thinking");
});

test("a stop in the same read outweighs a move", () => {
  const after = queue([item("a", "moved"), item("b", "stopped")]);
  const state = reads(queue([item("a", "working"), item("b", "waiting")], true), after);
  assert.equal(moodAt(state, after, 2000), "sad");
});

test("a later change starts a new short face; no change keeps the current one", () => {
  const one = queue([item("a", "moved"), item("b", "working")], true);
  const two = queue([item("a", "moved"), item("b", "stopped")]);
  const state = reads(queue([item("a", "working"), item("b", "waiting")], true), one, two);
  // Moved at 2000, stopped at 3000: sad until 3000 + FLASH_MS.
  assert.equal(moodAt(state, two, 3000 + FLASH_MS - 1), "sad");
  const quiet = readQueue(state, two, 4000);
  assert.deepEqual(quiet.flash, state.flash);
});

test("a file that leaves the queue is forgotten", () => {
  const state = reads(queue([item("a", "moved")]), queue([]));
  assert.equal(state.seen?.has("a"), false);
});

test("the queue is read every second while something is active", () => {
  const quiet = startState(0);
  assert.equal(pollMs(null, quiet, 0), QUIET_POLL_MS);
  assert.equal(pollMs(queue([]), quiet, 0), QUIET_POLL_MS);
  assert.equal(pollMs(queue([item("a", "moved")]), quiet, 0), QUIET_POLL_MS);
  assert.equal(pollMs(queue([], true), quiet, 0), ACTIVE_POLL_MS);
  assert.equal(pollMs(queue([item("a", "waiting")]), quiet, 0), ACTIVE_POLL_MS);
  assert.equal(pollMs(queue([item("a", "needs_answer")]), quiet, 0), ACTIVE_POLL_MS);
  assert.equal(pollMs(queue([item("a", "needs_source_authorization")]), quiet, 0), ACTIVE_POLL_MS);
});

test("while a short face shows, the queue is read every second", () => {
  const moved = queue([item("a", "moved")]);
  const state = reads(queue([item("a", "working")], true), moved);
  assert.equal(pollMs(moved, state, 2000), ACTIVE_POLL_MS);
  assert.equal(pollMs(moved, state, 2000 + FLASH_MS), QUIET_POLL_MS);
});

test("the bubble counts the files that wait for the user, as needsUser says", () => {
  assert.equal(needsUserCount(null), 0);
  assert.equal(needsUserCount(queue([])), 0);
  const mixed = queue([
    item("a", "needs_answer"),
    item("b", "working"),
    item("c", "needs_source_authorization"),
    item("d", "handed_over"),
    item("e", "handed_over", { needs_user: false, action_status: "committed" }),
    item("f", "moved"),
  ]);
  assert.equal(needsUserCount(mixed), 3);
  for (const view of [null, queue([]), mixed, queue([item("a", "stopped")]), queue([item("a", "needs_answer")])]) {
    assert.equal(needsUserCount(view) > 0, needsUser(view));
  }
});

test("the bubble shows the count, 9+ above nine, nothing for none", () => {
  assert.equal(bubbleText(0), "");
  assert.equal(bubbleText(1), "1");
  assert.equal(bubbleText(3), "3");
  assert.equal(bubbleText(9), "9");
  assert.equal(bubbleText(10), "9+");
  assert.equal(bubbleText(12), "9+");
});

test("the label says how many files wait, and what a click opens", () => {
  assert.equal(waitText(1), "1 file waits");
  assert.equal(waitText(2), "2 files wait");
  assert.equal(
    petLabel("question", 2),
    "Traeva needs you: 2 files wait. Drop files here to organize them. Click to open Traeva.",
  );
  assert.equal(
    petLabel("question", 1),
    "Traeva needs you: 1 file waits. Drop files here to organize them. Click to open Traeva.",
  );
  // The exact count, not the bubble's 9+.
  assert.match(petLabel("question", 12), /^Traeva needs you: 12 files wait\. /);
  // A short face over files that wait still says they wait.
  assert.equal(
    petLabel("wink", 1),
    "Traeva: done. 1 file waits for you. Drop files here to organize them. Click to open Traeva.",
  );
  assert.equal(
    petLabel("sad", 3),
    "Traeva stopped on a file. 3 files wait for you. Drop files here to organize them. Click to open Traeva.",
  );
  // Nothing waits: the sentences as before.
  assert.equal(petLabel("idle", 0), "Traeva. Drop files here to organize them. Click to find a file.");
  assert.equal(
    petLabel("thinking", 0),
    "Traeva is working on your files. Drop files here to organize them. Click to find a file.",
  );
  assert.equal(petLabel("wink", 0), "Traeva: done. Drop files here to organize them. Click to find a file.");
});

test("the bubble bumps when more files wait, not when it appears or fewer wait", () => {
  const one = nextBubble(NO_BUBBLE, 1);
  assert.deepEqual(one, { count: 1, bumps: 0 });
  const three = nextBubble(one, 3);
  assert.deepEqual(three, { count: 3, bumps: 1 });
  assert.equal(nextBubble(three, 3), three);
  const two = nextBubble(three, 2);
  assert.deepEqual(two, { count: 2, bumps: 1 });
  assert.deepEqual(nextBubble(two, 4), { count: 4, bumps: 2 });
  // Above nine it reads "9+" either way: no bump for nothing seen.
  const nine = nextBubble(nextBubble(NO_BUBBLE, 8), 10);
  assert.deepEqual(nine, { count: 10, bumps: 1 });
  assert.deepEqual(nextBubble(nine, 12), { count: 12, bumps: 1 });
  // Gone, then back: a fresh bubble that only scales in.
  assert.equal(nextBubble(two, 0), NO_BUBBLE);
  assert.deepEqual(nextBubble(nextBubble(two, 0), 5), { count: 5, bumps: 0 });
});

test("a new mood keeps the old face under the new one until it settles", () => {
  const first = firstFaces("idle");
  assert.deepEqual(faceLayers(first), [{ key: 0, mood: "idle", leaving: false }]);
  assert.equal(nextFaces(first, "idle"), first);
  const asked = nextFaces(first, "question");
  // The old face keeps its key (and so its element), under the new one.
  assert.deepEqual(faceLayers(asked), [
    { key: 0, mood: "idle", leaving: true },
    { key: 1, mood: "question", leaving: false },
  ]);
  const settled = settleFaces(asked);
  assert.deepEqual(faceLayers(settled), [{ key: 1, mood: "question", leaving: false }]);
  assert.equal(settleFaces(settled), settled);
});

test("a change before the faces settle keeps every face going out where it is", () => {
  // Wink, then thinking for a moment, then wink again (A, B, A).
  const moods: Mood[] = ["thinking", "wink"];
  const back = moods.reduce(nextFaces, firstFaces("wink"));
  // Nothing is dropped or started over: the first wink and the brief
  // thinking face keep their keys and go out; a new wink comes in on top.
  assert.deepEqual(faceLayers(back), [
    { key: 0, mood: "wink", leaving: true },
    { key: 1, mood: "thinking", leaving: true },
    { key: 2, mood: "wink", leaving: false },
  ]);
  // The same mood again changes nothing.
  assert.equal(nextFaces(back, "wink"), back);
  // Settled: only the current face, with its own key.
  assert.deepEqual(faceLayers(settleFaces(back)), [{ key: 2, mood: "wink", leaving: false }]);
  const keys = faceLayers(nextFaces(back, "question")).map((layer) => layer.key);
  assert.deepEqual(keys, [0, 1, 2, 3]);
});

test("a click goes to the quick search, the popup or Organize, by the first file that waits", () => {
  const view = (items: PetItemView[]): PetStateView => ({ items, running: false });
  assert.equal(clickTarget(null), "quick");
  assert.equal(clickTarget(view([item("a", "moved")])), "quick");
  assert.equal(clickTarget(view([item("a", "needs_answer")])), "popup");
  assert.equal(clickTarget(view([item("a", "handed_over")])), "popup");
  // A folder to allow: its explanation is in Organize.
  assert.equal(clickTarget(view([item("a", "needs_source_authorization"), item("b", "needs_answer")])), "organize");
  // The first that waits decides, not the first in the list.
  assert.equal(clickTarget(view([item("a", "moved"), item("b", "handed_over")])), "popup");
});

test("a newly dropped file is caught once, also on the first read", () => {
  const first = readQueue(startState(1000), queue([item("old", "moved", { received_at_ms: 500 })]), 1200);
  assert.equal(first.catches, 0);
  const dropped = readQueue(first, queue([item("old", "moved"), item("a", "waiting", { received_at_ms: 1300 })]), 2000);
  assert.equal(dropped.catches, 1);
  // The same file again, now working: no second hop.
  const again = readQueue(dropped, queue([item("old", "moved"), item("a", "working")]), 3000);
  assert.equal(again.catches, 1);
  // Two files in one read: one hop.
  const two = readQueue(again, queue([item("a", "working"), item("b", "waiting"), item("c", "waiting")]), 4000);
  assert.equal(two.catches, 2);
  // Dropped after the pet started looking and seen on its first read.
  assert.equal(readQueue(startState(1000), queue([item("n", "waiting", { received_at_ms: 1100 })]), 1200).catches, 1);
});

test("the speech bubble: dots while working, Needs you for a moment, then the count", () => {
  const working = queue([item("a", "working")]);
  let state = readQueue(startState(0), working, 1000);
  assert.equal(speechAt(state, working, 1000), "working");
  assert.equal(speechAt(state, queue([]), 1000), null);
  assert.equal(speechAt(state, null, 1000), null);

  const asks = queue([item("a", "needs_answer")]);
  state = readQueue(state, asks, 2000);
  assert.equal(speechAt(state, asks, 2000), "needs");
  assert.equal(speechAt(state, asks, 2000 + SAY_MS - 1), "needs");
  assert.equal(speechAt(state, asks, 2000 + SAY_MS), null);

  // Still one waiting at the next read: no new "Needs you".
  state = readQueue(state, asks, 2000 + SAY_MS + 500);
  assert.equal(speechAt(state, asks, 2000 + SAY_MS + 500), null);
  // A second one comes to wait: it says it again.
  const two = queue([item("a", "needs_answer"), item("b", "handed_over")]);
  state = readQueue(state, two, 9000);
  assert.equal(speechAt(state, two, 9000), "needs");
  // Fewer waiting: no new "Needs you"; while one waits and others work,
  // the count shows, not the dots.
  const mixed = queue([item("b", "handed_over"), item("c", "working")]);
  state = readQueue(state, mixed, 9000 + SAY_MS);
  assert.equal(speechAt(state, mixed, 9000 + SAY_MS), null);
});
