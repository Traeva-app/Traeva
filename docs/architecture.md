# Traeva — Project architecture

Design baseline: **v0.3** · 2026-09-30. This overview includes the approved Organize revisions through **v0.18** and the current Find and Memory scope. The baseline is retained for traceability; implementation is present in this source package. Test results apply only to the versions and checks recorded in the [acceptance record](https://github.com/P30g53/Traeva/issues/37).

Companion documents: [Organize spec](features/organize-spec.md) · [Implementation plan](features/organize-plan.md) · [Shared contract](development/shared-contract.md). The spec defines behavior and records approved revisions; the plan preserves the original task order and completion thresholds. See the [README](../README.md) for the current feature overview and setup.

## 1. Frozen choices and scope

| Part | Decision |
|---|---|
| Desktop and UI | Tauri 2 + React/TypeScript + Vite, macOS first |
| Local core | Separate Rust crate that does not depend on Tauri; all file operations and database writes belong to the core |
| Persistence | SQLite/rusqlite; one write connection, explicit SQL migrations |
| Style | Plain structs, enums, functions and a few modules; simple and explainable, no general plugin/agent framework |
| Current Organize scope | Local APFS, same volume, one file per operation; TXT/Markdown, Office text, PDF text layer, and Organize-only OCR or separately authorized image input; at most three clarification questions, one at a time; preview and confirmation in the normal flow, conditional automatic execution for active drops (Organize spec §13), durable records and conservative Undo |
| Original build plan | Deterministic safe executor and manual end-to-end loop first, then AI recommendation and clarification; later approved revisions extend this baseline |
| Current responsibilities | Follow [AGENTS.md §4](../AGENTS.md#4-session-responsibilities-and-shared-files); the student owns current Find and Memory core work, agents perform assigned integration and review, and independent reviewers verify changes. Individual contributions are described in the [README](../README.md#my-contributions-and-ai-assistance). |

The original Organize slice deferred Memory and watcher work. The current CAC product includes Find, Version Memory, Source Memory, and automatic version recording; their contracts and verification are separate from the Organize Gates. Learning, a general analysis cache, complex identity inference, OCR outside the approved Organize scope, and other platforms remain deferred. Current scope and assignments are recorded in [AGENTS.md](../AGENTS.md); historical implementation and review attribution remains in the original commits and reports.

## 2. Layers and interfaces

```text
React / TypeScript: rendering, collecting the user's choices, polling
  → Tauri commands: input parsing, window permissions, native pickers, background scheduling
    → traeva-core: observation, classification, plans, execution, journal, recovery, Undo, Find and Memory
      → rusqlite + macOS fd-relative file-system primitives
```

The frontend has no direct file/SQL/model access, and does not decide by itself from confidence whether to ask a clarification question or whether a plan can be confirmed. The command layer has no business SQL, move or classification rules. The core validates IDs, authorization, file observations, directory chains, candidates and operation state; a Tauri capability cannot replace business authorization.

IPC uses Tauri `Result<T, AppError>`, which maps to Promise resolve/reject; the custom version/ok/request envelope is removed. Data types use Rust structs/enums with TS mirrors, and unknown fields and invalid values are rejected. The database version is managed by migrations / `PRAGMA user_version`; schema_version is not added again to every object; a change to a persisted JSON structure must also go through a migration.

Each window has its own restricted capability: the main window, desktop pet, quick-search window, and ask window can call only their assigned commands. No remote page that can reach IPC is loaded, and no broad fs/sql/shell plugin permissions are opened. The [Tauri command](https://v2.tauri.app/develop/calling-rust/) and [capability](https://v2.tauri.app/security/capabilities/) boundaries remain part of the design.

## 3. Minimal code locations and runtime model

The following map shows representative Organize locations, not the entire current codebase. Small files may be merged; no trait/service layer is added for each module:

```text
src/features/organize/       # pages, api.ts, types.ts
src-tauri/src/lib.rs         # startup and holding the core
src-tauri/src/commands.rs    # thin commands and native pickers
src-tauri/capabilities/      # permissions for each window
crates/traeva-core/src/
  lib.rs                    # OrganizeService
  contracts.rs              # data and errors
  observe.rs                # registration, reading, hashing; text extraction as unit-testable pure functions
  classify.rs               # limited context, one model adapter, validation / bounded clarification
  plan.rs                   # immutable plans, current validity
  execute.rs                # executor shared by Organize/Undo
  recovery.rs               # record reconciliation
  store.rs                  # SQL, transactions
  platform/macos.rs         # small rustix / fd-relative wrappers
crates/traeva-core/migrations/
crates/traeva-core/tests/
```

Find and Memory include `find.rs`, `changes.rs`, `retrieval.rs`, `lineage.rs`, snapshot and source modules, plus their Tauri commands and UI. See the [shared contract](development/shared-contract.md) for the module inventory and [README](../README.md#my-contributions-and-ai-assistance) for authorship of the four student-written current source files.

Single instance, single-file flow, local changes run one at a time. Plain synchronous core functions are preferred; slow reads, model requests and blocking pickers run through Tauri background scheduling and must not block the UI thread. Narrow interfaces are used only at real test substitution points; no new async framework is built. Waiting for the model holds neither the execution lock nor a DB lock; the execution lock is taken before the short-lived DB lock, and no database transaction is held during file operations.

Reading/analysis uses an in-memory operation/generation and a cancel flag; polling runs about every 300 ms and stops at a terminal state. A confirmed action is queried by its persisted action_id; closing the window or a request timeout does not mean it was cancelled. A restart interrupts analysis and invalidates unconsumed plans; unfinished actions are reconciled first, and only then are new operations that do not conflict with them allowed.

The app holds a process lock in app-data; a second instance does not write the database or run organizing. Traeva's lock coordinates only Traeva itself; it does not isolate external editors.

## 4. Filesystem safety invariants

### Authorization and containment

An authorized root only distinguishes active/revoked; the permission bit set and the root revision are removed. A root_id is bound to that particular authorization and to the directory's physical evidence. Revoking permanently invalidates the related unconsumed plans; re-authorizing creates a new root_id, and old confirmations cannot come back to life.

**Authorized-root containment must be descriptor-anchored.** Starting from the directory handle of the verified authorized root, resolve one relative path component at a time, do not follow symlinks, verify the directory object at each level, and after the move re-verify the chain and the state at both ends. canonicalize may help with display / relative path derivation; canonicalize + starts_with must not be used as the final execution boundary.

The platform layer prefers `rustix` fd-relative primitives and OwnedFd, with a small wrapper for open/observe, level-by-level checks, mkdir, non-overwriting move and sync. The final move must have atomic no-replace semantics, which on macOS is `renameatx_np + RENAME_EXCL`. Checking the target in advance is for preview only; check-then-plain rename is forbidden, and there is no overwrite or copy-delete fallback.

The minimal supported scope is still local APFS same volume, regular single-link files. Following the existing consensus, local volumes, cloud placeholder files and bundles/packages are explicitly identified; the exact checks and naming rules are in the spec. Support for and behavior of native calls are accepted on the production Rust stack; success of a C probe or of the Lab must not be counted directly as a production PASS.

Directory handles and after-the-fact verification are not an atomic transaction over the whole directory tree. A race still exists when an external process moves an ancestor at the same time; a detected change goes to needs_user_decision: it is not reported as success, it is not moved back automatically, and there is no claim that every momentary escape out of bounds can be prevented.

### Zero writes before confirmation

**Before confirmation, Traeva performs no filesystem writes of any kind inside authorized roots.** No directories, temporary/probe files or hidden metadata are created; no content, name, location, permission or extended attribute is changed. Read-only observation, extraction, analysis and preview all follow this rule; a probe must not be used to test writability. In the active-drop auto-organize mode (v0.4, organize-spec §13), “confirmation” is the moment the core decides that all auto conditions are met and execution starts, based on a valid auto-mode agreement; before that moment there are likewise zero writes.

All internal writes are under `~/Library/Application Support/Traeva/`, and the default database is `traeva.sqlite3`. **app-data and all authorized roots must not overlap in either direction**: a root must not contain app-data, and must not be inside app-data; the actual directory relationship is checked at authorization, at startup and before execution, and when the relationship is unclear the operation is blocked.

zero-write means Traeva starts no writes; it does not claim to freeze the operating system. Isolation tests compare the tree's paths, types, contents, permissions, extended attributes and so on; access times may change because of reads and should be excluded. Changes by Finder / sync services / other processes need their source told apart; excluding them must not be used to hide Traeva's own writes.

## 5. Data and transactions

rusqlite with bundled SQLite; one write connection, keeping `foreign_keys=ON`, `journal_mode=DELETE`, `synchronous=EXTRA`, macOS `fullfsync=ON`, and checking the actual settings. Migration, corruption, disk-full or higher-schema errors all block new operations; the data is kept, and the database is not deleted and rebuilt. SQLite transactions do not cover file moves. [SQLite synchronous notes](https://www.sqlite.org/pragma.html#pragma_synchronous)

The original Organize persistent set consists of authorized_roots, files, analyses, plans, actions, action_events, and path_history. Later append-only migrations add the Find, Memory and Source structures described in the shared contract. No decisions learning lifecycle or general analysis cache is built. files stores identity, location and the latest observation; feature tables reference file_id.

**The execution content of an ActionPlan is immutable.** When the user changes the target or retries after a failure, a new plan_id is generated and confirmed again (in auto mode a failure is not retried automatically; trying again needs a new drop or the normal flow, organize-spec §13); plan revision/digest is removed. Cancelled, superseded, invalidated and consumed are separate lifecycle states and must not rewrite the source/target/preconditions in the plan. Confirmation passes only the plan_id; the core loads its persisted content and checks it again.

The database enforces `actions.plan_id NOT NULL UNIQUE`: **one plan → at most one action**. The confirmation transaction checks that the plan is usable, consumes the plan atomically, and inserts the action and the initial journal; if the transaction does not succeed, the authorized root must not be modified. Repeated clicks/requests, failure or recovery only return that action and do not create a second one. Undo of the same original operation uses a new reverse plan/action.

Every step with a side effect first commits the intent, then does the file operation, then records the result/receipt; no long transaction is held across system calls. The success transaction updates the file location/observation, PathHistory and action committed together; a successful Undo commits the reverse action, the original action undone and the path history together. The journal's `(action_id,sequence)` and PathHistory's `(action_id,reason)` are unique; one original action has at most one successful Undo. The current present path must be unique; physical evidence must not serve as a permanent file_id.

## 6. Recovery and Undo

**Recovery only reconciles records; no automatic rollback of user files.** Restart recovery does not move, rename, overwrite or delete user files, and does not replay moves that have not run yet.

- Source as expected, target absent: record failed_no_move; no automatic retry.
- Source absent, and the target's directory chain/physical evidence/hash all match the journal: idempotently complete the commit of the original action as committed, and record recovered.
- Any contradiction, missing item or insufficient evidence: needs_user_decision; keep the known locations; block conflicting new operations.

The only filesystem cleanup exception: a directory Traeva created itself that has a clear receipt, whose identity and authorization chain still match, and that is still empty. Without a receipt it is not claimed; if `.DS_Store` exists, the directory is kept, that file is not deleted, and a file restore already verified as successful is not blocked. The cleanup intent/result goes into the journal; when it cannot be recorded, the directory is kept.

Undo stays conservative: the current object, location and hash must match the original commit; the original parent directory must still be the expected object, the original name must be free, and the authorization must be valid. Edits, replacement, moving out or conflicts are all refused. A valid Undo first previews the reverse plan, then is explicitly confirmed, and uses the same executor; it does not restore old content and does not automatically look for another location.

## 7. AI and persistence boundaries

Organize extracts TXT/Markdown, supported text from DOCX/PPTX/XLSX, and the PDF text layer. On macOS 13 and later it can also recognize text from eligible scanned PDFs and JPG/JPEG/PNG images; separately authorized, reduced image copies may be sent to the model (Organize spec §14.8–§14.10). PDF parsing and OCR run in separate subprocesses; OCR uses a monitored soft memory budget, not a guaranteed process-wide hard cap. When neither usable text nor permitted image input is available, Organize falls back to manual pick. Find uses supported body text without OCR; Memory supports TXT/Markdown and the three Office formats, but not PDF or images. Each feature has its own sending permissions. The model has limited context and no file tools. Organize allows at most three clarification questions, one at a time (Organize spec §15); if the result is still unclear after the third answer, it goes to manual pick.

By default, the following are stored long-term: provider/model, time, prompt/schema/extractor/app version, file hash, request metadata that is not body text, the limited structured result, the classification suggestion, the user's correction/final choice and the execution result. By default, the full extracted body text and any raw prompt containing body text are not stored long-term; the full request is not written to disk under the name of “saving the request”. The limited explanation should not become an archiving channel for the model echoing the body text.

Diagnostic records may contain extra content only when explicitly turned on, and are kept separate from the production default behavior; keys never go into logs. The Keychain is still accessed by the core; the frontend does not persist the key. This policy can trace the run conditions; it cannot guarantee a complete replay or a word-for-word model reproduction without the original text.

## 8. Organize milestones and acceptance evidence

The criteria below describe the Organize milestones. Memory has separate verification, and final product acceptance is recorded in [#37](https://github.com/P30g53/Traeva/issues/37). These criteria do not themselves assert that a particular build passed.

**CAC Demo Milestone**: the key safety invariants have automated tests, the four recovery-logic checkpoints pass, a real macOS window completes two real-model flows and an Undo after restart, and at least one verification uses the packaged `.app`. It does not require deferred features or all production counterexamples to pass.

**Production Slice Complete Milestone**: the Demo has passed; the full counterexamples of the current scope, the independent semantic evaluation and Codex's independent tests pass; for a few key checkpoints, recovery succeeds after a child process is truly terminated abnormally. service recreation / same-DB reopen is explicitly labeled recovery-logic test, and abnormal termination such as `abort()` is labeled abnormal-process crash test; the two are not equivalent, and neither is equivalent to a whole-machine power loss. No large fault-injection framework is built.

The accepted architecture and safety invariants remain binding. Later changes follow the approved revisions in the Organize spec. Current implementation responsibilities follow AGENTS.md; this overview does not replace historical contribution records or independent test reports.

**Frozen baseline with approved revisions; acceptance is tracked separately.**
