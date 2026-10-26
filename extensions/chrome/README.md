# Traeva Source for Chrome

This Manifest V3 extension forwards permitted Chrome download records to a paired Traeva receiver on the same computer. Collection is off on installation. It does not inspect pages, read downloaded files, open links, display per-download popups, or assign association confidence. Source review and user confirmation happen in Traeva.

## Pair an isolated test instance

1. Start Traeva with its documented isolated app-data setting and authorize a synthetic download directory. Enable source collection and its local receiver.
2. Use a separate Chrome profile. In `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and select this directory. Keep the profile separate from personal browsing.
3. Open **Traeva Source settings** from the extension toolbar action. Enter the receiver's `http://127.0.0.1:PORT` address and pairing token. Use the values supplied by your own Traeva instance.
4. Select **Collect new downloads**, then **Save settings**. Saving performs a connection check; review the result and the available authorized folders. **Check connection** repeats that check after app or folder changes.
5. Download a synthetic file into one of those folders. Review its source candidate in Traeva. The extension reports an acknowledged save only after the receiver returns a successful durable-save response.

Turning collection off in Extension Options clears pending deliveries. It does not delete source records already saved in Traeva. Changing this Chrome profile's pairing address or token also clears pending deliveries and begins a new collection interval, preventing an old queue from being sent to a different instance. Turning collection off in the App stops its receiver; check Options for waiting/skipped records and explicitly import downloads that started outside the App's enabled interval.

The settings page requires explicit confirmation before either action clears waiting IDs. The worker checks this again when saving, so a newly queued download cannot be silently discarded by an older page snapshot. If the queue has already been cleared, recovery requires the explicit recent import and an existing Chrome history record.

For installation outside the test harness, follow Chrome's [Load unpacked instructions](https://developer.chrome.com/docs/extensions/get-started/tutorial/hello-world#load-unpacked). Select this extension directory, not the entire Traeva repository. There is no Chrome Web Store installation in this release. After updating these unpacked files, reload the extension in `chrome://extensions` and reopen its Options.

## Connection and recovery

| Settings status | What to check / what happens to downloads |
| --- | --- |
| Collection off in this extension | Select **Collect new downloads** and save. Earlier downloads require explicit import. |
| Connected | The saved address and token authenticated successfully. Download into a listed folder. This does not confirm a file association. |
| Pairing information does not match | Copy the address and token from the intended Traeva library. Waiting IDs remain unacknowledged until delivery or an explicitly confirmed queue reset. |
| Cannot reach Traeva | Open the paired app, enable Chrome collection in Source Memory, and compare the saved address. Retry waiting downloads after reconnecting. |
| Collection off in Traeva (when the receiver reports it) | Enable collection in the app. Downloads outside its enabled interval need explicit import. |
| No currently available authorized folders | Authorize the download folder in Traeva → Settings or restore access, then check again. Skipped records need explicit import. |
| Waiting for delivery | No completed source save has been acknowledged for this update. Transient failures retain the ID and retry with backoff. |
| Progress record saved | Source metadata was acknowledged, but Chrome has not reported completion. This does not claim a completed file. |
| Source record saved | The receiver acknowledged the source write. Review candidates in Traeva; associations remain User Confirmed or UNKNOWN. |
| Rejected / Chrome record erased | Automatic retries stopped. Resolve the reported problem and use explicit import if Chrome still has the record. An erased record cannot be reconstructed. |

App closure and app-side collection disablement both stop the existing HTTP listener. The extension cannot distinguish those from an obsolete address using a failed connection alone. It shows that uncertainty and points to the app's explicit collection state; it never invents an app-running diagnosis. Available folders are the receiver's last successful check, not evidence of filesystem identity.

The settings page refreshes delivery receipts while visible, without overwriting unsaved settings. A response or error from a refresh started before Save/Check/Retry/Import is ignored after that action starts. It shows up to 20 waiting entries and the full waiting count. The latest 20 per-ID results survive Chrome restarts and identify saved updates, skipped records and terminal failures. Filenames are read from Chrome for the current display only, at most 40 distinct IDs per refresh. These receipts are bounded delivery history for one pairing, not the full Traeva library, and a successful retry updates the same ID's receipt. Turning collection off in Extension Options or changing its pairing clears this local receipt summary together with the waiting queue; saved app records remain intact.

The review regression helper `test/options-order.cjs` loads the actual Options HTML/modules in the installed headless Chrome with a synthetic message adapter. Run it with `TRAEVA_TEST_PLAYWRIGHT` pointing to the existing Playwright package. It delays successful and failed refreshes across Save, Check, Retry and Import, also checking replies during an action and subsequent current refreshes. It tests UI response ordering, not the frequency or existence of natural Chrome message reordering; no real extension transport or download is exercised by that helper.

After restarting the app, check and retry using the same pairing. After restarting Chrome, persisted waiting IDs resume while collection is enabled. Startup recreates the retry alarm; extension reload/update also recreates it. Automatic retry can wait up to its bounded backoff and alarm interval. **Retry pending downloads** requests an immediate attempt. Recovery scans remain separately opt-in and bounded as described below.

There is no automatic history import by default. The optional **Recover missed downloads after restarting Chrome** setting scans at most 200 records from the last 24 hours, bounded by the current collection interval. **Import the last 24 hours** is a separate user action that can include earlier records. Both use currently authorized folders. A removed Chrome history record cannot be reconstructed by this extension.

Automatic delivery is also bounded by Traeva's persisted latest enablement time. Downloads started while Traeva collection was disabled are discarded from an offline queue after re-enabling; starting the app again while collection remains enabled preserves its interval. Unknown start times cannot authorize automatic import. Explicit history choices are retained as a boolean alongside pending IDs and sent as `explicit_history: true`; ordinary events default to false. Both paths still require collection to be on and core to verify current folder authorization.

## Receiver protocol

All requests carry `Authorization: Bearer <pairing-token>`. The token is 32–256 characters from `A–Z`, `a–z`, `0–9`, `_`, and `-`. Endpoint validation accepts only literal `127.0.0.1`, HTTP, an explicit nonzero port, and no credentials, path, query, or fragment. Requests omit cookies and referrers, refuse redirects, and time out after five seconds.

`GET /v1/source/state`:

```json
{"enabled":true,"roots":[{"root_id":"root-id","path":"/synthetic/downloads"}]}
```

`POST /v1/source/events`:

```json
{
  "profile_id":"9b76e6ba-efb1-42b8-b309-1a282a41042e",
  "download_id":"42",
  "started_at_ms":1791453600000,
  "completed_at_ms":1791453602000,
  "state":"complete",
  "filename":"/synthetic/downloads/sample.txt",
  "initial_url":"https://origin.example/sample",
  "final_url":"https://cdn.example/sample.txt",
  "referrer":null,
  "file_size":42,
  "incognito":false
}
```

`profile_id` is a UUID generated once per extension installation in that Chrome profile. It survives worker/browser restarts; reinstalling the extension creates a new identity. `download_id` comes from Chrome. A record's original start time is retained, including `null` when absent. The receiver owns deduplication across repeated notifications and lost acknowledgments; neither URL nor hash is an event identifier.

Fields unavailable from Chrome remain `null`. Initial URL, final URL, and referrer remain separate. Chrome's title-free download record does not gain a guessed title. Completion time is populated only for `complete`, never from an interrupted record's end time, file metadata, or import time. Both `in_progress` and terminal updates may be sent. A blank filename waits for a later observation.

Acknowledgment:

```json
{"outcome":"saved","source_id":"source-id","reason":null}
```

Or a terminal refusal:

```json
{"outcome":"ignored","source_id":null,"reason":"outside_scope"}
```

Failures use non-success HTTP status codes. The extension does not display or log arbitrary response/error text. Connection/storage failures stay pending and are not shown as saved. Event responses `400`, `413`, and `422`, or event payloads over 64 KiB, stop automatic retries and increment a visible rejected-delivery count. The receiver must enforce pairing, collection consent, origin/host restrictions, authorized-directory access, durable transactions, and idempotency independently. The extension's path-component check is a privacy prefilter and cannot validate symlinks, file existence, or physical identity.

## Privacy and retries

- Incognito use is disabled in the manifest and incognito records are rejected before delivery.
- Only `downloads`, `storage`, and `alarms` permissions are requested, with HTTP loopback host access. There are no content scripts, native messaging hosts, remote scripts, page/title lookups, or cloud requests.
- Extension storage contains configuration (including the pairing secret), a bounded queue of Chrome IDs and retry counters/times, fixed connection/error codes, and up to 20 ID-only delivery receipts with times and download state. Download URLs, filenames, titles, folder paths, and response bodies are not copied into extension storage or logs. The pairing secret is limited to trusted extension contexts.
- At most 200 IDs are pending. Overflow is reported in settings; it does not trigger unbounded collection. Pending requests use exponential retry delays up to one hour, serviced by a one-minute alarm. Each pass processes at most 20 due records.
- URL payloads are re-read from Chrome immediately before delivery. A crash after an acknowledged database write but before local queue removal can resend the same event, so receiver deduplication is required.
- `User Confirmed` remains distinct from `VERIFIED`. No automatic VERIFIED code or confidence field exists in this extension. The prior Chrome identity experiment's FAIL remains applicable until a separately approved evidence standard is implemented and verified.

## Tests

Run from the repository root:

```sh
node --test extensions/chrome/*.test.js
```

The dependency-free tests cover transport restrictions, unknown/time semantics, scope/incognito filtering, progress/completion updates, durable-ack failures and retries, restart identity, history consent, pairing changes, queue bounds, and absence of URL payloads from persisted state. They use synthetic records and mocked Chrome adapters. They do not count as a real Chrome-to-Traeva integration test or as automatic VERIFIED evidence.

Browser integration should additionally exercise unpacked installation, every settings control, real downloads/repeats, receiver interruption/restart, and a restarted Chrome profile against isolated app-data and synthetic roots. Record actual versions and outcomes; do not mark unrun checks PASS.

An opt-in real-browser adapter smoke is provided at `test/browser-smoke.cjs`. Set `TRAEVA_TEST_PLAYWRIGHT` to an existing Playwright module directory and optionally `TRAEVA_TEST_CHROME` to the actual Chrome executable, then run it with Node. It creates its own temporary profile, download roots, app-data, fixture server, and synthetic authenticated receiver. It opens a headed Chrome and closes its browser/servers afterward. The output identifies the retained evidence directory. This validates the extension adapter and settings; its synthetic receiver does not validate Traeva core persistence or association behavior.

`test/browser-core.cjs` instead connects to an already running, test-owned real receiver. Supply `TRAEVA_SOURCE_TEST_CONFIG` pointing to a private JSON file containing `endpoint`, `pairing_token`, `root`, and `app_data`, plus the same Playwright/Chrome variables. It performs a real redirected download and writes `app_data/chrome-delivery.json`, including the exact replayable event, after durable acknowledgment. Its caller must create the isolated app-data/root, start the real receiver, and independently verify core candidates, user confirmation, deduplication, and restart persistence. Never point it at personal roots or production app-data.

For a real desktop app restart, set `TRAEVA_SOURCE_TEST_EVIDENCE` to an existing canonical evidence directory disjoint from both app-data and the authorized root. The helper then writes all reports there instead of adding unrelated files to the app's validated storage directory. Its default app-data output layout remains for the older mock-IPC caller.

`test/browser-recovery.cjs` exercises first pairing, wrong tokens, unavailable authorized roots, waiting receipts, Chrome and receiver process restart, guarded queue resets, explicit import and disabled collection intervals. Compile `cargo test -p traeva-app --test source_chrome --no-run`; set `TRAEVA_SOURCE_RECOVERY_DRIVER` to that test executable and `TRAEVA_TEST_PLAYWRIGHT` to the installed module. It creates a fresh synthetic directory, app-data and Chrome profile and retains evidence there. The driver uses production core/receiver through mock IPC and private pipes, with no desktop instance or product control endpoint. `TRAEVA_TEST_CHROME_HEADLESS=1` selects real headless Chrome; otherwise it opens a headed test browser. Test-only CDP extension loading is not a product debugger permission. The test preserves original failures and never treats a row visible before the HTTP acknowledgment as a completed delivery.
