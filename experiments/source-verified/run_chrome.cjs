const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const crypto = require('node:crypto'), https = require('node:https'), http = require('node:http');
const zlib = require('node:zlib');
const {execFileSync} = require('node:child_process');
const playwrightModule = process.env.TRAEVA_PLAYWRIGHT_MODULE;
if (!playwrightModule) {
  throw new Error('Set TRAEVA_PLAYWRIGHT_MODULE to an installed Playwright module before running this opt-in lab.');
}
const {chromium} = require(playwrightModule);

const root = fs.mkdtempSync('/private/tmp/traeva-c-verified-chrome-');
const profile = path.join(root, 'profile'), extension = path.join(root, 'extension');
const run = path.join(root, 'native'), evidence = path.join(root, 'evidence'), downloads = path.join(root, 'chrome-downloads');
for (const dir of [profile, extension, evidence, downloads]) fs.mkdirSync(dir, {recursive: true, mode: 0o700});
const save = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2));
const sha = data => crypto.createHash('sha256').update(data).digest('hex');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
const body = Buffer.from('Synthetic HTTPS source acquisition fixture.\n'.repeat(6500)).subarray(0, 262144);
const distinct = Buffer.from('Different independent HTTPS response body!\n'.repeat(6500)).subarray(0, body.length);
const requests = [], stimuli = [], results = [], browserDownloads = [], timers = new Set();
let context, controller, browser, extensionId, origin, plainOrigin, scenarioNow = 'setup';
async function until(fn, label, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {const value = await fn(); if (value) return value; await sleep(30);}
  throw new Error('Timeout: ' + label);
}
const handler = (req, res) => {
  const parsed = new URL(req.url, 'https://localhost');
  requests.push({at: Date.now(), path: parsed.pathname, method: req.method});
  if (parsed.pathname === '/' || parsed.pathname.startsWith('/lab-page/')) {
    res.setHeader('Content-Type', 'text/html');
    res.end('<!doctype html><title>TRAEVA HTTPS acquisition fixture</title><h1>Synthetic HTTPS downloads</h1>' +
      ['normal', 'distinct', 'redirect', 'chunked', 'compressed', 'truncated', 'unsupported', 'http', 'uppercase', 'too-long', 'private-query'].map(name =>
        `<p><a id="${name}" href="${name === 'http' ? plainOrigin + '/file/http' : name === 'redirect' ? '/redirect' : name === 'private-query' ? '/file/normal?secret=SYNTHETIC_QUERY_SECRET' : '/file/' + name}">Download ${name}</a></p>`).join(''));
    return;
  }
  if (parsed.pathname === '/redirect') {res.writeHead(302, {Location: '/redirect2'}); res.end(); return;}
  if (parsed.pathname === '/redirect2') {res.writeHead(307, {Location: '/file/normal'}); res.end(); return;}
  if (!parsed.pathname.startsWith('/file/')) {res.writeHead(404); res.end(); return;}
  const name = parsed.pathname.split('/').pop();
  const payload = name === 'compressed' ? zlib.gzipSync(body) : name === 'distinct' ? distinct : body;
  const disposition = name === 'uppercase' ? 'ATTACHMENT; FILENAME="uppercase.txt"' :
    `attachment; filename="${name === 'too-long' ? 'a'.repeat(140) : name}.${name === 'unsupported' ? 'exe' : 'txt'}"`;
  res.writeHead(200, {'Content-Type': 'text/plain', 'Content-Disposition': disposition,
    ...(name === 'uppercase' ? {'Set-Cookie': 'SYNTHETIC_RESPONSE_COOKIE_SECRET'} : {}),
    'Cache-Control': 'no-store', ...(name === 'chunked' ? {'Transfer-Encoding': 'chunked'} : {'Content-Length': payload.length}),
    ...(name === 'compressed' ? {'Content-Encoding': 'gzip'} : {})});
  let offset = 0;
  const timer = setInterval(() => {
    const end = Math.min(offset + 32768, payload.length); res.write(payload.subarray(offset, end)); offset = end;
    if (name === 'truncated' && offset >= 65536) {clearInterval(timer); timers.delete(timer); res.destroy(); return;}
    if (offset === payload.length) {clearInterval(timer); timers.delete(timer); res.end();}
  }, 80);
  timers.add(timer); res.on('close', () => {clearInterval(timer); timers.delete(timer);});
};

const keyFile = path.join(root, 'tls-key.pem'), certFile = path.join(root, 'tls-cert.pem');
execFileSync('/usr/bin/openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=localhost',
  '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1', '-keyout', keyFile, '-out', certFile], {stdio: 'ignore'});
fs.chmodSync(keyFile, 0o600);
const pub = execFileSync('/usr/bin/openssl', ['x509', '-in', certFile, '-pubkey', '-noout']);
const der = execFileSync('/usr/bin/openssl', ['pkey', '-pubin', '-outform', 'DER'], {input: pub});
const spki = crypto.createHash('sha256').update(der).digest('base64');
const server = https.createServer({key: fs.readFileSync(keyFile), cert: fs.readFileSync(certFile)}, handler);
const plain = http.createServer(handler);
const command = value => controller.evaluate(message => chrome.runtime.sendMessage(message), value);
const snapshot = () => command({type: 'snapshot'});
async function nativeTasks() {return (await snapshot()).native?.records || [];}
async function newTab(scenario) {
  const page = await context.newPage(); await page.goto(origin + '/lab-page/' + scenario + '-' + crypto.randomUUID());
  const tabs = (await command({type: 'tabs'})).tabs;
  const matches = tabs.filter(tab => tab.url === page.url());
  if (matches.length !== 1) throw new Error('Ambiguous synthetic tab');
  const tabId = matches[0].id;
  const reply = await command({type: 'attach', tabId, scenario});
  if (!reply.ok) throw new Error('Attach: ' + reply.error);
  return {page, tabId};
}
async function receiving(scenario, count = 1) {
  return until(async () => {const xs = (await nativeTasks()).filter(x => x.scenario === scenario);
    return xs.length === count && xs.every(x => x.state === 'RECEIVING' && x.receivedBytes >= 32768) ? xs : null;}, 'receiving ' + scenario);
}
async function terminal(scenario, count = 1) {
  return until(async () => {const xs = (await snapshot()).acquisitions.filter(x => x.scenario === scenario);
    return xs.length === count && xs.every(x => ['HISTORICAL_ACQUISITION_OBSERVED', 'FAILED_AFTER_TAKE', 'OUTCOME_UNKNOWN', 'TAKE_OUTCOME_UNKNOWN'].includes(x.state)) ? xs : null;}, 'terminal ' + scenario);
}
function stat(file) {const s = fs.lstatSync(file); return {inode: s.ino, device: s.dev, size: s.size, sha256: sha(fs.readFileSync(file))};}

(async () => {
  const startedAt = Date.now();
  let error;
  try {
    await new Promise(resolve => plain.listen(0, '127.0.0.1', resolve));
    plainOrigin = 'http://127.0.0.1:' + plain.address().port;
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    origin = 'https://127.0.0.1:' + server.address().port;
    const token = crypto.randomBytes(32).toString('hex');
    for (const file of ['worker.js', 'control.html', 'control.js']) fs.copyFileSync(path.join(__dirname, file), path.join(extension, file));
    fs.writeFileSync(path.join(extension, 'config.js'), 'const LAB = ' + JSON.stringify({token, origins: [origin], downloadOrigins: [origin, plainOrigin], delivery: path.join(run, 'delivery')}) + ';\n', {mode: 0o600});
    fs.writeFileSync(path.join(extension, 'manifest.json'), JSON.stringify({manifest_version: 3, name: 'TRAEVA opt-in acquisition experiment', version: '0.0.1',
      permissions: ['debugger', 'nativeMessaging', 'downloads', 'tabs'], background: {service_worker: 'worker.js'}, incognito: 'not_allowed'}, null, 2));
    const wrapper = path.join(root, 'chrome-contained.sh');
    fs.writeFileSync(wrapper, '#!/bin/sh\nexec /usr/bin/sandbox-exec -p ' + quote('(version 1) (allow default) (deny file-write* (subpath "' + os.homedir() + '"))') +
      ' "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" "$@"\n', {mode: 0o700});
    context = await chromium.launchPersistentContext(profile, {executablePath: wrapper, headless: false, ignoreDefaultArgs: ['--disable-extensions'],
      args: ['--enable-unsafe-extension-debugging', '--no-first-run', '--no-default-browser-check', '--ignore-certificate-errors-spki-list=' + spki],
      acceptDownloads: true});
    browser = await context.browser().newBrowserCDPSession();
    await browser.send('Browser.setDownloadBehavior', {behavior: 'allow', downloadPath: downloads, eventsEnabled: true});
    for (const event of ['Browser.downloadWillBegin', 'Browser.downloadProgress']) browser.on(event, data => {
      const safe = {};
      for (const key of ['guid', 'state', 'totalBytes', 'receivedBytes']) if (Object.hasOwn(data, key)) safe[key] = data[key];
      browserDownloads.push({at: Date.now(), scenario: scenarioNow, event, data: safe});
    });
    ({id: extensionId} = await browser.send('Extensions.loadUnpacked', {path: extension}));
    const configPath = path.join(root, 'native-config.json');
    fs.writeFileSync(configPath, JSON.stringify({token, extensionOrigin: `chrome-extension://${extensionId}/`, run, origins: [origin]}), {mode: 0o600});
    const launcher = path.join(root, 'native-host.sh');
    fs.writeFileSync(launcher, '#!/bin/sh\nexec /usr/bin/python3 -B ' + quote(path.join(__dirname, 'native_host.py')) + ' --config ' + quote(configPath) + ' "$@"\n', {mode: 0o700});
    const manifests = path.join(profile, 'NativeMessagingHosts'); fs.mkdirSync(manifests, {recursive: true, mode: 0o700});
    fs.writeFileSync(path.join(manifests, 'org.traeva.source_verified_lab.json'), JSON.stringify({name: 'org.traeva.source_verified_lab', description: 'Isolated source acquisition lab',
      path: launcher, type: 'stdio', allowed_origins: [`chrome-extension://${extensionId}/`]}));
    controller = await context.newPage(); await controller.goto(`chrome-extension://${extensionId}/control.html`);
    save('environment.json', {root, run, profile, extensionId, origin, plainOrigin, headed: true, startedAt,
      browserVersion: await browser.send('Browser.getVersion'), platform: os.platform(), release: os.release(), arch: os.arch(),
      tlsScope: 'Real TLS to synthetic loopback server; exact generated fixture SPKI trusted by flag, not normal public CA acceptance',
      fixtureCertificateSha256: sha(fs.readFileSync(certFile)), nativeManifestScope: 'Only independent user-data-dir/NativeMessagingHosts',
      fixture: {bytes: body.length, sha256: sha(body), distinctSha256: sha(distinct)}});
    for (const file of ['run_chrome.cjs', 'worker.js', 'native_host.py', 'writer.py', 'receipt.py', 'base_receiver.py', 'test_writer.py', 'test_native_host.py', 'control.html', 'control.js']) {
      fs.copyFileSync(path.join(__dirname, file), path.join(evidence, file));
    }
    const disabled = await command({type: 'attach', tabId: 0, scenario: 'disabled'});
    results.push({scenario: 'disabled', actual: disabled, expected: 'takeover_disabled'});
    await controller.locator('#enable').click();
    await until(async () => (await controller.locator('#state').textContent()).includes('explicit_consent_required'), 'unchecked consent rejected');
    results.push({scenario: 'unchecked-consent', state: await controller.locator('#state').textContent()});
    await controller.locator('#consent').check(); await controller.locator('#enable').click();
    await until(async () => (await controller.locator('#state').textContent()).includes('"ok": true'), 'native authorization');
    save('consent.json', {actualExplicitClicks: ['unchecked authorize rejected', 'check consent', 'authorize displayed save location'], result: await controller.locator('#state').textContent()});
    if (process.env.TRAEVA_LAB_HOLD_UI) {
      const tab = await newTab('permission-ui'); await tab.page.bringToFront();
      console.log(JSON.stringify({uiReady: true, root, tabId: tab.tabId, origin}));
      await until(() => fs.existsSync(path.join(root, 'continue-ui')), 'external UI capture complete', 120000);
      await tab.page.close();
    }
    const scenarios = process.env.TRAEVA_LAB_SCENARIOS?.split(',') || ['normal', 'redirect', 'parallel', 'same-tab', 'same-url', 'fake-stage', 'stage-swap', 'target-exists', 'truncated', 'chunked', 'compressed', 'unsupported', 'http', 'debugger-detach', 'native-disconnect'];
    for (const scenario of scenarios) {
      scenarioNow = scenario;
      const entry = {scenario, startedAt: Date.now()};
      const before = (await snapshot()).downloadItems.length;
      const tabs = [await newTab(scenario)];
      if (['parallel', 'same-url', 'stage-swap'].includes(scenario)) tabs.push(await newTab(scenario));
      for (let i = 0; i < tabs.length; i++) await tabs[i].page.locator('#' + (scenario === 'redirect' ? 'redirect' : scenario === 'truncated' ? 'truncated' :
        ['chunked', 'compressed', 'unsupported', 'http', 'uppercase', 'too-long', 'private-query'].includes(scenario) ? scenario : i === 1 && scenario !== 'same-url' ? 'distinct' : 'normal')).click({noWaitAfter: true});
      if (scenario === 'same-tab') {
        await receiving(scenario);
        await tabs[0].page.locator('#distinct').click({noWaitAfter: true});
      }
      const bypass = ['chunked', 'compressed', 'unsupported', 'http', 'too-long', 'private-query'].includes(scenario);
      if (['fake-stage', 'stage-swap', 'target-exists', 'debugger-detach', 'native-disconnect'].includes(scenario)) {
        const xs = await receiving(scenario, tabs.length);
        if (scenario === 'fake-stage') {
          const file = path.join(run, 'app-data', 'staging', xs[0].stagingName); const original = stat(file);
          fs.renameSync(file, file + '.actor-preserved'); fs.writeFileSync(file, body);
          stimuli.push({scenario, action: 'replace_stage_with_same_content_object', original, fake: stat(file)});
        } else if (scenario === 'stage-swap') {
          const a = path.join(run, 'app-data', 'staging', xs[0].stagingName), b = path.join(run, 'app-data', 'staging', xs[1].stagingName);
          const before = [stat(a), stat(b)]; fs.renameSync(a, a + '.swap'); fs.renameSync(b, a); fs.renameSync(a + '.swap', b);
          stimuli.push({scenario, action: 'exchange_staging_names', before, after: [stat(a), stat(b)]});
        } else if (scenario === 'target-exists') {
          const file = path.join(run, 'delivery', xs[0].destination); fs.writeFileSync(file, 'Existing target must survive.');
          stimuli.push({scenario, action: 'precreate_target', before: stat(file), path: file});
        } else if (scenario === 'native-disconnect') {
          await command({type: 'disconnectNative'});
          stimuli.push({scenario, action: 'explicit_native_port_disconnect', receivedBytes: xs[0].receivedBytes});
        } else {
          await command({type: 'detach', tabId: tabs[0].tabId});
          stimuli.push({scenario, action: 'explicit_debugger_detach', receivedBytes: xs[0].receivedBytes});
        }
      }
      if (bypass) {
        await until(() => browserDownloads.some(x => x.scenario === scenario && x.data.state === 'completed'), 'Chrome fallback completion');
        entry.acquisitions = (await snapshot()).acquisitions.filter(x => x.scenario === scenario);
        entry.browserFallbackCompleted = true;
      } else entry.acquisitions = await terminal(scenario, scenario === 'same-tab' ? 2 : tabs.length);
      entry.downloadItemDelta = (await snapshot()).downloadItems.length - before;
      entry.endedAt = Date.now(); results.push(entry);
      save('scenario-results.json', results); save('extension-evidence.json', await snapshot()); save('stimuli.json', stimuli);
      for (const tab of tabs) await tab.page.close();
    }
    save('extension-evidence.json', await snapshot());
  } catch (failure) {
    error = 'harness_failure'; save('error.json', {scenario: scenarioNow, error});
    process.exitCode = 1;
  } finally {
    if (controller && !controller.isClosed()) save('extension-evidence.json', await snapshot().catch(failure => ({error: failure.message})));
    if (context) await context.close().catch(failure => save('close-error.json', {message: failure.message}));
    for (const timer of timers) clearInterval(timer);
    for (const s of [server, plain]) if (s.listening) await new Promise(resolve => s.close(resolve));
    save('server-requests.json', requests); save('browser-download-events.json', browserDownloads);
    save('stimuli.json', stimuli); save('scenario-results.json', results);
    save('lifecycle.json', {startedAt, endedAt: Date.now(), serverClosed: !server.listening, plainClosed: !plain.listening, browserClosed: !context || !context.browser()?.isConnected()});
    console.log(JSON.stringify({root, run, evidence, scenarios: results.length, error: error || null}));
  }
})();
