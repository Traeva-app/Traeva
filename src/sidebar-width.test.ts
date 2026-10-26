// Tests for sidebar-width.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DEFAULT,
  MAX_FULL,
  MIN_FULL,
  NARROW_WINDOW,
  RAIL,
  isRail,
  parseWidth,
  shownWidth,
  snapWidth,
  stepWidth,
  toggleWidth,
} from "./sidebar-width.ts";

test("a drag narrower than 120 px snaps to the rail; wider stays in range", () => {
  assert.equal(snapWidth(30), RAIL);
  assert.equal(snapWidth(119), RAIL);
  assert.equal(snapWidth(120), MIN_FULL);
  assert.equal(snapWidth(150), MIN_FULL);
  // A width kept before the narrowest full width grew to 184 px.
  assert.equal(parseWidth("160"), MIN_FULL);
  assert.equal(snapWidth(213.6), 214);
  assert.equal(snapWidth(500), MAX_FULL);
  assert.equal(snapWidth(Number.NaN), RAIL);
  assert.equal(isRail(RAIL), true);
  assert.equal(isRail(MIN_FULL), false);
});

test("arrow keys resize, and pass between the rail and the narrowest full width", () => {
  assert.equal(stepWidth(200, 1), 216);
  assert.equal(stepWidth(200, -1), 184);
  assert.equal(stepWidth(MAX_FULL, 1), MAX_FULL);
  assert.equal(stepWidth(195, -1), MIN_FULL);
  assert.equal(stepWidth(MIN_FULL, -1), RAIL);
  assert.equal(stepWidth(RAIL, -1), RAIL);
  assert.equal(stepWidth(RAIL, 1), MIN_FULL);
});

test("double-click goes to the rail and back to the last full width", () => {
  assert.equal(toggleWidth(240, 240), RAIL);
  assert.equal(toggleWidth(RAIL, 240), 240);
  assert.equal(toggleWidth(RAIL, RAIL), MIN_FULL);
  assert.equal(toggleWidth(RAIL, 999), MAX_FULL);
});

test("a missing or broken saved width is the default", () => {
  assert.equal(parseWidth(null), DEFAULT);
  assert.equal(parseWidth(""), DEFAULT);
  assert.equal(parseWidth("wide"), DEFAULT);
  assert.equal(parseWidth("64"), RAIL);
  assert.equal(parseWidth("230"), 230);
  assert.equal(parseWidth("9000"), MAX_FULL);
});

test("a narrow window shows the rail and keeps the width for later", () => {
  assert.equal(shownWidth(240, NARROW_WINDOW - 1), RAIL);
  assert.equal(shownWidth(240, NARROW_WINDOW), 240);
  assert.equal(shownWidth(RAIL, 1200), RAIL);
});
