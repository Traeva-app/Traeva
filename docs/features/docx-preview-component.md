# Historical DOCX local text preview component (#66)

Implementation commit: `7558f3b01ba6ad317383a9251cab66329a42dda0`; added StrictMode test commit: `3c3ac44134174e8b6d8edc33077f5a182d847102`. Starting main: `65df8c9b80f6624e486a16d22b0905a2315ef373`; the student has merged #128. This component is a new independent review scope and does not inherit #128's R1 LIMITED PASS.

## Runnable scope

`src/features/docx/DocxSnapshotPreview.tsx` takes the selected existing `SnapshotRef | null` and a required `previewApi`. The standalone browser tests run without Tauri. It shows the body text as is, Complete / Partial / Unsupported / Failed, the Reader coverage and notes, a warning that the citation cache may be stale, and the full Snapshot information, collapsed. The file sequence is clearly labeled File sequence and does not pose as the version number of the work's record line.

The body text keeps line breaks, blank lines and Unicode; all body text, paths, Reader notes and coverage entries are React text children. There is no HTML / Markdown parsing, auto-linking, document images, Word launch, opening of the original file or sending to the model. The scrolling body can be focused with the keyboard. It offers only the Snapshot information collapse, which really works, and a local retry after a load failure.

All Snapshot fields take part in request binding. Switching versions unmounts the old preview immediately; an API change adds a generation, so even switching back to the earlier API does not restore the old body text. Clearing the selection, switching, retrying and unmounting abort the original request; even if the underlying layer ignores the signal, a late result does not enter the UI. Re-rendering with equal Snapshot props does not read again. A general load error does not echo an arbitrary rejection, to avoid putting body text or paths into the error message.

The component rejects responses with a version mismatch, an unknown status, a wrong character count, invalid Unicode, Complete with omissions, Partial that does not disclose omissions, and Failed / Unsupported with body text. The body text limit follows the Reader's 1,000,000 Unicode scalar values; over the limit, the whole response is rejected, not silently truncated. Each UI note is at most 16,384 UTF-16 units, each coverage list at most 256 items, and the reader name at most 256 units; these are defensive bounds of the standalone component, not a new shared DTO. A normal Failed / Unsupported response has no body text, an empty coverage and a note.

The cache warning does not depend on the optional note: even for Complete or without a note, it shows that the cache may be missing/stale, that no update was run, and that unmarked flattened data cannot always be recognized or excluded. Complete only carries over the Reader's extraction status; it does not mean full understanding of the Word document or that the citation database is up to date. R126-02 is kept.

## A / B wiring

The component's local input is in `preview.ts`; it is not an IPC contract, and there is no default Tauri API:

```ts
type DocxPreviewApi = (snapshotId: string, signal: AbortSignal) => Promise<DocxPreview>;
// DocxPreview = { snapshot: existing SnapshotRef, extraction: { summary, text } }
```

A maintains the final shared DTO, the Core public export, background execution, and the registration and permissions of the local-only bounded command, and adapts the result of #128's `preview_docx_snapshot(service, id)` into the local input. It must keep reading only Snapshots that passed the hash/length check; a storage failure does not fall back to source, and `TextExtraction` is not used directly as a serializable shared DTO. The signal is a UI convention for dropping late results; it does not claim to stop a Rust Reader that is already running.

B passes in the full Snapshot from the real history record, provides a stable local API function reference, and coordinates the page entry, closing and selection. The adapter must not call analysis/model sending, open Word, copy documents or access external resources. The caller's implementation decides how safe the wiring is; this PR's mock tests cannot prove that a future adapter is safe. Estimated: A still needs 0.5–1 person-day, B's entry and adapter about 0.25–0.5 person-day, R1's integration re-verification about 0.5–1 person-day; these are estimates, not promises or completed work.

This PR did not change the shared schema, DTO mirrors, migrations, manifest/lockfile, registration/permissions, top-level UI or the Memory page. #103/#124, the default Reader, the cloud chain and the formal open allowlist are unchanged. Formally opening historical DOCX in Word is still No-Go; formal Word Memory is not enabled; #26/#37 are not affected.

## Test reproduction and evidence

The ordinary offline frontend tests automatically run 6 validation tests. The real browser tests must be turned on explicitly and are marked skip by default; an ordinary `npm test` must not be taken as browser evidence that has been run. It uses the project's existing Vite and React, with no dependency changes. Playwright is injected from an installed library location; Chrome uses an isolated temporary profile, headless, with the Chromium sandbox on; it does not use a personal profile, does not open windows and does not take the #65 full desktop UI slot.

```sh
node --test src/features/docx/preview.test.ts
npm test
npm run build
TRAEVA_DOCX_BROWSER_TEST=1 \
TRAEVA_DOCX_PLAYWRIGHT_PATH=/absolute/path/to/playwright/index.mjs \
TRAEVA_DOCX_CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
TRAEVA_DOCX_TEST_ARTIFACTS=/absolute/path/to/isolated/evidence/screens \
node --test src/features/docx/DocxSnapshotPreview.browser.test.ts
cargo fmt --all --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo test --offline --locked -p traeva-core
```

The browser harness serves only the loopback address, and routing blocks other origins; it uses only explicitly synthetic Snapshots and text, and reads no real Word documents, user files, formal app-data or Keychain. The Core default tests use the repository's isolated fixtures as before; the real model/Keychain/GUI items stay ignored. Machine results and code and log hashes are in [docx-preview-component-validation.json](docx-preview-component-validation.json). Raw logs and screenshots stay in this lane's isolated evidence directory, for R1's independent re-verification; at the end of this round it was checked that the default Traeva directory does not exist.

Recorded failures: at first the browser launch was blocked by the execution sandbox; browser/system security protections were not changed, and it launched successfully in an allowed isolated run. In the first Core round, 82 tests all hit PermissionDenied at the loopback TestServer bind; the original log is kept. The new API A → B → A regression first reproduced a real UI defect where the old result was restored, which was then fixed by remounting with an API generation; logs from before and after are kept. The first UI FAIL, in an uncommitted development state, has no separate commit, and no re-verifiable old commit is claimed for it; the final implementation is bound to the commits above and the source hashes in the machine evidence.

| Final check | Actual result |
|---|---|
| Standalone validation | 6 PASS, covering every Snapshot field, status and resource bound. |
| Actual browser component | 11 scenarios pass; Node counts 12 PASS (including the parent test), 0 FAIL, 0 skip; Chrome 155.0.8059.39. |
| Ordinary frontend suite | 114 PASS, 0 FAIL, 1 skip (the explicit browser test above); 115 in total. |
| Core | 791 PASS, 0 FAIL, 10 ignored; uses local mock services, no real model requests. |
| build / fmt / Clippy | All exit 0; Clippy is workspace / all targets / deny warnings. |
| Text limit | 1,000,000 scalars delivered and rendered in 324 ms; only the last local observation, not a performance guarantee. |

The 11 browser scenarios: empty selection/loading/Complete/all Snapshot fields; version switch and late success; load failure, mismatched result and keyboard/mouse retry; late failure after an API switch and after clearing the selection; Unsupported/Failed/empty body; malformed Failed does not leak body text; attack markup in body/note/coverage/path stays inert; keyboard focus, narrow screen, light and dark themes and actual text contrast; 1,000,000-scalar body and cancel on unmount; API A → B → A does not restore the old body; React StrictMode double mount and cancel on unmount, as in the formal App. The tests do not depend on Tauri commands that are not yet registered.

## Development self-review and untested scope

The developer checked the actual dependencies, failure states, request lifecycle and diff: apart from the type-only import of the existing Snapshot, the component has no cross-feature runtime API dependency; unknown rejections are not serialized; safe text covers both the body and the note fields, rather than checking only one output; no source/preview copy is created. The new regression really failed first and was then fixed. This is D's development self-review, not an independent review.

The local component is a conditional Go, ready to hand to A/B for wiring and to R1 for review. Not tested: real Tauri IPC, the production Memory page, the full chain from a real immutable Snapshot to the component, the packaged desktop UI, the Word save GUI, or Word → Semantic Change → Retrieval. #128's Core/real Snapshot evidence belongs to its reviewed commit; this round's mock does not replace it. R1 acceptance of the new UI is still pending; the final decision on 10/13 is still made by the student. The formal Word opening No-Go is kept.

## UI quality check (antislop)

Checked during implementation under the user's authorization to "decide ordinary implementation details on your own", without waiting for the skill's default mode choice. The direction is the existing Memory system font, theme variables, clear states and historical evidence; no separate product design and no change to the shared theme. Design read: a historical text check component, for Memory users, following Traeva, ENERGY 1 / RHYTHM 1 / MOTION 1. The body text is the focus, and Snapshot details are collapsed so they do not crowd reading; the warning uses the existing warn color to stress the guarantees that are not covered; the system font makes body text easy to read, and hashes use a monospace font for easy checking; whitespace separates status, limits and body text; a border marks the scroll area; there is no decorative left bar, icons, assets, card groups or animation. The existing product uses the system light/dark preference; the component reuses it and was tested in both themes; no shared theme switch was added.

| Check | Result and basis |
|---|---|
| R-02 | PASS: new product copy has no em dash; synthetic attack text is shown as is, input is not rewritten. |
| R-03 | PASS: no horizontal overflow measured at 320 / 960 px; 44 px retry button and details summary. |
| R-17 | PASS: only the passed-in Snapshot metadata and Reader chars; no promotional stats. |
| R-18 | PASS: no testimonials, people or avatars. |
| R-23 | PASS: no visual assets or navigation created; reuses the existing UI. |
| R-24 | PASS: no new navigation or external links. |
| R-25 | PASS: browser-computed light and dark text contrast for body, notes, warnings and metadata is at least 4.5:1. |
| R-26 | PASS: details expand by mouse/Enter; retry by mouse/Enter starts a new injected API request. |
| R-27 | PASS: empty selection, empty body, loading, each Reader status and API errors were all actually run. |
| R-28 | PASS: no FAQ. |
| R-32 | PASS: summary/retry keyboard operation, Tab to the body, visible focus outline tested. |
| R-33 | PASS: component and CSS maintained directly; no script was run to rewrite the UI. |
| R-34 | PASS: reuses the existing two themes; screenshots and layout/contrast checks pass. |
| R-35 | PASS: build, actual browser click and keyboard tests, no console/page errors. |
| R-36 | PASS: no unfounded claims such as full Word understanding, latest citations, system security or performance guarantees. |
| R-37 | PASS: follows the existing Memory style and the direction above; no new navigation/theme design. |
| R-38 | PASS: test data is clearly synthetic; production shows only the input content, no made-up product content. |
| R-01 / R-04 | PASS: no gradients, glow or icons. |
| R-06 | PASS: system font follows the body UI; monospace for hashes for character-by-character checking. |
| R-07 / R-08 / R-09 | PASS: no background grid, decorative arrows or promotional badges. |
| R-10 / R-12 / R-13 | PASS: no blur, shadow or glow. |
| R-14 / R-19 / R-22 | PASS: no card groups, template animation or illustrations. |
| Liveliness dials | PASS: the three dials are declared; a single body focus, structural whitespace, a limit-warning accent, the existing tone for historical information. |
| C-1 / C-2 / C-3 | PASS: visual choices have the purposes above; both actual controls tested; shows only what the preview needs. |
| C-4 / C-5 | PASS: states/keyboard/themes/narrow screen tested; no made-up evidence, limits listed separately. |
| R-05 / R-11 | PASS: the layout of historical information and body text is driven by content and reuses the 8 px border; not a marketing template or pill group. |
| R-15 / R-16 | PASS: Snapshot information / Retry local preview state their purpose clearly; no marketing words. |
| R-20 / R-21 | PASS: continues Traeva's historical check approach and the system theme; no forced dark mode or scope expansion. |
| R-29 / R-30 | PASS: neutral colors + one warn accent; no copying of external products. |
| R-31 | PASS: the reasons for font, layout, color, whitespace, border and not using assets are recorded. |

The PASS results above refer only to development checks of the standalone component and the synthetic harness; they do not represent production integration or an independent R1 PASS.
