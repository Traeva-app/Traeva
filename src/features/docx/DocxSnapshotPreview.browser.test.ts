import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

// Opt-in: the ordinary offline suite needs no browser or extra dependency.
// Uses the installed Playwright library, an isolated headless profile and synthetic text only.
test("DOCX preview component with an injected local API", { skip: process.env.TRAEVA_DOCX_BROWSER_TEST !== "1" }, async (t) => {
  const { createServer } = await import("vite");
  const { chromium } = await import(process.env.TRAEVA_DOCX_PLAYWRIGHT_PATH
    ? pathToFileURL(process.env.TRAEVA_DOCX_PLAYWRIGHT_PATH).href : "playwright");
  const root = fileURLToPath(new URL("../../..", import.meta.url));
  const temp = await mkdtemp(path.join(tmpdir(), "traeva-docx-component-"));
  const artifacts = process.env.TRAEVA_DOCX_TEST_ARTIFACTS;
  if (artifacts) await mkdir(artifacts, { recursive: true });
  const entry = `
    import React from 'react';
    import { createRoot } from 'react-dom/client';
    import { DocxSnapshotPreview } from '/src/features/docx/DocxSnapshotPreview.tsx';
    import '/src/styles.css';
    const s1 = {snapshot_id:'synthetic-s1',file_id:'synthetic-f1',root_id:'synthetic-r1',sequence:2,file_revision:3,
      relative_path:'Drafts/<img src="https://invalid.example/path">.docx',sha256:'a'.repeat(64),byte_length:1024,captured_at_ms:1728000000000};
    const s2 = {...s1,snapshot_id:'synthetic-s2',sequence:3,sha256:'b'.repeat(64)};
    const calls = [];
    const makeApi = () => (id,signal) => new Promise((resolve,reject) => calls.push({id,signal,resolve,reject}));
    const apis = [makeApi(),makeApi()];
    let apiIndex = 0, snapshot = null, strict = false, root = createRoot(document.getElementById('root'));
    const render = () => {
      const preview = <DocxSnapshotPreview snapshot={snapshot} previewApi={apis[apiIndex]} />;
      root.render(strict ? <React.StrictMode>{preview}</React.StrictMode> : preview);
    };
    window.rig = {
      select: (id) => {snapshot = id === null ? null : id === 1 ? s1 : s2; render();},
      replaceSameSnapshot: () => {snapshot = {...snapshot}; render();},
      setApi: (id) => {apiIndex=id; render();},
      strict: () => {strict=true; render();},
      calls: () => calls.map(c => ({id:c.id,aborted:c.signal.aborted})),
      resolve: (index, status='complete', text='First paragraph\\n\\n中文 🧪', overrides={}) => {
        const noText = status === 'failed' || status === 'unsupported';
        calls[index].resolve({snapshot:calls[index].id === s1.snapshot_id ? s1 : s2,
          extraction:{text:noText?'':text,summary:{reader:'docx-cache-local-1',status,chars:noText?0:[...text].length,
            covered:noText?[]:['body paragraphs'],left_out:status==='partial'?['field cache freshness (unknown; no evaluation or refresh)']:[],
            note:noText?'Synthetic Reader refusal':null}},...overrides});
      },
      reject: (index) => calls[index].reject(new Error('PRIVATE_DOCUMENT_CONTENT file:///private/sensitive')),
      unmount: () => root.unmount(),
    };
    render();
  `;
  const server = await createServer({ root, configFile: false, cacheDir: path.join(temp, "vite"),
    server: { host: "127.0.0.1", port: 0, strictPort: true, hmr: false, watch: null },
    plugins: [{ name: "docx-component-test-only", resolveId: (id) => id === "/__docx_component.tsx" ? id : null,
      load: (id) => id === "/__docx_component.tsx" ? entry : null,
      configureServer(s) { s.middlewares.use((req, res, next) => {
        if (req.url !== "/__docx_component") return next();
        res.setHeader("Content-Type", "text/html");
        res.end('<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><title>Synthetic DOCX component test</title><main class="page"><div id="root"></div></main><script type="module" src="/__docx_component.tsx"></script>');
      }); },
    }],
  });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true, chromiumSandbox: true,
      ...(process.env.TRAEVA_DOCX_CHROME_PATH ? { executablePath: process.env.TRAEVA_DOCX_CHROME_PATH } : {}),
      ignoreDefaultArgs: ["--unsafely-disable-devtools-self-xss-warnings", "--enable-unsafe-swiftshader"],
    });
    const address = server.httpServer!.address();
    assert.ok(address && typeof address !== "string");
    const origin = `http://127.0.0.1:${address.port}`;
    const context = await browser.newContext({ serviceWorkers: "block" });
    const external: string[] = [], errors: string[] = [];
    let dialogs = 0, popups = 0;
    await context.route("**/*", (route) => {
      if (new URL(route.request().url()).origin === origin) void route.continue();
      else { external.push(route.request().url()); void route.abort(); }
    });
    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (msg) => { if (msg.type() === "error") errors.push(msg.text()); });
    page.on("dialog", (dialog) => { dialogs += 1; void dialog.dismiss(); });
    page.on("popup", (popup) => { popups += 1; void popup.close(); });
    await page.goto(`${origin}/__docx_component`);
    const rig = (code: string) => page.evaluate(code);
    const called = (count: number) => page.waitForFunction((n) => window.rig.calls().length === n, count);
    const body = page.getByRole("region", { name: "Extracted body text" });

    await t.test("no selection, loading, complete and immutable Snapshot information", async () => {
      await page.getByText("Select a saved version to preview its text.").waitFor();
      assert.equal(await rig("window.rig.calls().length"), 0);
      await rig("window.rig.select(1)"); await called(1);
      await page.getByText("Loading saved text…").waitFor();
      assert.equal(await body.count(), 0);
      assert.equal(await rig("window.rig.calls()[0].id"), "synthetic-s1");
      await rig("window.rig.resolve(0)"); await body.waitFor();
      assert.equal(await body.textContent(), "First paragraph\n\n中文 🧪");
      await page.getByText("Snapshot information", { exact: true }).click();
      for (const text of ["synthetic-s1", "synthetic-f1", "synthetic-r1", "1024 bytes", "2024-10-04T00:00:00.000Z", "a".repeat(64)]) {
        await page.getByText(text, { exact: true }).waitFor();
      }
      await rig("window.rig.replaceSameSnapshot()");
      await rig("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
      assert.equal(await rig("window.rig.calls().length"), 1, "equivalent parent props do not re-read");
    });

    await t.test("changing version clears old body, aborts its request and ignores late success", async () => {
      await rig("window.rig.select(2)"); await called(2);
      assert.equal(await body.count(), 0);
      assert.equal(await rig("window.rig.calls()[0].aborted"), true);
      await rig("window.rig.select(1)"); await called(3);
      await rig("window.rig.resolve(1, 'complete', 'STALE_VERSION_TWO')");
      assert.equal(await body.count(), 0);
      await rig("window.rig.resolve(2, 'partial', 'Current saved citation (Author, 2020)')"); await body.waitFor();
      await page.getByText("Partial", { exact: true }).waitFor();
      await page.getByText("Not covered", { exact: true }).waitFor();
      assert.match(await page.getByRole("note").textContent(), /may be missing or out of date/);
      assert.match(await page.getByRole("note").textContent(), /cannot always be identified/);
      assert.doesNotMatch(await page.locator("body").textContent(), /STALE_VERSION_TWO/);
    });

    await t.test("adapter failures and misbound results fail closed; retry really reloads locally", async () => {
      await rig("window.rig.select(2)"); await called(4);
      await rig("window.rig.reject(3)"); await page.getByRole("alert").waitFor();
      assert.doesNotMatch(await page.locator("body").textContent(), /PRIVATE_DOCUMENT_CONTENT|private\/sensitive/);
      await page.getByRole("button", { name: "Retry local preview" }).focus();
      await page.keyboard.press("Enter"); await called(5);
      assert.equal(await body.count(), 0);
      await rig("window.rig.resolve(4, 'complete', 'WRONG_SNAPSHOT', {snapshot:{snapshot_id:'different'}})");
      await page.getByRole("alert").waitFor(); assert.equal(await body.count(), 0);
      await page.getByRole("button", { name: "Retry local preview" }).click(); await called(6);
      await rig("window.rig.resolve(5)"); await body.waitFor();
    });

    await t.test("changing the injected API clears prior body; late rejection after deselection is ignored", async () => {
      await rig("window.rig.setApi(1)"); await called(7);
      assert.equal(await body.count(), 0);
      await rig("window.rig.select(null)");
      await page.getByText("Select a saved version to preview its text.").waitFor();
      await rig("window.rig.reject(6)");
      assert.equal(await page.getByRole("alert").count(), 0);
      assert.equal(await rig("window.rig.calls()[6].aborted"), true);
    });

    await t.test("Unsupported, Failed and empty text remain distinct", async () => {
      for (const [offset, status] of ["unsupported", "failed", "complete"].entries()) {
        const index = 7 + offset;
        await rig(`window.rig.select(${offset % 2 + 1})`); await called(index + 1);
        await rig(`window.rig.resolve(${index}, '${status}', '')`);
        await page.getByText(status[0].toUpperCase() + status.slice(1), { exact: true }).waitFor();
        assert.equal(await body.count(), 0);
        if (status === "complete") await page.getByText("No body text was extracted.").waitFor();
        else await page.getByText("Synthetic Reader refusal", { exact: true }).waitFor();
      }
    });

    await t.test("malformed failed response never renders its body", async () => {
      await rig("window.rig.select(2)"); await called(11);
      await rig("window.rig.resolve(10, 'failed', '', {extraction:{text:'MUST_NOT_RENDER',summary:{reader:'x',status:'failed',chars:15,covered:[],left_out:[],note:'bad'}}})");
      await page.getByRole("alert").waitFor(); assert.equal(await body.count(), 0);
      assert.doesNotMatch(await page.locator("body").textContent(), /MUST_NOT_RENDER/);
    });

    await t.test("markup in body, coverage, note and path is inert ordinary text", async () => {
      await page.getByRole("button", { name: "Retry local preview" }).click(); await called(12);
      const attack = '<img src="https://invalid.example/image" onerror="alert(1)"><script>window.pwned=1</script><a href="file:///etc/passwd">open</a><iframe src="https://invalid.example/frame"></iframe>\n[link](https://invalid.example/md)';
      await page.evaluate(({ attack }) => window.rig.resolve(11, "partial", attack, {
        extraction: { text: attack, summary: { reader: "docx-cache-local-1", status: "partial", chars: [...attack].length,
          covered: [attack], left_out: [attack], note: attack } },
      }), { attack });
      await body.waitFor(); assert.equal(await body.textContent(), attack);
      assert.equal(await page.locator("#root img, #root script, #root a, #root iframe, #root object").count(), 0);
      assert.equal(await rig("window.pwned"), undefined);
      assert.equal(dialogs, 0); assert.equal(popups, 0); assert.deepEqual(external, []);
    });

    await t.test("keyboard, narrow layout and both existing themes", async () => {
      await page.getByText("Snapshot information", { exact: true }).focus();
      await page.keyboard.press("Enter");
      assert.equal(await page.locator("details").getAttribute("open"), "");
      await page.keyboard.press("Tab"); assert.equal(await body.evaluate((el) => el === document.activeElement), true);
      assert.notEqual(await body.evaluate((el) => getComputedStyle(el).outlineStyle), "none");
      for (const colorScheme of ["light", "dark"] as const) for (const width of [320, 960]) {
        await page.emulateMedia({ colorScheme }); await page.setViewportSize({ width, height: 800 });
        assert.equal(await rig("document.documentElement.scrollWidth <= innerWidth"), true, `${colorScheme} ${width}`);
        const styles = await page.locator(".docx-preview").evaluate((el) => {
          const root = getComputedStyle(el);
          const luminance = (color) => {
            const channels = color.match(/\d+/g).slice(0,3).map(Number).map(c => c/255).map(c => c <= .04045 ? c/12.92 : ((c+.055)/1.055)**2.4);
            return channels[0]*.2126+channels[1]*.7152+channels[2]*.0722;
          };
          const background = luminance(root.backgroundColor);
          return [...el.querySelectorAll('.docx-preview-context, .docx-preview-warning, dt'),el].map(node => {
            const foreground = luminance(getComputedStyle(node).color);
            return (Math.max(foreground,background)+.05)/(Math.min(foreground,background)+.05);
          });
        });
        assert.ok(styles.every(ratio => ratio >= 4.5), `${colorScheme} contrast ${styles}`);
        if (artifacts) await page.screenshot({ path: path.join(artifacts, `${colorScheme}-${width}.png`), fullPage: true });
      }
    });

    await t.test("maximum Reader output and unmount cancellation", async () => {
      await rig("window.rig.select(1)"); await called(13);
      const started = Date.now();
      await rig("window.rig.resolve(12, 'complete', '🧪'.repeat(1000000))"); await body.waitFor();
      assert.equal(await body.evaluate(el => [...el.textContent!].length), 1_000_000);
      t.diagnostic(`1,000,000 Unicode scalars delivered and rendered in ${Date.now()-started} ms (one local run, not a performance guarantee)`);
      await rig("window.rig.select(2)"); await called(14);
      await rig("window.rig.unmount()");
      assert.equal(await rig("window.rig.calls()[13].aborted"), true);
      await rig("window.rig.resolve(13)"); assert.equal(await body.count(), 0);
      assert.deepEqual(errors, []); assert.deepEqual(external, []);
    });
    await t.test("returning to an earlier API cannot revive its old result while loading", async () => {
      await page.reload();
      await page.getByText("Select a saved version to preview its text.").waitFor();
      await rig("window.rig.select(1)"); await called(1);
      await rig("window.rig.resolve(0, 'complete', 'OLD_API_RESULT')"); await body.waitFor();
      await rig("window.rig.setApi(1)"); await called(2);
      assert.equal(await body.count(), 0);
      await rig("window.rig.setApi(0)"); await called(3);
      assert.equal(await body.count(), 0, "old API result must stay cleared until a fresh response");
      await rig("window.rig.resolve(1, 'complete', 'LATE_OTHER_API')");
      await rig("window.rig.resolve(2, 'complete', 'FRESH_API_RESULT')"); await body.waitFor();
      assert.equal(await body.textContent(), "FRESH_API_RESULT");
      assert.deepEqual(errors, []); assert.deepEqual(external, []);
    });
    await t.test("the app's React StrictMode rejects aborted mount results", async () => {
      await rig("window.rig.strict()"); await called(5);
      assert.equal(await rig("window.rig.calls()[3].aborted"), true);
      await rig("window.rig.resolve(3, 'complete', 'ABORTED_STRICT_MOUNT')");
      assert.equal(await body.count(), 0);
      await rig("window.rig.resolve(4, 'complete', 'CURRENT_STRICT_MOUNT')"); await body.waitFor();
      assert.equal(await body.textContent(), "CURRENT_STRICT_MOUNT");
      await rig("window.rig.select(2)"); await called(7);
      await rig("window.rig.unmount()");
      assert.equal(await rig("window.rig.calls()[6].aborted"), true);
      await rig("window.rig.resolve(6)");
      assert.equal(await body.count(), 0);
      assert.deepEqual(errors, []); assert.deepEqual(external, []);
    });
    t.diagnostic(`Browser ${await browser.version()}; isolated loopback harness; no Tauri or Word launched`);
    await context.close();
  } finally {
    if (browser) await browser.close();
    await server.close();
    await rm(temp, { recursive: true, force: true });
  }
});
