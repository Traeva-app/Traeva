import { Collector } from "./collector.js";
import { createApi, RETRY_ALARM } from "./protocol.js";

const collector = new Collector({
  storage: chrome.storage.local,
  downloads: chrome.downloads,
  alarms: chrome.alarms,
  api: createApi(),
});

let serial = Promise.resolve();
function run(action) {
  const result = serial.then(action);
  serial = result.catch(() => {});
  return result;
}

// Keep settings and pairing credentials unavailable to content-script contexts.
run(() => chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" }));

chrome.downloads.onCreated.addListener((item) => {
  if (item.incognito !== false) return;
  run(async () => { await collector.enqueue(item.id); await collector.drain(); });
});
chrome.downloads.onChanged.addListener((change) => {
  if (!change.state && !change.filename && !change.endTime) return;
  run(async () => { await collector.enqueue(change.id); await collector.drain(); });
});
chrome.downloads.onErased.addListener((id) => {
  run(() => collector.erased(id));
});
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === RETRY_ALARM) run(() => collector.drain());
});
chrome.runtime.onStartup.addListener(() => {
  run(async () => {
    const { config } = await collector.load();
    if (config.enabled) await chrome.alarms.create(RETRY_ALARM, { periodInMinutes: 1 });
    await collector.replay();
    await collector.drain();
  });
});
chrome.runtime.onInstalled.addListener(() => { run(async () => {
  const { config } = await collector.load();
  if (config.enabled) await chrome.alarms.create(RETRY_ALARM, { periodInMinutes: 1 });
}); });
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id || sender.url !== chrome.runtime.getURL("options.html")) return false;
  run(async () => {
    switch (message?.type) {
      case "status": return collector.publicStatus();
      case "settings": return collector.saveSettings(message.settings ?? {}, { requireDiscardConsent: true });
      case "check": return collector.checkConnection();
      case "retry": {
        const data = await collector.load();
        await chrome.storage.local.set({ pending: data.pending.map((entry) => ({ ...entry, nextAt: 0 })) });
        await collector.drain();
        return collector.publicStatus();
      }
      case "history":
        await collector.replay({ explicitHistory: true });
        await collector.drain();
        return collector.publicStatus();
      default: throw new Error("unknown_request");
    }
  }).then((value) => sendResponse({ ok: true, value }), (error) => sendResponse({ ok: false, error: collector.safeError(error) }));
  return true;
});
