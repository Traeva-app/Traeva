// Tests for compact.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { folderTail, textTail } from "./compact.ts";

const ROOT = "/Users/x/School";

test("a folder's short place keeps its last three folders, from the target folder's name", () => {
  // Fewer than three.
  assert.equal(folderTail(ROOT, "Physics"), "School/Physics");
  // Exactly three.
  assert.equal(folderTail(ROOT, "Physics/Quantum"), "School/Physics/Quantum");
  // More than three: the last three, after "…/".
  assert.equal(folderTail(ROOT, "Science/Physics/Quantum/Notes"), "…/Physics/Quantum/Notes");
  // The target folder itself: its name.
  assert.equal(folderTail(ROOT, ""), "School");
  // Without the target folder's path: what is known.
  assert.equal(folderTail(null, "Science/Physics/Quantum/Notes"), "…/Physics/Quantum/Notes");
  assert.equal(folderTail(null, ""), "");
});

test("an option that is a path is cut the same way; other text stays", () => {
  assert.equal(textTail("School/Science/Physics/Quantum"), "…/Science/Physics/Quantum");
  assert.equal(textTail("School/Physics/Quantum"), "School/Physics/Quantum");
  assert.equal(textTail("History"), "History");
  assert.equal(textTail("For class or for the book club?"), "For class or for the book club?");
  // A path with a slash at the start or end counts its folders only.
  assert.equal(textTail("/School/Science/Physics/Quantum/"), "…/Science/Physics/Quantum");
});
