# Traeva Organize — Behavior Specification

Design baseline: **v0.3** · 2026-09-30, with approved revisions through **v0.18**. This document states the current Organize rules and preserves their revision history in §12. Implementation is present in this source package; requirements and acceptance criteria are not evidence that a particular build passed. See the [acceptance record](https://github.com/P30g53/Traeva/issues/37) for version-specific results.

Companion documents: [Architecture](../architecture.md) · [Implementation plan](organize-plan.md) · [Shared contract](../development/shared-contract.md). The stack remains Tauri 2 + React/TypeScript + Vite + Rust + SQLite/rusqlite, macOS first. Core business logic is independent of Tauri; file operations and database writes belong to Rust. Current responsibilities follow [AGENTS.md §4](../../AGENTS.md#4-session-responsibilities-and-shared-files); individual contributions and AI assistance are described in the [README](../../README.md#my-contributions-and-ai-assistance). The original implementation plan and review records retain their historical attribution.

Approved later revisions take precedence over the original baseline. Current clarification is at most three questions (§15), Organize sending uses notice version 7 (v0.16), and OCR or image-assisted results may move automatically only under every condition of §13.4 (v0.14). Notice-version transitions and earlier restrictions in §12 are historical records. Updated acceptance rows below describe the current requirements; they do not alter earlier test results or expand an earlier PASS.

## 1. Goal, current scope, and deferred scope

In the normal flow, the user selects one real file. Traeva reads its content, considers its original location and the existing folders, and recommends a destination; when needed, it asks up to three clarification questions, one at a time (§15). It shows the exact plan; after the user confirms, it safely creates folders, moves the file, records durably, and can Undo after a restart. Active drops may execute automatically only under the separate agreement and all conditions in §13.

```text
authorize root / choose file → read and hash → classify / up to three clarifications / manual pick
→ immutable ActionPlan → exact preview → confirm plan_id
→ full precondition check → atomically consume the plan, write the action and journal
→ create folders safely → re-check → atomic no-replace move → post-move verification
→ commit file location / PathHistory / action in one transaction → conservative Undo
```

Organize handles local APFS files on the same volume, one file per operation, using a native picker or an approved drop entry point. It reads UTF-8 TXT/Markdown, supported DOCX/PPTX/XLSX text (v0.16, shared contract §4.4), and the PDF text layer (§14); Organize-only OCR and optional image input follow §14.8–§14.10. It uses one configured real HTTPS model API. Bringing a file into the app registers its original location; it is not copied to a hidden inbox. Names are kept as they are; a name conflict only allows changing the folder / cancel / skip, with no automatic rename or overwrite. A manual pick must also pass all safety checks.

**Deferred:** learning / confirmed-preference retrieval, a general analysis cache, complex identity inference, local model inference, cross-volume moves, other platforms, and formats or OCR uses outside the approved scope. The original Organize baseline also deferred Memory, watcher, drag-in, and later formats; the current product includes Find, Version Memory, Source Memory, automatic version recording, active drops, and the format extensions above. Their own contracts and acceptance remain separate; Memory core behavior follows its shared contract and current assignment in AGENTS.md.

The original build plan put the deterministic safe executor and the manual closed loop before AI recommendation and clarification; the model must not drive an unverified file executor.

## 2. Safety invariants that must not be weakened

### 2.1 Descriptor-anchored authorized-root containment

- The first authorization opens the actual directory and records root_id, directory evidence, and active/revoked; permission bits and root revision are removed. Revoking permanently invalidates the related unconsumed plans; re-authorization uses a new root_id, and old plans do not come back.
- Containment is built from a verified authorized-root directory handle: walk the relative path one component at a time, do not follow symlinks, and verify the directory object at each level. Use fd-relative operations; re-check the chain and the objects at both ends before and after the move. After a restart, reopen and check the persisted root/directory evidence; do not rely on old fd numbers.
- canonicalize may help with display and with deriving relative paths, but canonicalize + starts_with is not the final boundary; a frontend absolute path or a model string must not directly become an execution path.
- The final move must be **atomic no-replace**; on macOS this is renameatx_np + RENAME_EXCL, and the small platform module prefers rustix. Checking whether the target exists for the preview is allowed, but using `if !exists: ordinary rename` as the execution guarantee is forbidden; there is no overwrite / copy-delete fallback.
- Directory handles and after-the-fact verification are not an atomic transaction over the whole directory tree. An external process moving an ancestor can still cause a race; a detected change goes to needs_user_decision, with no success report and no automatic move back, and there is no claim that transient escapes are fully eliminated.

### 2.2 Zero writes inside authorized roots before confirmation

**Before confirmation, Traeva performs no filesystem writes of any kind inside authorized roots.** This includes mkdir, temporary/probe files, hidden metadata, content/permission/xattr changes, rename/move/overwrite/delete; checking writability by a trial write is not allowed. In §13's auto mode, "confirmation" means the moment the core finds all conditions of §13.4 met and starts `apply_plan`, based on a valid auto-mode agreement; before that, there are zero writes as well (v0.4).

The only place that may be written is the separate app-data: `~/Library/Application Support/Traeva/`. Registrations, non-content analysis metadata, the limited structured result, pending plans, and the internal DB/journal are stored there. **app-data and each authorized root must not overlap in either direction**: if a root contains app-data, a root lies inside app-data, or the actual relation cannot be safely determined, authorization/execution is refused; the actual directory relation is re-checked at startup and before execution.

Automated tests use isolated directories and take tree snapshots before/after read, analysis, clarification question, preview, and cancel, comparing paths, types, file hash/bytes, permissions, extended attributes, and modification information; before confirmation there must be no change caused by Traeva. Reading may change atime, so access time should be excluded; writes by Finder / sync services / other processes need to be told apart. "The whole tree on the system never changes" is not a promise, and this must not be used to ignore Traeva's writes.

A valid confirmation does not mean mkdir/move is allowed right away: the confirmation transaction and the matching intent must first be persisted successfully. If the log cannot reach disk, file side effects are forbidden.

### 2.3 Identity, success, and idempotency

file_id is an app UUID, independent of path/hash/dev/inode. Copies with the same content have different UUIDs; only current direct observation and recorded operations by this app are used for continuity in this slice. An external replacement/move invalidates old plans; no inferred/ambiguous candidate service is implemented in the first version, and files are not merged because they have the same hash.

Before execution, fully re-hash and re-check identity / location / directory chain / authorization and protection conditions; size/mtime are only aids. Success is shown only when the post-state verification of the destination and the source succeeds and the database transaction commits; a returned "accepted", a model success, or a system call returning does not mean completion.

**`actions.plan_id NOT NULL UNIQUE`; one plan → at most one action.** A double click, UI retry, failure, or recovery must never make the same plan create a second action. A repeated call for a consumed plan returns the original action directly, even if it later expired / was revoked / the file changed; this does not authorize a new file operation. To try again after a failure, there must be a new plan, a new confirmation, and a new action.

## 3. Minimal data contract

Rust struct/enum are authoritative; TS uses the matching types. serde and the core validate input, not relying only on static types. UUIDs are strings, SHA-256 is hex; nanoseconds / object IDs use decimal strings to avoid JS precision loss. The object-level schema_version is removed; migrations manage the database and persisted JSON structures in one place. Unknown input fields / IDs / enums are rejected.

| Object | Required facts and constraints |
|---|---|
| AuthorizedRoot | root_id, display path, directory/volume evidence, active/revoked, grant/revoke time; the authorization role is chosen by the current flow, no permission-bit system is built |
| PhysicalEvidence / FileObservation | volume/object identifiers, birth time when available, link count, times; full hash, byte_length, mtime, observed_at; physical identifiers are evidence only, not a permanent identity; a read failure does not fake a valid observation |
| FileRecord | file_id, current root/relative path, basename, presence, latest observation, record_revision, created/updated time; file revision is for file freshness, not a plan revision; no source/decision back-pointers |
| FolderRef | folder_id, root_id, relative segments, parent ID, directory evidence, protected; the folder list can be rebuilt, execution still re-checks, folder_id is not treated as a permanent folder identity |
| AnalysisMetadata | request_id, file_id/hash/revision, provider/model, request time, prompt/schema/extractor/app versions, non-content context information, status/latency/usage; the exact storage rules are in §5 |
| ClassificationResult | request_id, disposition, 0–3 candidates, preferred ID, confidence or null, uncertain_field/clarification, bounded reason, valid evidence IDs; the app fills in the validation result; no execution commands / absolute paths |
| ActionPlan | plan_id, kind organize/undo, file_id, source root/path/full expected observation, destination root/relative segments/original basename, evidence for the existing directory chain, list of folders to create, analysis reference or null, user accept/correct/manual and final choice, undo_of_action_id or null, created_at; the execution payload is immutable |
| ActionRecord | action_id, **plan_id NOT NULL UNIQUE**, kind, undo_of_action_id, persisted status, full source/destination preconditions, created/committed/recovered time, result/error; the referenced plan is not deleted, and facts needed for recovery must not depend on memory |
| Journal / DirectoryReceipt | ordered sequence of action_events, step, intent/result, evidence/error; the receipt records the folders this action created and their physical identity; without a reliable receipt a folder is not claimed |
| PathHistory | path_event_id, file_id, from/to root/relative path, reason intake/organize/undo, action_id or null, physical evidence/time; append-only records; `(action_id,reason)` is unique |

The minimal tables are authorized_roots, files, analyses, plans, actions, action_events, path_history. No separate learning Decision lifecycle is built; the classification suggestion lives in analysis, the user's correction/final choice in plan, and the execution result in action. Later, learning eligibility can be derived from successful operations that were not undone; this version does not run that query feature.

In files, the same current `(root_id,relative_path)` has at most one present record; copies with the same content must not be wrongly merged because of duplicate registration. journal `(action_id,sequence)` is unique. undo_status, can_confirm, whether a plan has expired, and so on are derived when read; no duplicate boolean state is stored.

**Immutable plan** means the execution payload (source, destination, preconditions, choices, and so on) is never modified once persisted. Cancelled / superseded by a new plan / invalidated may update a separate lifecycle state; the fact of consumption is determined by the link to the unique action. Changing the destination creates a new plan_id and at the same time makes the old plan non-executable. The existing 15-minute preview deadline is kept, derived from created_at rather than persisting expired separately; the deadline cannot replace the pre-execution checks. A restart invalidates unconsumed plans.

## 4. File reading, content extraction, and folder rules

Keep the existing v0.2 budgets: a movable single file is at most 20 MiB (manual is limited the same way); extraction is at most 8,000 Unicode scalar values; the destination folder list is at most 500 nodes / 6 levels, with at most 100 bounded candidates given to the model; at most 3 levels are created at once. Exceeding the file limit blocks the move, and a manual pick cannot bypass it.

### 4.1 Observation and text

1. Open level by level from the authorized root fd without symlinks; check type, physical evidence, protection status, and size. Read the full bytes A from the same open object, record the attributes before and after, and compute the hash.
2. Keep the existing A/B observation budget: after 350 ms read B in full, even if size/mtime did not change; only when both hashes, the before/after evidence, and what the current path points to all match can it be used for a plan. At most 3 attempts, 5 seconds in total; if it is still changing, waiting_for_file. This does not prove the editor finished saving; v0.3 does not treat the optional single-read optimization as an approved change.
3. `.txt/.md` are strict UTF-8, BOM allowed; the hash is over the raw bytes, keeping the BOM / original line endings. UTF-16 / invalid UTF-8 / NUL: no guessing, no lossy decode; mark extraction failed or unsupported. Empty content goes straight to manual pick. A regular file whose extraction is unsupported may be manually picked, but must still meet all execution conditions, and this does not count as verified support for that format's meaning.
   `.pdf`: the text layer is read in a separate subprocess (§14); eligible scans without a usable text layer follow §14.8. `.docx/.pptx/.xlsx` use the existing readers and exclusions in shared contract §4.4 (v0.16); their text is excerpted under the same budget. JPG/JPEG/PNG recognition and separately authorized image input follow §14.9 and §14.10.
4. Extraction is a unit-testable pure function bytes→text. Markdown is text only; scripts are not run and links are not followed. Line endings are normalized for display; a short text is taken whole; above 8,000, head 6,000 + tail 2,000, stating clearly that the middle is omitted. The existing evidence IDs/ranges/line numbers are kept for this in-memory preview; head and tail must not be passed off as continuous original text.
5. extraction is ok/partial/unsupported/failed; observation validity is counted separately. The model may only analyze an excerpt of the same bytes that match this hash. A file change while waiting for the model / an answer invalidates the analysis and the old plan.

### 4.2 macOS support and names

Identify explicitly, per the accepted review comments: fstatfs on the fd requires local APFS (apfs and MNT_LOCAL); a placeholder file observed with SF_DATALESS blocks read/move, and reading must not trigger a download. A volume check does not prove there is no cloud sync; cloud-drive plug-ins remain deferred, and there is no claim of support for organizing concurrently with sync services. The first-version demo uses synthetic files confirmed to be local and already on disk.

Reject special files, multiple hard links, symlinks, hidden config trees, app data, protected projects, and bundles/packages. The folder list does not enter packages such as `.app`, `.photoslibrary`, `.rtfd`. Project markers stay `.git` (file/directory), `.hg`, `.svn`, package.json, Cargo.toml, pom.xml, build.gradle, CMakeLists.txt, Xcode projects, and so on; check level by level inside the root, and check accessible ancestors at authorization time. A root clearly inside a protected project must not be let through after only a warning. An unknown protection/permission state blocks execution.

A new name must not be empty, an absolute path, or `.`/`..`; must not contain `/`, `\`, NUL/control characters, or `:`; must not start with `.` or have leading/trailing spaces; its UTF-8 byte length must not exceed 255 or the file system's actual limit. An invalid name is rejected, not silently changed to another name. Existing basenames are kept as they are; when an existing folder has a special name, it is chosen only through a verified FolderRef and never reinterpreted as path syntax.

Case, Unicode equivalence, and same-name occupation follow the actual file system / no-replace result; safety is not ensured by string comparison. No full-disk scan, no reading the content of files that were not selected, no probe writes inside a root.

## 5. AI request, result, and bounded clarification

Before the user enables it, state clearly that the file name, a bounded content excerpt, relative folders, and this answer will be sent; for a short file the excerpt may cover the whole text. By default, the real absolute home-directory path, unrelated files, credentials, or past learning/Source information are not sent. The API key is used only to authenticate with the chosen service, is managed by the core's Keychain, and never enters frontend persistent storage or logs.

Connect only one real provider protocol/config. Request UTF-8 payload at most 64 KiB, response at most 64 KiB, timeout 60 seconds; missing config / send permission fails clearly, and manual pick is available. No automatic retry / model switch; each user-started analysis makes a new request and does not reuse an analysis cache. No execution/DB lock is held while waiting on the network; when the result comes back, verify the request generation, file freshness, and current authorization. A late result must not reactivate a cancelled flow.

File content, file names, folder names, and user-provided data are never system instructions. Strict validation of schema/types, candidate IDs, evidence IDs, path segments, lengths, and locked answers; if the service has structured output, use it, but still validate locally. summary at most 800 characters (from v0.15 it starts with the file's kind and subject), candidate reason 400, question 300, answer 500. Unknown fields, absolute paths, invalid references, and overwriting a locked answer are all invalid_model_output; no folder is picked by default.

Routing parameters: ≥0.70 (from v0.12; before that 0.80) and unique, with grounds, and no conflict → may recommend; medium score / several candidates / missing grounds for one level → ask about that level; low score / null / abstain or not enough information → a targeted question or manual pick. The score is not accuracy; can_confirm is computed by the core, and every recommendation still needs the user's preview and confirmation; the only exception is §13's active-drop auto-organize mode (v0.4).

**At most three clarification questions, one at a time (§15).** Each question targets a missing dimension; the answer is locked, and the next request carries the earlier questions and answers. If the model still cannot recommend after the third answer, or asks a fourth question, return needs_manual_location. generation guards against duplicate/stale answers. When neither usable extracted text nor separately authorized image input (§14.10) is available, go to manual pick; understanding of the content must not be claimed from the file name alone.

Revision note: v0.10 (the student's decision, 2026-10-14, §15) replaced the original one-question limit with the three-question rule above. The decision and other approved changes are preserved in §12.

v0.12 (the student's decision, 2026-10-14, #37): the score threshold for a recommendation drops from 0.80 to 0.70. The other recommendation conditions (the model answers recommend, a unique candidate, with grounds, no open question) are unchanged; `can_confirm` is still computed by the core, and the normal flow still needs preview and confirmation. §13's auto mode uses the same threshold, so files scoring 0.70–0.79 are also moved automatically in auto mode (the other conditions of §13.4 still apply).

### Kept long-term by default and not kept by default

| Kept long-term | Not kept long-term by default |
|---|---|
| provider/model, time, prompt/schema/extractor/app versions, file_id/hash, model parameters | the full extracted content, a raw prompt containing content, the full raw request |
| request ID, candidate folder IDs, truncation flag, extracted character count / selection strategy, generation / current answer state | copying the full excerpt into logs, caches, plans, or internal debug files |
| the validated, length-limited structured AI result, bounded classification reasons, user correction/final choice | content / long passages of original text echoed by the model, unknown fields, and unfiltered raw responses |
| status/error, duration, service request ID, usage (null when unknown), execution result | API key, authentication header |

The content used to build the request and the excerpt/evidence text used for the current preview are used only in this flow's memory and released when the flow ends. The plan stores bounded reasons and source versions/IDs, and does not copy content. The database not keeping the full text long-term does not mean no content is sent to the model; the interface explanation tells the two apart.

Any needed development diagnostic logging must be explicitly turned on, with a clear extra recording scope and a cleanup method, separate from the production default behavior; automated tests use only synthetic data. Non-content metadata is for tracing conditions and cannot promise full request replay; rebuilding the content needs the original file with the same hash, and the model may not reproduce its output word for word.

## 6. Confirmation, execution, and states

The preview shows the original location, the full destination, the kept basename, all folders to create, the reasons, and extraction limits. When the user chooses a new destination, a new plan is built and the old plan is invalidated. If the destination equals the source, already_in_place is returned, with no action/Undo. A cancelled or expired unconsumed plan cannot be executed.

v0.13 (the student's decision, 2026-10-14, #37): in the desktop pet popup (the ask window, #273), the recommendation and the preview for a dropped file are merged into one step: once the recommendation is out, the interface builds a plan for the recommended candidate right away (the core's build_plan; zero writes in the root before confirmation are unchanged). The card shows only the file name, the last three levels of the destination folder (the full path is in the hover tooltip), and Move file, Choose another folder; the original location, full destination, folders to create, reasons, and extraction limits go into Details, collapsed by default. Recommendation and preview in the main window are unchanged. The plan is still immutable; expiry or a conflict still needs a new plan and a new confirmation, as before.

```text
apply_plan(plan_id):
    action already exists → return that action, do not execute again
    under the serial lock, load the immutable plan; re-check current authorization and every precondition
    short DB transaction: confirm it can still be consumed; consume the plan; insert the unique action + prepared journal
    commit fails → the authorized root is not modified
    level by level: durable mkdir intent → fd-relative mkdir → verify → receipt
    re-check the full source hash/identity and both ends' directory chains
    durable move intent → atomic no-replace move
    re-check the chains, source state, target identity/hash, and complete the matching sync
    short DB transaction: update FileRecord, the unique PathHistory, action committed
```

If the destination folder is created first by someone else, it is a conflict and goes back to a new preview; no owned receipt is recorded on its own authority. If the intent cannot be recorded, no side effect happens; if it cannot be proven that the move did not happen, failed_no_move must not be written. After prepared, "cancel and treat as never happened" is not accepted; show in progress, and the result is queried from the action.

| Workflow state | Meaning and allowed next steps |
|---|---|
| observing / ready_for_analysis | only reads the file and writes the app-data registration; no writes to user folders before analysis |
| analyzing / needs_clarification | real API or the single question; can be cancelled |
| needs_manual_location / recommendation_ready | manual pick or pick a valid candidate, build a plan |
| preview_ready | only shows the immutable plan; root zero writes before confirmation |
| waiting_for_file / analysis_failed / invalid_model_output | show the reason; explicit retry / manual pick / cancel; no move |
| blocked / stale / conflict | the old plan does not continue; after the condition is fixed, a new plan and a new confirmation |
| applying | a persisted action exists; query only, do not pretend success / that it can be rolled back |
| committed / undone | both the file post-check and the DB succeeded; show organized / restored by action kind |
| needs_user_decision | the result or evidence is unclear; show known locations, no automatic move/rollback |
| failed_no_move | there is sufficient evidence that nothing moved; trying again must use a new plan |

Persisted ActionRecord states are prepared / running / committed / failed_no_move / needs_user_decision / undone; fine-grained steps stay in the journal. UI state does not replace persisted state, and IPC success is not used to mean the organize is complete.

## 7. Recovery: record reconciliation only

At startup, first reconcile unfinished actions. The basis is the immutable plan/journal + actual filesystem observation; do not trust only "the previous step recorded its intent", nor only dev/inode.

**Automatic recovery does not move, rename, overwrite, or delete user files, does not roll back automatically, and does not continue an unfinished move.** Before finalize, it must check the expected source state, expected destination state, expected content hash, available physical evidence, valid authorization, and the descriptor-anchored directory chain. Insufficient evidence → needs_user_decision.

| Actual evidence | Recovery state of the original action / allowed behavior |
|---|---|
| source is still the expected object/hash, destination does not exist, directory chain matches | failed_no_move; the file stays put, remaining steps are not run automatically |
| source does not exist, destination is the expected object/hash, directory chain matches, journal links agree | idempotent catch-up commit to committed, record recovered_at; update FileRecord/PathHistory in the same transaction; no second move |
| both ends exist, both are missing, object/hash mismatch, authorization/chain invalid, or insufficient evidence | needs_user_decision; keep known locations, block conflicting new operations |
| no reliable receipt after mkdir | do not claim or delete the folder; the file state is decided by the rules above, and the leftover folder is reported |

Undo actions use the same rules, with source/destination being the two ends of the reverse plan; a catch-up commit also marks the original organize action undone. Repeated startup or repeated reconciliation must not add an action or duplicate PathHistory. While the original action can still be reconciled, only it is updated; no "recovery operation" is created to move the file again.

**The only folder cleanup exception:** only with a reliable receipt, when the folder is still the object this action created, authorization/chain are valid, and it is truly empty, may it be deleted after a cleanup intent is recorded, and the result recorded. Pre-existing folders and folders of unknown ownership must not be deleted, and there is no recursive cleanup. When `.DS_Store` makes the folder non-empty, the folder is kept and `.DS_Store` is not deleted; this is not a first-version blocker. If the file was restored but the folder was not cleaned up, show a warning; the restore is not rolled back.

## 8. Conservative Undo

Undo only undoes the location change. A reverse plan can be created only when the original action is committed and not undone, the current object/location/hash match the commit, the original parent folder is still the expected object, the original basename is free, and authorization and protection at both ends are valid.

The current file edited, moved out, or replaced (including by a same-content copy), the original folder replaced/deleted, or the original location occupied: all are undo_conflict; the current state is kept, old bytes are not restored, and no new location is guessed. The user may start a new normal organize plan.

preview_undo shows the exact restore location; after the user confirms the new reverse plan_id, it shares the executor and the same log / unique constraints. The original plan cannot be consumed again, and Undo does not call AI. Reverse action committed, original action undone, FileRecord and PathHistory complete in the same transaction; a repeated Undo returns already undone. undo_status is derived when read and re-checked at execution.

## 9. Frontend–backend interface and errors

Use Tauri `Result<T, AppError>`, with no extra api_version/client_request_id/ok envelope. The frontend calls only through the api.ts wrapper; the core validates all input. Reading/analysis polls the operation; after confirmation, poll the persisted action. A window refresh reads get_app_state and does not automatically resubmit a confirmation.

| Command | Input → output and boundaries |
|---|---|
| get_app_state | no input → storage/model config state, roots, the latest 50 actions and all needs_user_decision items; no new apply before recovery reconciliation finishes |
| choose_root / revoke_root | choosing a purpose opens the native folder picker → RootView; revoke uses root_id; the core checks two-way app-data isolation and physical evidence |
| choose_file | source_root_id, target_root_id → OperationView; the native picker result is checked by the core; no arbitrary executable absolute path from the frontend is accepted |
| list_folders | root_id → bounded folder list, including the root itself; read-only, no probe writes |
| save_model_settings / set_analysis_consent | user config / enable info → non-secret config state; the key goes into Keychain; changing the service needs re-enabling; this is not move authorization |
| start_analysis | file_id, file_revision, target_root_id, send-permission reference → OperationView; reads and requests again, no cache hit |
| answer_clarification | operation_id, question_id, option_id or text → updated OperationView; only the current single round of questions is allowed (from v0.10: only the current question is allowed, at most 3 questions, see §15) |
| get_operation / cancel_operation | operation_id → current state / cancel result; after prepared it cannot be cancelled as never happened |
| build_plan | file_id, file_revision, target_root_id, a valid candidate ID or manually picked folder ID/new_segments → new PlanView; when it explicitly supersedes an old plan, the old plan is invalidated |
| get_plan | plan_id → immutable payload and the current can_confirm / invalidation reason; no revision/digest |
| apply_plan | **plan_id** → ActionView; in the DB one plan has at most one action; if already consumed, return the original action |
| get_action | action_id → persisted ActionView that can still be queried after restart |
| preview_undo | action_id → a new reverse PlanView or undo_conflict; if already undone, return the existing result, no new action created |
| reconcile_action | action_id → ActionView that only reconciles / fills in records; still bound by §7; not a rollback interface |

FileView provides file_id/revision, display path, size/hash, extraction/protection status. OperationView provides operation_id/generation, stage, current question/candidates, plan_id/action_id, and error; a lower generation cannot overwrite a newer state. PlanView provides plan_id/kind, both full ends, folders to create, bounded reasons, extraction warnings, and the can_confirm computed by the core. ActionView provides action_id/kind, status, locations, commit/recovery time, derived Undo status, and cleanup warning. All text is rendered as text; content or model HTML is never executed.

AppError keeps code/message/stage/retryable plus file_id/plan_id/action_id when available, and allowed_next; internal errno/SQL/provider diagnostics are redacted. retryable does not mean an automatic background re-move.

| Failure category | Behavior |
|---|---|
| invalid input / unknown ID / unauthorized source | reject; no unauthorized read/write |
| root revoked / overlaps app-data / protected / cross-volume / unsafe path / unsupported file / over size | blocked; manual pick cannot bypass it |
| file changed / unreadable / extraction failed | bounded wait or manual pick; no executable plan without a valid observation |
| model not configured / no permission / timeout / service failure / invalid output | analysis failed or manual pick; no default folder, file not touched |
| expired / source changed / destination or directory chain changed | stale/conflict; new preview and new confirmation |
| database unavailable / higher schema / second instance / intent write failed | no file operation starts, the database is not deleted to recover; existing actions return their real status |
| mkdir/move clearly failed | failed_no_move only with sufficient evidence; partial folder side effects are accounted for by receipt |
| move result unclear / post-check failed / commit or sync failed | needs_user_decision; no success report, no automatic rollback |
| Undo conflict | keep both ends; the user may start a new normal plan |

## 10. Acceptance IDs and deferral mapping

The v0.2 IDs are kept for tracking; items that have not been run are not marked PASS. O15 learning, O16 cache, O27 watcher dedup, O28 Source references are deferred with their features; they are not deleted from the long-term roadmap and are not part of this slice's production completion conditions. The safety check that external changes invalidate a plan is still covered by O07/O08.

| ID | Result this slice must verify |
|---|---|
| O01 | re-registering the same current object gives the same UUID; same-content copies get different UUIDs; external replacements are not forcibly linked |
| O02 | a clear file gets a recommended suitable existing folder, with reasons shown; no root write without confirmation |
| O03 | each question targets one missing dimension; answers are locked; at most three questions, one at a time (§15); still unclear after the third answer → manual pick |
| O04 | a high score without grounds / conflicting multiple candidates cannot directly become a confirmable guess |
| O05 | full preview for two levels of new folders; root tree snapshots for read/analysis/preview before confirmation show no app writes |
| O06 | changing the destination gives a new plan_id and invalidates the old plan; cancelled/expired cannot be executed; revoking and re-authorizing a root does not revive an old plan |
| O07 | after preview, a same-length content change with mtime restored is still judged stale because of the full hash |
| O08 | an external rename, a same-path replacement / same-content copy invalidates the old plan |
| O09 | destination occupied after confirmation is still atomic no-replace, no overwrite; including case / Unicode-equivalent occupation |
| O10 | fd-relative level by level, not following symlinks; when a middle link of the chain is replaced / moved out of the root, the anomaly is detected, no false success |
| O11 | revoked / permission / cross-volume / hard link / special file / placeholder file rejected; overlap of root and app-data in either direction rejected |
| O12 | project / package / hidden config protection; name rules correct, no probe writes |
| O13 | invalid model JSON / field / ID / path or overwriting an answer: rejected, and no move |
| O14 | offline / timeout / API failure can retry or manual pick, no automatic apply |
| O17 | no change if the journal does not reach disk; mkdir/move failure states are true, no unaccounted move |
| O18-R | recovery-logic check by service recreation / same-DB reopen; must not be labeled a process crash |
| O18-C | recovery after a real abnormal subprocess termination; required before Production Complete |
| O19 | after a move, source/destination/hash/physical/chain mismatch → needs_user_decision; do not finalize on inode alone |
| O20 | double click / UI retry / failure / recovery: at most one action for the same plan; retry after failure uses a new plan; a failed confirmation transaction leaves no root write |
| O21 | normal move, restart, confirm the reverse plan for Undo; bytes/hash/UUID match |
| O22 | at Undo, content changed, moved out, replaced, original path occupied, or parent folder replaced: all rejected |
| O23 | only cleans up the same folder that has a receipt and is still empty; no receipt / non-empty / .DS_Store → kept; the warning does not block the restore |
| O24 | repeated Undo: no extra move or duplicate record |
| O25 | instructions injected in the content are only data; the model has no file execution ability |
| O26 | truncated / unsupported / extraction failed are stated truthfully; over 20 MiB is blocked even for manual pick |
| O29 | Chinese / emoji / BOM / CRLF and invalid-encoding boundaries correct; hash always over the raw bytes |
| O30 | 8,000-character / 20 MiB boundaries and omission placement correct; no pretending to understand the full text |
| O31 | a change during reading gets a bounded wait; the final request excerpt comes from the same source as the hash; no claim that the writer finished |
| O32 | IPC rejects forged IDs / paths / unknown fields / windows; the default DB/logs hold no content, raw prompt, or secret, and keep the agreed metadata/result |
| O33 | cancel, repeated answer, late response, refresh/restart do not apply twice; unconsumed plans are invalidated on restart |
| O34 | DB not writable / corrupted / higher schema / second instance stops new operations; no database deletion / double write |
| O35 | same-location no-op has no action; a cleanup failure does not erase a successful restore; recovery forbids rolling back user files |
| O36 | core independent of Tauri; thin commands; a real macOS window and at least one closed loop in a packaged .app |

## 11. Two explicit completion bars

### CAC Demo Milestone

The following safety groups must have automated evidence: root zero writes / app-data isolation, no-replace (including late occupation), source change → stale, fd chain / symlink boundary, one unique action per plan, intent persisted first, conservative Undo / conflicts, invalid model output → no move, and isolation of injected data.

Run O18-R at four checkpoints: after mkdir, before move, after move but before commit, after the Undo move but before commit. Check user files and persisted records; recovery only reconciles records and does limited folder cleanup; no claim of equivalence to kill -9.

In a real window, use two synthetic documents not used for prompt tuning to demo a clear recommendation and one clarification question, each connected to the same safe executor; with Undo after restart. At least one packaged `.app` verifies native permissions / Keychain / picker; core automated tests do not use the real Keychain, and real model / .app verification uses explicitly configured credentials. The Demo does not require all production counterexamples or an independent semantic evaluation of more than two cases to be complete, and cannot be marked Production Complete.

### Production Slice Complete Milestone

The Demo has passed; all acceptance items in §10's current scope (excluding deferred IDs) have PASS evidence; O18-C really abnormally terminates a subprocess (for example abort) at the few key checkpoints above and verifies after restart; blockers found by Codex's independent review and tests are fixed and re-verified. Service recreation and abnormal termination each have their own results; neither is equivalent to whole-machine power loss; no large fault framework is built.

The independent model evaluation requirement is kept: of the eight v0.2 cases, the conflicting-history case is deferred with learning; this slice has 7 cases — clear destination 3, single-level ambiguity 2, malicious instructions but a clear topic 1, insufficient information 1. First fix the allowed destinations / question dimensions; all 7 cases must meet their own expectations, and passing by sending everything to manual pick is not allowed. Failures are kept; after a fix, keep the regression and re-verify with untuned samples; this is only the result for that sample set and does not extrapolate to accuracy. Missing real API config is BLOCKED, not disguised as PASS.

Any overwrite, escape out of bounds, wrong identity merge, root write before confirmation, unaccounted move, false success, or wrongly deleted folder is a blocker. Evidence records the source version, environment, input hash, expected/actual, PASS/FAIL/BLOCKED, test kind, and log path; use only isolated synthetic files.

## 12. Basis, revisions, and freeze

The baseline drew on the product discussions “Idea discussion” and “Open Source”, an open-source audit, the TraevaLab R1/R2/R2.5 reports and handoff, and a Claude specification review. These external development references are not included in this source package; machine-local links have been omitted.

Do not copy upstream code or prompts or re-run/extend the Lab as part of production work. Preserve the original failures and review records. Lab results and the Claude review's 22/22 local C-probe result do not establish a production Rust PASS. The packaged source's implementation and verification references are in the [README](../../README.md) and [acceptance record](https://github.com/P30g53/Traeva/issues/37).

The following revision history records the decisions as made, including rules that later revisions replaced. Amendments to the frozen v0.3 specification are distinct from the 10/20 Feature Freeze in the [release checklist](../development/final-release.md). The wording below makes that distinction explicit; recorded decision dates are retained:

v0.3 formally puts in place: a lighter first slice; immutable plan / unique action; descriptor containment / no-replace; record-reconciliation recovery; root zero writes before confirmation / two-way app-data isolation; by default, AI data persisted without content; conservative Undo / .DS_Store not a blocker; two kinds of recovery tests and two completion bars.

v0.4 (the student's decision, 2026-10-12, #168): adds §13 active-drop auto-organize mode; §2.2 and §5 each get one sentence pointing to §13. The other frozen rules (immutable plan, unique action, descriptor containment, atomic no-replace, intent persisted first, record-reconciliation recovery, conservative Undo, two-way app-data isolation) are all unchanged, and the normal organize flow is unchanged.

v0.5 (the student's decision, 2026-10-12, #186): adds §14 Organize reads the PDF text layer (subprocess isolation, send notice version 3, drops also accept PDF); §1, §4.1, §13.4 each get one sentence pointing to §14. Memory does not read PDF. The other frozen rules are unchanged. The same day the student confirmed one example in §14.2: a PDF over the page limit counts as "over the reader's limits" and the whole file goes to manual pick, not partial (matching the reader's actual behavior).

v0.6 (the student's decision, 2026-10-12, #208): §13's drop entry points add the desktop pet: dropping on the pet follows exactly the same rules as dropping on the Dock icon and goes into the same queue. The other frozen rules are unchanged.

v0.7 (decided by the coordinator under the student's 2026-10-12 authorization, #186, confirmed in #211): adds §14.8 on-device text recognition (OCR): only for Organize's scanned PDFs, separate subprocess, preparation at startup, soft memory budget and time limit, recognized text is partial, Organize send notice version 4, auto mode does not move files whose text was recognized, off by default (turned on only after R4's production verification passes); §1, §4.1, §13.4, §14.1, §14.2 each get one sentence pointing to §14.8. Memory and Find do no recognition. The other frozen rules are unchanged.

v0.7 addendum (A, 2026-10-13, review findings before turning recognition on, #186): §14.8 states that recognition needs macOS 13 or later and is off on earlier systems; for a scan dropped while preparing, the drop list states that recognition is still getting ready; it states that Apple Vision's own system cache is not a Traeva write. The other rules are unchanged.

v0.8 (the student agreed on 2026-10-13, in A's session, to turn it on per the #186 checklist; R4's production verification is in #211): recognition in §14.8 is on by default (macOS 13 or later); the verification-only environment-variable switch of v0.7 is removed, and the core's constant is the only switch. The other rules are unchanged.

v0.9 (the student's decision on 2026-10-14 in A's session, an approved amendment to the frozen v0.3 specification, #37): adds §14.9 on-device text recognition for JPG, JPEG, PNG images (tier one; the image itself is not sent), Organize send notice version 5. The other frozen rules are unchanged.

v0.10 (the student's decision on 2026-10-14 in A's session, an approved amendment to the frozen v0.3 specification, #37): adds §15 step-by-step clarification questions: at most 3 questions per file, one at a time; §5, answer_clarification in §9, and item 4 of §13.4 each get one sentence pointing to §15. The other frozen rules are unchanged.

v0.11 (the student agreed on 2026-10-14 in A's session, an approved amendment to the frozen v0.3 specification, #37): adds §14.10 letting the model look at images (tier two): after the user turns it on and gives a separate agreement, a reduced copy (without metadata) of the image or scanned PDF is sent to the model; Organize send notice version 6. The other frozen rules are unchanged.

v0.12 (the student's decision on 2026-10-14 in A's session, an approved amendment to the frozen v0.3 specification, #37): §5's score threshold for a recommendation drops from 0.80 to 0.70; item 4 of §13.4 and §15 change to match. Reason: for a long PDF whose beginning and end differ in content, the model gave only one candidate with grounds, but the score did not reach 0.80, so the file could only go to manual pick. The same day, the interface separately merged the manual-pick cards for "only one candidate, not recommended" into one block (UI's #279, no rule change). The other frozen rules are unchanged.

v0.13 (the student's decision on 2026-10-14 in A's session, an approved amendment to the frozen v0.3 specification, #37): §6 adds a paragraph: in the desktop pet popup, the recommendation and preview of a dropped file merge into one step; the confirmation card shows only the file name and the last three levels of the destination folder, and the rest of the preview goes into Details. The main window is unchanged; the other frozen rules are unchanged.

v0.14 (the student's decision on 2026-10-15 in A's session, an approved amendment to the frozen v0.3 specification, #37): in auto mode, files whose text comes from on-device recognition (scanned PDFs, images, §14.8, §14.9) and files whose images the model looked at (§14.10) no longer always stop for the user to confirm: if they meet the other conditions of §13.4 (send permission, a recommendation ≥0.70, an existing folder, within scope, not already in place, etc.), they are moved automatically and support Undo like other files. The auto-mode agreement notice did not mention this point, so the notice version is unchanged. Reason: in the student's use, having to confirm by hand every time was too much hassle. The other frozen rules are unchanged.

v0.15 (the student's decision on 2026-10-16 in A's session, an approved amendment to the frozen v0.3 specification, #37): §5's summary first says, in a few general words, what the file is and which subject or topic it belongs to (in English, plus the file's language when the file is in another language), then the details; when the file does not name the subject, it still gives the general name of the subject (for example, a problem set about moles and reactions gets “Chemistry problem set (化学练习题)”). Reason: Find uses this description to find files by subject, and when the student searched “chemistry pdf”, a file full of chemistry content that never wrote “chemistry” could not be found. Prompt `classify-6`, output schema unchanged; the rules for recommendation, clarification questions, preview, confirmation, and Undo are all unchanged. Existing descriptions are not rewritten; only files analyzed from then on get it.

v0.16 (the student's decision on 2026-10-17 in A's session, an approved amendment to the frozen v0.3 specification, #37): Organize also reads Word (.docx), PowerPoint (.pptx), Excel (.xlsx), using the readers Memory and Find already have; what is read and what is left out follow their rules (shared contract §4.4); the text read is excerpted, sent, and used for recommendation the same way as TXT, in auto mode as well. Reason: the student dropped in a .docx they wrote themselves, and it was treated as an unreadable file and sent to manual pick. Excel sheet names are not sent, same as Memory. Organize's send notice goes up to version 7, stating what is read in each of these three file types; earlier agreements must be given again. Prompt `classify-7` tells the model that lines with ⟦ ⟧ in slides and spreadsheets are location labels added by Traeva. The App declares that it can accept these three file types but does not become their default opener (same as §13.8). The other frozen rules are unchanged.

v0.17 (the student agreed on 2026-10-18 in A's session, an approved correction to the frozen v0.3 specification, #37): when §5's summary gives details, it must use the model's own words and not repeat sentences from the file (at most 8 words or 20 characters in a row), prompt `classify-8`. Reason: when R1 tested on RC14 and RC15 with short synthetic files, several answers copied 60 or more characters of original text into the summary, were rejected by the anti-copy check (§7, #15), and the file could only go to manual pick. The anti-copy check itself is unchanged; the other rules are unchanged.

v0.18 (the student agreed by merging #320 on 2026-10-20; a post-freeze fix, #37): §5's summary is cut where it starts to copy the file's original text (60 or more characters in a row, §7, #15), and “ …” is added at the end. When the cut point falls in the middle of a word made of letters and digits, it backs off to the start of that word (combining accent marks and full-width letters do not count as part of a word for now, so half a word may be left at the end; to be changed after the competition, #310). The recommendation is given as usual and the whole answer is no longer rejected because of the summary. Reason: when R1 tested on RC16 with a 245-byte synthetic file, the `classify-8` summary still copied 60 or more characters, and the file could only go to manual pick; changing only the prompt cannot guarantee that the model does not copy. The copied part is not shown and is not stored in the validated analysis result; only when diagnostic logging is turned on by hand is the raw reply written into the diagnostic file, under §5's exception. When the summary copies from its first character, the whole answer is still rejected as before. The anti-copy checks on reasons, questions, options, and new folder names are unchanged; the prompt is still `classify-8`, and the other rules are unchanged.

The accepted baseline and the explicitly approved revisions remain binding. Further behavior changes require the student's decision. Current responsibilities follow AGENTS.md; implementation defects are fixed against the applicable rules, and independent acceptance is recorded separately.

**Frozen baseline with approved revisions; acceptance is tracked separately.**

## 13. Active-drop auto-organize mode (v0.4, the student's decision 2026-10-12, #168)

### 13.1 Scope

- Handles only files the user **actively drops onto the Traeva icon**: dropped on the Dock icon, or Traeva chosen in "Open With"; this also includes dropping on the desktop pet (v0.6, #208, the student's decision 2026-10-12: the rules are exactly the same as dropping on the Dock icon, the same queue, the same set of conditions and authorization notice). No scanning, watching, or organizing of other files. Only files are accepted; folders are always rejected.
- When several files are dropped at once, they are handled one by one in order, each with its own operation, plan, and action. When a file needs a clarification question, processing pauses and continues after the answer; when automatic processing stops for a file, it stops at that file, and the files after it are listed as "not processed" and are not processed.
- When auto mode is off, dropped files go through the normal organize flow: read, analyze, recommend, preview, user confirmation.
- The normal organize flow (Choose file… and the other existing entry points) is unchanged.

### 13.2 Turning on, destination scope, and agreement

- The user turns on auto mode in Settings and chooses the **destination scope**: one or more authorized folders; subfolders are within the scope too.
- A notice is shown before turning it on; it takes effect only after the user explicitly agrees. The notice must state:
  - only files actively dropped on the icon are handled; other files are not scanned or organized;
  - files are automatically read, classified, and moved to folders that **already exist** within the chosen scope, without a confirmation page;
  - sending to the model still follows §5's Organize send permission;
  - after success, Undo is available, and auto mode can be turned off at any time.
- The agreement records the notice version and the destination scope. When the destination scope changes or the notice version goes up, the old agreement becomes invalid and a new agreement is needed. Turning it off revokes the agreement: no new automatic move starts after that; an action that has already started executing completes as usual, and Undo stays available as usual.
- If a folder in the scope has its authorization revoked or no longer exists, that scope becomes invalid. With no valid scope, auto mode does not take effect, and the interface states why.

### 13.3 Source authorization

- When a dropped file is not inside any valid authorized folder, its content is not read yet; show the full display path of the folder the file is in, and the authorization notice: after authorization Traeva can read and organize the files in this folder, the same as Add folder, and it can be removed in Settings later.
- User agrees: authorize this folder per §2.1. This is a permanent authorization; checks such as two-way non-overlap with app-data are unchanged. Then continue with this drop. User cancels: end, with no read and no write.
- Source authorization does not add this folder to the destination scope.

### 13.4 Conditions for automatic execution (all computed by the core; the frontend does not decide)

1. Auto mode is on and the agreement is valid: the notice version is the current version, and there is at least one valid destination scope.
2. The file is inside a valid authorized folder, opened level by level per §2.1 and registered. The supported text or separately authorized image input is usable (§4.1, §14). Under v0.14, on-device recognized text and image-assisted results are eligible only when every other condition below also holds; they do not bypass any permission or execution check.
3. §5's Organize send permission is valid.
4. After analysis and at most three clarification questions, one at a time (§15), the routing result is "recommend": ≥0.70 (from v0.12; before that 0.80), unique, with grounds, no conflict, and the core-computed `can_confirm` is true.
5. The destination is a folder that **already exists** within some valid destination scope, and the plan does not need to create any folder.
6. The destination differs from the original location. If they are the same, it is `already_in_place`, and no action is created.
7. The immutable plan generated per §6 passes re-checking: the source file's identity and hash have not changed, the destination has no item with the same name, and the directory chain and authorization are valid.

When all are met, the core uses this auto-mode agreement as the basis for confirmation and executes per §6's `apply_plan`: the same executor, intent recorded first, atomic no-replace, at most one execution per plan.

If any one is not met, automatic processing stops, with no move:
- Item 1 or item 3: state the reason; the user may switch to the normal flow.
- Item 2: neither usable text nor permitted image input is available → manual pick (§5, §14). OCR or image-assisted results alone do not require manual confirmation under v0.14; every other automatic-execution condition still applies.
- Item 4: still unclear after the third answer → `needs_manual_location`, manual pick.
- Item 5: a new folder is needed, or the destination is not within the scope → normal preview, and the user confirms.
- Item 7, or a file change, revoked authorization, name conflict, or invalidated plan during execution: stop and state the reason. No overwrite, no automatic rename, no automatic retry; to try again, a new drop or the normal flow is needed, and a new plan is generated.

### 13.5 Zero writes and confirmation

In auto mode, §2.2's "confirmation" means the moment the core finds all of 13.4 met and starts `apply_plan`, based on a valid auto-mode agreement. Before that (read, analysis, clarification question, plan generation), there are zero writes inside authorized folders. Source authorization itself writes only app-data.

### 13.6 Records

- A new migration adds two tables, without changing the seven tables of §3:
  - **Auto-mode agreement**: agreement ID, notice version, destination scope, time turned on, time revoked;
  - **Drop record**: drop ID, time received, file, operation, plan, action, the agreement ID it relied on (empty when auto mode is off), result, stop reason, end time.
  - Only metadata is stored, not content.
- Execution reuses §3's plans, actions, journal, and PathHistory. Recovery follows §7; Undo follows §8 and still needs preview and confirmation.

### 13.7 Showing results

After success, show the actual original location, where the file went, and the undo entry point. When stopped, show the reason and the available next steps: manual pick, normal preview, or cancel. With several files, show each file's status: organized, waiting for an answer, stopped, not processed.

### 13.8 Interface (implemented by A; details go into the shared contract)

- Settings: read and change auto mode, including the switch, destination scope, notice text, and version.
- Entry point: Tauri receives macOS open-file events (dropped on the Dock icon or "Open With") and passes only the file path to the core. The App declares the file types it can accept, but does not become the default opener for any type.
- Queue: query the drop queue, agree to or cancel source authorization, continue or cancel the remaining files; clarification questions reuse `answer_clarification`.
- All decisions are made in the core; the frontend only shows things and collects the user's choices.

### 13.9 Acceptance

| ID | Acceptance |
|---|---|
| A01 | explicit agreement is required before turning on; after turning off, no new automatic move starts; after the notice version or destination scope changes, the old agreement becomes invalid |
| A02 | only actively dropped files are handled; no background scanning; folders are rejected |
| A03 | source not authorized: show the notice for the specific folder; on agreement, authorize permanently and continue; on cancel, end with zero writes |
| A04 | the destination must be an existing folder within a valid scope; when a new folder is needed, switch to the normal preview |
| A05 | when all conditions are met, an immutable plan is generated automatically and the file is moved by the existing executor, without a confirmation page; after success, show where it went and the undo entry point, and Undo is available |
| A06 | at most three clarification questions, one at a time (§15); after an answer, execute automatically only if all conditions are met; still unclear after the third answer → manual pick |
| A07 | file change, revoked authorization, name conflict, invalidated plan: stop and state the reason; no overwrite, no rename, no retry |
| A08 | zero writes inside authorized folders before execution starts (tree snapshot comparison); at most one execution per plan |
| A09 | several files are handled one by one; pause at a clarification question and continue after the answer; when stopped, the later ones are listed as "not processed" |
| A10 | every drop has a record: the agreement it relied on, the actual source and destination, the result |
| A11 | sending to the model still requires the Organize send permission; without permission, nothing is sent and nothing moves |
| A12 | the normal organize flow and the O01–O32 safety regressions are unchanged |

## 14. Organize reads the PDF text layer (v0.5, the student's decision 2026-10-12, #186)

### 14.1 Scope

- Organize (Choose file… and §13's drops) may read the **text layer** of `.pdf` files to recommend a folder. For a scan with no text layer anywhere, whose pages are images, on-device text recognition is possible from v0.7 (§14.8). Metadata (title, author, keywords, etc.), forms, attachments, and annotations are not read.
- Memory does not read PDF; Memory's scope, permission, and notice are all unchanged.
- Other unsupported formats stay as before: content is not read, and manual pick is available.

### 14.2 Reading

- Observation follows §4.1: open level by level from the authorized root, read in full twice, usable only when the hash and the before/after evidence match; the movable-file limit is still 20 MiB.
- Only these stable bytes are given to the PDF reader; the reader does not open a path again and does not read other files.
- The reader's version is pinned; its own limits (page count, object count, decoded volume, etc.) are written into the shared contract.
- The result follows the extraction of §4.1 item 5: text read is ok; the reader leaving out a part by design (for example images) is partial; a scan, no text layer, encrypted, not a valid PDF, over the reader's limits (for example more than 500 pages) is unsupported or failed → manual pick, not sent to the model. For scans and PDFs without a text layer, from v0.7 on-device text recognition per §14.8 is tried first.
- The excerpt rules are the same as §4.1 item 4 (at most 8,000 Unicode scalar values; when over, take the head 6,000 and the tail 2,000 and state that the middle is omitted). Evidence and quotation checks are the same too; they may come only from the text read this time.

### 14.3 Isolation

- The PDF is parsed in a **separate subprocess**: the core passes the bytes of §14.2 to the subprocess through a pipe, and the subprocess returns only text and status.
- The subprocess does not receive a path, does not read or write any file, does not use the network, and does not access the database or the keychain.
- The subprocess has a memory limit and a time limit; the exact values are written into the shared contract. Over a limit, timeout, crash, or malformed returned content is always handled as failed: manual pick, not sent to the model, no automatic retry, and the App stays usable.
- So §2.2's zero writes before confirmation and two-way app-data isolation are unchanged.

### 14.4 Sending and permission

- What is sent is the same as §5: file name, the folder it is in, the list of destination folders to choose from, a text excerpt (here an excerpt of the PDF text layer), plus the answer when there is a clarification question.
- Historical notice transition (v0.5): Organize's send notice rose to version 3, stating that a passage of text from TXT, Markdown, or the PDF text layer will be sent. Send permissions for version 2 and earlier become invalid; sending happens only after a new agreement. Current sending requires version 7 under v0.16 (§12).
- Content in PDF text that looks like instructions is, as in §5, always treated only as data.

### 14.5 Auto mode

"The type is supported" in item 2 of §13.4 includes PDF. The App declares that it can accept PDF but does not become its default opener (same as §13.8). The other conditions of §13 are all unchanged.

### 14.6 Unchanged

Immutable plan, at most one action per plan, descriptor containment, atomic no-replace, intent persisted first, record-reconciliation recovery, conservative Undo, two-way app-data isolation: all unchanged.

### 14.7 Acceptance

| ID | Acceptance |
|---|---|
| P01 | PDF with clear text: recommendation, at most three clarification questions, one at a time (§15), exact preview, confirmation, Undo as usual |
| P02 | scan, no text layer, encrypted, invalid PDF, over the reader's limits: manual pick, not sent to the model |
| P03 | subprocess over the memory or time limit, crash, malformed returned content: manual pick, not sent to the model, no retry, the App stays usable |
| P04 | only the text-layer excerpt and §5's other fields are sent to the model, no PDF metadata |
| P05 | historical v0.5 transition: send notice version 3; old permission invalid, sending only after a new agreement; current sending requires notice version 7 (v0.16, §12) |
| P06 | zero writes inside authorized folders before confirmation (tree snapshot comparison); the subprocess writes no file |
| P07 | dropped on the icon: a PDF meeting all conditions of §13.4 moves automatically; otherwise it is handled per §13.4 |
| P08 | Memory does not read PDF (regression) |
| P09 | regressions for the normal organize flow, O01–O32, and A01–A12 unchanged |

### 14.8 On-device text recognition (OCR, v0.7)

The following rules were set by the coordinator under the student's 2026-10-12 authorization (#186, confirmed in #211; R4's static review of the experiment, R4-OCR-02/03/04, is written into the rules below).

**On by default** (v0.8): on 2026-10-13 the student agreed to turn it on per the #186 checklist; on 2026-10-13 R4 did a native-window verification on a separate build of `137b29a` (#211): showing and agreeing to send notice v4, real suggestions → preview for s06 / s13, handoff in auto mode, quitting while preparing and while recognizing, ready turning into failed and recovery, and so on, passed. On that build, the note about missed pages was shown in full only in the file info (FileSummary); the suggestion card and the preview had only the recognition label. After #236 all three places show the full note; native verification of this on the new layout is covered by the final regression of #37. Offline items are in the same Issue. The "Open" button of the Choose file system picker was sometimes greyed out; R4 recorded it separately as BLOCKED with the cause undetermined, and it is not part of recognition itself (see "Switch" below).

**Scope**

- Only for Organize: Choose file… and §13's drops, including the re-read before sending. Memory, Find, and Source do no recognition (nor do Memory and Source when registering files).
- Only for the case where the PDF reader reports that **the whole PDF has no usable text layer and its pages are images**: the reader's status is unsupported, and its note starts with “No page has a text layer that this reader can use.” (mostly images, possibly a scan) or “No page has a text layer; the PDF holds only images and drawings.”. A PDF where some pages have a text layer still uses the text layer (partial), with no recognition; encrypted, corrupted, over the reader's limits, characters without a Unicode mapping, fonts that cannot be decoded, and blank pages get no recognition.
- Image files (PNG, JPEG, etc.) are out of scope; from v0.9, for JPG, JPEG, PNG see §14.9.
- Needs macOS 13 or later (Apple Vision text recognition revision 3). On earlier systems the App does not set up recognition, the state is "off", the interface shows no status line, and scans go to manual pick as in v0.5.

**Preprocessing budget before recognition** (passed by the core to the subprocess as parameters; the subprocess enforces it itself)

- Recognize at most the first 3 pages without a text layer; render at 200 dpi; at most 8 million pixels per page.

**Preparation**

- When the switch is on, the App runs one preparation subprocess in the background at every startup: the same executable with the preparation argument and the App's own process ID (if the parent process is gone, the subprocess exits), using a synthetic image, with no user content and no input. When ready, the exit code is 0 and it outputs `{"status":"ready"}`. Time limit 90 seconds; memory rules as below.
- Preparation states: off, not prepared, preparing, ready, failed, timed out, cancelled (when the App quits). If preparation fails, times out, or is cancelled, it is not retried in this run; it is prepared again at the next startup. "Ready" is valid only for this run.
- An image-only PDF chosen while preparing: manual pick, with the note “Text recognition is getting ready (the first time can take about a minute).”. One dropped on the icon (§13, common when this drop is what opened Traeva) also goes to manual pick, and the reason in the drop list is “Text recognition is still getting ready (the first time can take about a minute): drop the file again in a moment, or choose its folder by hand.”. Other not-ready states (off, not prepared, failed, timed out, cancelled): manual pick, with the same note as v0.5 (the reader's own note). After preparation completes, this file is not re-read automatically.

**Stopping while running**

- Recognition runs in a separate subprocess: the same executable with the recognition argument, the preprocessing parameters above, and the App's process ID. The core gives it the bytes of §14.2 through stdin (at most 20 MiB); the subprocess returns one JSON object with the same shape as §14.3's PDF subprocess (status, text, note; extra fields rejected).
- After ready, at most 15 seconds per file. The parent process reads the subprocess's memory every 5 milliseconds, counting "the larger of the current use (physical footprint) and this process's lifetime peak", and terminates it above 384 MiB. This is a **soft budget**, not a hard limit: between two reads the subprocess may briefly go over before it is terminated. It is also terminated when its memory cannot be read. This rule is separate from the 512 MiB counted-heap limit of the PDF text-layer subprocess (§14.3), which is unchanged.
- Only a subprocess that has really ended counts as exited (normal exit, terminated by a signal, or crashed). A subprocess that is paused (for example after receiving SIGSTOP) or resumed does not count as exited, and is still bound by the time limit, the stop at App quit, and the memory checks: terminate it, reap it, and only then let the next subprocess start.
- After the subprocess ends on its own and before it is reaped, the parent process checks once more: it reads the memory a last time (including the lifetime peak) and re-checks the stop and the time limit. Over memory, stopped, or timed out: discard its answer; if the memory cannot be read at this point: discard the answer and revoke ready.
- Only one recognition or preparation **subprocess** runs at a time. While waiting for this slot, reading and observing other files (Organize's normal flow) is not affected; only the next recognition subprocess has to wait. A second image-only PDF waits for the previous subprocess to end (at most its time limit), and is recognized then if it is still ready.
- Any stop or failure (timeout, over memory, memory unreadable, crash, malformed returned content): discard the recognized text, manual pick, not sent to the model, no retry. On timeout, over memory, or memory unreadable, also revoke "ready" for this run, and the state becomes failed: later scans all go to manual pick until preparation runs again at the next startup.
- **The same bytes are recognized only once per run**: the core records in memory the hash of the bytes it has recognized (only the hash and whether it succeeded, not the text), separate from the cache that keeps recognized text. Bytes that failed are not recognized again in this run, even if they are no longer in the cache (manual pick, with the note “Text recognition did not work on this PDF earlier in this run; Traeva tries it again only after it restarts.”); for bytes recognized successfully whose text has been pushed out of the cache, no subprocess is started again either (manual pick, with the note “This PDF's text was recognized earlier in this run and is no longer kept; choose it again after restarting Traeva.”).

**Result and sending**

- The subprocess's answer counts as successful recognition only when `status` is `partial` (`complete` is treated as malformed). `text` must be 1–3 page sections: the first line of each section is `[Page N]` (N is an integer starting from 1, with no leading zeros, each section larger than the one before, so none repeat), followed by at least one line of recognized text. A section with only the header line, page 0, a repeated page, reversed order, more than 3 sections, text before the header line, extra fields, or wrong types are all treated as malformed. `unsupported` / `failed` are answers without text → manual pick, showing the subprocess's note.
- Recognized text is always `partial`, with the note “Read by on-device text recognition from N scanned page(s); it may contain mistakes.” (N is the number of pages recognized), followed by the subprocess's note on pages not recognized and other things the reader left out (at most 300 characters; with the subprocess's own repeated warning sentence removed, R4-OCR-05); extraction is marked as recognized (`recognized`). This note stays together with the text in the 8 recognition results below.
- Excerpt, evidence, and quotation checks are the same as §14.2; what is sent is the same as §5 and §14.4, with no fields added to the data sent to the model. The metadata of this request records that the text source is recognition (`text_source` is `ocr`).
- Recognized text is not written to the database or files. So that the re-read before sending and the request after a clarification question use the same text, the latest 8 recognition results are kept in memory for this run (keyed by the hash of the bytes), and are lost when the App quits.
- Historical notice transition (v0.7): Organize's send notice rose to version 4, stating that the excerpt may also be text recognized on this Mac from scanned PDF pages. Send permissions for version 3 and earlier become invalid; sending happens only after a new agreement. Current sending requires version 7 under v0.16 (§12).

**Auto mode**

- Under v0.14 (the student's decision 2026-10-15), recognized text may support automatic execution only when every condition of §13.4 holds; Undo remains available under the normal checks. This replaced the original v0.7 rule that always required a manual preview and confirmation for OCR results.

**Switch**

- Recognition as a whole is switched by **one constant** in the core, read at App startup; from v0.8 it is on (before macOS 13 it is still off, see "Scope"). When changed to off: no preparation, no recognition, the state is "off", and scans go to manual pick as in v0.5.
- There is no other switch: no environment variable, and it cannot be turned on or off in the interface. The v0.7 environment variable `TRAEVA_OCR_TEST`, which was only for R4's verification, has been removed.

**Unchanged**

- §14.3's isolation (the subprocess receives no path, does not read or write files, does not use the network, does not access the database or the keychain), §2.2's zero writes before confirmation, two-way app-data isolation, and the rules listed in §14.6 are all unchanged.
- Apple's Vision framework may store its own data in macOS cache locations (on the same Mac, with a new App identity the first preparation is clearly slower, which is consistent with such a cache). This data is managed by Apple's framework itself inside the recognition subprocess, and is not in Traeva's app-data or authorized folders; Traeva's own code does not write, read, or delete it. Preparation uses a synthetic image.

**Acceptance**

| ID | Acceptance |
|---|---|
| P10 | scan chosen while preparing: manual pick, showing the preparing note; other not-ready states the same as v0.5 |
| P11 | scan chosen after ready: partial, marked as recognized, the note includes the page count; the excerpt is sent per §5, and the metadata records that the source is recognition |
| P12 | recognition timeout, over memory, memory unreadable, crash, malformed returned content: manual pick, not sent to the model, no retry; the first three revoke ready; if the last read after the subprocess ends and before it is reaped shows over memory or cannot be read, the answer is also discarded |
| P13 | only one recognition subprocess at a time, the second waits; a paused subprocess is terminated and reaped at the time limit, and later preparation and recognition proceed as usual |
| P19 | the same bytes are recognized only once per run: for bytes that failed, or whose text was pushed out of the cache, no subprocess is started again |
| P20 | answer format check: only `partial` with 1–3 valid page sections is accepted; anything else is treated as malformed |
| P14 | OCR-based recommendations move automatically only when every condition of §13.4 holds (v0.14); the original v0.7 criterion required manual confirmation |
| P15 | historical v0.7 transition: send notice version 4; version 3 permission invalid, sending only after a new agreement; current sending requires notice version 7 (v0.16, §12) |
| P16 | Memory, Find, Source do no recognition; PDFs with a text layer or encrypted PDFs get no recognition |
| P17 | with the switch off (on by default from v0.8), behavior is the same as v0.5 |
| P18 | after preparation fails, times out, or is cancelled, no retry in this run; zero writes inside authorized folders before confirmation (regression) |

### 14.9 On-device text recognition for images (v0.9)

The student decided on 2026-10-14 in A's session (an approved amendment to the frozen v0.3 specification, #37) to add on-device text recognition for JPG, JPEG, PNG images (tier one). Optional image sending (tier two) was separately approved in v0.11 and is defined in §14.10.

**Scope**

- Only for Organize: Choose file… and §13's drops, including the re-read before sending. The extension is `.jpg`, `.jpeg`, or `.png` (case-insensitive), and the file header really is JPEG or PNG. Other image formats (HEIC, GIF, TIFF, WebP, etc.) still have no content read, and manual pick is available.
- Memory, Find, Source do not read images and do no recognition (same as §14.8).
- Depends on §14.8's recognition: the same constant, macOS 13 or later, preparation already ready in this run. When not ready → manual pick, with the note as in §14.8 (with PDF replaced by image).

**Reading and isolation**

- Observation follows §4.1 and §14.2: read in full twice, usable only when the hash and the before/after evidence match, limit 20 MiB. Only these bytes are given through stdin to §14.8's recognition subprocess. The subprocess receives no path; §14.3's isolation is unchanged.
- The subprocess decodes only the first frame; it uses only the image's orientation information to turn the image upright, and does not read, keep, or return location or other metadata.
- Before decoding, read the dimensions from the file header: over 50 million pixels, or any side over 20,000 pixels, goes straight to manual pick. An incomplete file end (PNG without IEND, JPEG without FF D9) counts as corrupted. Decoding scales down to at most 8 million pixels, the same as §14.8's per-page limit. Time limit, memory budget, only one subprocess at a time, the same bytes recognized only once per run, revoking ready after failure: these rules are all the same as §14.8.
- Cannot be decoded, unsupported format, or corrupted file: manual pick, not sent to the model.

**Result and sending**

- The subprocess's answer has the same shape as §14.8 (`status`, `text`, `note`; extra fields rejected), but for an image `text` is the recognized lines joined in order with newlines, with no `[Page N]` markers, and at least 10 non-whitespace characters. The parent process uses the image's own format check; on success the subprocess's `note` is not used.
- Recognized text is always `partial`, with the note “Read by on-device text recognition from an image; it may contain mistakes.”; extraction is marked as recognized (`recognized`), and the request's metadata records `text_source` as `ocr`. With no usable text → manual pick, with the note as in §14.8.
- In the on-device recognition path, what is sent is the same as §5: file name, current folder, candidate destinations, an excerpt of the recognized text, and any clarification questions and answers. This text-only path sends no image pixels or metadata. Separately authorized image sending follows §14.10.
- Historical notice transition (v0.9): Organize's send notice rose to version 5, stating that the excerpt may also be text recognized on this Mac from an image. Send permissions for version 4 and earlier become invalid; sending happens only after a new agreement. Current sending requires version 7 under v0.16 (§12).

**Auto mode**

- Same as §14.8: from v0.14, recognized text may support automatic execution only when every condition of §13.4 holds. The App declares that it can accept JPEG and PNG but does not become their default opener (same as §13.8).

**Unchanged**

All of §14.8's "Unchanged" applies; the reading and recognition rules for PDF are unchanged.

**Acceptance**

| ID | Acceptance |
|---|---|
| P21 | JPG or PNG with clear text: `partial`, marked as recognized, the note is the image version; recommendation, at most three clarification questions, one at a time (§15), exact preview, confirmation, Undo as usual |
| P22 | no text, corrupted, over the size limit, other image format: manual pick, not sent to the model |
| P23 | in the text-only recognition path, only the excerpt and §5's other fields are sent; image bytes and metadata stay on this Mac; separately authorized image sending is tested under §14.10 |
| P24 | an image whose orientation info says it is rotated is turned upright and then recognized; for a multi-frame PNG only the first frame is recognized |
| P25 | automatic execution requires every condition of §13.4 (v0.14); both the Dock icon and the pet accept dropped JPG and PNG |
| P26 | historical v0.9 transition: send notice version 5; version 4 permission invalid, sending only after a new agreement; current sending requires notice version 7 (v0.16, §12) |
| P27 | Memory, Find, Source do not read images (regression); PDF reading and recognition unchanged (regression) |

### 14.10 Letting the model look at images (tier two, v0.11)

The student agreed on 2026-10-14 in A's session (an approved amendment to the frozen v0.3 specification, #37): after the user turns it on, Organize may send a reduced copy of an image or scanned PDF to the model so that the model looks at the image directly. §14.9's on-device text recognition stays the default approach.

**Switch and agreement**

- The Model card in Settings has a three-way choice: **Off** (default), **Ask each time**, **Don't ask**. Next to it, the image notice is shown: what will be sent, how small it is reduced, what information is removed, and that images may show faces, ID documents, screens, or places.
- This agreement is stored separately and is valid only for the same provider, the same model, and the same version of the image notice. Organize's current send agreement must also be valid (version 7 from v0.16; version 6 when image sending was introduced). Changing the provider or model, or revoking Organize's agreement, invalidates it. Changing to Off takes effect immediately, and no image is sent after that.
- **Ask each time**: before sending, Organize asks once whether to send a reduced copy of this image for the model to look at, with the buttons **Send image** and **Use recognized text only**. Ticking **Don't ask again** remembers this choice: choosing **Send image** changes the setting to **Don't ask**, and choosing **Use recognized text only** changes it to **Off**. Auto-organize drops (§13) and the pet panel cannot ask, so they use only the on-device recognized text; when the user personally goes through the normal flow in the main window, it asks as usual.
- Historical notice transition (v0.11): Organize's send notice rose to version 6: the image itself is sent only after this switch is turned on, and only as a reduced copy without location and camera information. Current sending requires version 7 under v0.16 (§12).

**What is sent**

- Only for Organize's JPG, JPEG, PNG (§14.9) and scanned PDFs with no text layer anywhere (§14.8). Memory, Find, Source do not send images.
- The reduced copy is made in §14.8's recognition subprocess: for an image only the first frame is taken and turned upright by its orientation tag; for a scanned PDF the first 3 pages without a text layer are taken. Long side at most 1568 pixels, re-encoded as JPEG, **with no metadata at all** (no EXIF, GPS, XMP, IPTC, or embedded thumbnails). Each copy at most 1 MiB, at most 3 copies. Time limit, memory budget, only one subprocess at a time, revoking ready: these rules are all the same as §14.8.
- The request carries §5's fields as usual (file name, the folder it is in, the destination folder list, text excerpt), plus these copies. If there is on-device recognized text, it is sent as the excerpt as usual; an image without text can also be recommended from the image alone.
- Image copies are not written to the database or files; even when diagnostic logging is on, it records only whether there were images and how many, not the images themselves.

**Result**

- Recommendations made with image input follow the same clarification, plan, execution and Undo rules. From v0.14, automatic execution is allowed only when every condition of §13.4 holds; this replaced v0.11's mandatory manual-confirmation rule.

**Acceptance**

| ID | Acceptance |
|---|---|
| P28 | off by default: no image is sent, behavior is the same as §14.9 |
| P29 | Ask each time: ask once before sending; choosing **Use recognized text only** sends no image; ticking **Don't ask again** remembers this choice (Send image → Don't ask, text only → Off) |
| P30 | the copy sent has a long side of no more than 1568 pixels and no metadata at all (checked with an image carrying synthetic location and camera data) |
| P31 | after changing the provider or model, revoking Organize's agreement, or changing to Off, no more images are sent |
| P32 | dropped files use only recognized text under **Ask each time**; when image input is separately permitted, automatic execution still requires every condition of §13.4 (v0.14) |
| P33 | Memory, Find, Source do not send images; regressions for §14.8, §14.9 unchanged |

## 15. Step-by-step clarification questions (v0.10, the student's decision 2026-10-14, #37)

The student decided on 2026-10-14 in A's session (an approved amendment to the frozen v0.3 specification): §5's "at most one clarification" is cancelled and replaced by at most 3 questions per file, one at a time.

### 15.1 Rules

- When the model is unsure, it may ask a question. At most 3 questions per file, only one at a time; after the user answers, the answer is locked, and the next request carries all earlier questions and answers, in order.
- Each question should narrow down only one level where possible: first ask which of at most 3 possible upper-level folders, then which subfolder within the chosen folder; or ask about one fact that decides the location. When asking "which folder", the options give the paths of the listed folders.
- The shape and validation of questions are unchanged (§5, #15): Traeva's own question_id and option IDs; question 1–300 characters; no options, or 2–4 distinct options, each 1–80 characters; the answer is one option or the user's own words, 1–500 characters. Options are only data: candidates can still only be the folders listed this time, and §5's validation is unchanged.
- At any question, the user may switch to manual pick ("None of these", outside the options, goes to manual pick), or cancel.
- After the 3rd answer, the model may only recommend or abstain; if it asks again, it also goes to manual pick as `needs_manual_location`; there is no 4th question. The other conditions for a recommendation follow §5 (from v0.12 ≥0.70, unique, with grounds, no conflict).
- A locked answer cannot be changed; to change it, cancel and choose the file again. An old question already superseded by the next question cannot be answered any more. Sending the same answer again does nothing.
- A retry after failure (`analysis_failed` / `invalid_model_output`) resends all locked answers.
- Text that looks like instructions in file content, file names, folder names, and answers is still treated only as data (§5).

### 15.2 Sending

- The fields sent are the same as §5, except that the original single `answered_question` is replaced by the list `answered_questions`: each item is the question, options, answer, and dimension; question_id is not sent. The request metadata records the number of answers (`answered_questions`); the original boolean `answered_question` (whether there was an answer) is kept for compatibility.
- When asking "which folder", an option whose path is longer than 80 characters may give only the last few levels.
- Prompt `classify-4`, output schema `classification-3`: the JSON shape is unchanged; asking another question after an answer, within 3 questions, is a valid result.

### 15.3 Auto mode (§13)

- Same as v0.4: every question pauses the drop queue to wait for the user's answer (`needs_answer`), and the queue continues after the answer. After at most 3 answers, the file moves automatically only if the routing result is recommend and all other conditions of §13.4 are met; if it is still unclear, manual pick. A file whose question the user has not answered does not move.

### 15.4 Interface

- The question card shows “Question N of up to 3” and the earlier questions and answers; each step has only one amber button (Send answer). When there are options, the manual-pick button reads “None of these: pick the folder myself” and leads to manual pick (two-level folder selection, #266).
- While waiting for the model, the answers already sent are listed; when a recommendation is accepted, the preview lists all answers.

### 15.5 Unchanged

§2's safety invariants, zero writes before confirmation, immutable plan, §6's execution, §7 Recovery, §8 Undo, the other conditions of §13.4, and §14's reading and recognition rules are all unchanged.

### 15.6 Acceptance

| ID | Acceptance |
|---|---|
| Q01 | uncertain file: question 1 → answer → question 2 (the request carries the 1st answer) → answer → recommendation; the preview lists both answers |
| Q02 | the model still wants to ask after 3 answers: manual pick, no 4th question, 4 requests in total |
| Q03 | an old question superseded by the next one cannot be answered; a locked answer cannot be changed; the same answer sent again is not sent |
| Q04 | each request carries all earlier questions and answers in order, without question_id; metadata records only the count |
| Q05 | at any question, the user may switch to manual pick or cancel; manual pick uses two-level folder selection |
| Q06 | auto mode: every question pauses the queue for the user; still unclear after 3 answers → manual pick |
