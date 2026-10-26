# Source first connection and recovery

Refs #27. This change follows the student's request to improve setup and recovery using the existing Chrome extension and local HTTP pairing. It does not change the accepted association rules, core, schema, permissions or shared interfaces. Auto VERIFIED remains unavailable. No change in this PR constitutes #26 or #37 acceptance.

## User flow

The Source page now explains folder authorization, unpacked extension installation, both collection switches, pairing, and a small test download. Enabling collection reveals the local address and masked token. A ready local receiver is explicitly distinguished from a successfully paired extension. A failed local receiver can be retried using the existing enable command.

Extension Options starts with setup instructions and disables edits until saved configuration has loaded. Saving configuration checks the actual receiver. An invalid token, unavailable receiver, disabled extension collection, and no currently usable authorized folders have separate messages and next steps. The app's own Source page explicitly shows whether app collection is off.

The existing protocol stops listening when app collection is off. A connection refusal cannot distinguish that from app closure, a changed port, or another local connection problem. The extension tells the user to open the app, check its collection switch and compare the address. It does not invent an exact diagnosis or keep a new listener running without consent. A precise remote distinction would require a separate shared protocol/consent decision from A and the student; this PR does not make that change.

Options shows waiting IDs, retry reasons and scheduled times, plus the last 20 per-ID delivery results. An acknowledged progress update is labelled as progress; an interrupted download is not described as a completed file. A saved receipt means the receiver acknowledged source storage, not that a file association was confirmed. Terminal rejections, inaccessible scope, collection-interval skips and erased Chrome history have explicit recovery explanations.

Waiting IDs and receipts survive Chrome restart. Transient delivery failures retry with the existing bounded backoff. Manual Retry requests immediate attempts, up to 20 records per pass. The waiting list displays the first 20 entries and states the total; filename reads for the current page are bounded to 40 unique IDs, including recent receipts. Manual history import and optional startup recovery retain their existing 24-hour/200-record consent bounds.

Turning collection off in Extension Options or changing that profile's pairing still clears the old queue and local receipt summary. This existing policy prevents forwarding an old queue to another library. The page now requires an explicit checkbox when waiting IDs will be cleared, and the serialized worker rechecks that consent for downloads arriving after the displayed snapshot. Previously saved app records are never deleted by that operation. Turning collection off in the App stops the receiver; it does not perform the extension's queue-reset operation. Check Options for waiting or skipped records, and explicitly import downloads that started outside the App's enabled interval.

## Privacy and ownership

- The extension manifest is unchanged: downloads, storage, alarms, loopback HTTP; Incognito remains excluded. No debugger or Native Messaging permissions, new downloader, page inspection, AI or cloud calls.
- Download URLs, filenames, titles, folder paths and arbitrary response/error bodies are not persisted in extension status. Receipts contain only ID, fixed outcome/reason, download state and time. Names and available folder paths are ephemeral display data; text uses DOM `textContent`.
- Settings/status messages remain restricted to this extension's exact Options URL and trusted extension contexts. The pairing token remains masked in the UI and redacted in test reports.
- The HTTP protocol, core association logic, app-data DB, FileRecord registration and shared DTOs are unchanged. Source may display a candidate, User Confirmed or UNKNOWN; no path/time/hash inference is promoted to VERIFIED.
- A pending delivery means acknowledgment is missing. A write may already exist if the response was lost; the UI directs users to the app and relies on existing idempotent replay. It does not claim a definite failed database commit.

## Developer evidence

Initial implementation commit: `93ae6701482e81f036e14039ace56603c765b3ea`. Main at that test, `65df8c9b80f6624e486a16d22b0905a2315ef373`, was merged into the branch without changing Source code; actual desktop testing and required integrated checks used `a129d93f75457cc4bb997d1d26ec0cd9d13b99c7`. The initial report-only head was `aee437d0c72a761abc407556a33bc2956793c570`; the subsequent R1 fixes below change Options response ordering and App recovery copy. Source hashes, environment, synthetic IDs and results are recorded in [the sanitized validation summary](source-connection-recovery-validation.json). Actual Chrome was `155.0.8059.39` on macOS arm64. All profiles, app-data and files were synthetic and independent; no personal profile, default Traeva library, model key or cloud provider was used.

The real recovery helper uses Chrome's actual Options/download APIs and the production HTTP receiver/core. A private pipe driver uses mock Tauri IPC to authorize the synthetic root, query saved records, confirm a candidate and recreate the receiver process. It does not start a desktop WebView or add a product test-control endpoint. The Chrome test uses CDP only to load the unpacked extension and select the synthetic download directory; this is not a product extension debugger permission.

| Check | Actual result |
| --- | --- |
| Fresh profile starts collection off; valid-format wrong pairing token | PASS: rejected pairing, no connected claim and no saved source |
| Correct pairing and normal TXT download | PASS: authorized folder displayed, completed source acknowledged, candidate remains unassociated until explicit User Confirmed |
| Authorized directory renamed away, then restored | PASS: no available folder state and empty list; recheck restores the same authorized directory |
| Receiver exit, offline download, Chrome exit/restart, receiver process restart | PASS: waiting IDs and profile ID retained; paired port/token retained; retry saves one source; prior User Confirmed and first import time preserved |
| Pairing change with waiting records | PASS: no silent queue clear; explicit checkbox required; manual history import recovers the cleared download without duplicates |
| App collection disabled during download | PASS: no automatic import after re-enabling; explicit history action required |
| Storage privacy and responsive Options page | PASS: no download metadata in persisted extension state; no page errors or horizontal overflow at 360px |
| App closure vs collection off from connection refusal alone | Known protocol limit: truthful combined diagnostic, not an inferred exact cause |

The actual packaged App was also tested on a new empty library and synthetic download root. The native folder picker authorized only that root. Source initially showed collection off and the expanded setup guide; enabling collection revealed the ready local receiver with a masked token and explicit pairing boundary. A fresh real Chrome profile downloaded a redirected TXT through the extension into this actual App. The Source GUI displayed a candidate, separate initial/final URLs and unknown title. Explicit GUI confirmation saved User Confirmed and the supplied note.

After normal Command-Q exit and explicit same-library restart, the GUI still displayed the confirmed source. Read-only comparisons of the synthetic library before/after found identical source rows, association/evidence/history, first import time, pairing token digest/port and file hashes. This is normal App restart evidence, not OS crash or power-loss evidence. Both owned App processes exited normally; the default Traeva library was absent before and after. The App window ran on the Dell display. Chrome used the actual installed binary in headless mode; loading the extension through the test helper does not verify manual installation dialogs. The previous locked-screen BLOCKED attempt remains separate.

Required checks: extension 39/39; combined frontend/extension 118/118; integrated core 791 passed, 10 opt-in ignored, zero failures; formatting, workspace Clippy, frontend build and original packaged App build all passed. The original real-Chrome redirect/core regression passed again, independently of the seven-group recovery matrix.

## Original failures and fixes

Original logs/screenshots are retained separately and are not overwritten by later PASS evidence.

1. The initial unit run expected the old pending-entry shape. The new fixed per-ID error code caused that assertion to fail. The assertion was updated to retain all old backoff/privacy checks and require the new code; the expanded extension suite passes.
2. The first recovery run compared `/var/...` and `/private/var/...` directory spellings. The test now canonicalizes its temporary base before authorization and Chrome setup. No product containment rule changed.
3. The second run read an existing, still-empty file before Chrome reported completion. The test now waits for the actual Chrome completed state and checks the completed fixture bytes. An empty placeholder is not successful download evidence.
4. The third run inspected a newly committed source row before the POST finished building its candidate and before the extension received acknowledgment. It observed UNKNOWN and failed the premature candidate assertion. The test now waits for the actual complete saved receipt before checking the candidate; it does not relax the expected candidate or association result.
5. Visual inspection of a passing run found the folder list could stay empty after a delivery restored connectivity. The collector now returns its last checked roots from memory with the corresponding check time. They never enter extension storage. Reopening a worker without a current root snapshot asks for another connection check. The real recovery matrix passed again after the fix.
   The existing Chrome smoke also verifies automatic alarm recovery updates the folder list and clears an obsolete connection error without a manual retry. Its synthetic receiver scope is separate from the production-core matrix.
6. A first test-driver compile duplicated Tauri's embedded plist symbol by expanding `generate_context!` twice. A single test-local context function fixes the build. No app startup/configuration change.
7. A sandboxed core run failed loopback listener tests with `Operation not permitted`. The full required suite was rerun with the synthetic loopback test environment enabled: 791 passed, 10 opt-in ignored, zero failures. The original sandbox failure remains in local logs and is not classified as a Source defect.
8. The older redirect helper inspected the folder list during an unfinished second connection check, using the previous check's Connected text as its signal. A retained reproduction confirmed the empty-list assertion. It now waits for that check's completion; Options explicitly shows Checking while it runs and ignores older polling responses during actions.
9. The initial desktop isolation guard treated WebKit's OS security-cache database as a Traeva product database. The guard now explicitly separates that cache while still rejecting unexpected product DB/lock paths; the original guard failure is retained. A later stale accessibility index was rejected before a click; the current tree was reread. Neither tooling failure changes the product evidence or association rule.

## R1 review fixes

R1 reviewed `aee437d0c72a761abc407556a33bc2956793c570` and blocked only this enhancement, identifying two P2 issues. The original #27 CAC acceptance remains separate and unchanged. The new fixes require R1's targeted recheck; developer PASS is not independent approval.

| Finding | Fix and developer validation |
| --- | --- |
| R130-01: a background poll started before an action can arrive after it finishes and overwrite the new connection/error state | Each action advances a generation before it starts. Poll success and failure can update the page only while idle and in the same generation. The actual Options HTML/modules in real headless Chrome were tested with controlled message delays: before the fix, 2 PASS / 12 FAIL; after, 14 PASS / 0 FAIL. All four actions cover stale connected status, stale rejected status and stale poll failure; two checks cover responses arriving during an action. Every case also verifies that a later current-generation poll still updates the page. |
| R130-02: App copy incorrectly attributes the extension's queue clear to the App collection switch | Source and the extension README separately explain App receiver shutdown, the Extension Options queue-reset operation, and explicit import for downloads outside the App's enabled interval. No queue or consent rule changed. The existing true-Chrome/production-core recovery matrix was rerun on the fix: 7 groups PASS, including saved/waiting records across process restarts and the disabled collection interval. |

The delayed-message helper tests UI ordering through a synthetic Chrome runtime adapter, not natural Chrome transport reordering or its frequency. The seven-group recovery helper separately exercises actual Chrome extension APIs and the production receiver. Original failing results remain retained alongside later PASS; they are not replaced. The fixes also pass frontend/extension tests (118), core tests (791, 10 opt-in ignored), formatting, workspace Clippy and frontend build. Native App GUI was not rerun for these two fixes; its earlier `a129d93` evidence above remains bound to that commit. Auto VERIFIED remains unimplemented, and no #26/#37 acceptance is claimed.

## Reproduce and review

Run the usual formatting, workspace Clippy, core tests, frontend/extension tests and frontend build. See `extensions/chrome/README.md` for the opt-in Chrome helpers. The driver binary comes from `cargo test -p traeva-app --test source_chrome --no-run`; it is launched only by `browser-recovery.cjs` with a newly created synthetic base.

For the two review fixes, run `extensions/chrome/test/options-order.cjs` with the existing Playwright dependency and repeat the recovery matrix. R1 should recheck stale successful and failed polls across Save/Check/Retry/Import and the App-versus-Options recovery wording. Exact app/window testing must follow #65 isolation and UI-slot coordination; the previous desktop claim/release is recorded there. Native installation dialogs, minimum Chrome 120, abrupt OS shutdown, queue-capacity behavior in a real browser and a busy saved receiver port remain untested; they must not inherit the recovery or desktop PASS.
