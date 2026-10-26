// Tests for quick.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  MAX_QUICK_RESULTS,
  QUICK_MAX_HEIGHT,
  QUICK_MIN_HEIGHT,
  enterAction,
  moveChoice,
  quickWindowHeight,
  shownResults,
  statusText,
} from "./quick.ts";
import type { FindCoverage, FindResult, FindView } from "../features/find/types.ts";

function coverage(): FindCoverage {
  return {
    folders: 1,
    folders_unavailable: 0,
    folders_left_out: false,
    files: 12,
    file_limit_reached: false,
    described: 0,
    texts_searched: 0,
    texts_too_large: 0,
    texts_in_cloud: 0,
    texts_unreadable: 0,
    texts_not_reached: 0,
  };
}

function result(id: string): FindResult {
  return {
    result_id: id,
    root_id: "root-1",
    folder: "/Users/test/Synthetic/Trips",
    relative_path: `Trips/${id}.md`,
    name: `${id}.md`,
    kind: "markdown",
    size: 100,
    modified_at_ms: 0,
    matches: [],
    described: false,
    description_outdated: false,
    memory_chain_id: null,
    opened_at_ms: null,
    added_at_ms: null,
    time_found: null,
  };
}

function view(extra: Partial<FindView> = {}): FindView {
  return {
    find_id: "find-1",
    state: "done",
    words: { things: [["kyoto"]], related: [], kinds: [], when: null },
    words_by: null,
    results: [],
    matched: 0,
    coverage: coverage(),
    problem: null,
    ...extra,
  };
}

const many = Array.from({ length: 30 }, (_, i) => result(`R${i + 1}`));

test("the window shows the first results of a finished search, in the core's order", () => {
  assert.deepEqual(shownResults(null), []);
  assert.deepEqual(shownResults(view({ state: "working", results: many })), []);
  const shown = shownResults(view({ results: many, matched: 45 }));
  assert.equal(shown.length, MAX_QUICK_RESULTS);
  assert.deepEqual(
    shown.map((r) => r.result_id),
    ["R1", "R2", "R3", "R4", "R5", "R6"],
  );
});

test("Return opens a result only while the results are for what the box says", () => {
  const found = view({ results: [result("R1")], matched: 1 });
  assert.equal(enterAction(found, true), "open");
  // The words changed after the search: search again.
  assert.equal(enterAction(found, false), "search");
  assert.equal(enterAction(null, false), "search");
  // That search is still under way: nothing.
  assert.equal(enterAction(view({ state: "working" }), true), "none");
  // Nothing matched, or it was cancelled: search again.
  assert.equal(enterAction(view(), true), "search");
  assert.equal(enterAction(view({ state: "cancelled" }), true), "search");
});

test("the arrow keys stay within the shown results", () => {
  assert.equal(moveChoice(0, 3, 1), 1);
  assert.equal(moveChoice(2, 3, 1), 2);
  assert.equal(moveChoice(0, 3, -1), 0);
});

test("the status line says what happened, and nothing when the list says it", () => {
  assert.equal(statusText(null, null), null);
  assert.equal(statusText(view(), "Traeva could not start the search."), "Traeva could not start the search.");
  assert.equal(statusText(view({ state: "working" }), null), "Searching…");
  assert.equal(statusText(view({ state: "cancelled" }), null), "The search was cancelled.");
  assert.equal(statusText(view({ problem: "Traeva could not read its records." }), null), "Traeva could not read its records.");
  assert.equal(
    statusText(view({ words: { things: [], related: [], kinds: [], when: null }, coverage: null }), null),
    "Type a word about the file.",
  );
  assert.equal(statusText(view(), null), "No files matched. Searched 12 files in the folder you added.");
  assert.equal(statusText(view({ results: [result("R1")], matched: 1 }), null), null);
  assert.equal(
    statusText(view({ results: many, matched: 45 }), null),
    "45 files matched; the first 6 are here.",
  );
});

test("the window asks for the height of what it shows, within its range (#242)", () => {
  // Six results with their lines: shown again, the window asks for this,
  // not the 64 points the core opens it at.
  assert.equal(quickWindowHeight(461.4), 462);
  assert.equal(quickWindowHeight(64), QUICK_MIN_HEIGHT);
  assert.equal(quickWindowHeight(10), QUICK_MIN_HEIGHT);
  assert.equal(quickWindowHeight(900), QUICK_MAX_HEIGHT);
  assert.equal(quickWindowHeight(Number.NaN), QUICK_MIN_HEIGHT);
});
