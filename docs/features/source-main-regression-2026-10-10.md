# #37 Source Memory: main integration regression (2026-10-10)

Conclusion of this round: **The covered Source scenarios pass C's developer regression; no Source bug was found. One Memory issue was found, a permission description that shows the old path; it has been handed to A/B. This must not be used to declare that #37 as a whole has passed acceptance.** Automatic VERIFIED research is frozen; it is not implemented or enabled in the product and does not count as PASS.

This PR only adds cross-module tests and evidence. It does not change the product, the shared core, permissions or A/B features. The student's decisions, R's independent re-verification and merge rights keep their existing boundaries.

## Versions and isolation

| Item | Actual value |
| --- | --- |
| First-round main | `6cc3a60742c54cac6e3e056eb422cf2af36d2bd2` |
| Final main | `7c64dbac0772ccf8835a81ab59d734db8a7ce067`; #113 was merged in between |
| Version difference | Only the five Memory frontend files related to "add to work"; Source, the shared core, the Extension, the receiver, the database and configuration did not change |
| Latest .app binary SHA-256 | `2851ebae399c1e3dfcc4b02c5d9f320fbaa018271892b5dc0c1e8236275add19` |
| Original .app binary SHA-256 | `41e95e5845744b83a3cbcdcb3b84fda3b625defd08e6f0e185c83ca6c7d46f62` |
| System / browser | macOS 27.0.1 arm64; Google Chrome 155.0.8059.39 with a window |
| Contract / schema | v16 / 9 |
| Data root | `/private/tmp/traeva-c-main-source-aoommqit`; app-data, downloads and browser-profile are separate |
| Model | Not configured, not authorized; no real model calls; the consent / analysis tables are both 0 |

The `.app` was opened normally with the original Tauri bundle configuration; only the supported `TRAEVA_APP_DATA_DIR` was set. HOME, the bundle identifier, permissions, the database and locks were not changed. Real Chrome loaded main's Source Extension and reused C's existing synthetic download method: pair with the real receiver through the settings page, click the download link, wait for the persistence confirmation. There was no mock receiver, no mock Tauri IPC and no record written directly into the database.

The browser used a separate Profile. The download was a harmless TXT from a local synthetic HTTP server, through one 302 redirect. The script used CDP to load the test extension and observe downloads, **with no Fetch interception and no product debugger permission**. This round does not evaluate Chrome's safety protections, public HTTPS sites or automatic VERIFIED.

## Real app tests

"Pass" only means that the expectations listed in the table below held on the listed versions, in the synthetic environment. The interface was operated through the native UI; results were also checked with read-only database exports and by observing the file objects. The JSON table export by itself does not contain all dynamic UI state; neither may stand in for the other.

| Scenario | Expected and actual result | Evidence / version |
| --- | --- | --- |
| Chrome automatic collection | Download 1 reached the non-interrupting pending-confirmation list; 1 source, 1 candidate, 0 confirmed associations | `collected`; 6cc3a60 |
| Fields and times | The initial URL and the post-redirect URL are kept separate; the referrer comes with a hint on what it actually means; source page and title unknown; start, completion and first import are each saved | `chrome-delivery`, UI; both versions |
| Cancel candidate | Click Review, then Cancel: associations and decision history stay at 0 | `after-cancel`; 6cc3a60 |
| User Confirmed | The association is written only after explicit confirmation; the interface states clearly "not system-verified"; the original download time and the first import time are not refreshed | `after-confirm`, `latest-confirmed`; both versions |
| Source + Memory | Memory turned on for the same download and versions saved; after an edit on the same inode a new snapshot is added; the basis of the Source confirmation is unchanged | `memory-on`, `edited-in-place`; 6cc3a60 |
| Organize move | Real preview → confirm; same file_id / inode; Source shows the new location and is still User Confirmed; the Memory record card follows | `organize-preview`, `organize-moved`; 6cc3a60 |
| Undo | Real preview → Put it back; the original file returns to Inbox; Source, Memory and the saved versions are kept | `undo-preview`, `undone`; 6cc3a60 |
| External rename and move | A synthetic operation moved the same object; Memory showed it as lost, and a manual relocate succeeded. Under v16, relocate by itself does not continue Source; the history stays on the old file_id and the location is unconfirmed | `memory-relocated`, UI; 6cc3a60 |
| Source explicit locate | The moved object was picked with Source's native picker; after an on-the-spot check it was continued onto the new file_id, keeping the original file_id reference, the confirmation time, the first import time and User Confirmed | `source-relocated`, UI; 6cc3a60 |
| Same-content replacement isolation | A new file replaced the original path, same hash but a different inode; Memory captured a candidate; after the user chose Continue, Source is still associated with the old object and the current location is unconfirmed | `replaced`, `memory-continued`; 6cc3a60 |
| Replacement file details | Organize picks the new object; Source shows 0 confirmed / 0 candidates and unknown; it does not inherit the download source because of the same hash, path or work relation | `replacement-details`, UI; 6cc3a60 |
| Repeated download of the same URL | Chrome download 2 is a separate event with a separate file_id; download 1's source and confirmation were not merged into it | `repeat-download`; 6cc3a60 |
| Reject → UNKNOWN | After the candidate of download 2 is rejected, it shows Unknown / no reliable candidate; no association is created; the rejection is still kept after restarts | `rejected`, UI after two restarts |
| Duplicate notification dedup | Download 1 replayed through the real receiver: it still returns the original source_id; the source count, association count and import time are not increased or refreshed | `before-restart`, `before-close` |
| Update and restart | The old process quit normally, then the latest main .app was opened; Source's four tables, the snapshots, the Memory members and edges are the same as before quitting. The old object is still "current location unconfirmed" | `restarted-latest`, `restart-comparison`; 7c64dba |
| New download on the latest main | Download 3 after a normal Chrome restart; the new candidate can be explicitly confirmed and stays User Confirmed | `chrome-latest-delivery`, `latest-confirmed`; 7c64dba |
| Latest Memory add to work | The #113 entry was used to add `unrelated.md`, which has no browser source, to the same work, choosing "Same work, place not known"; its Source is still 0 confirmed / 0 candidates, unknown | `latest-associated`, `latest-family-source-unknown`; 7c64dba |
| Normal restart on the same version | 7c64dba quit again and reopened: two User Confirmed, one Unknown; the four source tables, the snapshots and the work relations are identical item by item; download 3's location is still confirmed | `latest-normal-close`, `latest-normal-restart`, `restart-same-version-comparison` |
| Coexistence after turning Source off | Collection turned off in the UI; past sources can still be viewed; a synthetic edit of the Memory file actually added a 5th version; the source association is unchanged | `source-off-memory-continues`; 7c64dba |

The first round's move / Undo / relocate / replacement were not redone as a whole chain on the second UI version. The corresponding Source/core files are identical in both versions; the latest version was additionally tested for the changed Memory entry, a real download, confirmation, UNKNOWN, restart, and coexistence after collection is turned off. The report does not label operations done on the old version as done on the new version.

## Exact object chain

Download 1's source_id is `b831545a-a0d0-43ee-86fd-5eba904c0c0d`, browser environment `3831c641-09ab-4dbb-8e86-871ff24a2f88`. First import `1791670625345` ms, explicit confirmation `1791670654912` ms; both stayed unchanged for the whole test. The import is earlier than the completion because the in-progress event was saved successfully first and the completion event updated it afterwards; the import time was not taken as the completion time.

| Object | file_id | inode | Source result |
| --- | --- | --- | --- |
| Original download F0 | `d357887c-7c95-4e3c-bf19-9b960499c595` | 169497582 | Manually confirmed; Organize / Undo do not change the ID |
| Re-registered after external move F1 | `537607c8-e5ff-4853-878b-4b6fe7bc5a43` | 169497582 | Continued after Source Locate; the original reference F0 is kept |
| Same-content replacement F2 | `ee3cce58-2447-41f3-9c78-2431b7b149d3` | 169634388 | Does not inherit F1's download association, even with Memory Continue |
| Second download F3 | `3a233653-b0d8-4f1f-894f-5bf625be7280` | 169646861 | Separate source event; not associated after rejection |

The SHA-256 before and after the replacement is the same, `377b7ea9281322eb35e08b4a160fae9b331302673fbcdf60f746683252a67469`; this hash is used to confirm the counterexample condition and **is not used as proof of a source association**. When download 1 was replayed later, F3 could become a path candidate of S1, but it was not bound automatically; rejections are saved per source event, so the rejection of S2 is not quietly turned into a rejection of all events.

## Failures found and handed over

**The Memory candidate capture permission description still shows the path from before the move (real UI FAIL).**

1. Turn on Memory in Inbox, keep candidate capture Off, look at "Only this exact place".
2. Organize submits a move to Organized.
3. Go back to Memory: the record card shows Organized, but the candidate capture description still shows Inbox. Waiting for the list to refresh does not fix it.
4. After Undo the actual path is back in Inbox, and the stale description just happens to be correct again.

`recordSignature()` does not include place; CandidateCaptureCard refreshes only on chainId / signature, so an old CandidateCaptureInfo can stay for a long time. This is a UI refresh issue that belongs to B. There is also a shared-boundary issue that is **derived from the code, not tested by clicking**: when there is no unread occupant, the tokens of both the old and the new place can be null, so a turn-on request made under the old description may be applied to the new place. A needs to check whether the permission should be bound to the exact place shown at display time. C did not change this feature or decide the interface rules on anyone's behalf.

Handed over: [Reproduction, ownership and evidence for A/B](https://github.com/P30g53/Traeva/issues/65#issuecomment-6102864317). This FAIL is not closed because the Source tests passed.

## Automated checks and kept failures

| Check | Result and scope |
| --- | --- |
| Original main frontend / Extension | 95 passed, 0 failed |
| Original main Rust workspace | 794 passed, 0 failed, 10 ignored; does not yet include the tests added in this PR |
| New main frontend / Extension | 97 passed, 0 failed |
| New coexistence tests | 4 passed, 0 failed; real production API, temporary synthetic files; a service reopen is not a process restart |
| Checks | `cargo fmt --all --check` and clippy `-D warnings` on the new test target pass |
| New main build | TypeScript / Vite / original Tauri debug .app build succeeded |

The four new tests cover, respectively: Memory edit + Organize / Undo / reopen; same-content replacement + user Continue does not propagate Source; the contract boundary between Memory relocate and Source registration; copy-delete / independent Memory / Source off / UNKNOWN. The product was not changed to make the tests pass.

Kept environment / tool failures:

- In the first workspace test run the sandbox blocked local sockets, and 6 receiver tests got EPERM; the original log is kept. After loopback was authorized, the same command completed with the 794 / 0 / 10 above.
- The first time the new Chrome script reused the Profile, it did not wait for the extension settings page to finish its initial load; waiting for "Settings saved" timed out before any download was started; failure.json is kept. After the load wait was added, the actual third download succeeded. The Extension was not changed.
- Native menu operations once ran into stale accessibility state; after the state was refreshed the selection completed. The tool's failed click is not counted as a product failure.
- The first port check inside the sandbox returned EPERM (which does not prove the ports were closed); at wrap-up it was checked again in an environment that allows loopback and got connection refused. The first process query had wrong arguments and is also not used as cleanup evidence.
- The 10 ignored tests include disk-image, real-model, sealed-evaluation and child-process entry points started by parent tests; this round did not run these entry points on its own initiative.

## Evidence, limits and wrap-up

- [Reviewable stage data and SHA-256 index](source-main-regression-2026-10-10.json). Raw exports, browser notifications, failure logs, and build and test logs are kept in the local evidence bundle in `outputs/source-main-regression/`; the browser Profile, pairing credentials and the database do not go into the evidence bundle or the PR.
- Local archive: `outputs/source-main-regression/source-regression-evidence-2026-10-10.tar.gz`, SHA-256 `025a318b9046c21a60e203d84b39ca63eb955175f1cb4b05ea979e107a145913`.
- Original run directory `/private/tmp/traeva-c-main-source-aoommqit/evidence`; all reads were of the test database, and all written decisions were made through the real UI / receiver or explicit synthetic file operations.
- The content, objects and captured metadata of the two control files stayed unchanged; no extra/hidden files were found. The cancel interval and the reject interval each had a ctime-only difference that cannot be attributed; the text, inode, mtime and xattr digest did not change. Therefore this report **does not claim that all metadata had absolutely zero change**, nor that a per-process write audit was done.
- The four baseline stat metadata items of the default app-data did not change; its database contents were not read. This round used only harmless TXT / MD and did not access personal files, cookies, login credentials or sealed data.
- Not tested for real: forced crash, power-loss recovery, public HTTPS, network failure, iCloud, cross-volume, large files, dangerous downloads, or real AI classification / semantics / retrieval. The existing automated failure paths do not replace these real scenarios.
- Source collection was turned off; this round's Traeva, the separate Chrome and the synthetic server were quit normally; all three test ports refuse connections. [The UI slot has been released](https://github.com/P30g53/Traeva/issues/65#issuecomment-6102896948). Test materials are kept for re-checking; locks and history records are not deleted.

Recommendation for #37: accept this **Source developer regression contribution**; A/B first handle the Memory permission description above, then a targeted re-verification is run on the fixed version. #37's full CAC acceptance, R's independent opinion and the student's merge decision still each need to be completed separately.
