import assert from "node:assert/strict";
import { test } from "node:test";
import { sourceFileId } from "./organize-source.ts";
import type { Flow } from "../organize/shared.tsx";

test("the file detail section follows the actual operation, plan and action IDs", () => {
  assert.equal(sourceFileId({ step: "start" }), null);
  assert.equal(sourceFileId({ step: "operation", operation: { file: null } } as Flow), null);
  assert.equal(sourceFileId({ step: "operation", operation: { file: { file_id: "observed" } } } as Flow), "observed");
  assert.equal(sourceFileId({ step: "preview", plan: { file_id: "planned" } } as Flow), "planned");
  assert.equal(sourceFileId({ step: "applying", plan: { file_id: "moving" } } as Flow), "moving");
  assert.equal(sourceFileId({ step: "done", action: { file_id: "committed" } } as Flow), "committed");
  assert.equal(sourceFileId({ step: "in_place", file: { file_id: "already-there" } } as Flow), "already-there");
  assert.equal(sourceFileId({ step: "failed", title: "Failure", error: null, detail: "Unavailable" }), null);
});
