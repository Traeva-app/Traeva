import { MAX_PENDING, RETRY_ALARM, REPLAY_WINDOW_MS, backoffMs, downloadEvent, endpointUrl, validToken } from "./protocol.js";

const SAFE_ERRORS = new Set(["invalid_endpoint", "invalid_token", "pairing_rejected", "receiver_unavailable", "receiver_rejected", "invalid_response", "record_rejected", "pending_clear_confirmation"]);

export class Collector {
  constructor({ storage, downloads, alarms, api, now = Date.now, uuid = () => crypto.randomUUID() }) {
    Object.assign(this, { storage, downloads, alarms, api, now, uuid });
  }

  async load() {
    const data = await this.storage.get(["config", "pending", "status"]);
    const config = data.config ?? {
      enabled: false, endpoint: "", token: "", profileId: this.uuid(), captureSinceMs: null, recoverRecent: false,
    };
    if (!data.config) await this.storage.set({ config });
    return { config, pending: data.pending ?? [], status: data.status ?? {} };
  }

  async saveSettings(input, { requireDiscardConsent = false } = {}) {
    const data = await this.load();
    const enabled = input.enabled === true;
    let endpoint = data.config.endpoint;
    let token = data.config.token;
    try {
      endpoint = input.endpoint ? endpointUrl(input.endpoint) : "";
      token = typeof input.token === "string" ? input.token.trim() : "";
      if (enabled && !endpoint) throw new Error("invalid_endpoint");
      if ((enabled || token) && !validToken(token)) throw new Error("invalid_token");
    } catch (error) {
      if (enabled) throw error;
      endpoint = data.config.endpoint;
      token = data.config.token;
    }
    const changedPair = endpoint !== data.config.endpoint || token !== data.config.token;
    if (requireDiscardConsent && data.pending.length && (!enabled || changedPair) && input.discardPending !== true) {
      throw new Error("pending_clear_confirmation");
    }
    const config = {
      enabled, endpoint, token, profileId: data.config.profileId,
      captureSinceMs: enabled ? (!data.config.enabled || changedPair ? this.now() : data.config.captureSinceMs) : null,
      recoverRecent: input.recoverRecent === true,
    };
    await this.storage.set({ config, ...(!enabled || changedPair ? { pending: [], status: {} } : {}) });
    if (enabled) await this.alarms.create(RETRY_ALARM, { periodInMinutes: 1 });
    else await this.alarms.clear(RETRY_ALARM);
    return this.publicStatus();
  }

  async publicStatus() {
    const { config, pending, status } = await this.load();
    const displayedPending = pending.slice(0, 20);
    const ids = [...new Set([...displayedPending.map((entry) => entry.id), ...(status.recent ?? []).map((entry) => entry.id)])].slice(0, 40);
    const labels = {};
    for (const id of ids) {
      try {
        const item = (await this.downloads.search({ id }))[0];
        if (item?.incognito === false && item.filename) labels[id] = item.filename.split(/[\\/]/).pop();
      } catch { /* Labels are optional and never persisted. */ }
    }
    const roots = status.connection?.kind === "connected" && status.connection.checkedAt === this.visibleRoots?.checkedAt ? this.visibleRoots.roots : [];
    return { config, pendingCount: pending.length, pending: displayedPending.map(({ id, nextAt, error }) => ({ id, nextAt, error })), status, labels, roots };
  }

  async checkConnection() {
    const data = await this.load();
    try {
      const state = await this.api.state(data.config);
      this.connected(data, state);
      await this.storage.set({ status: data.status });
      return { enabled: state.enabled, roots: state.roots, delivery: await this.publicStatus() };
    } catch (error) {
      data.status.connection = { kind: this.safeError(error), checkedAt: this.now() };
      await this.storage.set({ status: data.status });
      throw error;
    }
  }

  connected(data, state) {
    data.status.connection = { kind: !state.enabled ? "collection_disabled" : state.roots.length ? "connected" : "no_folders", checkedAt: this.now() };
    this.visibleRoots = { roots: state.roots, checkedAt: data.status.connection.checkedAt };
  }

  receipt(data, entry, outcome, reason, downloadState) {
    data.status.recent = [{ id: entry.id, outcome, reason, downloadState, at: this.now() },
      ...(data.status.recent ?? []).filter((value) => value.id !== entry.id)].slice(0, 20);
  }

  async erased(id) {
    const data = await this.load();
    const entry = data.pending.find((value) => value.id === id);
    if (!entry) return;
    this.receipt(data, entry, "needs_attention", "download_erased");
    await this.storage.set({ pending: data.pending.filter((value) => value.id !== id), status: data.status });
  }

  async enqueue(id, { explicitHistory = false } = {}) {
    if (!Number.isSafeInteger(id) || id < 0) return;
    const data = await this.load();
    if (!data.config.enabled) return;
    let items;
    try { items = await this.downloads.search({ id }); } catch { return; }
    const item = items[0];
    if (!item || item.incognito !== false) return;
    const existing = data.pending.find((entry) => entry.id === id);
    if (!existing && !explicitHistory && !(Date.parse(item.startTime) >= data.config.captureSinceMs)) return;
    if (existing) {
      existing.nextAt = 0;
      if (explicitHistory) existing.explicitHistory = true;
    } else if (data.pending.length < MAX_PENDING) {
      data.pending.push({ id, attempt: 0, nextAt: 0, explicitHistory });
    } else {
      data.status.queueFull = true;
    }
    await this.storage.set({ pending: data.pending, status: data.status });
    await this.alarms.create(RETRY_ALARM, { periodInMinutes: 1 });
  }

  async replay({ explicitHistory = false } = {}) {
    const { config } = await this.load();
    if (!config.enabled || (!explicitHistory && !config.recoverRecent)) return;
    // Pending IDs resume separately; a broad, implicit history scan never runs.
    const since = Math.max(this.now() - REPLAY_WINDOW_MS, explicitHistory ? 0 : config.captureSinceMs);
    const items = await this.downloads.search({ startedAfter: new Date(since).toISOString(), limit: MAX_PENDING, orderBy: ["-startTime"] });
    for (const item of items) await this.enqueue(item.id, { explicitHistory });
  }

  async drain() {
    const data = await this.load();
    if (!data.config.enabled || data.pending.length === 0) return;
    const due = data.pending.filter((entry) => entry.nextAt <= this.now()).slice(0, 20);
    if (!due.length) return;
    let state;
    try {
      state = await this.api.state(data.config);
    } catch (error) {
      for (const entry of due) { this.retry(entry); entry.error = this.safeError(error); }
      data.status.error = this.safeError(error);
      data.status.connection = { kind: data.status.error, checkedAt: this.now() };
      await this.storage.set({ pending: data.pending, status: data.status });
      return;
    }
    this.connected(data, state);
    if (!state.enabled) {
      for (const entry of data.pending) this.receipt(data, entry, "not_saved", "collection_disabled");
      await this.storage.set({ pending: [], status: { ...data.status, error: "collection_disabled" } });
      return;
    }
    data.status.error = null;
    for (const entry of due) {
      let item;
      try { item = (await this.downloads.search({ id: entry.id }))[0]; } catch {
        this.retry(entry);
        entry.error = "download_unavailable";
        data.status.error = "download_unavailable";
        continue;
      }
      if (!item || item.incognito !== false) {
        if (!item) this.receipt(data, entry, "needs_attention", "download_erased");
        data.pending = data.pending.filter((value) => value.id !== entry.id);
        continue;
      }
      if (!entry.explicitHistory && !(Number.isSafeInteger(state.enabled_since_ms)
        && Date.parse(item.startTime) >= Math.max(data.config.captureSinceMs, state.enabled_since_ms))) {
        this.receipt(data, entry, "not_saved", "outside_interval");
        data.pending = data.pending.filter((value) => value.id !== entry.id);
        continue;
      }
      if (!item.filename) { this.retry(entry); entry.error = "waiting_filename"; continue; }
      const event = downloadEvent(item, data.config.profileId, state.roots);
      if (!event) {
        this.receipt(data, entry, "not_saved", state.roots.length ? "outside_folders" : "no_folders");
        data.pending = data.pending.filter((value) => value.id !== entry.id);
        continue;
      }
      event.explicit_history = entry.explicitHistory === true;
      try {
        const ack = await this.api.send(data.config, event);
        if (ack.outcome === "saved") {
          data.status.lastSavedAt = this.now();
          this.receipt(data, entry, "saved", null, event.state);
        } else {
          this.receipt(data, entry, "not_saved", "receiver_ignored");
        }
        if (event.state !== "in_progress" || ack.outcome === "ignored") {
          data.pending = data.pending.filter((value) => value.id !== entry.id);
        } else {
          entry.attempt = 0;
          entry.error = "waiting_completion";
          entry.nextAt = this.now() + 60_000;
        }
      } catch (error) {
        if (error?.message === "record_rejected") {
          data.pending = data.pending.filter((value) => value.id !== entry.id);
          data.status.rejectedCount = (data.status.rejectedCount ?? 0) + 1;
          this.receipt(data, entry, "needs_attention", "record_rejected");
        } else {
          this.retry(entry);
          entry.error = this.safeError(error);
        }
        data.status.error = this.safeError(error);
        if (["pairing_rejected", "receiver_unavailable", "invalid_response"].includes(data.status.error)) {
          data.status.connection = { kind: data.status.error, checkedAt: this.now() };
        }
      }
    }
    await this.storage.set({ pending: data.pending, status: data.status });
  }

  retry(entry) {
    entry.nextAt = this.now() + backoffMs(entry.attempt);
    entry.attempt = Math.min(entry.attempt + 1, 8);
  }

  safeError(error) {
    return SAFE_ERRORS.has(error?.message) ? error.message : "delivery_failed";
  }
}
