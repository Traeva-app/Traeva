import assert from "node:assert/strict";
import { test } from "node:test";
import {
  isWebLink,
  recordName,
  replaceRecord,
  sourceBadge,
  sourceError,
  sourceGroup,
  sourcesForFile,
  statusText,
  timeText,
} from "./view.ts";
import type { SourceRecordView, SourceState } from "./types.ts";

function record(overrides: Partial<SourceRecordView> = {}): SourceRecordView {
  return {
    source_id: "source-1", profile_id: "synthetic-profile", download_id: "1",
    download_state: "complete", status: "unknown", filename: "/synthetic/downloads/paper.txt",
    initial_url: "https://example.test/download", final_url: null, referrer: null, title: null,
    started_at_ms: 1000, completed_at_ms: null, imported_at_ms: 2000, updated_at_ms: 2000,
    reason: "No current file observation.", candidates: [], association: null,
    ...overrides,
  };
}

test("unconfirmed, user-confirmed and copy records never acquire a Verified label", () => {
  assert.equal(statusText("unknown"), "Unknown");
  assert.match(statusText("likely"), /needs your confirmation/);
  assert.equal(statusText("user_confirmed"), "User Confirmed");
});

test("open actions exclude local files, script protocols, blobs and malformed addresses", () => {
  for (const url of [null, "", "not a URL", "/local/file", "file:///tmp/file", "javascript:alert(1)", "data:text/html,example", "blob:https://example.test/123", "traeva://open", "mailto:a@example.test"]) {
    assert.equal(isWebLink(url), false, String(url));
  }
  assert.equal(isWebLink("https://example.test/download?token=synthetic"), true);
  assert.equal(isWebLink("http://127.0.0.1/synthetic"), true);
});

test("missing download time stays unknown, even when an import time is present", () => {
  const source = record();
  assert.equal(timeText(source.completed_at_ms, "Completion unknown"), "Completion unknown");
  assert.notEqual(timeText(source.imported_at_ms, "Import unknown"), "Import unknown");
  assert.equal(timeText(Number.NaN, "Unknown"), "Unknown");
  assert.notEqual(timeText(0, "Unknown"), "Unknown");
});

test("a completed command is shown even when the wall clock moves backwards", () => {
  const state: SourceState = { enabled: true, records: [record({ updated_at_ms: 3000 })] };
  const confirmed = record({ status: "user_confirmed", updated_at_ms: 2000 });
  assert.equal(replaceRecord(state, confirmed)?.records[0], confirmed);
});

test("two download events remain separate while repeated updates replace only their record", () => {
  const first = record();
  const second = record({ source_id: "source-2", download_id: "2" });
  const state: SourceState = { enabled: false, records: [first] };
  const both = replaceRecord(state, second)!;
  assert.equal(both.enabled, false);
  assert.equal(both.records.length, 2);
  const updated = replaceRecord(both, { ...second, updated_at_ms: 4000 })!;
  assert.equal(updated.records.length, 2);
  assert.equal(updated.records.find((item) => item.source_id === "source-1"), first);
});

test("download names come from recorded paths, never an inferred title", () => {
  assert.equal(recordName(record()), "paper.txt");
  assert.equal(recordName(record({ filename: "" })), "Download name unknown");
});

test("a structured app-data error is visible without serializing its other properties", () => {
  assert.equal(sourceError({ message: "Source storage is unavailable.", privateDetail: "do not display" }), "Source storage is unavailable.");
  assert.equal(sourceError(null), "The request did not finish. Try again.");
});

test("file details select exact associations or candidates, never matching names or content", () => {
  const target = record({ candidates: [{ file_id: "file-1", display_path: "/synthetic/paper.txt", reason: "Candidate", can_confirm: true }] });
  const unrelated = record({ source_id: "source-other" });
  const linked = record({ source_id: "source-linked", status: "user_confirmed", association: {
    file_id: "file-1", display_path: "/synthetic/moved.txt", relation: "copy", basis: "user_confirmed", note: "", confirmed_at_ms: 2000, current_location_confirmed: false,
  } });
  assert.deepEqual(sourcesForFile([target, unrelated, linked], "file-1"), [target, linked]);
  assert.deepEqual(sourcesForFile([target, unrelated, linked], "same-family-another-file"), []);
});

test("a candidate needs the user; unmatched or running downloads are in progress; confirmed or interrupted ones are finished", () => {
  const likely = record({ status: "likely" });
  assert.equal(sourceGroup(likely), "needs");
  assert.deepEqual(sourceBadge(likely), { label: "Needs your confirmation", tone: "warn" });

  // Unknown: Traeva keeps looking, and the user may locate the file (A, 10/14).
  assert.equal(sourceGroup(record()), "progress");
  assert.equal(sourceBadge(record()).tone, "neutral");
  const running = record({ download_state: "in_progress" });
  assert.equal(sourceGroup(running), "progress");
  assert.equal(sourceBadge(running).label, "Downloading");

  const confirmed = record({ status: "user_confirmed" });
  assert.equal(sourceGroup(confirmed), "finished");
  assert.deepEqual(sourceBadge(confirmed), { label: "User Confirmed", tone: "ok" });
  const interrupted = record({ download_state: "interrupted" });
  assert.equal(sourceGroup(interrupted), "finished");
  assert.deepEqual(sourceBadge(interrupted), { label: "Interrupted", tone: "neutral" });

  // Only a candidate is amber, and no badge claims verification.
  for (const each of [likely, record(), running, confirmed, interrupted]) {
    assert.equal(sourceBadge(each).tone === "warn", sourceGroup(each) === "needs");
    assert.doesNotMatch(sourceBadge(each).label, /verif/i);
  }
});
