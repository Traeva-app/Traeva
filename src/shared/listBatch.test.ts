// The rules of long lists: a first batch, one batch more per click, "Show
// less" once everything shows, and no button for a short list.

import assert from "node:assert/strict";
import { test } from "node:test";
import { moreShown, showMoreText, shownItems } from "./listBatch.ts";

const rows = Array.from({ length: 23 }, (_, index) => index);

test("a long list shows its first batch, then one batch per click", () => {
  let shown = 10;
  let state = shownItems(rows, shown, 10);
  assert.deepEqual(state.visible, rows.slice(0, 10));
  assert.equal(state.hidden, 13);
  assert.equal(showMoreText(state), "Show more (13 more)");

  shown = moreShown(shown, 10);
  state = shownItems(rows, shown, 10);
  assert.deepEqual(state.visible, rows.slice(0, 20));
  assert.equal(showMoreText(state), "Show more (3 more)");

  shown = moreShown(shown, 10);
  state = shownItems(rows, shown, 10);
  assert.deepEqual(state.visible, rows);
  assert.equal(state.hidden, 0);
  assert.equal(showMoreText(state), "Show less");
});

test("Show less goes back to the first batch", () => {
  const state = shownItems(rows, 10, 10);
  assert.equal(state.visible.length, 10);
  assert.equal(state.canShowLess, false);
});

test("a list that fits in the first batch has no button", () => {
  for (const length of [0, 1, 10]) {
    const state = shownItems(rows.slice(0, length), 10, 10);
    assert.equal(state.visible.length, length);
    assert.equal(showMoreText(state), null, `${length} rows`);
  }
  // Exactly one more than a batch: one click, then Show less.
  assert.equal(showMoreText(shownItems(rows.slice(0, 11), 10, 10)), "Show more (1 more)");
});

test("rows that must stay are never hidden and keep their place", () => {
  // Odd rows wait for the user: all of them show, wherever they are.
  const keep = (row: number) => row % 2 === 1;
  const state = shownItems(rows.slice(0, 12), 2, 2, keep);
  assert.deepEqual(state.visible, [0, 1, 2, 3, 5, 7, 9, 11]);
  assert.equal(state.hidden, 4, "only the other rows are counted and cut");
  // Only kept rows: no button at all.
  assert.equal(showMoreText(shownItems([1, 3, 5], 2, 2, keep)), null);
});
