importScripts('config.js');

const events = [], acquisitions = [], sessions = new Map(), paused = new Map(), pending = new Map();
const downloadItems = new Map(), active = new Set();
const MAX_TASKS = 32, MAX_ACTIVE = 4, MAX_EVENTS = 2048;
let port, authorized = false, serial = 0, transportEpoch = 0;
const record = (event, data = {}) => {
  const safe = {};
  for (const key of ['acquisitionId', 'scenario', 'requestId', 'tabId', 'state', 'sequence', 'eof', 'reason', 'nativePid']) {
    if (Object.hasOwn(data, key)) safe[key] = data[key];
  }
  if (events.length === MAX_EVENTS) events.shift();
  events.push({at: Date.now(), event, ...safe});
};
const send = (source, method, params = {}) => chrome.debugger.sendCommand(source, method, params);
const key = (source, id) => JSON.stringify([source.tabId, source.sessionId || '', sessions.get(source.tabId)?.epoch, id]);
const current = (source, config, epoch) => authorized && sessions.get(source.tabId) === config && transportEpoch === epoch;

function disconnected() {
  record('native_disconnected'); port = null; authorized = false; transportEpoch++;
  for (const promise of pending.values()) {clearTimeout(promise.timer); promise.reject(new Error('native_port_closed'));}
  pending.clear();
}
function connect() {
  if (port) return;
  const connected = chrome.runtime.connectNative('org.traeva.source_verified_lab');
  port = connected;
  connected.onMessage.addListener(reply => {
    if (port !== connected) return;
    const promise = pending.get(reply.rid);
    if (!promise) return;
    clearTimeout(promise.timer); pending.delete(reply.rid);
    if (reply.ok) promise.resolve(reply);
    else {
      const error = new Error('native_refused'); error.nativeRefusal = true;
      error.outcomeUnknown = reply.outcome === 'OUTCOME_UNKNOWN'; promise.reject(error);
    }
  });
  connected.onDisconnect.addListener(() => {if (port === connected) disconnected();});
}
function rpc(command, payload = {}) {
  connect();
  const rid = ++serial;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {pending.delete(rid); reject(new Error('native_reply_timeout'));}, 15000);
    pending.set(rid, {resolve, reject, timer});
    try {port.postMessage({rid, auth: LAB.token, command, payload});}
    catch {clearTimeout(timer); pending.delete(rid); reject(new Error('native_send_failed'));}
  });
}
function safeURL(value, downloadDiagnostic = false) {
  if (typeof value !== 'string' || value.length > 2048) return null;
  try {
    const url = new URL(value);
    const origins = downloadDiagnostic ? LAB.downloadOrigins || LAB.origins : LAB.origins;
    return (url.protocol === 'https:' || downloadDiagnostic && url.protocol === 'http:') && origins.includes(url.origin) && !url.username && !url.password &&
      !url.search && !url.hash ? url.href : null;
  } catch {return null;}
}

// This single parser decides support AND produces the eventual destination before take.
function parsePlan(event, acquisitionId) {
  const url = safeURL(event.request?.url);
  if (!authorized || !url || event.request?.method !== 'GET' || event.responseStatusCode !== 200) return null;
  if (typeof event.requestId !== 'string' || !event.requestId.length || event.requestId.length > 512 ||
      event.networkId != null && (typeof event.networkId !== 'string' || !event.networkId.length || event.networkId.length > 512)) return null;
  const hs = event.responseHeaders;
  if (!Array.isArray(hs) || hs.length > 100 || hs.some(h => typeof h.name !== 'string' || typeof h.value !== 'string')) return null;
  const values = name => hs.filter(h => h.name.toLowerCase() === name).map(h => h.value.trim());
  const lengths = values('content-length'), mime = values('content-type'), cd = values('content-disposition');
  const encoding = values('content-encoding');
  const match = cd.length === 1 && /^attachment;\s*filename="([a-zA-Z0-9_-]+\.(?:txt|md))"$/i.exec(cd[0]);
  if (lengths.length !== 1 || !/^[0-9]+$/.test(lengths[0]) || Number(lengths[0]) <= 0 || Number(lengths[0]) > 20 * 1024 * 1024 ||
      values('transfer-encoding').length || encoding.length > 1 || encoding.some(x => x.toLowerCase() !== 'identity') ||
      mime.length !== 1 || !/^text\/plain(?:;|$)/i.test(mime[0]) || !match) return null;
  const filename = match[1], destination = acquisitionId + '-' + filename;
  if (new TextEncoder().encode(destination).length > 180) return null;
  const headers = hs.filter(h => ['content-length', 'content-type', 'content-disposition', 'content-encoding'].includes(h.name.toLowerCase()))
    .map(h => ({name: h.name.toLowerCase(), value: h.name.toLowerCase() === 'content-length' ? String(Number(lengths[0])) : h.value.trim()}));
  if (headers.some(h => new TextEncoder().encode(h.value).length > 256)) return null;
  return {filename, destination, response: {url, status: 200, headers}};
}
function remember(source, event) {
  const url = safeURL(event.request?.url);
  if (!url || event.request?.method !== 'GET' || typeof event.requestId !== 'string') return null;
  const id = key(source, event.requestId), prior = paused.get(id);
  const metadata = {requestId: event.requestId, redirectedRequestId: event.redirectedRequestId || prior?.redirectedRequestId || null,
    request: {url}, responseStatusCode: event.responseStatusCode || null};
  if (paused.size >= 128) paused.delete(paused.keys().next().value);
  paused.set(id, metadata);
  return metadata;
}
async function onPaused(source, event) {
  const config = sessions.get(source.tabId), epoch = transportEpoch;
  const merged = config ? remember(source, event) : null;
  record('fetch_paused', {tabId: source.tabId, scenario: config?.scenario, requestId: event.requestId});
  const acquisitionId = crypto.randomUUID();
  const validSource = Number.isInteger(source.tabId) && source.tabId >= 0 &&
    (source.sessionId == null || typeof source.sessionId === 'string' && source.sessionId.length > 0 && source.sessionId.length <= 512);
  const plan = config && validSource && parsePlan(event, acquisitionId);
  if (!plan || active.size >= MAX_ACTIVE || acquisitions.length + active.size >= MAX_TASKS) {
    await send(source, 'Fetch.continueRequest', {requestId: event.requestId});
    record('continued_before_take', {scenario: config?.scenario, requestId: event.requestId, reason: 'unsupported_or_budget'});
    return;
  }
  active.add(acquisitionId);
  let task, stream, began = false, completing = false;
  try {
    try {
      await rpc('prepare', {response: plan.response, destination: plan.destination});
      if (!current(source, config, epoch)) throw new Error('session_changed_before_take');
    } catch {
      await send(source, 'Fetch.continueRequest', {requestId: event.requestId});
      record('continued_before_take', {scenario: config.scenario, requestId: event.requestId, reason: 'prepare_refused_or_session_changed'});
      return;
    }
    const chain = [], seen = new Set(); let node = merged;
    while (node && !seen.has(node.requestId) && chain.length < 10) {
      seen.add(node.requestId);
      chain.unshift({requestId: node.requestId, redirectedRequestId: node.redirectedRequestId, url: node.request.url, status: node.responseStatusCode});
      node = node.redirectedRequestId ? paused.get(key(source, node.redirectedRequestId)) : null;
    }
    task = {acquisitionId, scenario: config.scenario, response: plan.response, destination: plan.destination, redirectChain: chain,
      state: 'PREPARED', source, browserRequestDisposition: 'PAUSED_NOT_TAKEN'};
    acquisitions.push(task);
    ({stream} = await send(source, 'Fetch.takeResponseBodyAsStream', {requestId: event.requestId}));
    task.browserRequestDisposition = 'STREAM_TAKEN';
    if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
    record('stream_taken', {acquisitionId, requestId: event.requestId});
    task.binding = {tabId: source.tabId, sessionId: source.sessionId || null, debuggerEpoch: config.epoch,
      fetchRequestId: event.requestId, stream, networkId: event.networkId || null};
    await rpc('begin', {acquisitionId, binding: task.binding, response: plan.response, scenario: task.scenario, destination: plan.destination});
    began = true; task.state = 'RECEIVING';
    if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
    let sequence = 0, eof = false;
    while (!eof) {
      const part = await send(source, 'IO.read', {handle: stream, size: 32768});
      if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
      let encoded = part.data;
      if (!part.base64Encoded) {
        let raw = ''; for (const n of new TextEncoder().encode(part.data)) raw += String.fromCharCode(n);
        encoded = btoa(raw);
      }
      if (encoded) await rpc('chunk', {acquisitionId, sequence: sequence++, base64: encoded});
      if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
      record('stream_chunk', {acquisitionId, sequence: sequence - 1, eof: !!part.eof});
      eof = !!part.eof;
    }
    await send(source, 'IO.close', {handle: stream}); stream = null;
    if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
    await send(source, 'Fetch.failRequest', {requestId: event.requestId, errorReason: 'Aborted'});
    if (!current(source, config, epoch)) throw new Error('session_changed_after_take');
    task.browserRequestDisposition = 'CHROME_REQUEST_ABORTED_AFTER_TAKE';
    completing = true;
    task.completion = await rpc('complete', {acquisitionId, eof: true});
    task.state = 'HISTORICAL_ACQUISITION_OBSERVED';
    record('acquisition_observed', {acquisitionId});
  } catch (error) {
    if (!task) throw error;
    task.state = task.browserRequestDisposition === 'PAUSED_NOT_TAKEN' ? 'TAKE_OUTCOME_UNKNOWN' :
      completing && (!error.nativeRefusal || error.outcomeUnknown) ? 'OUTCOME_UNKNOWN' : 'FAILED_AFTER_TAKE';
    task.error = completing ? 'completion_unacknowledged_or_refused' : 'take_or_receive_failed';
    record('failure_after_take', {acquisitionId, state: task.state});
    if (began && task.state !== 'OUTCOME_UNKNOWN') await rpc('abort', {acquisitionId, reason: task.error}).catch(() => {});
    await send(source, 'Fetch.failRequest', {requestId: event.requestId, errorReason: 'Aborted'}).catch(() => {});
    task.browserRequestDisposition = task.state === 'TAKE_OUTCOME_UNKNOWN' ? 'TAKE_OUTCOME_UNKNOWN' : 'FAILED_NOT_NORMAL_CHROME_DOWNLOAD';
  } finally {
    active.delete(acquisitionId);
    if (stream) await send(source, 'IO.close', {handle: stream}).catch(() => {});
  }
}
chrome.debugger.onEvent.addListener((source, method, data) => {
  if (method === 'Fetch.requestPaused') onPaused(source, data).catch(() => record('handler_error'));
});
chrome.debugger.onDetach.addListener((source) => {
  record('debugger_detached', {tabId: source.tabId}); sessions.delete(source.tabId);
  for (const id of paused.keys()) if (JSON.parse(id)[0] === source.tabId) paused.delete(id);
});
chrome.downloads.onCreated.addListener(item => {
  // Retain only this isolated fixture's query-free downloads, never browser history.
  if (downloadItems.size >= MAX_TASKS || !safeURL(item.url, true)) return;
  downloadItems.set(item.id, {id: item.id, url: safeURL(item.url, true), state: item.state, bytesReceived: item.bytesReceived, totalBytes: item.totalBytes});
});
chrome.downloads.onChanged.addListener(change => {
  const item = downloadItems.get(change.id);
  if (item && ['in_progress', 'interrupted', 'complete'].includes(change.state?.current)) item.state = change.state.current;
});
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (sender.id !== chrome.runtime.id || sender.url !== chrome.runtime.getURL('control.html')) return false;
  (async () => {
    if (message.type === 'authorize') {
      if (message.takeoverConsent !== true || message.saveLocationConsent !== LAB.delivery) throw new Error('explicit_consent_required');
      const hello = await rpc('hello'), consent = await rpc('authorize', message);
      authorized = true; record('explicit_lab_consent', {nativePid: hello.pid}); return consent;
    }
    if (message.type === 'attach') {
      if (!authorized) throw new Error('takeover_disabled');
      if (sessions.size >= 8 || !Number.isInteger(message.tabId) || typeof message.scenario !== 'string' || !/^[a-zA-Z0-9_-]{1,120}$/.test(message.scenario)) throw new Error('invalid_lab_session');
      await chrome.debugger.attach({tabId: message.tabId}, '1.3');
      sessions.set(message.tabId, {scenario: message.scenario, epoch: crypto.randomUUID()});
      await send({tabId: message.tabId}, 'Fetch.enable', {patterns: LAB.origins.flatMap(origin => [
        {urlPattern: origin + '/*', requestStage: 'Request'}, {urlPattern: origin + '/*', requestStage: 'Response'}])});
      record('attached', {tabId: message.tabId, scenario: message.scenario}); return {ok: true};
    }
    if (message.type === 'detach') {await chrome.debugger.detach({tabId: message.tabId}); return {ok: true};}
    if (message.type === 'disconnectNative') {port?.disconnect(); disconnected(); return {ok: true};}
    if (message.type === 'snapshot') return {ok: true, events, acquisitions, authorized,
      native: port ? await rpc('status').catch(() => ({error: 'status_unavailable'})) : null, downloadItems: [...downloadItems.values()]};
    if (message.type === 'tabs') return {ok: true, tabs: (await chrome.tabs.query({})).map(t => ({id: t.id, url: safeURL(t.url)}))};
    throw new Error('control_command_not_allowed');
  })().then(reply, error => reply({ok: false, error: ['explicit_consent_required', 'takeover_disabled', 'invalid_lab_session', 'control_command_not_allowed'].includes(error.message) ? error.message : 'control_command_failed'}));
  return true;
});
