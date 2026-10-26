// Actual Options page with controlled Chrome message delays; no download or real extension transport.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

if (!process.env.TRAEVA_TEST_PLAYWRIGHT) throw new Error('Set TRAEVA_TEST_PLAYWRIGHT to the existing test dependency.');
const { chromium } = require(process.env.TRAEVA_TEST_PLAYWRIGHT);
const extension = path.resolve(__dirname, '..');
const evidence = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'traeva-source-options-order-')));
const results = [];
const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2));
const server = http.createServer((req, res) => {
  const name = req.url.slice(1);
  if (!['options.html', 'options.css', 'options.js', 'diagnostics.js', 'protocol.js'].includes(name)) { res.writeHead(404); res.end(); return; }
  res.setHeader('Content-Type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html');
  res.end(fs.readFileSync(path.join(extension, name)));
});
let browser;
async function scenario(action, previous, next, rejectPoll = false, releaseWhileBusy = false) {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => {
    const value = (kind) => ({
      config: { endpoint: 'http://127.0.0.1:18080', token: 'a'.repeat(64), enabled: true, recoverRecent: false },
      pendingCount: 1, pending: [{ id: 1, nextAt: 1000, error: 'waiting_completion' }], labels: {},
      roots: kind === 'connected' ? [{ path: '/synthetic-downloads' }] : [],
      status: { connection: { kind, checkedAt: 1000 }, error: kind === 'pairing_rejected' ? kind : null, recent: [] },
    });
    const test = window.testOrder = { value: value('connected'), makeValue: value };
    const interval = window.setInterval;
    window.setInterval = (callback, ms, ...args) => {
      if (ms === 3000) { test.tick = callback; return 1; }
      return interval(callback, ms, ...args);
    };
    window.chrome = { runtime: { sendMessage: async ({ type }) => {
      if (type === 'status' && test.holdPoll) {
        test.holdPoll = false;
        return new Promise((resolve, reject) => { test.releasePoll = resolve; test.rejectPoll = reject; });
      }
      if (test.holdAction && type !== 'status') {
        await new Promise((resolve) => { test.releaseAction = resolve; });
        test.holdAction = false;
      }
      if (type === 'check') {
        if (test.value.status.connection.kind === 'pairing_rejected') return { ok: false, error: 'pairing_rejected' };
        return { ok: true, value: { enabled: true, roots: test.value.roots, delivery: test.value } };
      }
      return { ok: true, value: test.value };
    } } };
  });
  const snapshot = () => page.evaluate(() => Object.fromEntries(['connection', 'message', 'roots', 'delivery', 'pending', 'recent', 'checked'].map((id) => [id, document.getElementById(id).textContent])));
  const ready = () => page.waitForFunction(() => !document.getElementById('save').disabled);
  try {
    await page.goto(`http://127.0.0.1:${server.address().port}/options.html`);
    await ready();
    await page.evaluate(({ previous, next, releaseWhileBusy }) => {
      const test = window.testOrder;
      test.old = test.makeValue(previous);
      test.holdPoll = true;
      test.poll = test.tick();
      test.value = test.makeValue(next);
      test.holdAction = releaseWhileBusy;
    }, { previous, next, releaseWhileBusy });
    await page.locator(`#${action}`).click();
    if (releaseWhileBusy) {
      await page.waitForFunction(() => !!window.testOrder.releaseAction);
      const whileBusy = await snapshot();
      await page.evaluate(async () => { const test = window.testOrder; test.releasePoll({ ok: true, value: test.old }); await test.poll; });
      assert.deepEqual(await snapshot(), whileBusy, 'A poll response must not repaint while the action is in progress');
      await page.evaluate(() => window.testOrder.releaseAction());
    }
    await ready();
    const expected = await snapshot();
    assert.match(expected.connection, next === 'connected' ? /^Connected to Traeva\./ : /does not match/);
    if (!releaseWhileBusy) {
      await page.evaluate(async (reject) => {
        const test = window.testOrder;
        if (reject) test.rejectPoll(new Error('synthetic delayed status failure'));
        else test.releasePoll({ ok: true, value: test.old });
        await test.poll;
      }, rejectPoll);
      assert.deepEqual(await snapshot(), expected, 'A poll started before the action must not replace its completed result');
    }
    // A later current-generation poll must still update the live connection state.
    const later = next === 'connected' ? 'pairing_rejected' : 'connected';
    await page.evaluate(async (kind) => { window.testOrder.value = window.testOrder.makeValue(kind); await window.testOrder.tick(); }, later);
    assert.match((await snapshot()).connection, later === 'connected' ? /^Connected to Traeva\./ : /does not match/);
    assert.deepEqual(errors, []);
    return { result: 'PASS' };
  } finally { await page.close(); }
}

(async () => {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  browser = await chromium.launch({ executablePath: process.env.TRAEVA_TEST_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
  save('environment.json', { platform: os.platform(), arch: os.arch(), browser: await browser.version(), options_sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(extension, 'options.js'))).digest('hex'), scope: 'actual Options HTML/module in real headless Chrome with controlled synthetic Chrome messages; not natural Chrome message reordering or extension transport evidence' });
  for (const action of ['save', 'check', 'retry', 'history']) {
    for (const [previous, next, reject] of [['connected', 'pairing_rejected', false], ['pairing_rejected', 'connected', false], ['connected', 'connected', true]]) {
      const name = `${action}: old ${reject ? 'failure' : previous} poll arrives after new ${next} result`;
      try { results.push({ name, ...await scenario(action, previous, next, reject) }); }
      catch (error) { results.push({ name, result: 'FAIL', message: error.message }); }
    }
  }
  for (const next of ['connected', 'pairing_rejected']) {
    const name = `check: old poll arrives while ${next} action is pending`;
    try { results.push({ name, ...await scenario('check', 'connected', next, false, true) }); }
    catch (error) { results.push({ name, result: 'FAIL', message: error.message }); }
  }
  const passed = results.filter((entry) => entry.result === 'PASS').length;
  save('results.json', results);
  save('outcome.json', { result: passed === results.length ? 'PASS' : 'FAIL', passed, failed: results.length - passed, total: results.length });
  console.log(JSON.stringify({ evidence, passed, failed: results.length - passed, total: results.length }));
  if (passed !== results.length) process.exitCode = 1;
})().catch((error) => { save('failure.json', { message: error.message, stack: error.stack }); process.exitCode = 1; })
  .finally(async () => { if (browser) await browser.close(); await new Promise((resolve) => server.close(resolve)); });
