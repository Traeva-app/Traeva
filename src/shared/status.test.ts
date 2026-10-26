// Tests for status.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { statusSummary, toneClass } from "./status.ts";

test("tones: amber waits for the user, green is a done move, grey the rest", () => {
  assert.equal(toneClass("warn"), "badge-warn");
  assert.equal(toneClass("ok"), "card-ok");
  assert.equal(toneClass("neutral"), "");
});

test("a closed group says what it holds, most first", () => {
  const row = (label: string) => ({ badge: { label } });
  assert.equal(
    statusSummary([row("Stopped"), row("Already there"), row("Already there"), row("Cancelled")]),
    "2 already there · 1 stopped · 1 cancelled",
  );
  assert.equal(statusSummary([]), "");
});
