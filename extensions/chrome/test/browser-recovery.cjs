// Real Chrome → production receiver/core via a private, headless mock-IPC driver.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');

if (!process.env.TRAEVA_TEST_PLAYWRIGHT || !process.env.TRAEVA_SOURCE_RECOVERY_DRIVER) {
  throw new Error('Set TRAEVA_TEST_PLAYWRIGHT and TRAEVA_SOURCE_RECOVERY_DRIVER to explicit existing test dependencies.');
}
const { chromium } = require(process.env.TRAEVA_TEST_PLAYWRIGHT);
const run = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'traeva-source-recovery-')));
const root = path.join(run, 'downloads');
const profile = path.join(run, 'profile');
const evidence = path.join(run, 'evidence');
for (const dir of [root, evidence, path.join(profile, 'Default')]) fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(profile, 'Default', 'Preferences'), JSON.stringify({ download: { default_directory: root, prompt_for_download: false, directory_upgrade: true } }));
const payload = Buffer.from('Synthetic source recovery fixture.\n'.repeat(1024));
const results = [];
const errors = [];
const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2));
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, timeout = 20000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const value = await check(); if (value) return value; await delay(100); }
  throw new Error('Condition timed out');
}
const fixture = http.createServer((req, res) => {
  const name = req.url.match(/^\/([a-z]+)$/)?.[1];
  if (!name) {
    res.setHeader('Content-Type', 'text/html');
    res.end('<!doctype html><title>Synthetic recovery downloads</title>' + ['normal', 'offline', 'repair', 'disabled'].map((id) => `<p><a id="${id}" href="/${id}">${id}.txt</a></p>`).join(''));
  } else {
    res.writeHead(200, { 'Content-Type': 'text/plain', 'Content-Length': payload.length, 'Content-Disposition': `attachment; filename="${name}.txt"` });
    res.end(payload);
  }
});
let driver, context, options, page, extensionId;
let replies = [], waiters = [];
function nextReply() {
  if (replies.length) return Promise.resolve(replies.shift());
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Receiver driver reply timed out')), 20000);
    waiters.push((value) => { clearTimeout(timer); resolve(value); });
  });
}
async function startDriver() {
  replies = []; waiters = [];
  driver = spawn(process.env.TRAEVA_SOURCE_RECOVERY_DRIVER, ['source_recovery_receiver_driver', '--exact', '--ignored', '--nocapture'], {
    env: { ...process.env, TRAEVA_SOURCE_RECOVERY: '1', TRAEVA_SOURCE_RECOVERY_BASE: run }, stdio: ['pipe', 'pipe', 'pipe'],
  });
  driver.stderr.on('data', (data) => fs.appendFileSync(path.join(evidence, 'driver-stderr.txt'), data));
  createInterface({ input: driver.stdout }).on('line', (line) => {
    if (!line.startsWith('SOURCE_TEST ')) return;
    const value = JSON.parse(line.slice(12));
    if (waiters.length) waiters.shift()(value); else replies.push(value);
  });
  return Promise.race([nextReply(), new Promise((_, reject) => driver.once('exit', (code) => reject(new Error(`Receiver driver exited ${code}`))))]);
}
async function invoke(cmd, args = {}) {
  driver.stdin.write(`${JSON.stringify({ cmd, args })}\n`);
  return nextReply();
}
async function stopDriver() {
  if (driver.exitCode !== null) { const code = driver.exitCode; driver = null; assert.equal(code, 0); return; }
  const exiting = new Promise((resolve) => driver.once('exit', resolve));
  driver.stdin.end(`${JSON.stringify({ cmd: 'quit' })}\n`);
  assert.equal(await exiting, 0);
  driver = null;
}
async function launchChrome() {
  context = await chromium.launchPersistentContext(profile, {
    executablePath: process.env.TRAEVA_TEST_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    headless: process.env.TRAEVA_TEST_CHROME_HEADLESS === '1', ignoreDefaultArgs: ['--disable-extensions'],
    args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check'], acceptDownloads: true,
  });
  const cdp = await context.browser().newBrowserCDPSession();
  await cdp.send('Browser.setDownloadBehavior', { behavior: 'default', eventsEnabled: true });
  const loaded = await cdp.send('Extensions.loadUnpacked', { path: path.resolve(__dirname, '..') });
  if (extensionId) assert.equal(loaded.id, extensionId);
  extensionId = loaded.id;
  options = await context.newPage();
  options.on('pageerror', (error) => errors.push(error.message));
  await options.goto(`chrome-extension://${extensionId}/options.html`);
  await until(() => options.locator('#save').isEnabled());
  page = await context.newPage();
  await page.goto(`http://127.0.0.1:${fixture.address().port}`);
  return cdp.send('Browser.getVersion');
}
const stored = () => options.evaluate(() => chrome.storage.local.get(null));
async function checked(kind) {
  await options.locator('#check').click();
  await until(async () => await options.locator('#check').isEnabled() && (await stored()).status.connection?.kind === kind);
}
async function submit() {
  await options.locator('#save').click();
  await until(() => options.locator('#save').isEnabled());
}
async function download(name) {
  const pending = page.waitForEvent('download');
  await page.locator(`#${name}`).click();
  await pending;
  await until(async () => (await options.evaluate((filename) => chrome.downloads.search({ query: [filename] }), `${name}.txt`))
    .some((item) => item.filename === path.join(root, `${name}.txt`) && item.state === 'complete'));
  assert.ok(fs.readFileSync(path.join(root, `${name}.txt`)).equals(payload), 'Completed synthetic download has the expected bytes');
}
async function saved(name) {
  return until(async () => (await invoke('source_state')).records.find((record) => record.filename.endsWith(`/${name}.txt`) && record.download_state === 'complete'));
}
async function screenshot(name) {
  await options.screenshot({ path: path.join(evidence, `${name}.png`), fullPage: true });
}

(async () => {
  await new Promise((resolve) => fixture.listen(0, '127.0.0.1', resolve));
  const pairing = await startDriver();
  const environment = await launchChrome();
  save('environment.json', { browser: environment, platform: os.platform(), arch: os.arch(), run, headless: process.env.TRAEVA_TEST_CHROME_HEADLESS === '1', scope: 'real Chrome options/downloads, production HTTP receiver/core, mock Tauri IPC; not desktop WebView or VERIFIED evidence' });
  assert.match(await options.locator('#connection').innerText(), /off in this extension/);
  assert.equal((await stored()).config.enabled, false);
  await options.locator('#endpoint').fill(pairing.endpoint);
  await options.locator('#token').fill('0'.repeat(64));
  await options.locator('#enabled').check();
  await submit();
  assert.equal((await stored()).status.connection.kind, 'pairing_rejected');
  assert.match(await options.locator('#connection').innerText(), /does not match/);
  assert.equal((await invoke('source_state')).records.length, 0);
  await screenshot('wrong-pairing');
  results.push({ name: 'first setup defaults off; wrong valid-format token is not connected or saved', result: 'PASS' });

  await options.locator('#token').fill(pairing.pairing_token);
  await submit();
  assert.equal((await stored()).status.connection.kind, 'connected');
  assert.equal(await options.locator('#roots li').innerText(), root);
  await download('normal');
  await until(async () => {
    const value = await stored();
    return value.pending.length === 0 && value.status.recent?.some((entry) => entry.outcome === 'saved' && entry.downloadState === 'complete');
  });
  const normal = await saved('normal');
  assert.equal(normal.status, 'likely');
  assert.equal(normal.association, null);
  await until(async () => (await options.locator('#recent').innerText()).includes('normal.txt'));
  const confirmed = await invoke('source_confirm', { source_id: normal.source_id, file_id: normal.candidates[0].file_id, relation: 'download', note: 'Synthetic recovery confirmation' });
  assert.equal(confirmed.status, 'user_confirmed');
  results.push({ name: 'correct pairing and normal download: acknowledged source, candidate, explicit User Confirmed', result: 'PASS' });

  fs.renameSync(root, `${root}-unavailable`);
  await checked('no_folders');
  assert.match(await options.locator('#connection').innerText(), /No currently available authorized folders/);
  assert.equal(await options.locator('#roots li').count(), 0);
  await screenshot('folder-unavailable');
  fs.renameSync(`${root}-unavailable`, root);
  await checked('connected');
  results.push({ name: 'unavailable authorized root is not reported ready; access restoration can be rechecked', result: 'PASS' });

  const profileId = (await stored()).config.profileId;
  await stopDriver();
  await checked('receiver_unavailable');
  await download('offline');
  await until(async () => (await stored()).pending.some((entry) => entry.error === 'receiver_unavailable'));
  await until(async () => (await options.locator('#pending').innerText()).includes('offline.txt'));
  await screenshot('receiver-offline');
  const before = await stored();
  await context.close(); context = null;
  await launchChrome();
  const restarted = await stored();
  assert.equal(restarted.config.profileId, profileId);
  assert.deepEqual(restarted.pending.map((entry) => entry.id), before.pending.map((entry) => entry.id));
  assert.equal(restarted.status.recent.find((entry) => entry.id === Number(normal.download_id)).outcome, 'saved');
  const pairingAfter = await startDriver();
  assert.equal(pairingAfter.endpoint, pairing.endpoint);
  assert.ok(pairingAfter.pairing_token === pairing.pairing_token, 'Pairing token retained');
  await checked('connected');
  await options.locator('#retry').click();
  await saved('offline');
  await until(async () => (await stored()).pending.length === 0);
  await until(async () => (await options.locator('#roots li').allTextContents()).includes(root));
  const retained = (await invoke('source_state')).records.find((record) => record.source_id === normal.source_id);
  assert.equal(retained.status, 'user_confirmed');
  assert.equal(retained.imported_at_ms, normal.imported_at_ms);
  results.push({ name: 'receiver process and Chrome restart retain saved sources, waiting IDs, profile identity and first import time; retry delivers once', result: 'PASS' });

  await stopDriver();
  await download('repair');
  await until(async () => (await stored()).pending.length > 0);
  await options.locator('#token').fill('1'.repeat(64));
  await submit();
  assert.ok((await stored()).config.token === pairing.pairing_token, 'Pairing change was not silently saved');
  assert.ok((await stored()).pending.length > 0);
  assert.match(await options.locator('#message').innerText(), /warning/);
  await options.locator('#discard').check();
  await submit();
  assert.equal((await stored()).pending.length, 0);
  await startDriver();
  await options.locator('#token').fill(pairing.pairing_token);
  await submit();
  await options.locator('#history').click();
  await saved('repair');
  assert.equal((await invoke('source_state')).records.length, 3);
  assert.equal((await invoke('source_state')).records.find((record) => record.source_id === normal.source_id).imported_at_ms, normal.imported_at_ms);
  results.push({ name: 'pairing change warns before clearing waiting IDs; explicit recent import recovers missed source without duplicate records', result: 'PASS' });

  await invoke('source_set_enabled', { enabled: false });
  await checked('receiver_unavailable');
  assert.match(await options.locator('#connection').innerText(), /cannot tell whether/);
  await download('disabled');
  await until(async () => (await stored()).pending.length > 0);
  await invoke('source_set_enabled', { enabled: true });
  await options.locator('#retry').click();
  await until(async () => (await stored()).pending.length === 0);
  assert.equal((await invoke('source_state')).records.length, 3);
  assert.ok((await stored()).status.recent.some((entry) => entry.reason === 'outside_interval'));
  await options.locator('#history').click();
  await saved('disabled');
  results.push({ name: 'downloads while app collection is off require explicit history import; no interval bypass', result: 'PASS' });

  await until(() => options.locator('#save').isEnabled());
  const final = await stored();
  for (const sensitive of [root, 'normal.txt', '127.0.0.1:' + fixture.address().port, 'Synthetic source recovery fixture']) assert.equal(JSON.stringify(final).includes(sensitive), false, sensitive);
  save('extension-storage-redacted.json', { ...final, config: { ...final.config, token: '[redacted]' } });
  save('source-state.json', await invoke('source_state'));
  assert.deepEqual(errors, []);
  await screenshot('connected-results');
  await options.setViewportSize({ width: 360, height: 900 });
  assert.equal(await options.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await screenshot('options-mobile');
  results.push({ name: 'no sensitive download metadata persisted in extension; desktop/mobile page has no exceptions or overflow', result: 'PASS' });
  save('outcome.json', { result: 'PASS', automaticVerified: 'NOT_IMPLEMENTED', desktopAppRestart: 'not tested by this headless driver' });
})().catch(async (error) => {
  save('failure.json', { message: error.message, stack: error.stack });
  if (options) await screenshot('failure').catch(() => {});
  save('outcome.json', { result: 'FAIL' });
  process.exitCode = 1;
}).finally(async () => {
  save('results.json', results);
  if (context) await context.close();
  if (driver) await stopDriver();
  await new Promise((resolve) => fixture.close(resolve));
  console.log(JSON.stringify({ run, completedChecks: results.length, outcome: process.exitCode ? 'FAIL' : 'PASS' }));
});
