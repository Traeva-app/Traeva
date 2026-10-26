// Explicit adapter for a test-owned, real Traeva receiver. No synthetic receiver.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

if (!process.env.TRAEVA_SOURCE_TEST_CONFIG || !process.env.TRAEVA_TEST_PLAYWRIGHT) {
  throw new Error('Set TRAEVA_SOURCE_TEST_CONFIG and TRAEVA_TEST_PLAYWRIGHT for this explicit integration check.');
}
const config = JSON.parse(fs.readFileSync(process.env.TRAEVA_SOURCE_TEST_CONFIG, 'utf8'));
assert.match(config.endpoint, /^http:\/\/127\.0\.0\.1:[1-9][0-9]{0,4}$/);
assert.match(config.pairing_token, /^[a-f0-9]{64}$/);
assert.ok(path.isAbsolute(config.root) && path.isAbsolute(config.app_data));
const root = fs.realpathSync(config.root);
const appData = fs.realpathSync(config.app_data);
assert.ok(!root.startsWith(`${appData}/`) && !appData.startsWith(`${root}/`) && root !== appData);
const externalEvidence = process.env.TRAEVA_SOURCE_TEST_EVIDENCE;
const evidence = externalEvidence ? fs.realpathSync(externalEvidence) : path.join(appData, 'chrome-adapter');
if (externalEvidence) {
  for (const protectedDir of [root, appData]) {
    assert.ok(evidence !== protectedDir && !evidence.startsWith(`${protectedDir}/`) && !protectedDir.startsWith(`${evidence}/`));
  }
}
fs.mkdirSync(evidence, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'traeva-source-core-chrome-'));
fs.mkdirSync(path.join(profile, 'Default'));
fs.writeFileSync(path.join(profile, 'Default', 'Preferences'), JSON.stringify({ download: { default_directory: root, prompt_for_download: false, directory_upgrade: true } }));
const { chromium } = require(process.env.TRAEVA_TEST_PLAYWRIGHT);
const extension = path.resolve(__dirname, '..');
const payload = Buffer.from('Source integration fixture.\n'.repeat(1024));
const filename = `source-${crypto.randomBytes(4).toString('hex')}.txt`;
const server = http.createServer(async (req, res) => {
  if (req.url === '/') {
    res.setHeader('Content-Type', 'text/html');
    res.end(`<!doctype html><title>Traeva synthetic source test</title><a id="download" href="/download">${filename}</a>`);
    return;
  }
  if (req.url === '/download') { res.writeHead(302, { Location: '/final-download' }); res.end(); return; }
  if (req.url !== '/final-download') { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': 'text/plain', 'Content-Length': payload.length, 'Content-Disposition': `attachment; filename="${filename}"` });
  for (let offset = 0; offset < payload.length; offset += 4096) {
    res.write(payload.subarray(offset, offset + 4096));
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  res.end();
});
let context;
let options;
const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2));
const errors = [];
(async () => {
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  context = await chromium.launchPersistentContext(profile, {
    executablePath: process.env.TRAEVA_TEST_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    headless: process.env.TRAEVA_TEST_CHROME_HEADLESS === '1', ignoreDefaultArgs: ['--disable-extensions'],
    args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check'], acceptDownloads: true,
  });
  const cdp = await context.browser().newBrowserCDPSession();
  await cdp.send('Browser.setDownloadBehavior', { behavior: 'default', eventsEnabled: true });
  const { id } = await cdp.send('Extensions.loadUnpacked', { path: extension });
  options = await context.newPage();
  options.on('pageerror', (error) => errors.push(error.message));
  await options.goto(`chrome-extension://${id}/options.html`);
  await options.getByText('Collection is off. Follow the setup steps above.', { exact: true }).waitFor();
  await options.locator('#endpoint').fill(config.endpoint);
  await options.locator('#token').fill(config.pairing_token);
  await options.locator('#enabled').check();
  await options.getByRole('button', { name: 'Save settings', exact: true }).click();
  await options.getByText('Settings saved. Review the connection status and available folders below.', { exact: true }).waitFor();
  await options.getByRole('button', { name: 'Check connection', exact: true }).click();
  await options.getByText('Connection check finished. Review the status below.', { exact: true }).waitFor();
  await options.locator('#connection').getByText(/^Connected to Traeva\./).waitFor();
  const displayedRoots = await options.locator('#roots li').allTextContents();
  assert.ok(displayedRoots.includes(root));
  save('environment.json', { browser: await cdp.send('Browser.getVersion'), platform: os.platform(), arch: os.arch(), playwright: require(path.join(process.env.TRAEVA_TEST_PLAYWRIGHT, 'package.json')).version, extensionId: id, profile, root, appData, kind: 'actual Chrome extension to actual test-owned Traeva receiver; browser download association remains unverified' });
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const download = page.waitForEvent('download');
  await page.locator('#download').click();
  await download;
  const deadline = Date.now() + 20_000;
  let delivered;
  while (Date.now() < deadline) {
    delivered = await options.evaluate(async (filename) => {
      const items = await chrome.downloads.search({ query: [filename] });
      const item = items.find((value) => value.filename.endsWith(`/${filename}`));
      const saved = await chrome.storage.local.get(['config', 'pending', 'status']);
      if (item?.state !== 'complete' || saved.pending?.length || !saved.status?.lastSavedAt || saved.status?.error) return null;
      return { profile_id: saved.config.profileId, download_id: String(item.id), filename: item.filename, started_at_ms: Date.parse(item.startTime), completed_at_ms: Date.parse(item.endTime), state: item.state, initial_url: item.url || null, final_url: item.finalUrl || null, referrer: item.referrer || null, file_size: item.fileSize >= 0 ? item.fileSize : null, incognito: item.incognito, acknowledged_at_ms: saved.status.lastSavedAt };
    }, filename);
    if (delivered) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(delivered, 'Chrome download must receive durable acknowledgement from the actual receiver');
  assert.equal(delivered.incognito, false);
  assert.notEqual(delivered.initial_url, delivered.final_url);
  assert.deepEqual(fs.readFileSync(path.join(root, filename)), payload);
  assert.deepEqual(errors, []);
  const { acknowledged_at_ms, ...event } = delivered;
  const delivery = { ...delivered, event, sha256: crypto.createHash('sha256').update(payload).digest('hex'), automaticVerified: 'NOT_IMPLEMENTED' };
  save('chrome-delivery.json', delivery);
  if (!externalEvidence) fs.writeFileSync(path.join(appData, 'chrome-delivery.json'), JSON.stringify(delivery, null, 2));
  save('outcome.json', { result: 'PASS', automaticVerified: 'NOT_IMPLEMENTED' });
  console.log(JSON.stringify({ result: 'PASS', evidence, filename: delivered.filename, download_id: delivered.download_id }));
})().catch((error) => {
  save('failure.json', { message: error.message, stack: error.stack });
  save('outcome.json', { result: 'FAIL', automaticVerified: 'NOT_IMPLEMENTED' });
  process.exitCode = 1;
}).finally(async () => {
  if (options) {
    const state = await options.evaluate(() => chrome.storage.local.get(null)).catch(() => null);
    if (state) save('extension-storage-redacted.json', { ...state, config: { ...state.config, token: '[redacted]' } });
  }
  if (context) await context.close();
  fs.rmSync(profile, { recursive: true, force: true });
  await new Promise((resolve) => { server.closeAllConnections(); server.close(resolve); });
});
