import test from "node:test";
import assert from "node:assert/strict";
import { checkedPreview, snapshotKey } from "./preview.ts";
import type { DocxPreview } from "./preview.ts";
import type { SnapshotRef } from "../family/types";

const snapshot: SnapshotRef = {
  snapshot_id: "synthetic-s1", file_id: "synthetic-f1", root_id: "synthetic-r1",
  sequence: 2, file_revision: 3, relative_path: "Drafts/synthetic.docx",
  sha256: "a".repeat(64), byte_length: 1024, captured_at_ms: 1_728_000_000_000,
};
const preview = (text = "First paragraph\n\n中文 🧪\n<script>alert(1)</script>"): DocxPreview => ({
  snapshot: { ...snapshot },
  extraction: { text, summary: { reader: "docx-cache-local-1", status: "complete", chars: [...text].length,
    covered: ["body text"], left_out: [], note: null } },
});

test("plain text is preserved exactly, including markup, paragraphs and Unicode", () => {
  const result = preview();
  assert.equal(checkedPreview(snapshot, result), result);
  assert.equal(checkedPreview(snapshot, result).extraction.text, result.extraction.text);
});

test("every immutable Snapshot field binds the result and the component key", () => {
  for (const field of Object.keys(snapshot) as (keyof SnapshotRef)[]) {
    const changed = { ...snapshot, [field]: typeof snapshot[field] === "number" ? Number(snapshot[field]) + 1 : `${snapshot[field]}-changed` };
    assert.notEqual(snapshotKey(changed), snapshotKey(snapshot), field);
    assert.throws(() => checkedPreview(snapshot, { ...preview(), snapshot: changed }), undefined, field);
  }
});

test("Partial retains body and disclosed omissions; empty extracted text is valid", () => {
  const result = preview();
  result.extraction.summary.status = "partial";
  result.extraction.summary.left_out = ["field cache freshness (unknown; no evaluation or refresh)"];
  assert.equal(checkedPreview(snapshot, result), result);
  assert.equal(checkedPreview(snapshot, preview("")).extraction.text, "");
});

test("Unsupported and Failed cannot carry body or invented coverage", () => {
  for (const status of ["unsupported", "failed"] as const) {
    const result = preview("");
    Object.assign(result.extraction.summary, { status, covered: [], note: "Synthetic failure reason" });
    assert.equal(checkedPreview(snapshot, result), result);
    result.extraction.text = "hidden body";
    assert.throws(() => checkedPreview(snapshot, result));
    result.extraction.text = "";
    result.extraction.summary.covered = ["body"];
    assert.throws(() => checkedPreview(snapshot, result));
  }
});

test("inconsistent statuses and character counts fail closed", () => {
  const mutations = [
    (p: DocxPreview) => { p.extraction.summary.chars += 1; },
    (p: DocxPreview) => { p.extraction.summary.chars = NaN; },
    (p: DocxPreview) => { p.extraction.summary.chars = -1; },
    (p: DocxPreview) => { p.extraction.summary.status = "partial"; },
    (p: DocxPreview) => { p.extraction.summary.left_out = ["missing text"]; },
    (p: DocxPreview) => { p.extraction.summary.status = "future" as never; },
    (p: DocxPreview) => { p.extraction.summary.note = {} as never; },
    (p: DocxPreview) => { p.extraction.summary.covered = [1] as never; },
    (p: DocxPreview) => { p.extraction.text = "\ud800"; p.extraction.summary.chars = 1; },
  ];
  for (const change of mutations) { const p = preview(); change(p); assert.throws(() => checkedPreview(snapshot, p)); }
  for (const bad of [null, {}, { snapshot }, { snapshot, extraction: null }]) {
    assert.throws(() => checkedPreview(snapshot, bad as DocxPreview));
  }
});

test("Reader-sized text is accepted but oversized text and metadata are rejected", () => {
  assert.equal(checkedPreview(snapshot, preview("🧪".repeat(1_000_000))).extraction.summary.chars, 1_000_000);
  assert.throws(() => checkedPreview(snapshot, preview("a".repeat(1_000_001))));
  assert.throws(() => checkedPreview(snapshot, preview("🧪".repeat(1_000_001))));
  const p = preview();
  p.extraction.summary.note = "x".repeat(16_385);
  assert.throws(() => checkedPreview(snapshot, p));
  p.extraction.summary.note = null;
  p.extraction.summary.covered = Array(257).fill("x");
  assert.throws(() => checkedPreview(snapshot, p));
});
