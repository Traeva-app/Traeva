// Executes the lab worker source. CDP and native replies are simulated, not Chrome acceptance.
const assert = require('node:assert/strict');
const fs = require('node:fs'), vm = require('node:vm'), crypto = require('node:crypto'), path = require('node:path');
const source = process.env.TRAEVA_WORKER_SOURCE || path.join(__dirname, 'worker.js');
const baseline = process.argv.includes('--baseline'), origin = 'https://fixture.invalid';
const cases = [];
function setup(options = {}) {
  const commands = [], messages = [], handlers = {}, callbacks = {};
  const listener = key => ({addListener(fn) {handlers[key] = fn;}});
  let context;
  const port = {onMessage: {addListener(fn) {callbacks.message = fn;}}, onDisconnect: {addListener(fn) {callbacks.disconnect = fn;}},
    postMessage(message) {
      messages.push(message);
      queueMicrotask(() => {
        if (message.command === 'prepare' && options.detachPrepare) vm.runInContext('sessions.delete(7)', context);
        if (message.command === 'complete' && options.loseReply) return;
        let reply = {rid: message.rid, ok: true};
        if (message.command === 'prepare' && options.refusePrepare) reply = {rid: message.rid, ok: false};
        if (message.command === 'complete' && options.unknownCommit) reply = {rid: message.rid, ok: false, outcome: 'OUTCOME_UNKNOWN'};
        callbacks.message(reply);
      });
    }};
  context = vm.createContext({URL, TextEncoder, crypto, Object, queueMicrotask,
    setTimeout: (fn, ms) => setTimeout(fn, ms === 15000 ? 50 : ms), clearTimeout,
    btoa: s => Buffer.from(s, 'binary').toString('base64'), importScripts() {},
    LAB: {origins: [origin], token: 'isolated-test-token', delivery: '/private/tmp/traeva-c-verified-vm/delivery'},
    chrome: {debugger: {onEvent: listener('event'), onDetach: listener('detach'),
      sendCommand: async (src, method, params) => {
        commands.push({method, params});
        if (method === 'Fetch.takeResponseBodyAsStream') return {stream: 'stream-' + params.requestId};
        if (method === 'IO.read') {
          if (options.privateFailure) throw new Error('SYNTHETIC_ERROR_SECRET');
          if (options.detachRead) vm.runInContext('sessions.set(7,{epoch:"new-epoch",scenario:"vm"})', context);
          return {data: Buffer.from('synthetic-' + params.handle).toString('base64'), base64Encoded: true, eof: true};
        }
        return {};
      }}, runtime: {onMessage: listener('control'), id: 'test-extension', getURL: p => 'chrome-extension://test-extension/' + p, connectNative: () => port},
      downloads: {onCreated: listener('download'), onChanged: listener('changed'), search: async () => {throw new Error('unrelated_history_export');}},
      tabs: {query: async () => [{id: 8, url: 'https://private.invalid/?SYNTHETIC_QUERY_SECRET', title: 'SYNTHETIC_TITLE_SECRET'}]}}});
  vm.runInContext(fs.readFileSync(source, 'utf8'), context, {filename: source});
  vm.runInContext('authorized=true; sessions.set(7,{epoch:"vm-epoch",scenario:"vm"});', context);
  const snapshot = () => JSON.parse(vm.runInContext('JSON.stringify({events, acquisitions, paused:[...paused.values()]})', context));
  return {commands, messages, handlers, context, snapshot};
}
function event(id = 'request', cd = 'attachment; filename="file.txt"') {
  return {requestId: id, request: {url: origin + '/' + id, method: 'GET', headers: {}}, responseStatusCode: 200,
    responseHeaders: [{name: 'Content-Length', value: String(Buffer.byteLength('synthetic-stream-' + id))},
      {name: 'Content-Type', value: 'text/plain; charset=utf-8'}, {name: 'Content-Disposition', value: cd}]};
}
async function invoke(t, input) {t.context.input = input; await vm.runInContext('onPaused({tabId:7},input)', t.context);}
function taken(t) {return t.commands.some(c => c.method === 'Fetch.takeResponseBodyAsStream');}
function saved(name, t, extra = {}) {cases.push({name, scope: 'worker source VM; mocked CDP/native', ...extra, commands: t.commands.map(c => c.method), states: t.snapshot().acquisitions.map(a => a.state)});}
(async () => {
  const privacy = {requestId: 'post', request: {url: origin + '/login?SYNTHETIC_QUERY_SECRET', method: 'POST',
    headers: {Cookie: 'SYNTHETIC_COOKIE_SECRET', Authorization: 'SYNTHETIC_AUTH_SECRET'}, postData: 'SYNTHETIC_POST_SECRET'}};
  if (baseline) {
    const t = setup(); await invoke(t, event('uppercase', 'ATTACHMENT; FILENAME="file.txt"'));
    assert(taken(t)); assert.equal(t.snapshot().acquisitions[0].state, 'FAILED_AFTER_TAKE');
    saved('B3-uppercase-parser', t, {result: 'FAIL_ORIGINAL'});
    const p = setup(); await invoke(p, privacy);
    for (const marker of ['SYNTHETIC_QUERY_SECRET', 'SYNTHETIC_COOKIE_SECRET', 'SYNTHETIC_AUTH_SECRET', 'SYNTHETIC_POST_SECRET']) assert(JSON.stringify(p.snapshot()).includes(marker));
    saved('B4-private-POST-cache', p, {result: 'FAIL_ORIGINAL', privateFieldsRetained: true});
  } else {
    for (const cd of ['attachment; FILENAME="file.txt"', 'ATTACHMENT; filename="FILE.MD"', 'Attachment;   FiLeNaMe="test_name-1.txt"', 'attachment; filename="' + 'a'.repeat(139) + '.txt"']) {
      const t = setup(); await invoke(t, event('valid', cd)); assert(taken(t));
      assert.equal(t.snapshot().acquisitions[0].state, 'HISTORICAL_ACQUISITION_OBSERVED');
      assert(t.messages.find(m => m.command === 'prepare').payload.destination);
      saved('B3-supported-boundary', t, {disposition: cd, result: 'PASS_VM'});
    }
    const invalid = [
      e => e.responseHeaders[2].value = 'attachment; filename="' + 'a'.repeat(140) + '.txt"',
      e => e.responseHeaders[2].value = 'attachment; filename="../file.txt"',
      e => e.responseHeaders[2].value = 'attachment; filename="file.exe"',
      e => e.responseHeaders[2].value = 'attachment; filename*=UTF-8\'\'file.txt',
      e => e.responseHeaders[2].value = 'attachment; filename="文件.txt"',
      e => e.responseHeaders.push(e.responseHeaders[2]),
      e => e.responseHeaders[0].value = '0', e => e.responseHeaders[0].value = '20971521',
      e => e.responseHeaders.push(e.responseHeaders[0]),
      e => e.responseHeaders.push({name: 'Transfer-Encoding', value: 'chunked'}),
      e => e.responseHeaders.push({name: 'Content-Encoding', value: 'gzip'}),
      e => e.responseStatusCode = 206, e => e.request.method = 'POST',
      e => e.request.url += '?SYNTHETIC_QUERY_SECRET', e => e.request.url += '#secret',
      e => e.request.url = 'https://user:password@fixture.invalid/file',
      e => e.request.url = 'http://fixture.invalid/file', e => e.responseHeaders[1].value = 'application/octet-stream',
      e => e.responseHeaders[1].value = 'text/plain; ' + 'a'.repeat(257),
      e => e.responseHeaders[2].value = 'attachment; filename="file.txt"; extra=yes',
      e => e.responseHeaders = null,
      e => e.requestId = null, e => e.networkId = true,
    ];
    for (const [index, change] of invalid.entries()) {
      const t = setup(), input = event(); change(input); await invoke(t, input);
      assert(!taken(t)); assert(t.commands.some(c => c.method === 'Fetch.continueRequest')); assert.equal(t.messages.length, 0);
      saved('B3-unsupported-' + index, t, {result: 'PASS_BEFORE_TAKE'});
    }
    for (const options of [{refusePrepare: true}, {detachPrepare: true}]) {
      const t = setup(options); await invoke(t, event()); assert(!taken(t)); saved('prepare-fallback', t, {result: 'PASS_BEFORE_TAKE'});
    }
    for (const options of [{unknownCommit: true}, {loseReply: true}]) {
      const t = setup(options); await invoke(t, event()); assert.equal(t.snapshot().acquisitions[0].state, 'OUTCOME_UNKNOWN');
      assert(!t.messages.some(m => m.command === 'abort')); saved('B2-completion-unknown', t, {result: 'PASS_VM'});
    }
    const leading = setup(), leadingEvent = event('leading'); leadingEvent.responseHeaders[0].value = '00' + leadingEvent.responseHeaders[0].value;
    await invoke(leading, leadingEvent); assert.equal(leading.snapshot().acquisitions[0].state, 'HISTORICAL_ACQUISITION_OBSERVED');
    assert.equal(leading.messages.find(m => m.command === 'begin').payload.response.headers[0].value, String(Buffer.byteLength('synthetic-stream-leading')));
    saved('canonical-content-length', leading, {result: 'PASS_VM'});
    const detached = setup({detachRead: true}); await invoke(detached, event());
    assert.equal(detached.snapshot().acquisitions[0].state, 'FAILED_AFTER_TAKE');
    assert(!detached.messages.some(m => m.command === 'chunk' || m.command === 'complete')); saved('epoch-invalidation', detached, {result: 'PASS_VM'});
    const parallel = setup(); await Promise.all([invoke(parallel, event('a')), invoke(parallel, event('b'))]);
    const snap = parallel.snapshot(); assert.equal(snap.acquisitions.length, 2);
    for (const a of snap.acquisitions) {
      const begin = parallel.messages.find(m => m.command === 'begin' && m.payload.acquisitionId === a.acquisitionId);
      const chunk = parallel.messages.find(m => m.command === 'chunk' && m.payload.acquisitionId === a.acquisitionId);
      assert.equal(Buffer.from(chunk.payload.base64, 'base64').toString(), 'synthetic-' + begin.payload.binding.stream);
      assert.equal(a.state, 'HISTORICAL_ACQUISITION_OBSERVED');
    }
    saved('same-tab-concurrent-correlation', parallel, {result: 'PASS_VM'});
    for (const options of [{}, {privateFailure: true}]) {
      const p = setup(options); await invoke(p, privacy);
      const input = event('private'); input.responseHeaders.push({name: 'Set-Cookie', value: 'SYNTHETIC_RESPONSE_COOKIE_SECRET'});
      input.request.headers = {Authorization: 'SYNTHETIC_AUTH_SECRET', Cookie: 'SYNTHETIC_COOKIE_SECRET'};
      await invoke(p, input);
      p.handlers.download({id: 99, url: 'https://private.invalid/?SYNTHETIC_QUERY_SECRET', filename: 'SYNTHETIC_FILENAME_SECRET'});
      const exported = await new Promise(resolve => p.handlers.control({type: 'snapshot'}, {id: 'test-extension', url: 'chrome-extension://test-extension/control.html'}, resolve));
      const text = JSON.stringify([p.snapshot(), exported, p.messages]);
      assert(!/SYNTHETIC_(?:QUERY|COOKIE|AUTH|POST|RESPONSE_COOKIE|ERROR|FILENAME)_SECRET/.test(text));
      saved('B4-private-POST-and-receive-' + (options.privateFailure ? 'failure' : 'success'), p, {result: 'PASS_PRIVACY', forbiddenMarkersFound: 0});
    }
    const budget = setup(); vm.runInContext('for(let i=0;i<4;i++)active.add("busy"+i)', budget.context);
    await invoke(budget, event()); assert(!taken(budget)); saved('active-budget', budget, {result: 'PASS_BEFORE_TAKE'});
    vm.runInContext('for(let i=0;i<10000;i++)record("bounded")', budget.context);
    assert.equal(budget.snapshot().events.length, 2048);
  }
  const output = {sourceSha256: crypto.createHash('sha256').update(fs.readFileSync(source)).digest('hex'), cases};
  if (process.env.TRAEVA_WORKER_RESULTS) fs.writeFileSync(process.env.TRAEVA_WORKER_RESULTS, JSON.stringify(output, null, 2));
  console.log(JSON.stringify({cases: cases.length, baseline, result: baseline ? 'ORIGINAL_FAIL_REPRODUCED' : 'PASS_VM'}));
})().catch(e => {console.error(e); process.exitCode = 1;});
