// Tests for note.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { NO_NOTES, dropsCaught, readNotes } from "./note.ts";
import type { DropState } from "../features/autodrop/types.ts";
import type { PetItemView, PetStateView } from "../features/pet/api.ts";

function item(id: string, state: DropState): PetItemView {
  return { drop_id: id, state, received_at_ms: 0, needs_user: false, action_status: null };
}

function queue(...items: PetItemView[]): PetStateView {
  return { items, running: false };
}

test("one file dropped on the pet and moved: a note, once", () => {
  let state = readNotes(NO_NOTES, queue(item("old", "moved"))).state;
  state = dropsCaught(state, 1);
  let read = readNotes(state, queue(item("old", "moved"), item("a", "waiting")));
  assert.equal(read.note, null);
  read = readNotes(read.state, queue(item("old", "moved"), item("a", "working")));
  assert.equal(read.note, null);
  read = readNotes(read.state, queue(item("old", "moved"), item("a", "moved")));
  assert.equal(read.note, "a");
  // Told once.
  read = readNotes(read.state, queue(item("old", "moved"), item("a", "moved")));
  assert.equal(read.note, null);
});

test("several files dropped at once: no note", () => {
  let state = readNotes(NO_NOTES, queue()).state;
  state = dropsCaught(state, 1);
  let read = readNotes(state, queue(item("a", "waiting"), item("b", "waiting")));
  read = readNotes(read.state, queue(item("a", "moved"), item("b", "moved")));
  assert.equal(read.note, null);
});

test("two drops before a reading, or a drop the counts do not match: no note", () => {
  let state = readNotes(NO_NOTES, queue()).state;
  state = dropsCaught(state, 2);
  let read = readNotes(state, queue(item("a", "waiting"), item("b", "waiting")));
  read = readNotes(read.state, queue(item("a", "moved"), item("b", "moved")));
  assert.equal(read.note, null);
  state = dropsCaught(read.state, 2);
  read = readNotes(state, queue(item("a", "moved"), item("b", "moved"), item("c", "waiting")));
  read = readNotes(read.state, queue(item("a", "moved"), item("b", "moved"), item("c", "moved")));
  assert.equal(read.note, null);
});

test("a file dropped on the Dock icon (no drop on the pet): no note", () => {
  let read = readNotes(NO_NOTES, queue());
  read = readNotes(read.state, queue(item("dock", "waiting")));
  read = readNotes(read.state, queue(item("dock", "moved")));
  assert.equal(read.note, null);
});

test("a file dropped alone that did not move by itself: no note, and it is let go", () => {
  for (const end of ["stopped", "handed_over", "already_in_place", "cancelled", "not_processed"] as const) {
    let state = dropsCaught(readNotes(NO_NOTES, queue()).state, 1);
    let read = readNotes(state, queue(item("a", "waiting")));
    read = readNotes(read.state, queue(item("a", end)));
    assert.equal(read.note, null, end);
    assert.deepEqual(read.state.single, [], end);
    // Moved later in the usual flow: still no note (its own window says so).
    read = readNotes(read.state, queue(item("a", "moved")));
    assert.equal(read.note, null, end);
  }
});

test("a question first, then moved after the answer: a note", () => {
  let state = dropsCaught(readNotes(NO_NOTES, queue()).state, 1);
  let read = readNotes(state, queue(item("a", "needs_answer")));
  read = readNotes(read.state, queue(item("a", "working")));
  read = readNotes(read.state, queue(item("a", "moved")));
  assert.equal(read.note, "a");
});

test("the first reading only learns what is there", () => {
  const state = dropsCaught(NO_NOTES, 1);
  let read = readNotes(state, queue(item("a", "moved")));
  assert.equal(read.note, null);
  assert.deepEqual([...(read.state.known ?? [])], ["a"]);
  // That drop is settled: a later Dock drop is not taken for it.
  read = readNotes(read.state, queue(item("a", "moved"), item("dock", "waiting")));
  read = readNotes(read.state, queue(item("a", "moved"), item("dock", "moved")));
  assert.equal(read.note, null);
});

test("a drop whose file the reading just before already saw: a note", () => {
  let read = readNotes(NO_NOTES, queue());
  // The regular reading saw the file before the signal counted the drop.
  read = readNotes(read.state, queue(item("a", "waiting")));
  const state = dropsCaught(read.state, 1);
  read = readNotes(state, queue(item("a", "working")));
  read = readNotes(read.state, queue(item("a", "moved")));
  assert.equal(read.note, "a");
});

test("a reading sent before the drop comes back empty: the next one decides", () => {
  let read = readNotes(NO_NOTES, queue());
  const state = dropsCaught(read.state, 1);
  read = readNotes(state, queue());
  assert.equal(read.state.pending, 1);
  read = readNotes(read.state, queue(item("a", "waiting")));
  read = readNotes(read.state, queue(item("a", "moved")));
  assert.equal(read.note, "a");
});

test("a counted drop whose file never shows is let go after one more reading", () => {
  let read = readNotes(NO_NOTES, queue(item("x", "moved")));
  const state = dropsCaught(read.state, 1);
  read = readNotes(state, queue(item("x", "moved")));
  read = readNotes(read.state, queue(item("x", "moved")));
  assert.equal(read.state.pending, 0);
  // A later Dock drop is not taken for it.
  read = readNotes(read.state, queue(item("x", "moved"), item("dock", "waiting")));
  read = readNotes(read.state, queue(item("x", "moved"), item("dock", "moved")));
  assert.equal(read.note, null);
});
