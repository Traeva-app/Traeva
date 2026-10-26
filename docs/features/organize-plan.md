# Traeva Organize — Implementation and acceptance plan

> Historical v0.3 implementation plan. The milestones, deferred scope, owners and statements about work not yet started below describe the original planning stage. The product has since implemented Memory and additional formats. Current behavior follows the approved revisions in [Organize rules](organize-spec.md), the [README](../../README.md), and the current assignment in [AGENTS.md](../../AGENTS.md#4-session-responsibilities-and-shared-files). This plan is retained as development history, not as a list of work still awaiting approval.

Version: **v0.3** · 2026-09-30. **The consensus is accepted and the architecture is frozen; this round only updates documents, and production coding waits for the user's approval to begin.**

Behavior contract: [organize-spec.md](organize-spec.md). Layers/storage: [architecture.md](../architecture.md). The user and Opus implement; Codex does independent code review and testing; chats are not created or handed over automatically, and the architecture review does not continue.

## 1. Frozen scope and execution order

Tauri 2 + React/TypeScript + Vite + Rust + SQLite/rusqlite; macOS, local APFS same volume, single file, TXT/Markdown, one real model service, at most one clarification, exact preview, safe execution, durable records, conservative Undo.

**First the deterministic safe executor + the manual organize end-to-end loop, then AI recommendation and one clarification question.** All rules are in the separate core; commands are thin, and the frontend only expresses choices/renders/polls. No extra architecture or general platform.

learning, cache, Source/Version Memory, watcher, complex identity inference, later formats and the like stay on the long-term roadmap; all are deferred, are not part of this slice's delivery, and no placeholder services are created for them. O15/O16/O27/O28 of v0.2 are marked DEFERRED: their history is not deleted and they are not recorded as PASS; the other numbers are kept, and O18 is split into O18-R and O18-C.

This round produces only the existing three v0.3 documents; later implementation is judged separately at M1's CAC Demo Milestone and at M2's Production Slice Complete Milestone.

## 2. The first production coding task: M0 minimal runnable skeleton and persistence entry point

**Precondition: the user explicitly approves the start. Owners: the user and Opus.** This is the next coding task, not something implemented in this round.

### Task content

1. Following the architecture document, create the Tauri/React/Vite shell and the separate `crates/traeva-core`, and set up the real dependencies and lockfile; keep the core free of any tauri dependency. Existing Lab/audit material stays reference only; it is not mixed into production source or changed along the way.
2. The core sets up the app-data directory, the process lock, the rusqlite connection, the initial SQL migration and the PRAGMA checks; a failed migration / higher schema / held lock is refused explicitly, and the database is not deleted and rebuilt. The database contains the minimal tables and `actions.plan_id NOT NULL UNIQUE`.
3. Implement one thin `get_app_state` command; the page shows the real storage state; the normal and failure cases are decided by the core. The command is restricted by the main-window capability/CSP; the frontend has no direct fs/sql/shell access.
4. Set up a standalone test entry point for the core; reading/writing real user directories, file moves, model calls and the full UI are not part of this skeleton task.

### Completion criteria

- `cargo test -p traeva-core` runs without starting a Tauri window.
- The real window shows, through get_app_state, that the database is available, or an understandable storage error.
- PRAGMA/migration version, the second-instance lock and an unusable DB path each have minimal automated verification; failures are not hidden.
- core has no Tauri dependency; commands have no business SQL/file operations; no deferred feature is installed/implemented.
- Submit reproducible run steps, the code version/working tree state, the actual tool versions and the results already run, for Codex to re-check independently.

M0 does not re-choose the stack, run the Lab or set up a multi-model platform. Missing API configuration does not block this skeleton task.

## 3. M1-A: platform primitives, observation and immutable manual plans

### Small platform module

Implement around the chosen rustix/fd-relative primitives: open and verify the authorized-root handle, per-component no-symlink checks, observe physical evidence, mkdir, atomic no-replace move and directory sync. Keep it a small module and do not build a filesystem framework; if a very small native wrapper is needed, explain its safety preconditions in one place only and do not spread it into the command layer.

Check local APFS/same volume, regular single link, SF_DATALESS placeholders, projects/bundles and name boundaries. canonicalize is only an aid, not the final containment guarantee. Re-check the chain after the move; when an external race leaves the outcome uncertain, needs_user_decision. Platform tests run on the production Rust stack; the C probe in the report is not marked PASS directly.

### Zero-write and authorization isolation

Authorization only distinguishes active/revoked; re-authorizing uses a new root_id, and revoked plans do not come back to life. Refuse both kinds of overlap: a root containing app-data and a root inside app-data. Registration, extraction, classification and preview write only to app-data, and create no probes/temporary files/hidden metadata in a root.

Isolated fixtures take a tree snapshot before authorization / after reading / after preview / after cancel: compare paths, types, bytes/hash, permissions, xattr and modification info; exclude the atime changed by reading, and tell external writes apart. CI and local tests must never use personal files as material.

### Observation and plans

Per spec §4, keep 20 MiB, 8,000 characters and the existing A/B read budget; extraction is a pure function. Register a UUID; a repeated direct observation may reuse it, and copies are not merged; an external change invalidates old plans; no complex identity inference is implemented.

build_plan produces a persisted immutable payload, and the preview shows the full target and every directory to be created; a new target produces a new plan_id and invalidates the old plan. plan revision/digest is removed; expired/can_confirm are derived results. Choosing the current location is a no-op and creates no action.

**Completion criteria:** O01, the preview stage of O05/O06, O10–O12, and the related parts of O26/O29–O32; zero writes in roots before confirmation, app-data isolated in both directions, and a valid plan can be read back but cannot be changed in place into another action.

## 4. M1-B: deterministic safe executor, record-reconciling recovery, conservative Undo

### Execution

apply_plan takes only a plan_id: first look up an existing action; if there is none, re-check the immutable plan under the execution lock, and in a short transaction consume the plan and insert the single action/initial journal. `actions.plan_id NOT NULL UNIQUE` is the backstop at the DB layer; a failed transaction has zero side effects in roots. Double clicks/UI retry/failure/recovery must not create a second action for the same plan; after a failure, the user trying again needs a new plan and a new confirmation.

Each step is durable intent → fd-relative side effect → verification/result/receipt; the no-replace move is used only after the full source hash is re-checked. The early exists check serves only the preview; the system call itself must refuse a target that becomes occupied late. File operations/network waits hold no DB transaction; one final transaction commits FileRecord, PathHistory and the action state. The frontend does not show an accepted state as completed.

### Recovery only reconciles records

At startup, for each unfinished action, check the journal and the actual source/target/directory chain/hash/physical evidence: source matches and target missing → failed_no_move; source missing and target fully matching → the original action's commit is completed as committed/recovered; otherwise needs_user_decision. inode alone is not enough, and “the intent exists” must not be taken to mean it was executed.

Automatic recovery does not move/rename/overwrite/delete user files, does not roll back automatically, and does not continue the remaining moves. The only cleanup exception is a self-created directory that has a receipt, the same identity/a valid authorization chain, and is still empty; cleanup also records intent/result. With no receipt, when it is not empty, or when .DS_Store exists, the directory is kept, .DS_Store is not deleted, and a verified restore is not blocked.

### Undo

After checking the current object/location/hash, the original parent directory's identity, that the original name is free, and authorization, generate a separate reverse plan. After the user confirms, the shared executor is used; reverse action committed, original action undone and FileRecord/PathHistory are in the same transaction. Edited, replaced/moved out, or a conflict at the original location are all refused; old content is not restored, and another location is not quietly chosen. A cleanup failure only shows a warning.

**Completion criteria:** a real synthetic file: manual pick → create two directory levels → move → restart → Undo, with bytes/hash/UUID identical; the basic safety paths of O07–O09/O17/O19–O24/O34/O35 pass. O18-R passes at the four checkpoints; AI is connected only after this stage is complete.

## 5. M1-C: real AI, one clarification question and the full page

One configured provider API; every analysis makes a new request; no cache or learning hints are implemented. The request contains only the current limited content, location/directory candidates and this time's answer, with no Source/historical preferences. With missing configuration or a service failure, manual pick is available; real-AI acceptance is marked BLOCKED rather than passed with a mock.

Strict DTO/ID/path/length/locked-answer validation; recommendation/clarification question/can_confirm are all decided by the core. **At most one clarification**; after the answer, the model is called once more; if it is still unclear, it goes to manual pick. Network waits hold no lock; the generation blocks late results; cancel/source change must not publish an old executable preview.

Store long-term only what spec §5 allows: request metadata, the limited structured result, the limited classification reason, user correction/final choice and the execution result; body text/raw prompt are not stored. Explicit development diagnostics are kept separate from the production default; keys never go into logs. Core automated tests do not use the real Keychain; real-model/.app verification uses explicitly configured credentials.

A single page connects selection, read-only observation, analysis, one question, preview, the processing result and Undo. Reading/analysis polls the operation, a confirmed execution polls the action, and polling stops at a terminal state; a refresh reads get_app_state and does not resend the confirmation. The success shown in the window matches the actual file/DB state.

## 6. CAC Demo Milestone

It is marked **CAC Demo Milestone PASS** only when all of the following conditions hold at the same time:

1. The required safety groups have automated tests: zero writes in roots before confirmation/app-data isolated in both directions, atomic no-replace (including late occupation), stale on content/identity change, fd chain/symlink/out-of-bounds refusal, a single action per plan, intent written to disk first, conservative Undo and conflicts, no move on an invalid model result, injection in the body text treated only as data.
2. The four recovery checkpoints complete the **O18-R recovery-logic test**: after mkdir, before move, after move before commit, after Undo move before commit. Use service recreation / same-DB reopen, and check the recovery decision and that files are not automatically rolled back.
3. Two real-model flows: a recommendation for a clear file that was not used for prompt debugging, and a file that asks only one necessary question; both go through the same safe executor, and the files and records can be checked.
4. A real macOS window goes through the full flow, and Undo succeeds after a restart; at least once, the packaged `.app` verifies picker, permission and credential behavior. A development window/browser mock cannot replace this verification by itself.

The Demo does not require O18-C or all production counterexamples / the independent 7-case semantic evaluation to be complete; items not yet passed are recorded truthfully, and a Demo PASS must not be called Production Slice Complete. An empty directory kept because of `.DS_Store` is only a warning.

## 7. M2 — Production Slice Complete Milestone

The user and Opus submit the implementation version, how to run it, a configuration example and the Demo evidence; Codex independently reviews the code and runs all acceptance for the spec's current scope, without starting a new architecture/technology selection. A blocker found goes to the implementer for a fix at the exact point; the original FAIL is kept, and it is re-verified.

### Current acceptance scope

Keep the O numbers of spec §10; O15/O16/O27/O28 are DEFERRED and PASS is not required. The rest cover identity/plans, the read-only stage, paths/authorization, real execution, persistence, extraction, IPC, concurrency, privacy and conservative Undo; O18-R and O18-C report results separately. Each complex number may be split into sub-cases; a process exit code / the executor's self-report does not replace real file and DB assertions.

| checkpoint | O18-R: quick recovery-logic test | O18-C: real abnormal-termination test before production completion |
|---|---|---|
| after mkdir | After dropping the service, reopen the same DB; with the receipt missing, do not claim/delete | The child process aborts after mkdir succeeds and before the receipt is committed; a new process checks the leftover directory and the original file |
| before move | Intent already persisted, source matches and target missing → failed_no_move | Abort after the move intent is committed and before the syscall; restart does not move automatically |
| after move, before commit | Records are completed only when both ends/hash/physical/chain match | Abort after the syscall succeeds and before the success transaction; a new process idempotently completes the commit and does not move again |
| after Undo move, before commit | Check the original action and path records against the reverse plan | Abort after the Undo move and before the closing transaction; a new process completes the reverse commit only once |

No large fault-injection framework is needed: a few test-only checkpoints, ordinary core tests and controlled child processes are enough. `abort()`/abnormal termination must not be replaced by a normal Drop or panic unwinding. O18-R must not claim to be equivalent to kill -9; O18-C also does not claim verification of whole-machine power loss/hardware failure. Fault switches do not enter the normal user flow.

Extra counterexamples keep the existing requirements: partial mkdir, journal write failure, target taken first, directory chain replaced, same-length change with mtime restored, replacement by a same-content copy, commit succeeded but the UI did not receive it, hash mismatch afterwards, authorization revoke/re-authorize, DB corruption/higher schema/second instance; all follow record reconciliation, and no automatic rollback branch is added.

### Independent AI and persistence acceptance

The historical-preference conflict case among v0.2's eight cases is deferred together with learning; the current independent set is 7 cases: clear 3, single-level ambiguity 2, body-text injection but clear topic 1, insufficient information 1. Before the run, the allowed targets/clarification dimensions are fixed, and the provider/model/prompt/schema versions are kept; it must not pass by manual pick for everything or by changing the answers. Failures are kept; after a fix, the regression is kept and re-verified with samples not used for debugging. The results represent only that sample set.

Check that the production default DB/logs contain no body text/raw prompt/auth header; that the allowed metadata and limited structured result really are stored; and that the original request is not archived in disguise through other fields/files. Development diagnostics may be extended only when explicitly turned on, use synthetic data, and are cleaned up.

### Production Slice Complete conditions

- The CAC Demo Milestone has passed.
- All acceptance of the spec's current scope is PASS; DEFERRED and BLOCKED must not be passed off as PASS.
- The real O18-C for the few key checkpoints all pass; restarts/repeated reconciliation add no action and no duplicate PathHistory.
- The independent 7-case semantic evaluation meets expectations; the privacy defaults, the real .app and the core's standalone-run verification pass.
- Blockers from Codex's independent code review/testing are fixed by the implementer and re-verified; untested environments and concurrency limits are stated explicitly.

Once reached, this slice stops; no deferred features are added along the way; later scope is decided separately from the kept roadmap.

## 8. Verification entry points, evidence and start status

The implementer provides the usual entry points: `cargo test -p traeva-core`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`; frontend typecheck/test/build; `npm run tauri dev` and the steps to package the `.app`. Real-model evaluation is turned on explicitly; default offline tests send no API calls; production has no mock switch. This round does not create or run these production entry points.

Later acceptance results record the code version, environment, input hash, expected/actual, PASS/FAIL/BLOCKED/DEFERRED, the evidence path and the test kind. O18-R is labeled recovery-logic, O18-C is labeled abnormal-process crash; frozen documents do not mean verified code. All fault data is in isolated synthetic directories; evidence contains no personal body text/keys.

Overwrite, out-of-bounds access, root writes before confirmation, wrong identity merges, unrecorded moves, false success and wrongly deleted directories are all blockers; fix the current end-to-end loop first; new features must not be added to hide a failure.

This round, with the user's authorization, updated and froze the three v0.3 documents: no fourth explanatory document, no production coding, no research/Lab extension. The next step waits for the user's approval to begin; the user and Opus carry out the first task in §2, and Codex reviews and tests independently.

**ARCHITECTURE FROZEN — READY FOR IMPLEMENTATION**

## 9. M3 — Active-drop auto-organize (v0.4, the student's decision 2026-10-12, #168)

Rules in organize-spec §13. The normal organize flow and all safety requirements of M1/M2 are unchanged.

1. The spec revision is merged by the student; `AGENTS.md` is updated to match.
2. Migration `0010`: two tables, auto-mode agreement and drop records; the original seven tables are not changed; both an empty database and an upgrade from an old one must be tested.
3. Core: agreement and target scope; the auto conditions of §13.4; the drop queue (one at a time, paused on a clarification question, the rest “not processed” after a stop); drop records; calling the existing `apply_plan` with the agreement as its basis.
4. Tauri: receive the macOS open-file event, declare the file types it accepts (without becoming the default app for opening them), commands, permissions and TS mirrors.
5. UI: the settings switch and explanation, the authorization prompt for the source, the queue and results, the undo entry point.
6. Tests: A01–A12, plus the normal-flow regression of O01–O32; R independently re-verifies on the exact commit with the packaged App.

**Completion criteria:** A01–A12 have automated tests or real-App evidence; zero writes inside authorized folders before execution starts; no overwrite, no rename, no retry; the normal-flow regression passes; R's independent re-verification passes, and the student accepts.

## 10. M4 — Organize reads the PDF text layer (v0.5, the student's decision 2026-10-12, #186)

Rules in organize-spec §14. Memory does not read PDF; the normal organize flow and all safety requirements of M1–M3 are unchanged.

1. The spec revision is merged by the student; `AGENTS.md` is updated to match.
2. The PDF session moves the reader from `experiments/pdf-reader` into the core as a module that is not wired in yet, with normal test files; the dependency manifest and lockfile are integrated by A.
3. Core: parsing in a separate child process (bytes passed through a pipe, memory and time limits, failure goes to manual pick); connected only to Organize's read path, and Memory's read dispatch is unchanged; version 3 of the sending explanation; the shared contract.
4. Tauri: the child-process entry point; add pdf to the file type declaration (without becoming the default app for opening them).
5. UI: places that say TXT/Markdown are changed to include PDF.
6. R independently re-verifies P01–P09 with the packaged App, plus the normal-flow regression; R4 adds the resource tests under the child-process approach.
