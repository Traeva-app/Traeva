// Tests for cleared.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { clearedIds, clearRows } from "./cleared.ts";

test("cleared files are kept for the run and added to, never dropped", () => {
  assert.equal(clearedIds().size, 0);
  assert.deepEqual([...clearRows(["d1", "d2"])].sort(), ["d1", "d2"]);
  assert.deepEqual([...clearRows(["d2", "d3"])].sort(), ["d1", "d2", "d3"]);
  // A copy: changing it does not change what is kept.
  const copy = clearedIds() as Set<string>;
  copy.delete("d1");
  assert.equal(clearedIds().has("d1"), true);
});
