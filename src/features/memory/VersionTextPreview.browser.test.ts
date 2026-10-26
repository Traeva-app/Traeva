import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

// Opt-in, like the DOCX browser test (same Playwright and Chrome settings):
// TRAEVA_PREVIEW_BROWSER_TEST=1, TRAEVA_DOCX_PLAYWRIGHT_PATH, optionally
// TRAEVA_DOCX_CHROME_PATH. Synthetic text only; the preview command is a
// promise the test settles, so nothing reaches Tauri or a model.
//
// R1-M28-O3: the page must never show one version's text under another
// version's choice, in any DOM commit (not only after effects settle).
test("a deck's or workbook's preview shows only the chosen version", { skip: process.env.TRAEVA_PREVIEW_BROWSER_TEST !== "1" }, async (t) => {
  const { createServer } = await import("vite");
  const { chromium } = await import(process.env.TRAEVA_DOCX_PLAYWRIGHT_PATH
    ? pathToFileURL(process.env.TRAEVA_DOCX_PLAYWRIGHT_PATH).href : "playwright");
  const root = fileURLToPath(new URL("../../..", import.meta.url));
  const temp = await mkdtemp(path.join(tmpdir(), "traeva-preview-component-"));
  const entry = `
    import React, { Profiler, useLayoutEffect, useState } from 'react';
    import { createRoot } from 'react-dom/client';
    import { VersionTextPreview } from '/src/features/memory/VersionTextPreview.tsx';
    const kind = new URLSearchParams(location.search).get('kind');
    const base = {file_id:'synthetic-f1',root_id:'synthetic-r1',file_revision:1,byte_length:2048,captured_at_ms:1728000000000};
    const name = kind === 'pptx' ? 'talk.pptx' : 'readings.xlsx';
    const versions = [
      {...base,snapshot_id:kind+'-1',sequence:1,relative_path:name,sha256:'a'.repeat(64)},
      {...base,snapshot_id:kind+'-2',sequence:2,relative_path:name,sha256:'b'.repeat(64)},
    ];
    const texts = kind === 'pptx'
      ? ['⟦slide 1 · id 256⟧\\n⟦text⟧\\nSurvey launch is August.\\n', '⟦slide 1 · id 256⟧\\n⟦text⟧\\nSurvey launch is September.\\n']
      : ['⟦sheet 1 · id 1 · name "Data"⟧\\n⟦cell A2 · literal · text⟧\\nAlpha reading\\n', '⟦sheet 1 · id 1 · name "Data"⟧\\n⟦cell A2 · literal · text⟧\\nBeta reading\\n'];
    const calls = [], commits = [];
    const record = () => { commits.push(document.getElementById('view').innerText); };
    let choose = null;
    function Page() {
      const [index, setIndex] = useState(null);
      choose = setIndex;
      useLayoutEffect(record);
      return <>
        <button onClick={() => setIndex(0)}>Version one</button>
        <button onClick={() => setIndex(1)}>Version two</button>
        <div id="view">{index !== null && <>
          <h2>Selected version {index + 1}</h2>
          <Profiler id="preview" onRender={record}>
            <VersionTextPreview snapshot={versions[index]} note="Synthetic note" />
          </Profiler>
        </>}</div>
      </>;
    }
    window.rig = {
      request: (id) => new Promise((resolve, reject) => calls.push({id, resolve, reject})),
      calls: () => calls.map((call) => call.id),
      commits: () => commits,
      resolve: (n, wrong = false) => {
        const index = versions.findIndex((v) => v.snapshot_id === calls[n].id);
        const text = texts[index];
        calls[n].resolve({snapshot: wrong ? {...versions[index], sha256: '0'.repeat(64)} : versions[index],
          extraction: {text, summary: {reader: kind + '-preview-1', status: 'complete', chars: [...text].length,
            covered: ['Synthetic text'], left_out: [], note: null}}});
      },
    };
    createRoot(document.getElementById('root')).render(<Page />);
  `;
  const server = await createServer({ root, configFile: false, cacheDir: path.join(temp, "vite"),
    server: { host: "127.0.0.1", port: 0, strictPort: true, hmr: false, watch: null },
    plugins: [{ name: "preview-component-test-only", enforce: "pre",
      resolveId: (id) => id === "/__preview_component.tsx" ? id : null,
      load: (id) => id === "/__preview_component.tsx" ? entry
        // The preview command, settled by the test instead of Tauri.
        : id.endsWith("/src/features/preview/api.ts") ? "export const previewVersionText = (id) => window.rig.request(id);" : null,
      configureServer(s) { s.middlewares.use((req, res, next) => {
        if (!req.url?.startsWith("/__preview_component?")) return next();
        res.setHeader("Content-Type", "text/html");
        res.end('<!doctype html><meta charset="utf-8"><title>Synthetic preview component test</title><div id="root"></div><script type="module" src="/__preview_component.tsx"></script>');
      }); },
    }],
  });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true, chromiumSandbox: true,
      ...(process.env.TRAEVA_DOCX_CHROME_PATH ? { executablePath: process.env.TRAEVA_DOCX_CHROME_PATH } : {}),
    });
    const address = server.httpServer!.address();
    assert.ok(address && typeof address !== "string");
    const origin = `http://127.0.0.1:${address.port}`;
    const context = await browser.newContext({ serviceWorkers: "block" });
    const external: string[] = [];
    await context.route("**/*", (route) => {
      if (new URL(route.request().url()).origin === origin) void route.continue();
      else { external.push(route.request().url()); void route.abort(); }
    });

    for (const [kind, first, second] of [
      ["pptx", "Survey launch is August.", "Survey launch is September."],
      ["xlsx", "Alpha reading", "Beta reading"],
    ] as const) {
      await t.test(kind, async () => {
        const page = await context.newPage();
        const errors: string[] = [];
        page.on("pageerror", (error) => errors.push(error.message));
        await page.goto(`${origin}/__preview_component?kind=${kind}`);
        const rig = (code: string) => page.evaluate(code);
        const called = (count: number) => page.waitForFunction((n) => window.rig.calls().length === n, count);
        const view = page.locator("#view");
        const one = page.getByRole("button", { name: "Version one" });
        const two = page.getByRole("button", { name: "Version two" });

        await one.click(); await called(1);
        await rig("window.rig.resolve(0)"); await page.getByText(first).waitFor();
        // Another version chosen while its text is still being read.
        await two.click(); await called(2);
        assert.doesNotMatch(await view.innerText(), new RegExp(first));
        await page.getByText("Reading the saved version…").waitFor();
        await rig("window.rig.resolve(1)"); await page.getByText(second).waitFor();
        // A late result for the version left is dropped.
        await one.click(); await called(3);
        await two.click(); await called(4);
        await rig("window.rig.resolve(2)");
        assert.doesNotMatch(await view.innerText(), new RegExp(first));
        await rig("window.rig.resolve(3)"); await page.getByText(second).waitFor();
        // A result bound to other bytes shows no text.
        await one.click(); await called(5);
        await rig("window.rig.resolve(4, true)");
        await page.getByText("This saved version could not be read. No text is shown.").waitFor();
        assert.doesNotMatch(await view.innerText(), new RegExp(`${first}|${second}`));

        // Every commit: version 2 never with version 1's text, nor 1 with 2's.
        const commits = await rig("window.rig.commits()") as string[];
        assert.ok(commits.length > 10, `${commits.length} commits recorded`);
        const wrong = commits.filter((text) =>
          (text.includes("Selected version 2") && text.includes(first)) ||
          (text.includes("Selected version 1") && text.includes(second)));
        assert.deepEqual(wrong, []);
        assert.deepEqual(errors, []);
        await page.close();
      });
    }
    assert.deepEqual(external, []);
  } finally {
    await browser?.close();
    await server.close();
    await rm(temp, { recursive: true, force: true });
  }
});
