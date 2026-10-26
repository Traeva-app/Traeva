// Tests for focusBack.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { focusBackTarget } from "./focusBack.ts";

const body = { isConnected: true, name: "body" };
const field = { isConnected: true, name: "field" };
const other = { isConnected: true, name: "other" };
const gone = { isConnected: false, name: "gone" };

test("the focus goes back where it was only when it was lost", () => {
  // Lost to the page itself (or to nothing): back to the field.
  assert.equal(focusBackTarget(field, body, body), field);
  assert.equal(focusBackTarget(field, null, body), field);
  // Somewhere already (Find put its search box back): left alone.
  assert.equal(focusBackTarget(field, other, body), null);
  // Nothing to go back to: the body, nothing, or an element no longer on the page.
  assert.equal(focusBackTarget(body, body, body), null);
  assert.equal(focusBackTarget(null, body, body), null);
  assert.equal(focusBackTarget(gone, body, body), null);
});
