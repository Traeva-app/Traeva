// Explicit real-browser check; the receiver is a synthetic adapter, not Traeva core.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');

const playwrightPath = process.env.TRAEVA_TEST_PLAYWRIGHT;
if (!playwrightPath) throw new Error('Set TRAEVA_TEST_PLAYWRIGHT to an installed Playwright module path.');
const { chromium } = require(playwrightPath);
const extension = path.resolve(__dirname, '..');
const run = fs.mkdtempSync(path.join(os.tmpdir(), 'traeva-source-extension-'));
const appData = path.join(run, 'app-data');
const downloads = path.join(run, 'downloads');
const outside = path.join(run, 'outside');
const profile = path.join(run, 'profile');
for (const dir of [appData, downloads, outside, path.join(profile, 'Default')]) fs.mkdirSync(dir, { recursive: true });
const token = crypto.randomBytes(32).toString('hex');
const payload = Buffer.from('Synthetic Traeva source fixture.\n'.repeat(16384));
const deliveries = [];
const records = new Map();
const results = [];
const browserErrors = [];
const save = (name, value) => fs.writeFileSync(path.join(appData, name), JSON.stringify(value, null, 2));
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, timeout = 12_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = await check(); if (result) return result; await delay(100); }
  throw new Error('Condition timed out');
}
async function listen(server, port = 0) {
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return server.address().port;
}
async function close(server) { await new Promise((resolve) => { server.closeAllConnections(); server.close(resolve); }); }
function receiver() {
  return http.createServer(async (req, res) => {
    const origin = req.headers.origin;
    if (origin && origin.startsWith('chrome-extension://')) res.setHeader('Access-Control-Allow-Origin', origin);
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return; }
    if (req.headers.authorization !== `Bearer ${token}`) { res.writeHead(401); res.end(); return; }
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/v1/source/state' && req.method === 'GET') { res.end(JSON.stringify({ enabled: true, enabled_since_ms: 0, roots: [{ root_id: 'synthetic-root', path: downloads }] })); return; }
    if (req.url !== '/v1/source/events' || req.method !== 'POST') { res.writeHead(404); res.end(); return; }
    let body = '';
    for await (const chunk of req) body += chunk;
    const event = JSON.parse(body);
    assert.equal(event.incognito, false);
    assert.ok(event.filename.startsWith(`${downloads}/`));
    assert.equal(Object.hasOwn(event, 'confidence'), false);
    deliveries.push(event);
    const key = `${event.profile_id}:${event.download_id}:${event.started_at_ms}`;
    records.set(key, event);
    save('receiver-records.json', [...records.values()]);
    res.end(JSON.stringify({ outcome: 'saved', source_id: key, reason: null }));
  });
}
const fixtures = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://fixture.invalid');
  if (url.pathname === '/') {
    res.setHeader('Content-Type', 'text/html');
    res.end('<!doctype html><title>Source synthetic fixture</title><h1>Synthetic downloads</h1>' + ['normal', 'offline', 'outside', 'incognito', 'disabled'].map((name) => `<p><a id="${name}" href="/files/${name}?signature=synthetic-only">${name}.txt</a></p>`).join(''));
    return;
  }
  const name = url.pathname.match(/^\/files\/([a-z]+)$/)?.[1];
  if (!name) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': 'text/plain', 'Content-Disposition': `attachment; filename="${name}.txt"`, 'Content-Length': payload.length });
  for (let offset = 0; offset < payload.length; offset += 32_768) { res.write(payload.subarray(offset, offset + 32_768)); await delay(90); }
  res.end();
});

let context;
let server = receiver();
(async () => {
  const port = await listen(server);
  const fixturePort = await listen(fixtures);
  const origin = `http://127.0.0.1:${fixturePort}`;
  fs.writeFileSync(path.join(profile, 'Default', 'Preferences'), JSON.stringify({ download: { default_directory: downloads, prompt_for_download: false, directory_upgrade: true }, profile: { default_content_setting_values: { automatic_downloads: 1 } } }));
  context = await chromium.launchPersistentContext(profile, { executablePath: process.env.TRAEVA_TEST_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: process.env.TRAEVA_TEST_CHROME_HEADLESS === '1', ignoreDefaultArgs: ['--disable-extensions'], args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check'], acceptDownloads: true });
  const browser = context.browser();
  const cdp = await browser.newBrowserCDPSession();
  await cdp.send('Browser.setDownloadBehavior', { behavior: 'default', eventsEnabled: true });
  const { id } = await cdp.send('Extensions.loadUnpacked', { path: extension });
  const options = await context.newPage();
  options.on('pageerror', (error) => browserErrors.push(error.message));
  await options.goto(`chrome-extension://${id}/options.html`);
  await options.getByText('Collection is off. Follow the setup steps above.', { exact: true }).waitFor();
  save('environment.json', {
    browser: await cdp.send('Browser.getVersion'), platform: os.platform(), release: os.release(), arch: os.arch(),
    playwright: require(path.join(playwrightPath, 'package.json')).version,
    commit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: extension, encoding: 'utf8' }).trim(),
    extensionSha256: crypto.createHash('sha256').update(['manifest.json', 'worker.js', 'collector.js', 'protocol.js', 'options.html', 'options.js', 'options.css'].map((file) => fs.readFileSync(path.join(extension, file))).join('\n')).digest('hex'),
    profile, downloads, outside, appData, extensionId: id, receiver: `http://127.0.0.1:${port}`, fixture: origin,
    kind: 'real Chrome extension/options with synthetic authenticated receiver; not product-core E2E; not VERIFIED proof',
  });
  await options.locator('#endpoint').fill(`http://127.0.0.1:${port}`);
  await options.locator('#token').fill(token);
  await options.locator('#enabled').check();
  await options.getByRole('button', { name: 'Save settings', exact: true }).click();
  await options.getByText('Settings saved. Review the connection status and available folders below.', { exact: true }).waitFor();
  await options.getByRole('button', { name: 'Check connection', exact: true }).click();
  await options.locator('#connection').getByText(/^Connected to Traeva\./).waitFor();
  assert.equal(await options.locator('#roots li').innerText(), downloads);
  results.push({ name: 'options default-off, pair/save, authenticated connection and authorized roots', result: 'PASS' });
  const page = await context.newPage();
  await page.goto(origin);
  async function download(name, target = page) {
    const wait = target.waitForEvent('download');
    await target.locator(`#${name}`).click();
    const file = await wait;
    await until(async () => (await options.evaluate((name) => chrome.downloads.search({ query: [name] }).then((items) => items.some((item) => item.state === 'complete')), name)));
    return file;
  }
  await download('normal');
  await until(() => deliveries.some((event) => event.state === 'complete' && event.filename.endsWith('/normal.txt')));
  const normal = deliveries.filter((event) => event.filename.endsWith('/normal.txt'));
  assert.ok(normal.some((event) => event.state === 'in_progress'));
  assert.ok(normal.some((event) => event.state === 'complete'));
  assert.equal(new Set(normal.map((event) => event.download_id)).size, 1);
  results.push({ name: 'real download automatically delivers in_progress then complete', result: 'PASS', downloadId: normal[0].download_id });

  await download('normal');
  await until(() => deliveries.some((event) => event.state === 'complete' && event.filename.endsWith('/normal (1).txt')));
  const repeated = deliveries.find((event) => event.state === 'complete' && event.filename.endsWith('/normal (1).txt'));
  assert.notEqual(repeated.download_id, normal[0].download_id);
  assert.equal(repeated.initial_url, normal[0].initial_url);
  results.push({ name: 'repeat download has separate Chrome ID and filename', result: 'PASS', downloadId: repeated.download_id });

  await cdp.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: outside });
  await download('outside');
  await delay(500);
  assert.equal(deliveries.some((event) => event.filename.includes('outside.txt')), false);
  await cdp.send('Browser.setDownloadBehavior', { behavior: 'default', eventsEnabled: true });
  results.push({ name: 'real download outside authorized root sends no event', result: 'PASS' });

  const incognito = await browser.newContext({ acceptDownloads: true });
  try {
    const privatePage = await incognito.newPage();
    await privatePage.goto(origin);
    const awaited = privatePage.waitForEvent('download');
    await privatePage.locator('#incognito').click();
    const file = await awaited;
    await file.saveAs(path.join(outside, 'incognito.txt'));
    await delay(500);
    assert.equal(deliveries.some((event) => event.filename.includes('incognito.txt')), false);
    assert.equal(await options.evaluate(() => chrome.extension.isAllowedIncognitoAccess()), false);
    results.push({ name: 'real incognito download is not delivered; extension incognito access disabled', result: 'PASS' });
  } finally { await incognito.close(); }

  await close(server);
  await download('offline');
  const pending = await until(async () => (await options.evaluate(() => chrome.storage.local.get(['pending', 'status']))).pending?.find((entry) => entry.attempt > 0));
  assert.ok(pending);
  await options.locator('#check').click();
  await options.locator('#message').getByText(/^Cannot reach Traeva\./).waitFor();
  const beforeReconnect = await options.evaluate(() => chrome.storage.local.get(null));
  save('offline-storage-redacted.json', { ...beforeReconnect, config: { ...beforeReconnect.config, token: '[redacted]' } });
  assert.equal(JSON.stringify(beforeReconnect).includes('signature=synthetic-only'), false);
  assert.equal(JSON.stringify(beforeReconnect).includes('/files/'), false);
  server = receiver();
  await listen(server, port);
  const nextAttemptAt = Math.max(...beforeReconnect.pending.map((entry) => entry.nextAt));
  await until(() => deliveries.some((event) => event.state === 'complete' && event.filename.endsWith('/offline.txt')), Math.max(0, nextAttemptAt - Date.now()) + 90_000);
  results.push({ name: 'offline download stays pending and alarm automatically delivers after reconnect without user action', result: 'PASS' });
  await options.locator('#message').getByText(/^Connection recovered\./).waitFor();
  assert.equal(await options.locator('#roots li').innerText(), downloads);
  await options.reload();

  const count = records.size;
  await options.getByRole('button', { name: 'Import the last 24 hours', exact: true }).click();
  await options.getByText('Delivery check finished. Review waiting records and recent results below.', { exact: true }).waitFor();
  assert.equal(records.size, count);
  const state = await options.evaluate(() => chrome.storage.local.get(null));
  assert.equal(JSON.stringify(state).includes('signature=synthetic-only'), false);
  assert.equal(JSON.stringify(state).includes('/files/'), false);
  assert.equal(state.pending.length, 0);
  save('extension-storage-redacted.json', { ...state, config: { ...state.config, token: '[redacted]' } });
  results.push({ name: 'explicit replay preserves receiver event identities; extension storage contains no source URLs', result: 'PASS' });

  await options.locator('#enabled').uncheck();
  await options.getByRole('button', { name: 'Save settings', exact: true }).click();
  await options.getByText('Collection is off. Waiting deliveries were cleared; sources already saved in Traeva remain there.', { exact: true }).waitFor();
  await download('disabled');
  await delay(500);
  assert.equal(deliveries.some((event) => event.filename.includes('disabled.txt')), false);
  results.push({ name: 'settings disable stops new collection without popup', result: 'PASS' });
  await options.screenshot({ path: path.join(appData, 'options.png'), fullPage: true });
  await options.setViewportSize({ width: 360, height: 900 });
  assert.equal(await options.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await options.screenshot({ path: path.join(appData, 'options-mobile.png'), fullPage: true });
  assert.deepEqual(browserErrors, []);
  results.push({ name: 'options desktop/mobile render and no page exceptions', result: 'PASS' });
  save('outcome.json', { result: 'PASS', automaticVerified: 'NOT_IMPLEMENTED', productionCoreE2E: 'NOT_TESTED' });
})().catch((error) => {
  save('failure.json', { message: error.message, stack: error.stack });
  save('outcome.json', { result: 'FAIL', automaticVerified: 'NOT_IMPLEMENTED', productionCoreE2E: 'NOT_TESTED' });
  process.exitCode = 1;
}).finally(async () => {
  save('results.json', results);
  save('deliveries.json', deliveries);
  save('browser-errors.json', browserErrors);
  if (context) {
    const options = context.pages().find((page) => page.url().endsWith('/options.html'));
    if (options) {
      const state = await options.evaluate(() => chrome.storage.local.get(null)).catch(() => null);
      if (state) save('final-storage-redacted.json', { ...state, config: { ...state.config, token: '[redacted]' } });
      const alarms = await options.evaluate(() => chrome.alarms.getAll()).catch(() => null);
      if (alarms) save('final-alarms.json', alarms);
    }
  }
  if (context) await context.close();
  fs.rmSync(profile, { recursive: true, force: true });
  await close(server).catch(() => {});
  await close(fixtures).catch(() => {});
  console.log(JSON.stringify({ run, completedChecks: results.length, outcome: process.exitCode ? 'FAIL' : 'PASS' }));
});
