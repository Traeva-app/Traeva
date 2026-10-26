import test from "node:test";
import assert from "node:assert/strict";
import { endpointUrl, validToken, insideRoot, downloadEvent, parseState, createApi, backoffMs } from "./protocol.js";

const token = "a".repeat(32);
const config = { endpoint: "http://127.0.0.1:43127", token };
const profileId = "11060739-c135-40c8-8704-48d40c90af78";
const roots = [{ root_id: "r1", path: "/synthetic/downloads" }];
const item = {
  id: 42, incognito: false, filename: "/synthetic/downloads/report.txt", state: "complete",
  startTime: "2026-10-08T10:00:00Z", endTime: "2026-10-08T10:00:02Z", fileSize: 500,
  url: "https://origin.invalid/start?token=private", finalUrl: "https://cdn.invalid/file", referrer: "https://origin.invalid/",
};

test("pairing endpoint is literal IPv4 loopback with explicit port and no URL extras", () => {
  assert.equal(endpointUrl("http://127.0.0.1:43127/"), config.endpoint);
  assert.equal(endpointUrl("http://127.0.0.1:80"), "http://127.0.0.1:80");
  for (const value of ["https://127.0.0.1:43127", "http://localhost:43127", "http://2130706433:43127", "http://127.0.0.1.evil.invalid:43127", "http://127.0.0.1:0", "http://127.0.0.1:65536", "http://user@127.0.0.1:43127", "http://127.0.0.1:43127/path", "http://127.0.0.1:43127?redirect=evil", "http://127.0.0.1:43127#ignored", "http://127.0.0.1"]) {
    assert.throws(() => endpointUrl(value), undefined, value);
  }
  assert.ok(validToken(token));
  assert.equal(validToken("x\r\nAuthorization: stolen"), false);
});

test("scope filter uses whole path components and rejects dot traversal", () => {
  assert.ok(insideRoot("/synthetic/downloads/a.txt", roots[0].path));
  assert.ok(insideRoot("/synthetic/downloads/nested/a.txt", `${roots[0].path}/`));
  for (const file of ["/synthetic/downloads-evil/a.txt", "/synthetic/downloads", "/synthetic/downloads/../outside.txt", "/synthetic/downloads/./a.txt", "relative/a.txt", "/synthetic/downloads/a\0.txt"]) {
    assert.equal(insideRoot(file, roots[0].path), false, file);
  }
});

test("event preserves original/final/referrer meanings and has no title or association claim", () => {
  const value = downloadEvent(item, profileId, roots);
  assert.equal(value.initial_url, item.url);
  assert.equal(value.final_url, item.finalUrl);
  assert.equal(value.referrer, item.referrer);
  assert.equal(value.download_id, "42");
  assert.equal(value.completed_at_ms, Date.parse(item.endTime));
  assert.equal(value.started_at_ms, Date.parse(item.startTime));
  assert.equal(Object.hasOwn(value, "title"), false);
  assert.equal(Object.hasOwn(value, "confidence"), false);
  assert.equal(Object.hasOwn(value, "imported_at"), false);
});

test("unknown fields stay unknown and interrupted end time is not download completion", () => {
  const value = downloadEvent({ ...item, state: "interrupted", fileSize: -1, finalUrl: "", referrer: undefined, startTime: "invalid" }, profileId, roots);
  assert.equal(value.completed_at_ms, null);
  assert.equal(value.started_at_ms, null);
  assert.equal(value.file_size, null);
  assert.equal(value.final_url, null);
  assert.equal(value.referrer, null);
  assert.equal(downloadEvent({ ...item, state: "in_progress" }, profileId, roots).completed_at_ms, null);
  assert.equal(downloadEvent({ ...item, fileSize: 0 }, profileId, roots).file_size, 0);
});

test("incognito, missing privacy flag, out-of-scope and unnamed events are never sent", () => {
  for (const override of [{ incognito: true }, { incognito: undefined }, { filename: "" }, { filename: "/elsewhere/file" }, { state: "unknown" }]) {
    assert.equal(downloadEvent({ ...item, ...override }, profileId, roots), null);
  }
});

test("receiver state rejects malformed authorization data", () => {
  assert.deepEqual(parseState({ enabled: true, roots }), { enabled: true, roots });
  for (const value of [{ enabled: "true", roots }, { enabled: true, roots: [{ root_id: "r", path: "/safe/../bad" }] }, { enabled: true }]) {
    assert.throws(() => parseState(value), /invalid_response/);
  }
});

test("transport never follows redirects or attaches cookies/referrer and keeps bearer local", async () => {
  const calls = [];
  const api = createApi(async (...args) => {
    calls.push(args);
    return new Response(JSON.stringify(args[1].method === "GET" ? { enabled: true, roots } : { outcome: "saved", source_id: "s1", reason: null }));
  });
  await api.state(config);
  await api.send(config, downloadEvent(item, profileId, roots));
  assert.equal(calls[0][0], `${config.endpoint}/v1/source/state`);
  assert.equal(calls[1][0], `${config.endpoint}/v1/source/events`);
  for (const [, options] of calls) {
    assert.equal(options.redirect, "error");
    assert.equal(options.credentials, "omit");
    assert.equal(options.cache, "no-store");
    assert.equal(options.referrerPolicy, "no-referrer");
    assert.equal(options.headers.Authorization, `Bearer ${token}`);
  }
});

test("failure bodies never become error text and malformed acknowledgments are not saved", async () => {
  for (const [response, code] of [
    [new Response("secret URL", { status: 400 }), "record_rejected"],
    [new Response("secret URL", { status: 413 }), "record_rejected"],
    [new Response("secret URL", { status: 401 }), "pairing_rejected"],
    [new Response("secret URL", { status: 503 }), "receiver_rejected"],
    [new Response("secret URL", { status: 200 }), "invalid_response"],
    [Response.json({ outcome: "saved" }), "invalid_response"],
  ]) {
    await assert.rejects(() => createApi(async () => response).send(config, item), new RegExp(`^Error: ${code}$`));
  }
  await assert.rejects(() => createApi(async () => { throw new Error("private URL"); }).state(config), /^Error: receiver_unavailable$/);
});

test("oversized payloads fail visibly without attempting a receiver POST", async () => {
  let called = false;
  await assert.rejects(() => createApi(async () => { called = true; }).send(config, { url: "x".repeat(65_536) }), /^Error: record_rejected$/);
  assert.equal(called, false);
});

test("retry delay grows and is bounded", () => {
  assert.equal(backoffMs(0), 30_000);
  assert.equal(backoffMs(1), 60_000);
  assert.equal(backoffMs(100), 3_600_000);
});
