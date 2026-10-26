// Tests for filter.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { filterAfter } from "./filter.ts";

type Key = "all" | "waiting" | "done";
const hides = (key: Key) => key === "done";

test("a filter that hides waiting things goes back to all when more wait", () => {
  assert.equal(filterAfter<Key>("done", "all", hides, 0, 1), "all");
  assert.equal(filterAfter<Key>("done", "all", hides, 2, 3), "all");
});

test("otherwise the user's filter stays", () => {
  // Nothing new waits.
  assert.equal(filterAfter<Key>("done", "all", hides, 1, 1), "done");
  assert.equal(filterAfter<Key>("done", "all", hides, 2, 1), "done");
  // The filter shows waiting things anyway.
  assert.equal(filterAfter<Key>("waiting", "all", hides, 0, 2), "waiting");
  assert.equal(filterAfter<Key>("all", "all", hides, 0, 2), "all");
});
