export const MAX_PENDING = 200;
export const RETRY_ALARM = "traeva-source-retry";
export const REPLAY_WINDOW_MS = 24 * 60 * 60 * 1000;

export function endpointUrl(value) {
  if (typeof value !== "string" || !/^http:\/\/127\.0\.0\.1:[1-9][0-9]{0,4}\/?$/.test(value)) {
    throw new Error("invalid_endpoint");
  }
  const url = new URL(value);
  if (Number(url.port || 80) > 65535) throw new Error("invalid_endpoint");
  return `http://127.0.0.1:${url.port || 80}`;
}

export function validToken(value) {
  return typeof value === "string" && /^[A-Za-z0-9_-]{32,256}$/.test(value);
}

function pathParts(value) {
  if (typeof value !== "string" || !value.startsWith("/") || value.includes("\0")) return null;
  const parts = value.split("/").filter(Boolean);
  if (parts.some((part) => part === "." || part === "..")) return null;
  return parts;
}

export function insideRoot(filename, root) {
  const fileParts = pathParts(filename);
  const rootParts = pathParts(root);
  return fileParts !== null && rootParts !== null && fileParts.length > rootParts.length
    && rootParts.every((part, index) => fileParts[index] === part);
}

export function parseState(value) {
  if (!value || typeof value.enabled !== "boolean" || !Array.isArray(value.roots)) {
    throw new Error("invalid_response");
  }
  if (value.roots.some((root) => !root || typeof root.root_id !== "string" || !pathParts(root.path))) {
    throw new Error("invalid_response");
  }
  return value;
}

function timestamp(value) {
  if (typeof value !== "string" || !value) return null;
  const result = Date.parse(value);
  return Number.isFinite(result) && result >= 0 ? result : null;
}

function textOrUnknown(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

export function downloadEvent(item, profileId, roots) {
  if (!item || !Number.isSafeInteger(item.id) || item.id < 0 || item.incognito !== false) return null;
  if (!["in_progress", "complete", "interrupted"].includes(item.state)) return null;
  if (!roots.some((root) => insideRoot(item.filename, root.path))) return null;
  return {
    profile_id: profileId,
    download_id: String(item.id),
    started_at_ms: timestamp(item.startTime),
    completed_at_ms: item.state === "complete" ? timestamp(item.endTime) : null,
    state: item.state,
    filename: item.filename,
    initial_url: textOrUnknown(item.url),
    final_url: textOrUnknown(item.finalUrl),
    referrer: textOrUnknown(item.referrer),
    file_size: Number.isSafeInteger(item.fileSize) && item.fileSize >= 0 ? item.fileSize : null,
    incognito: false,
  };
}

export function backoffMs(attempt) {
  return Math.min(60 * 60 * 1000, 30_000 * 2 ** Math.min(attempt, 7));
}

export function createApi(fetchImpl = fetch) {
  async function request(config, path, body) {
    const endpoint = endpointUrl(config.endpoint);
    if (!validToken(config.token)) throw new Error("invalid_token");
    const payload = body === undefined ? undefined : JSON.stringify(body);
    if (payload && new TextEncoder().encode(payload).length > 65_536) throw new Error("record_rejected");
    let response;
    try {
      response = await fetchImpl(`${endpoint}${path}`, {
        method: body === undefined ? "GET" : "POST",
        headers: {
          Authorization: `Bearer ${config.token}`,
          ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: payload,
        credentials: "omit",
        cache: "no-store",
        redirect: "error",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.timeout(5000),
      });
    } catch {
      throw new Error("receiver_unavailable");
    }
    if (response.status === 401 || response.status === 403) throw new Error("pairing_rejected");
    if (body !== undefined && [400, 413, 422].includes(response.status)) throw new Error("record_rejected");
    if (!response.ok) throw new Error("receiver_rejected");
    try {
      return await response.json();
    } catch {
      throw new Error("invalid_response");
    }
  }
  return {
    async state(config) {
      return parseState(await request(config, "/v1/source/state"));
    },
    async send(config, event) {
      const value = await request(config, "/v1/source/events", event);
      if (!value || !["saved", "ignored"].includes(value.outcome)
        || (value.outcome === "saved" && typeof value.source_id !== "string")) {
        throw new Error("invalid_response");
      }
      return value;
    },
  };
}
