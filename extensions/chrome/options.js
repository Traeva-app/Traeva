import { clearsPending, connectionText, errors, receiptText } from "./diagnostics.js";

const form = document.querySelector("#settings");
const message = document.querySelector("#message");
const fields = Object.fromEntries(["endpoint", "token", "enabled", "recoverRecent"].map((id) => [id, document.getElementById(id)]));
let current;
let busy = false;
let refreshBusy = false;
let messageErrorCode;
let generation = 0;

function show(text, error = false, code) {
  message.textContent = text;
  message.dataset.error = String(error);
  messageErrorCode = code;
}

async function request(type, extra = {}) {
  const result = await chrome.runtime.sendMessage({ type, ...extra });
  if (!result?.ok) throw new Error(result?.error ?? "delivery_failed");
  return result.value;
}

function input() {
  return { endpoint: fields.endpoint.value.trim(), token: fields.token.value.trim(),
    enabled: fields.enabled.checked, recoverRecent: fields.recoverRecent.checked,
    discardPending: document.getElementById("discard").checked };
}

function controls() {
  for (const fieldset of document.querySelectorAll(".settings-fields")) fieldset.disabled = busy || !current;
  document.getElementById("save").disabled = busy || !current;
  document.getElementById("check").disabled = busy || !current || !current.config.endpoint || !current.config.token;
  document.getElementById("retry").disabled = busy || !current?.config.enabled || !current.pendingCount;
  document.getElementById("history").disabled = busy || !current?.config.enabled;
  const discard = current?.pendingCount > 0 && clearsPending(current.config, input());
  document.getElementById("discard-warning").hidden = !discard;
  if (!discard) document.getElementById("discard").checked = false;
}

function list(id, entries, empty, describe) {
  document.getElementById(id).replaceChildren(...(entries.length ? entries.map((entry) => {
    const li = document.createElement("li");
    const name = current.labels?.[entry.id] ?? `Chrome record #${entry.id}`;
    li.textContent = `${name} (#${entry.id}): ${describe(entry)}`;
    return li;
  }) : [Object.assign(document.createElement("li"), { textContent: empty })]));
}

function update(value, fillFields = false) {
  current = value;
  if (fillFields) {
    fields.endpoint.value = value.config.endpoint;
    fields.token.value = value.config.token;
    fields.enabled.checked = value.config.enabled;
    fields.recoverRecent.checked = value.config.recoverRecent;
    document.getElementById("setup").open = !value.config.enabled;
  }
  document.getElementById("connection").textContent = connectionText(value);
  if (value.status.connection?.kind === "connected" && ["pairing_rejected", "receiver_unavailable", "invalid_response", "collection_disabled"].includes(messageErrorCode)) {
    show("Connection recovered. Retry waiting downloads or wait for their scheduled attempts.");
  }
  document.getElementById("roots").replaceChildren(...(value.roots ?? []).map((root) => {
    const li = document.createElement("li");
    li.textContent = root.path;
    return li;
  }));
  document.getElementById("folders-help").textContent = value.status.connection?.kind === "connected" && !value.roots?.length
    ? "Check connection to display the available authorized folders." : "";
  document.getElementById("checked").textContent = value.status.connection
    ? `Last connection check: ${new Date(value.status.connection.checkedAt).toLocaleString()}. Check again after changing app or folder settings.` : "";
  document.getElementById("delivery").textContent = `${value.pendingCount} waiting download record(s).${value.status.lastSavedAt ? ` Last acknowledged save: ${new Date(value.status.lastSavedAt).toLocaleString()}.` : " No save acknowledged yet."}${value.status.queueFull ? " The 200-record queue filled. Some downloads were not queued; use the manual import once space is available." : ""}`;
  list("pending", value.pending ?? [], "No downloads waiting for delivery.", (entry) => `${errors[entry.error] ?? "Waiting for a delivery attempt."} Next scheduled attempt: ${entry.nextAt ? new Date(entry.nextAt).toLocaleString() : "now"}.`);
  document.getElementById("pending-help").textContent = value.pendingCount > (value.pending?.length ?? 0)
    ? `Showing the first ${value.pending.length} of ${value.pendingCount} waiting records. Remaining records stay queued; each retry pass handles up to 20.` : "";
  list("recent", value.status.recent ?? [], "No recent delivery results for this pairing.", (entry) => `${receiptText(entry)} (${new Date(entry.at).toLocaleString()})`);
  controls();
}

async function check() {
  document.getElementById("roots").replaceChildren();
  document.getElementById("connection").textContent = "Checking the saved connection…";
  const state = await request("check");
  update(state.delivery);
}

async function act(action) {
  if (busy) return;
  generation++;
  busy = true;
  controls();
  try { await action(); } catch (error) {
    show(current ? errors[error?.message] ?? errors.delivery_failed : "Could not load saved settings. Reload this page before editing.", true, error?.message);
    try { update(await request("status")); } catch { /* Keep the last readable state. */ }
  } finally { busy = false; controls(); }
}

form.addEventListener("input", () => { document.getElementById("discard").checked = false; controls(); });
document.getElementById("discard").addEventListener("input", (event) => event.stopPropagation());
form.addEventListener("submit", (event) => {
  event.preventDefault();
  if (current?.pendingCount && clearsPending(current.config, input()) && !document.getElementById("discard").checked) {
    show("Review the waiting-delivery warning and select the checkbox before saving this change.", true);
    return;
  }
  void act(async () => {
    const value = await request("settings", { settings: input() });
    update(value, true);
    document.getElementById("roots").replaceChildren();
    if (value.config.enabled) {
      show("Settings saved. Checking the connection…");
      await check();
      show("Settings saved. Review the connection status and available folders below.");
    } else show("Collection is off. Waiting deliveries were cleared; sources already saved in Traeva remain there.");
  });
});

document.getElementById("check").addEventListener("click", () => void act(async () => {
  show("Checking the saved connection…");
  await check();
  show("Connection check finished. Review the status below.");
}));
for (const type of ["retry", "history"]) {
  document.getElementById(type).addEventListener("click", () => void act(async () => {
    show(type === "history" ? "Checking recent downloads…" : "Retrying waiting downloads…");
    update(await request(type));
    show(current.status.error ? errors[current.status.error] ?? errors.delivery_failed : "Delivery check finished. Review waiting records and recent results below.", Boolean(current.status.error));
  }));
}

void act(async () => {
  update(await request("status"), true);
  show(current.config.enabled ? "Saved settings loaded. Checking the connection…" : "Collection is off. Follow the setup steps above.");
  if (current.config.enabled) { await check(); show("Saved settings loaded. Review the connection status below."); }
});

// Read delivery receipts without overwriting edits or sending download events.
setInterval(async () => {
  if (busy || refreshBusy || document.hidden) return;
  const started = generation;
  refreshBusy = true;
  try {
    const value = await request("status");
    if (!busy && started === generation) update(value);
  } catch { if (!busy && started === generation) show("Cannot read the extension state. Reload this settings page.", true); }
  finally { refreshBusy = false; }
}, 3000);
