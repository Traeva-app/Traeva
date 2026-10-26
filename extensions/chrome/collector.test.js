import test from "node:test";
import assert from "node:assert/strict";
import { Collector } from "./collector.js";
import { MAX_PENDING, REPLAY_WINDOW_MS } from "./protocol.js";

const epoch = Date.parse("2026-10-08T10:00:00Z");
const settings = { enabled: true, endpoint: "http://127.0.0.1:43127", token: "x".repeat(40), recoverRecent: false };
const sample = (id, extra = {}) => ({
  id, incognito: false, filename: `/synthetic/downloads/${id}.txt`, state: "complete", fileSize: 42,
  startTime: new Date(epoch + 1).toISOString(), endTime: new Date(epoch + 1000).toISOString(),
  url: "https://private.invalid/signed?secret=never-store", finalUrl: "https://cdn.invalid/file", ...extra,
});

function fixture() {
  let time = epoch;
  const data = {};
  const items = new Map();
  const sent = [];
  const searches = [];
  const alarmCalls = [];
  const storage = {
    async get(keys) { return structuredClone(Object.fromEntries(keys.filter((key) => key in data).map((key) => [key, data[key]]))); },
    async set(value) { Object.assign(data, structuredClone(value)); },
  };
  const downloads = { async search(query) {
    searches.push(query);
    return structuredClone("id" in query ? (items.has(query.id) ? [items.get(query.id)] : []) : [...items.values()].filter((item) => Date.parse(item.startTime) > Date.parse(query.startedAfter)).slice(0, query.limit));
  } };
  const alarms = { async create(...args) { alarmCalls.push(["create", ...args]); }, async clear(...args) { alarmCalls.push(["clear", ...args]); } };
  const api = {
    async state() { return { enabled: true, enabled_since_ms: epoch, roots: [{ root_id: "r1", path: "/synthetic/downloads" }] }; },
    async send(config, event) { sent.push(event); return { outcome: "saved", source_id: `s${event.download_id}`, reason: null }; },
  };
  const deps = { storage, downloads, alarms, api, now: () => time, uuid: () => "9b76e6ba-efb1-42b8-b309-1a282a41042e" };
  return { data, items, sent, searches, alarmCalls, api, deps, collector: new Collector(deps), advance: (ms) => { time += ms; } };
}

test("default off creates one stable profile identity and never queries downloads", async () => {
  const f = fixture();
  const first = await f.collector.load();
  assert.equal(first.config.enabled, false);
  await f.collector.enqueue(1);
  await f.collector.replay();
  await f.collector.drain();
  assert.equal(f.searches.length, 0);
  assert.equal((await new Collector(f.deps).load()).config.profileId, first.config.profileId);
});

test("progress and completion are delivered separately, acknowledgement required before dequeue", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { state: "in_progress", fileSize: -1, endTime: undefined }));
  await f.collector.enqueue(1);
  await f.collector.drain();
  assert.equal(f.sent[0].completed_at_ms, null);
  assert.equal(f.data.pending.length, 1);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  await f.collector.drain();
  assert.deepEqual(f.sent.map((event) => event.state), ["in_progress", "complete"]);
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.sent[0].profile_id, f.sent[1].profile_id);
});

test("network outage keeps IDs only, backs off, and resumes after worker restart", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.send = async () => { throw new Error("receiver_unavailable"); };
  await f.collector.drain();
  assert.deepEqual(f.data.pending, [{ id: 1, attempt: 1, nextAt: epoch + 30_000, explicitHistory: false, error: "receiver_unavailable" }]);
  assert.equal(f.data.status.lastSavedAt, undefined);
  assert.equal(JSON.stringify(f.data).includes("private.invalid"), false);
  assert.equal(JSON.stringify(f.data).includes("never-store"), false);
  assert.equal(JSON.stringify(f.data).includes("1.txt"), false);
  f.api.send = async (_, event) => { f.sent.push(event); return { outcome: "saved", source_id: "s1" }; };
  const restarted = new Collector(f.deps);
  await restarted.drain();
  assert.equal(f.sent.length, 0);
  f.advance(30_000);
  await restarted.drain();
  assert.equal(f.sent.length, 1);
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.data.status.lastSavedAt, epoch + 30_000);
});

test("a save followed by lost response safely resends the same event identity", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.send = async (_, event) => { f.sent.push(event); throw new Error("receiver_unavailable"); };
  await f.collector.drain();
  assert.equal(f.data.status.lastSavedAt, undefined);
  f.advance(30_000);
  f.api.send = async (_, event) => { f.sent.push(event); return { outcome: "saved", source_id: "s1" }; };
  await f.collector.drain();
  assert.deepEqual(f.sent[0], f.sent[1]);
});

test("incognito and unauthorized paths never send metadata", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { incognito: true }));
  f.items.set(2, sample(2, { filename: "/synthetic/downloads-other/file.txt" }));
  f.items.set(3, sample(3, { filename: "/synthetic/downloads/../other.txt" }));
  for (const id of [1, 2, 3]) await f.collector.enqueue(id);
  await f.collector.drain();
  assert.equal(f.sent.length, 0);
  assert.equal(f.data.pending.length, 0);
});

test("blank filename waits; later final path change permits delivery", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { filename: "" }));
  await f.collector.enqueue(1);
  await f.collector.drain();
  assert.equal(f.sent.length, 0);
  assert.equal(f.data.pending.length, 1);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  await f.collector.drain();
  assert.equal(f.sent.length, 1);
});

test("disabling clears pending even if unsaved settings contain an invalid address", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  await f.collector.saveSettings({ ...settings, enabled: false, endpoint: "http://remote.invalid", token: "bad" });
  assert.equal(f.data.config.enabled, false);
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.alarmCalls.at(-1)[0], "clear");
  await f.collector.drain();
  assert.equal(f.sent.length, 0);
});

test("changing pairing does not forward old pending events to a new app", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  const profile = f.data.config.profileId;
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.advance(5000);
  await f.collector.saveSettings({ ...settings, endpoint: "http://127.0.0.1:12345" });
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.data.config.captureSinceMs, epoch + 5000);
  assert.equal(f.data.config.profileId, profile);
  await f.collector.enqueue(1);
  assert.equal(f.data.pending.length, 0);
});

test("receiver disablement discards pending without any event POST", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.state = async () => ({ enabled: false, roots: [] });
  await f.collector.drain();
  assert.equal(f.sent.length, 0);
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.data.status.error, "collection_disabled");
});

test("offline queue and startup recovery exclude downloads before the app's new interval", async () => {
  const f = fixture();
  await f.collector.saveSettings({ ...settings, recoverRecent: true });
  f.api.state = async () => { throw new Error("receiver_unavailable"); };
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  await f.collector.drain();
  f.advance(30_000);
  f.api.state = async () => ({ enabled: true, enabled_since_ms: epoch + 30_000, roots: [{ root_id: "r1", path: "/synthetic/downloads" }] });
  const restarted = new Collector(f.deps);
  await restarted.replay();
  await restarted.drain();
  assert.equal(f.sent.length, 0);
  assert.equal(f.data.pending.length, 0);
  f.items.set(2, sample(2, { startTime: new Date(epoch + 30_001).toISOString() }));
  await restarted.enqueue(2);
  await restarted.drain();
  assert.deepEqual(f.sent.map((event) => event.download_id), ["2"]);
  assert.equal(f.sent[0].explicit_history, false);
});

test("explicit history choice survives an outage and worker restart", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { startTime: new Date(epoch - 1000).toISOString() }));
  await f.collector.replay({ explicitHistory: true });
  f.api.state = async () => { throw new Error("receiver_unavailable"); };
  await f.collector.drain();
  f.advance(30_000);
  f.api.state = async () => ({ enabled: true, enabled_since_ms: epoch + 30_000, roots: [{ root_id: "r1", path: "/synthetic/downloads" }] });
  await new Collector(f.deps).drain();
  assert.equal(f.sent.length, 1);
  assert.equal(f.sent[0].explicit_history, true);
  assert.equal(f.data.pending.length, 0);
  assert.equal(JSON.stringify(f.data).includes("private.invalid"), false);
});

test("missing app capture window fails closed for automatic events", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.state = async () => ({ enabled: true, roots: [{ root_id: "r1", path: "/synthetic/downloads" }] });
  await f.collector.drain();
  assert.equal(f.sent.length, 0);
  assert.equal(f.data.pending.length, 0);
});

test("history only scans on explicit request or separately enabled startup recovery", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { startTime: new Date(epoch - 1000).toISOString() }));
  await f.collector.replay();
  assert.equal(f.searches.length, 0);
  await f.collector.enqueue(1);
  assert.equal(f.data.pending?.length ?? 0, 0);
  await f.collector.replay({ explicitHistory: true });
  assert.equal(f.data.pending.length, 1);
  const query = f.searches.find((value) => "startedAfter" in value);
  assert.equal(query.startedAfter, new Date(epoch - REPLAY_WINDOW_MS).toISOString());
  assert.equal(query.limit, MAX_PENDING);
});

test("startup recovery never reaches before enablement time", async () => {
  const f = fixture();
  await f.collector.saveSettings({ ...settings, recoverRecent: true });
  f.items.set(1, sample(1, { startTime: new Date(epoch - 1).toISOString() }));
  f.items.set(2, sample(2));
  await f.collector.replay();
  assert.equal(f.searches[0].startedAfter, new Date(epoch).toISOString());
  assert.deepEqual(f.data.pending.map((entry) => entry.id), [2]);
});

test("pending IDs deduplicate while distinct downloads with identical URLs remain separate", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  f.items.set(2, sample(2));
  await f.collector.enqueue(1);
  await f.collector.enqueue(1);
  await f.collector.enqueue(2);
  assert.equal(f.data.pending.length, 2);
  await f.collector.drain();
  assert.deepEqual(f.sent.map((event) => event.download_id), ["1", "2"]);
});

test("queue is bounded and overflow is visible without recording URL payloads", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  for (let id = 0; id <= MAX_PENDING; id++) {
    f.items.set(id, sample(id));
    await f.collector.enqueue(id);
  }
  assert.equal(f.data.pending.length, MAX_PENDING);
  assert.equal(f.data.status.queueFull, true);
  const displayed = await f.collector.publicStatus();
  assert.equal(displayed.pendingCount, MAX_PENDING);
  assert.equal(displayed.pending.length, 20);
  assert.equal(Object.keys(displayed.labels).length, 20);
  assert.equal(JSON.stringify(f.data).includes("never-store"), false);
});

test("erased history and core ignored results leave no retry loop", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  f.items.set(2, sample(2));
  await f.collector.enqueue(1);
  await f.collector.enqueue(2);
  f.items.delete(1);
  f.api.send = async () => ({ outcome: "ignored", source_id: null, reason: "outside_scope" });
  await f.collector.drain();
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.data.status.lastSavedAt, undefined);
});

test("unexpected errors never persist source-bearing exception text", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.send = async () => { throw new Error("https://private.invalid/?secret=token"); };
  await f.collector.drain();
  assert.equal(f.data.status.error, "delivery_failed");
  assert.equal(JSON.stringify(f.data).includes("private.invalid"), false);
});

test("terminal record rejection is visible without claiming a save or retrying forever", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  f.api.send = async () => { throw new Error("record_rejected"); };
  await f.collector.drain();
  assert.equal(f.data.pending.length, 0);
  assert.equal(f.data.status.lastSavedAt, undefined);
  assert.equal(f.data.status.rejectedCount, 1);
  assert.equal(f.data.status.error, "record_rejected");
});

test("live connection checks distinguish pairing, unavailable receiver, disabled capture and unavailable folders without storing paths", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  for (const error of ["pairing_rejected", "receiver_unavailable"]) {
    f.api.state = async () => { throw new Error(error); };
    await assert.rejects(() => f.collector.checkConnection(), { message: error });
    assert.equal(f.data.status.connection.kind, error);
  }
  f.api.state = async () => ({ enabled: false, roots: [] });
  await f.collector.checkConnection();
  assert.equal(f.data.status.connection.kind, "collection_disabled");
  f.api.state = async () => ({ enabled: true, roots: [] });
  await f.collector.checkConnection();
  assert.equal(f.data.status.connection.kind, "no_folders");
  f.api.state = async () => ({ enabled: true, roots: [{ path: "/private-not-persisted", root_id: "r" }] });
  const checked = await f.collector.checkConnection();
  assert.equal(checked.delivery.status.connection.kind, "connected");
  assert.equal(checked.roots[0].path, "/private-not-persisted");
  assert.equal(checked.delivery.roots[0].path, "/private-not-persisted");
  assert.deepEqual((await new Collector(f.deps).publicStatus()).roots, []);
  assert.equal(JSON.stringify(f.data).includes("private-not-persisted"), false);
});

test("progress acknowledgement, terminal acknowledgement and unacknowledged failures remain separate across restart", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1, { state: "in_progress" }));
  f.items.set(2, sample(2));
  await f.collector.enqueue(1);
  await f.collector.enqueue(2);
  f.api.send = async (_, event) => {
    if (event.download_id === "2") throw new Error("receiver_unavailable");
    return { outcome: "saved", source_id: "s1" };
  };
  await f.collector.drain();
  const restarted = new Collector(f.deps);
  let state = await restarted.publicStatus();
  assert.equal(state.pending.length, 2);
  assert.equal(state.pending.find((entry) => entry.id === 1).error, "waiting_completion");
  assert.equal(state.pending.find((entry) => entry.id === 2).error, "receiver_unavailable");
  assert.deepEqual(state.status.recent.map((entry) => [entry.id, entry.downloadState]), [[1, "in_progress"]]);
  f.items.set(1, sample(1));
  await restarted.enqueue(1);
  await restarted.drain();
  state = await restarted.publicStatus();
  assert.deepEqual(state.pending.map((entry) => entry.id), [2]);
  assert.deepEqual(state.status.recent.map((entry) => [entry.id, entry.downloadState]), [[1, "complete"]]);
});

test("a later successful delivery cannot hide an earlier failure in the same pass", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  for (const id of [1, 2]) { f.items.set(id, sample(id)); await f.collector.enqueue(id); }
  f.api.send = async (_, event) => {
    if (event.download_id === "1") throw new Error("record_rejected");
    return { outcome: "saved", source_id: "s2" };
  };
  await f.collector.drain();
  assert.equal(f.data.status.error, "record_rejected");
  assert.deepEqual(f.data.status.recent.map((entry) => [entry.id, entry.outcome]), [[2, "saved"], [1, "needs_attention"]]);
});

test("skipped folders, collection intervals, erased history and ignored receipts explain why there is no acknowledged save", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  for (const id of [1, 2, 3]) { f.items.set(id, sample(id)); await f.collector.enqueue(id); }
  await f.collector.erased(1);
  f.items.set(2, sample(2, { filename: "/outside/2.txt" }));
  f.api.send = async () => ({ outcome: "ignored", reason: "https://private.invalid/?secret=never-store" });
  await f.collector.drain();
  assert.deepEqual(f.data.status.recent.map((entry) => [entry.id, entry.reason]), [[3, "receiver_ignored"], [2, "outside_folders"], [1, "download_erased"]]);
  assert.equal(f.data.status.lastSavedAt, undefined);
  assert.equal(JSON.stringify(f.data).includes("private.invalid"), false);
});

test("receipts are bounded and names are only read into the current options response", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  for (let id = 1; id <= 25; id++) {
    f.items.set(id, sample(id));
    await f.collector.enqueue(id);
    await f.collector.drain();
  }
  const value = await f.collector.publicStatus();
  assert.equal(value.status.recent.length, 20);
  assert.equal(value.labels[25], "25.txt");
  for (const sensitive of ["private.invalid", "never-store", "/synthetic", ".txt", "finalUrl"]) {
    assert.equal(JSON.stringify(f.data).includes(sensitive), false, sensitive);
  }
});

test("serialized settings refuse to silently discard downloads arriving after the options snapshot", async () => {
  const f = fixture();
  await f.collector.saveSettings(settings);
  f.items.set(1, sample(1));
  await f.collector.enqueue(1);
  await assert.rejects(() => f.collector.saveSettings({ ...settings, token: "y".repeat(40) }, { requireDiscardConsent: true }), { message: "pending_clear_confirmation" });
  assert.equal(f.data.config.token, settings.token);
  assert.equal(f.data.pending.length, 1);
  await f.collector.saveSettings({ ...settings, token: "y".repeat(40), discardPending: true }, { requireDiscardConsent: true });
  assert.equal(f.data.pending.length, 0);
});
