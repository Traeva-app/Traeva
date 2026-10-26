# Source Memory — CAC implementation and integration handoff

Refs #27. Product rules are the student's [accepted v2](https://github.com/P30g53/Traeva/issues/27#issuecomment-6075074252) and [Progressive Disambiguation decision](https://github.com/P30g53/Traeva/issues/27#issuecomment-6075331399).

A Chrome download is collected automatically after explicit setup. Core registers the current authorized file and recommends it as a candidate. The pending list does not interrupt downloads. The user can confirm a download or copy relationship, reject, cancel, select a location, correct, or unlink. Only a successful database write produces a saved result. A confirmed association is **User Confirmed**, including after restart or same-object relocation.

**First automatic VERIFIED is DEFERRED.** No current Source status or command can create it. The [failed real Chrome experiment](https://github.com/P30g53/Traeva/issues/27#issuecomment-6075172687) remains relevant: observed temporary/final identity, matching hash, time, name and size did not prove the browser event's first object binding.

## Integration by Claude-A

Claude-A applied the four integration proposals to the shared files on this branch (commit `c38d054`, then `b4544f1` and the review fixes), so the patch files are gone:

1. Core: `mod source` and its exports; the migration is numbered `0005_sources.sql` (after #33's `0004_memory_changes.sql`, as agreed on #65); schema tests cover the five tables, their keys and triggers and a version-4 upgrade; the hook runs right after a successful `register_file` and never makes choosing the file fail (`source_hook_tests.rs`).
2. Tauri: the two modules, the managed receiver, the nine commands in `command_names.rs`, the handler and `capabilities/main-window.json`; `serde`/`serde_json` became direct dependencies of the app (versions already in the lockfile).
3. The app shell is `src/App.tsx` (Claude-A): Organize and Source Memory tabs.
4. `FileSourceSection` sits in Organize's current-file area.

The shared contract describes the result in §4.5 (v8).

Review revision after A's `6928276`: `source_settings.enabled_since_ms` enforces the latest collection interval after restart. Claude-A integrated it as `migrations/0006_source_capture_window.sql` (appended after 0005; no existing migration is edited) and registered it in `store.rs`. `tests/source_capture_window.rs` upgrades a version-five database: pairing/settings are kept and an unknown old enablement time stays unknown; `tests/schema.rs` covers the empty database and the version-four upgrade.

The core registration hook consumes the row from the just-completed stable observation. It compares saved association evidence with that observation, retains original event/file references and basis, and checks current authorization/revision before committing continuity history. Calling it with an arbitrary previously stored row is invalid. #31's future manual relocation path must use the same stable-registration hook when it introduces a separate registration route. It does not enable Memory.

The reception proposal and isolation were [posted to #65 before implementation](https://github.com/P30g53/Traeva/issues/65#issuecomment-6075401816). A has integrated the shared files; R independent verification remains required. This PR is not the final #27 acceptance or a merge request.

## Setup and behavior

Use Traeva's existing folder authorization. In Source, enable collection and reveal the local connection settings. Load `extensions/chrome` as an unpacked extension in the intended Chrome profile, open its settings, enter that endpoint and pairing token, and explicitly enable collection there. See the extension README for the exact setup and opt-in recovery controls.

The receiver is off by default. Once enabled it binds `127.0.0.1` on an OS-assigned port and persists that port and a random token in local app-data. Restart reuses them; an occupied saved port fails closed. Requests require the token, the exact loopback Host, bounded framing/body and a read deadline. It has no wildcard CORS or arbitrary-path read endpoint. The extension disables redirects, cookies and referrers on transport requests. This is a local bearer-token transport, not a TLS service or protection against another process already controlling the same user's local data.

Only active authorized paths are accepted; the extension prefilter is not the security boundary. Core proves the root and walks the directory chain with handles without following symlinks. Files that fail registration retain their scoped source metadata and the real failure reason. Collection never writes user files, directory metadata or xattrs. Incognito and out-of-scope events are ignored. Source URLs are not sent to AI or cloud services, or emitted in production diagnostics/logs.

Fields remain independently unknown. Initial/final URL, browser referrer, started/completed time and first import time are distinct. Chrome supplies no trustworthy page title. HTTP(S) links are opened only through a user action and the dedicated command; a download-link action warns that it may download again. Referrer is never relabeled as an independently established source page.

The durable event key is profile installation UUID plus browser download ID. Repeat delivery updates the same event without resetting first import time. Conflicting known start time or original URL is rejected, retaining the original evidence, rather than silently merging a reused/colliding ID. The extension keeps pending IDs, not URL payloads, and re-queries Chrome on retry. Erased Chrome history cannot be reconstructed. History recovery is separately explicit and bounded; queue overflow/rejected records are visible in settings.

The database has Source settings, event records, candidate evidence, associations and append-only decision/continuity history. The schema is migration `crates/traeva-core/migrations/0005_sources.sql` (numbered at integration, after #33's 0004). Existing file records remain core-owned. Matching bytes do not link two downloads; a replacement or copy does not inherit an existing association. Historical relations remain visible when the current location cannot be confirmed.

## Verification

The original `fc550f1` validation used a disposable integration copy: 467 core tests (6 existing opt-in tests skipped), 16 Tauri tests, 25 frontend tests and 26 extension tests. Its explicit real Chrome test passed separately; that report remains in `source-validation.json`.

The review revision `ec81f8c`, based on A's `6928276` and tested with the then-pending capture-window migration proposal, passed 558 workspace Rust tests (7 opt-in tests skipped), 58 frontend/extension tests (29 extension tests), formatting, workspace Clippy and frontend build. The macOS delayed-header/body test fails when the receiver fix is removed. `source-review-validation.json` preserves that historical conditional run and its module/proposal hashes; it is not the current integration result.

A packaged debug app also passed real WebView and native chooser checks: authorize the synthetic folder, enable capture, receive a candidate without a popup, cancel locating/confirming, select a file, confirm as User Confirmed, open a safe local referrer page in the browser, exit and restart with the relation and capture interval preserved. The sender was synthetic HTTP, **not a Chrome download**; it does not prove the real Chrome-to-desktop end-to-end acceptance. The isolated evidence is under `/private/tmp/traeva-c-source-desktop-gp1zqcj0/evidence`; the CUA transcript contains the desktop screenshots. The test app, owned Chrome Profile and server exited, and the UI slot was returned.

After A integrated migration 0006, the actual PR branch at `1a40f874ac3d784f87f2c334025e8ba7a5161c6d` passed formatting, workspace Clippy, 558 Rust tests (0 failed, 7 opt-in skipped), all 58 frontend/extension tests and frontend build. A debug `.app` built from this checkout passed **real Chrome → actual extension → production desktop receiver/core → non-interrupting candidate → WebView user confirmation → process exit/restart**. This run did not use mock IPC. The native folder chooser authorized the synthetic root; the native file chooser subsequently opened the downloaded file and its embedded Source section displayed the same User Confirmed association. Canceling confirmation created no association/history. Initial/final redirect URLs remained distinct, title and independent source page remained unknown, and the actual referrer button opened the recorded safe localhost URL.

Duplicate delivery after successful desktop restart retained one record, one association, the original import time, enablement time and decision history. The downloaded file's bytes, object, times and existing Chrome/macOS xattrs were unchanged from the post-download baseline through confirmation, restart, replay and file-details display; no extra root files appeared. `source-integrated-validation.json` records the exact code, hashes, identifiers and evidence. First restart was correctly blocked because the reused adapter placed test reports inside app-data; only those test-owned artifacts were moved to the separate evidence directory before restarting successfully. The database was preserved. The test `.app`, isolated Chrome Profile and local server were closed; only the browser tab opened by the referrer test was closed in the default browser. These are C developer results, not R independent acceptance.

Original base: `4be3fd5` (contract v6); latest tested integration: `1a40f87` (contract v8, schema 6). Environment: macOS arm64/APFS, Rust 1.98.1, Node 24.20.0, real Chrome 155.0.8059.39, Playwright 1.62.1. Download capture used an isolated Profile, synthetic roots and app-data. Opening the synthetic localhost referrer used the system default browser; it did not change its settings or inspect personal page content. No production app-data, model or Keychain operation was used.

Run the following on the integrated branch:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test -p traeva-core --offline
cargo test -p traeva-app --offline
npm test
npm run build
node --test extensions/chrome/*.test.js
```

The optional real Chrome check is `TRAEVA_SOURCE_CHROME=1 cargo test -p traeva-app --test source_chrome -- --ignored --nocapture`. It uses a subprocess so restart means the old process has exited and released its lock. Set `TRAEVA_TEST_PLAYWRIGHT` to the absolute installed Playwright module path; `TRAEVA_TEST_CHROME` optionally overrides the Chrome binary. This test does not launch a full desktop window or use a personal Profile.

Coverage includes replacement between recommendation and confirmation, repeated/colliding records, user rejection/cancel, same-object core re-registration, committed Organize move, explicit copy relation, correction/unlink history, authorization and unsupported-file reasons, disabled-capture manual checks, more than 500 records, transactional failures, and an exact user-tree snapshot including bytes, names, times and xattrs. Migration checks cover an empty database and upgrades retaining prior data. HTTP tests cover authentication, framing, size/deadline limits and receiver restart/collision; IPC checks cover main-window permissions.

Actual Chrome integration demonstrates redirect download → real extension → production receiver/core → candidate → mock Tauri IPC user confirmation → capture-process exit → service reopen → duplicate delivery, preserving one source record, import time and User Confirmed. This is not a WebView click-through test. The redacted result is in `source-validation.json`.

Separate browser checks exercise extension settings, complete/progress delivery, duplicate downloads, out-of-root/incognito exclusion, natural alarm retry after offline recovery, explicit history, disabling and narrow screens. Separate UI checks exercise confirmation, rejection, cancellation, copy relation, corrections/unlink, failed writes/reads, link wording, keyboard focus and narrow layouts, using synthetic API data. The final 8-case browser run also checked exact file filtering, cross-file correction, clearing a stale confirmation when the file changes, unrelated-file UNKNOWN and Organize/Source navigation.

Remaining: R's independent verification on the final PR commit and student acceptance/merge. Development-mode extension setup is the tested CAC delivery; distribution outside development mode is untested. A's original four integrations and capture-window migration are present. Cloud placeholders/cross-volume behavior inherit core refusal but were not newly exercised by the real Chrome harness. A's optional follow-up suggestions remain disclosed in the PR; they are not claimed fixed. No result here upgrades the original automatic VERIFIED FAIL to PASS.
