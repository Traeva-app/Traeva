# CAC five sessions: collaboration and development setup

These are the execution notes for the approved CAC plan, not a fourth frozen spec. The three Organize v0.3 documents, the safety semantics and the three Gates do not change. Live status follows [CAC Sprint](https://github.com/users/P30g53/projects/1/views/5); this page reflects a check of `main` `04f270c9735cd25a066aa1de191ceb5e818af6e8` as of 2026-10-08, and does not claim that the parallel environment has passed real testing.

## Current Find and Memory assignment

The student now owns the current Find and Memory work described in [AGENTS.md §4](../../AGENTS.md#4-session-responsibilities-and-shared-files). In particular, `crates/traeva-core/src/find.rs`, `changes.rs`, `retrieval.rs` and `lineage.rs` are student-owned source files. Agents may identify logic, syntax and coding-practice errors and suggest corrections; they must not edit these files or other Memory core logic unless the student explicitly reassigns that work. The [README contribution statement](../../README.md#my-contributions-and-ai-assistance) describes the student's work on the four files.

| Current owner | Current work | Boundary |
|---|---|---|
| Student | Find and Memory implementation, rules and fixes under the current assignment | Supplies the code and interfaces for integration |
| Claude-A | Shared integration, commands, permissions and Find wiring | Uses the supplied interfaces; does not revise the student's core logic |
| Claude-B | Memory UI and interface wiring | Reports core issues to the student; does not implement analysis or retrieval changes |
| Codex-C | Source collection and UI/receiver wiring | Calls the supplied Memory interfaces; does not change association or persistence rules |
| Codex-R | Independent tests, review and evidence | Reports failures; does not fix product code |

This assignment supersedes the earlier implementation assignments below. The remaining startup, scheduling and evidence notes describe the original development stages and do not reopen those assignments. Earlier contributions and test results retain their original authors, commits and scope.

## Scope and execution lanes

The following table records the original execution lanes. Use the current assignment above for new Find and Memory work.

| Execution lane | Full responsibilities | Current entry point and prerequisites |
|---|---|---|
| Claude-A — Core | #65 shared foundation, #31 family/lineage, #32 watcher/durable snapshots, shared files/migrations, #37 integration | #65 and the #32 independent capture foundation can start; #31 waits for the shared interface PR to merge and for the student's A material to be Accepted |
| Claude-B — Intelligence | #33 semantic changes, #34 retrieval, #35 Memory UI | develops #33 as soon as the shared interface PR merges, without waiting for #32 to finish; #34 still needs the student's A material and the existing prerequisites |
| Codex-C — Source | #27 Chrome source, receiver, association, persistence, UI | formal implementation after the shared interface PR merges and the student's B material is Accepted, without waiting for #65 to close |
| Codex-D — DOCX | #66 Limited Reader, real Word saving/parsing, conditional Word Memory integration | develops the Reader after the shared interface PR merges; 10/13 Go/No-Go; a No-Go does not block Memory |
| Codex-E — PPTX | #28 PPTX Reader independent experiment (added 10/11) | experiment only, not connected to the main program, no changes to shared files; not part of CAC Required |
| Codex-F — XLSX | #28 XLSX Reader independent experiment (added 10/11) | same as E; the experiment crate is in `experiments/xlsx-reader` |
| Claude — PDF (experiment session) | #28 PDF Reader independent experiment (added 10/11) | same as E; the experiment crate goes under `experiments/` |
| Claude — UI polish (session) | visual and interaction polish of the existing UI (added 10/12) | changes only the UI layer; a new small feature first gets an Issue and the student's approval; shared wiring goes to A; after the freeze only fixes defects |
| Codex-R — Review | #19, blocker re-verification, #26 Memory independent verification, #37 final verification | #19 Ready, lock the review commit first; only reviews, and never implements product features or fixes them for others |

Ownership A/B/C is the student contribution requirement; the Execution Lane is the one responsible for execution; C's historical option name `Opus Implementation` does not exclude Codex-C/D. #26/#30 are tracking parents, not a new implementation stage. Memory is CAC Required; DOCX is a conditional enhancement; for PPTX/XLSX the student decided on 10/12 to connect them fully to Memory (#162); PDF: only Organize reads the text layer (#186, organize-spec §14), and on-device text recognition for scanned PDFs is on by default after the student agreed on 10/13 (§14.8, Organize only, macOS 13 and later; R4's production verification in #211); Memory, Find and Source do no recognition; other long-term directions stay Post-MVP.

## Starting implementation and final acceptance

Once the student merges #65's shared interface milestone PR, each lane that meets the isolation requirements below can start headless feature development, without waiting for #65's desktop override, real testing of every environment, or the whole item to close. A records the interface PR, the merge commit and the consumers' check results in #65. The interface PR has been delivered: #68, merge commit `03d2470`.

| Issue | Implementation allowed to start | Final acceptance still requires |
|---|---|---|
| #27 | after the shared interface PR merges + this item's B material is Accepted, independent headless Source development | real Chrome source, association, persistence and display; complete isolation evidence |
| #31 | after the shared interface PR merges + this item's A material is Accepted, implement family/lineage | evidence of real association, ambiguity/branches and restart |
| #32 | A may first use the existing #6/#7 primitives to build the watcher, hash and durable snapshot foundation, without waiting for #31 to finish as a whole; no family association rules written in advance | family-related implementation first waits for #31's material to be Accepted, then connects to the available interface; the original rename/move, restart and snapshot acceptance is all kept |
| #33 | after the shared interface PR merges, implement semantic changes with contract-conforming deterministic fixtures / a fake provider, without waiting for #32 or #65 to close | the original 5–10 version acceptance must run on real snapshots that #32 actually captured, persisted and verified; fixtures do not count as an end-to-end PASS |
| #66 | after the shared interface PR merges, implement the limited Reader and independent Word tests | 10/13 Go/No-Go; Word Memory still needs the real #31/#32 foundation and the conditional #33–#35 integration |

Native blocked-by keeps only start dependencies that require a whole item to be complete: the whole-item blocking of #27/#31/#66 by #65, of #32 by #31 and of #33 by #32 is removed. The stage condition of the shared interface PR is written into the Issue and the Project's Next Action, and maintainers update Status after checking the merge evidence; nothing is unlocked automatically by #65 closing. Currently #32 is Ready (capture foundation only), and #27/#31/#33/#66 are still Blocked because the interface has not been delivered or the material is not Accepted. Ready does not mean work has started or that all acceptance is met, and a partial implementation cannot close an Issue.

#33's Memory automatic watching/local capture does not constitute permission to send to the cloud, and must not implicitly reuse Organize's send permission. Before real AI analysis, an explicit authorization specific to Memory must be obtained, stating the scope of files/snapshots/excerpts or diffs sent, and whether later automatic analysis is included; a revoke entry point is provided, and after revocation no queued or new request under that authorization is started. No promise can be made that data already sent will be taken back. In the original plan, B owned the analysis authorization boundary, A coordinated shared wiring, and R was assigned to independently verify sending without authorization, beyond scope and after revocation. Under the current assignment, the student owns Memory consent logic, A/B wire the supplied interfaces and controls, and R independently verifies the integrated behavior. The sending requirements above remain unchanged.

## Run isolation

### Current code facts

| Entry point | Current behavior | What this must not be used to claim |
|---|---|---|
| `vite.config.ts` | `port: 1420`, `strictPort: true` | two UI sessions cannot coexist by switching ports automatically |
| `src-tauri/tauri.conf.json` | `devUrl` points to 1420, and `devCsp`'s HMR websocket is also fixed at 1420 | changing only the Vite port is not a complete override |
| `src-tauri/src/lib.rs::run` | calls `start_for_app()`: without `TRAEVA_APP_DATA_DIR` it uses the default folder; with it, it uses the given test folder, and if the test folder does not qualify, storage is blocked with no fallback (#65) | a worktree, a branch or a different app file name does not by itself isolate the DB; that takes `TRAEVA_APP_DATA_DIR` |
| `traeva-core::OrganizeService::start(PathBuf)` | temporary app-data can be given explicitly; startup first checks isolation, then opens the database and recovers records | not yet a development parameter of the desktop program |
| `crates/traeva-core/tests/startup.rs` | already has test code for refusing a second instance on the same folder, for restart and so on | not rerun this time, and it does not prove five-process run isolation |
| `src-tauri/tests/ipc.rs` | injects a service on a temporary folder and uses the mock runtime | not the same as verifying a real window/packaged permissions |
| Model tests / Keychain | the fake server binds `127.0.0.1:0`; the real Keychain service/account is fixed | a temporary DB does not isolate real credentials |

### Minimal operating plan

1. Once feature development is approved, each lane has its own branch/worktree; each checkout has its own `target/`, `node_modules/` and `dist/`. Do not share `CARGO_TARGET_DIR`, build outputs or runtime DBs; do not create five repositories in advance for this plan.
2. What runs in parallel by default is headless feature development and testing that meets the start conditions. The core uses an explicit `OrganizeService::start(temp_path)`, reusing the existing tempfile fixtures; IPC uses `app_builder(mock_builder(), service)`. Each test run's app-data, authorized synthetic roots, snapshot/output and log folders are separate; app-data and roots do not contain each other, and the common ancestor of all run folders is not authorized.
3. Only one full Tauri dev **or** packaged app is open at a time. A lane that needs the UI claims the slot in #65's run register (lane, commit, PID, port, data folder, root, return status); Claude-A coordinates, and Codex-R has priority during independent acceptance. When finished, verify that the processes you hold have exited and that 1420 is released; do not kill other sessions' processes.
4. Every full UI run uses `TRAEVA_APP_DATA_DIR` to point at this session's own test folder (see item 5); never try a migration on the user's default database. Gate 2 first does an offline review against the locked baseline; a real window may use the default folder of a prepared dedicated macOS test account, or a default data folder that the student confirmed is reserved for synthetic acceptance, used exclusively by R and not used by other branches. Without such an environment, explicitly record the window part as BLOCKED, coordinate follow-up verification in #65, and do not merge new features into the baseline under review. Never change `HOME`, delete the lock or empty a personal DB to assemble an environment; this plan does not require creating a system account now.
5. **app-data development override (implemented in #65)**:
   - dev: `TRAEVA_APP_DATA_DIR=/absolute/empty/folder npm run tauri dev`.
   - packaged `.app`: `open -n --env TRAEVA_APP_DATA_DIR=/absolute/empty/folder target/release/bundle/macos/Traeva.app`.
   - The commands are also in the README's [Running the project](../../README.md#running-the-project). Folder requirements: an absolute path, without `.`/`..`, not a symbolic link, a new folder, an empty folder or a test folder Traeva has used, not overlapping the default folder.
   - If it does not qualify, the window shows “Storage blocked” and the reason, opens no database, and does not fall back to the default database; the existing isolation, lock and initialization checks still apply.
   - The API key for a test folder uses a separate Keychain account `openai@test-<folder hash>`.
   - When this variable is not set, the folder and safety behavior of a normal launch are exactly the same as before.
6. Several concurrent desktop windows are not required, so 1420 does not need to change now. If two concurrent UIs are truly needed in the future, A first explains the need in #65, together with a minimal local override for Vite, devUrl and devCsp, and then verifies it; feature lanes must not change the shared port/CSP or loosen permissions on their own.
7. The real model/key is coordinated by the slot holder; other tests use a fake provider/key. Codex-C's browser tests use a separate temporary profile / download folder; the receiver endpoint matches the profile and explicitly connects only to that lane's instance, so the default browser never sends events to another lane.

Existing headless entry points (run in your own checkout, offline by default):

```bash
cargo test -p traeva-core
npm ci
npm run build
cargo test -p traeva-app --test ipc
```

Do not start `cargo test -- --ignored`, real-model routines, disk images or real browser download tests automatically in parallel. The build-cache download lock may cause waiting, but should not make different tests share app state.

### Resources each lane must keep separate

| Development lane | Separate resources | Full UI use |
|---|---|---|
| A | checkout/build, temporary DB, migration upgrade copies, watch roots, snapshot/evidence folders | claims the slot for #65 and for final integration |
| B | checkout/build, read-only test copies of real snapshots, analysis/retrieval DB, fake server, logs | claims the slot for #35 wiring/acceptance |
| C | checkout/build, Source DB, Chrome profile, download root, receiver endpoint, logs | claims the slot for the end-to-end source demonstration |
| D | checkout/build, Word synthetic file folder, parsing fixtures, snapshot copies, logs | the Reader does not open Tauri by default; real Word tests use its own files; claims the slot only for Word Memory integration |
| R | checkout/build of the exact review commit, separate synthetic inputs, DB, evidence folder | claims the slot for Gate/final re-verification; does not use the implementer's DB or #22's sealed material |

### #65's environment exit conditions (tested 10/9, final check 10/10)

Code tested: `3691c30` of PR #77 (`TRAEVA_APP_DATA_DIR`), merged as `ddfb16d`. Only synthetic folders were used; before and after each test the hash of the default app-data was checked read-only, and it was the same before and after. The full local evidence is not included in this source package; public results are in [#65 issuecomment-6073419299](https://github.com/P30g53/Traeva/issues/65#issuecomment-6073419299).

| Condition | Result | Evidence |
|---|---|---|
| Record each lane's actual paths/launch method and the owner of shared files | PASS (method in “per-lane registration” below): the owner of shared files and each lane's own paths are written in the [shared contract](shared-contract.md) §1/§2. For every full UI run, each lane writes the holder, commit, launch method and its own app-data/root in its #65 UI slot claim, and on return writes the state of the processes and of 1420. The concrete folders on the machine are not shared outside (C's [handover addendum](https://github.com/P30g53/Traeva/issues/65#issuecomment-6093834850)) | #65's claim/return comments, 20-plus rounds over 10/9–10/10 |
| Two headless paths run at the same time in different OS processes, unique canaries do not cross databases, independent restarts do not affect each other | PASS: two core processes (PID 4607, 4608) cross-queried 0/0; with one stopped, the other kept capturing; after a restart it picked up the original watch | `m65/multiprocess/` |
| A second instance on the same app-data is still refused; separate folders each work; root/app-data do not overlap; a failed migration deletes no database and changes no old migration | PASS: a third headless process and a second packaged-app window both report `another_instance_running`; the test for rollback on a failed migration has always been in `cargo test`; 0001–0009 append only | `m65/multiprocess/`, `m65/override/round2/`, `tests/schema.rs` |
| Development path override: valid/invalid paths, same-folder collision, persistence across restart, default folder untouched; no fallback to the default database on error | PASS: two rounds on the packaged `.app`, 8 kinds of invalid values (including firmlink, case, `traeva.lock` in a personal folder); invalid values show “Storage blocked” and open no database | `m65/override/` |
| UI slot handover: the old process exits, 1420 is released, the next instance opens only its own DB | PASS: `m65/handover/` (dev1 → exit → 1420 released → dev2 cannot see dev1's root); afterwards A/B/C/D/R did 20-plus real handovers, all recording the PID exit and the 1420 state | `m65/handover/`, #65 comments |
| Fake provider/key by default; Keychain and send permission do not change each other | PASS: a test folder uses a separate Keychain account `openai@test-<path hash>` (contract §8), and the default `openai` item's attributes were unchanged before and after the test; send permission is in each app-data's `model-settings.json` | `m65/override/`, unit tests |
| C's profile / download folder / receiver point only to this lane | PASS, recorded in #27: [C's real test delegated by the user](https://github.com/P30g53/Traeva/issues/27#issuecomment-6091431126), [R's independent re-verification](https://github.com/P30g53/Traeva/issues/27#issuecomment-6090760642) | #27 |
| Record commit, commands, paths, expected/actual and conclusion, keeping service rebuild, abnormal exit and the real App separate | PASS: see the test comments above; O18-R and O18-C are recorded separately in Gate 3 | #65, #23 |

**Incident record (kept; it does not affect the conclusion about the isolation mechanism, but the default database really was changed)**: during the Gate 3 verification on 10/10, R1 reused the screenshot helper from the previous round, and the helper, following an old binding, launched another old-version app (PID 5091) **without** `TRAEVA_APP_DATA_DIR`. This app opened the student's default database: the default DB went from 184320 bytes to 499712 bytes, and its timestamp changed too; the default settings file did not change. R did not read, edit or roll back the default database, and stopped only that wrongly launched process ([record](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101187666), [corrected attribution](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101247999)).
- The cause was the test tool reusing an old binding, not an isolation defect in the product: the process launched explicitly with `open -n --env` was shown by PID to have opened only its own database; using the default folder without the variable is the behavior the official release requires.
- R then redid it with a safe method ([A's suggestion](https://github.com/P30g53/Traeva/issues/23#issuecomment-6101465104)), and at Gate 3 PASS checked that the default app-data metadata had not changed ([#23](https://github.com/P30g53/Traeva/issues/23#issuecomment-6102345052)).
- How to handle this change to the default database (keep it, inspect it or restore it from a backup) is for the student to decide.

**Rules added as a result**: the full UI may only be launched with `open -n --env TRAEVA_APP_DATA_DIR=<this lane's empty folder> <the .app from this build>` or `TRAEVA_APP_DATA_DIR=… npm run tauri dev`.
- Before each observation, check by PID that its executable path is the one from this build, and confirm with `lsof` that it opened only its own DB/lock; take screenshots by the window ID of this PID.
- Do not let desktop automation tools, screenshot helpers or a bundle ID “open” or “activate” Traeva: they may launch an older registered bundle, which uses the default folder.
- Use a new output folder each round; do not reuse the previous round's tool objects or bindings.
- When stronger protection is needed, use a separate macOS test account.

## Shared files, PRs and integration

Claude-A is the single integrator of shared DTOs/migrations/registration/configuration; the exact file list, each lane's own paths and the cross-lane data contracts are in the [shared contract](shared-contract.md). For current Find and Memory work, the student supplies the core changes and agents supply only the permitted wiring, tests and review. Other lanes submit their assigned feature modules and tests; A coordinates shared diffs. Migrations are appended after main's current last number, and no committed migration is changed; A verifies a new database and an upgrade of an old one. Cargo.lock conflicts are regenerated by A from the approved dependencies and the diff is checked; never pick ours/theirs for the whole file.

Feature-First stays unchanged: the existing Issues track acceptance, and agents arrange internal functions/modules on their own; a few milestone PRs, no Issue for each internal step. Before merging, resolve conflicts on top of the latest main; branches already shared/under review keep their history, prefer merging main, and do not force-push. The original parallel plan assigned capture to A, semantic changes to B, Source to C and Readers to D after the shared interface PR merged; semantic-change acceptance required real persisted snapshots. Those historical lane assignments are superseded for current Find and Memory work by the assignment above. #37 brings them together after the original prerequisites are complete; D's conditional results must not be inserted into the core's required path.

Each PR records intermediate results with `Refs #N`; only when the whole Issue is satisfied does it say `Closes #N`, and only the student merges. Gates never use any link that would close automatically. GitHub's [Development link](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/linking-a-pull-request-to-an-issue) may close an Issue on merge; an intermediate PR must not be set up as a closing relation just to show it in the Linked pull requests column. Intermediate PRs' native cross-references stay in the timeline, and their URL and status also go into the Project's Next Action; the final closing PR checks the Linked pull requests field.

Every real piece of progress also updates the Project's Status / Next Action (including the active PR and the current blocker) and the necessary dates and evidence links. Target Date is not a promised completion date; an intermediate documentation PR does not prove that #65's isolation is complete, and #18 Done does not prove #19 PASS. A PR that does not exist stays blank; links must never be faked.

## Minimal student material

#31's material was Accepted on 2026-10-08 ([material](https://github.com/P30g53/Traeva/issues/31#issuecomment-6074393105), [acceptance record](https://github.com/P30g53/Traeva/issues/31#issuecomment-6074400661)); its constraints on other lanes are written in §4.2b and §4.3 of the [shared contract](shared-contract.md) v6. #34's material was also Accepted on 2026-10-08 ([material and acceptance record](https://github.com/P30g53/Traeva/issues/34#issuecomment-6074761739)). #27 was still Todo as of the same day. One line per question is enough, and the student may replace the example wording with their own answers; agents may only point out missing items, and must never choose an algorithm for the student or mark it Accepted.

| Issue | What the student needs to decide | Preparation agents can do now |
|---|---|---|
| #31 A | family membership and the branch rules for Save As/copies; how continuous rename/move evidence carries over; the candidate and confirmation flow for replacement/insufficient evidence; how forks or unclear order are represented | read the FileRecord/evidence interfaces, list real save scenarios, check #65; must not write default automatic merge rules first |
| #34 A | before/after/last boundaries; how repeated changes list candidates/ask questions; not happened vs not analyzed/incomplete history; the user's next step after cancelling/still ambiguous | inventory the snapshot/change contracts, list input/output test tables; must not choose result priorities for the student |
| #27 B | minimal display fields and the direct evidence for each field; the difference between acquisition/import time; association/candidate/unknown constraints for rename/move, repeated downloads and copy-delete | read the Chrome API/existing Lab conclusions, confirm profile/endpoint isolation, list gaps in the receiver contract; must not guess titles or associate wrongly by default |

The three Issue bodies provide question sheets; after submission they are first marked Submitted; the owner checks that the material is enough to implement and meets the established safety thresholds, then records Accepted and the material link. Student material can be prepared at the same time, without waiting for upstream code to finish. The #22 sealed evaluation is still in Production scope and is not unsealed early to hurry for CAC.

## Timeline and priority path

| 2026 date | Target checkpoint, not a completed fact |
|---|---|
| 10/9 | #65 Step 0; #19 independent review starts |
| 10/13 | basic Memory snapshot, real Source evidence, DOCX Go/No-Go |
| 10/15–18 | real snapshots connected to Intelligence / UI, Source integration, conditional Word verification |
| 10/17–19 | R reviews usable increments of #26 independently on a rolling basis; findings go straight to the implementer for fixing, aiming to finish the key fixes before 10/20 |
| 10/20 | Feature Freeze; after that only defects that affect the submission are fixed |
| 10/22–24 | independent re-verification, packaged app, video, AI disclosure / submission materials |
| 10/25 | aim for the student to submit early |
| 10/26 | official deadline 12:00 PM ET / 09:00 AM PT |

Final integration still has to connect #31 family → #32 real capture → #33 semantic changes → #34 retrieval → #35 UI → #26 acceptance → #37; this evidence chain does not require each item to close before development of the next one starts. The #32 foundation can be done first, and #33 can run in parallel after the shared interface PR merges; the existing prerequisites of #34/#35 are kept. R reviews already usable increments on 10/17–19, starting without waiting for all sub-items to close; final completion of #26 still requires acceptance of all sub-items, and the final re-verification after integration is kept for 10/22–24. #19, #27 and #38 flow into #37 in parallel, and #13's safety prerequisite is unchanged. The current start gap is the shared interface PR and the corresponding student material; real testing of desktop isolation still has to be done, but does not block compliant headless development. If D has not proven a bounded Reader and real Word behavior by 10/13, record No-Go/partial evidence and stop CAC Word integration; the core chain is not postponed to make way for Word.
