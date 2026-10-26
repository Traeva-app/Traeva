// Tests for popup.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { noteItem, noteTail, othersWaiting, popupDrop, popupHeight, takesFile } from "./popup.ts";
import type { DropItemView, DropState } from "../features/autodrop/types.ts";

function item(id: string, state: DropState, extra: Partial<DropItemView> = {}): DropItemView {
  return {
    drop_id: id,
    file_name: `${id}.md`,
    batch_id: "b1",
    received_at_ms: 0,
    state,
    automatic: true,
    folder_to_authorize: null,
    operation_id: null,
    plan_id: null,
    action_id: null,
    moved_to: null,
    reason: null,
    needs_user: state === "needs_answer" || state === "needs_source_authorization" || state === "handed_over",
    action_status: null,
    ...extra,
  };
}

test("the popup takes a file with a question or its usual steps, never a folder to allow", () => {
  assert.equal(takesFile(item("a", "needs_answer")), true);
  assert.equal(takesFile(item("b", "handed_over")), true);
  assert.equal(takesFile(item("c", "needs_source_authorization")), false);
  // A handed-over file whose usual steps ended, and finished or working files.
  assert.equal(takesFile(item("d", "handed_over", { needs_user: false })), false);
  for (const state of ["waiting", "working", "moved", "stopped", "cancelled"] as DropState[]) {
    assert.equal(takesFile(item("e", state)), false);
  }
  assert.equal(takesFile(item("", "needs_answer")), false);
});

test("the popup keeps its file while the user is on it, else takes the first it can", () => {
  const items = [item("allow", "needs_source_authorization"), item("q", "needs_answer"), item("h", "handed_over")];
  assert.equal(popupDrop(items, null, false)?.drop_id, "q");
  // In the middle of h's steps: h stays, even though q comes first.
  assert.equal(popupDrop(items, "h", true)?.drop_id, "h");
  // Not in the middle of anything: the first again.
  assert.equal(popupDrop(items, "h", false)?.drop_id, "q");
  // Kept, but no longer listed (the queue keeps 50): the first.
  assert.equal(popupDrop(items, "gone", true)?.drop_id, "q");
  // Kept and listed even after it stopped waiting: its result shows.
  const moved = [item("h", "moved", { needs_user: false })];
  assert.equal(popupDrop(moved, "h", true)?.drop_id, "h");
  assert.equal(popupDrop(moved, "h", false), null);
  assert.equal(popupDrop([], null, false), null);
  // A file the user cancelled here is left out until the queue says so too.
  assert.equal(popupDrop(items, null, false, new Set(["q"]))?.drop_id, "h");
});

test("the others that wait, and the window's height", () => {
  const items = [item("q", "needs_answer"), item("h", "handed_over"), item("allow", "needs_source_authorization"), item("m", "moved", { needs_user: false })];
  assert.equal(othersWaiting(items, "q"), 2);
  assert.equal(othersWaiting(items, null), 3);
  assert.equal(othersWaiting(items, null, new Set(["h"])), 2);
  assert.equal(popupHeight(311.2), 314);
});

test("a Moved note only for the only file of its drop, moved, with its action", () => {
  const alone = item("a", "moved", { batch_id: "b-a", action_id: "act-a" });
  assert.equal(noteItem([alone], "a"), alone);
  // Dropped with another file: no note.
  const pair = [item("p", "moved", { batch_id: "b-p", action_id: "act-p" }), item("q", "moved", { batch_id: "b-p", action_id: "act-q" })];
  assert.equal(noteItem(pair, "p"), null);
  // Not moved (yet), no action, or not listed: no note.
  for (const state of ["waiting", "working", "stopped", "handed_over", "already_in_place"] as DropState[]) {
    assert.equal(noteItem([item("s", state, { batch_id: "b-s", action_id: "act-s" })], "s"), null, state);
  }
  assert.equal(noteItem([item("n", "moved", { batch_id: "b-n" })], "n"), null);
  assert.equal(noteItem([alone], "gone"), null);
});

test("the note's tail points toward the pet only when its side is sure", () => {
  // Far from the left edge: the core put it left of the pet.
  assert.equal(noteTail(900, 0), "right");
  assert.equal(noteTail(440, 0), "right");
  assert.equal(noteTail(1440 + 600, 1440), "right");
  // Near the left edge it may be right of the pet: no tail.
  assert.equal(noteTail(439, 0), null);
  assert.equal(noteTail(40, 0), null);
  assert.equal(noteTail(1500, 1440), null);
});
