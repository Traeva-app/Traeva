import test from "node:test";
import assert from "node:assert/strict";
import { clearsPending, connectionText, receiptText } from "./diagnostics.js";

test("settings and a receiver-ready signal are not presented as verified extension pairing", () => {
  const value = { config: { enabled: true }, status: {} };
  assert.match(connectionText(value), /not been checked/);
  value.status.connection = { kind: "receiver_unavailable" };
  assert.match(connectionText(value), /cannot tell whether/);
  value.config.enabled = false;
  assert.match(connectionText(value), /off in this extension/);
});

test("acknowledging source progress or an interrupted download is not completion or a confirmed association", () => {
  assert.match(receiptText({ outcome: "saved", downloadState: "in_progress" }), /Waiting.*completion/);
  assert.match(receiptText({ outcome: "saved", downloadState: "interrupted" }), /does not mean the file finished/);
  assert.match(receiptText({ outcome: "saved", downloadState: "complete" }), /suggested file match/);
  assert.match(receiptText({ outcome: "needs_attention", reason: "record_rejected" }), /retries stopped/);
});

test("the queue warning applies to pairing changes and disablement, but not recovery preference changes", () => {
  const config = { enabled: true, endpoint: "http://127.0.0.1:1234", token: "x".repeat(40) };
  assert.equal(clearsPending(config, { ...config, recoverRecent: true }), false);
  assert.equal(clearsPending(config, { ...config, enabled: false }), true);
  assert.equal(clearsPending(config, { ...config, token: "y".repeat(40) }), true);
});

test("invalid settings cannot clear the queue and do not ask for discard consent before validation", () => {
  const config = { enabled: true, endpoint: "http://127.0.0.1:1234", token: "x".repeat(40) };
  assert.equal(clearsPending(config, { ...config, token: "short" }), false);
  assert.equal(clearsPending(config, { ...config, endpoint: "https://example.invalid" }), false);
  assert.equal(clearsPending(config, { ...config, endpoint: "http://127.0.0.1:01234" }), false);
  assert.equal(clearsPending(config, { ...config, enabled: false, token: "short" }), true);
});
