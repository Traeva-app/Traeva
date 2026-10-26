# Traeva — Collaboration rules (shared by all agents)

This file is the single entry point for agent rules in the Traeva repository. Claude/Opus, Codex and every other agent follow it; `CLAUDE.md` only imports this file and adds no rules of its own. Where this file conflicts with them, the three frozen documents and the user's explicit decisions in Issues or chat take precedence.

## 1. Scope: the frozen Organize spec and the CAC product plan

- [docs/architecture.md](docs/architecture.md), [docs/features/organize-spec.md](docs/features/organize-spec.md) and [docs/features/organize-plan.md](docs/features/organize-plan.md) define the frozen **AI Organize v0.3 baseline and its explicitly approved revisions**. The safety invariants and three Gates remain binding. Current clarification limits and supported formats follow the approved revisions in organize-spec §12, including §15: at most three questions per file, one at a time. The original deferral of Memory/watcher does not reduce the scope of the whole Traeva CAC product.
- Do not reopen the architecture review, re-choose the stack or redesign the product. Fix implementation defects at the exact point the spec describes; any change to a frozen document needs the user's explicit decision.
- Stack: Tauri 2 + React/TypeScript + Vite + a separate Rust core (`crates/traeva-core`) + SQLite/rusqlite; macOS first.
- Execution scope: local APFS, same volume, one file, one real model API, exact preview, safe move, durable records and conservative Undo. The original TXT/Markdown slice has the approved format extensions described below. Current Organize allows at most three clarification questions per file, one at a time (§15). Active-drop auto-organize (§13, approved in #168) moves a dropped file without a separate confirmation page only under a valid auto-mode agreement and when every condition of the core check holds; the normal flow still requires user confirmation.
- Order: deterministic safe executor + manual end-to-end loop → Gate 1 → AI recommendation with bounded clarification (§15). Before Gate 1 PASS, AI must not be connected to the file execution chain.
- The Traeva CAC product = Foundation + AI Organization + Version Memory + Source Memory. The family, watcher/durable snapshot, semantic changes, retrieval and UI of #26/#30–#35, and the minimal Source of #27, are all **CAC Required / core differentiation**, built according to the approved Issues. Memory first saves real snapshots, then analyzes them asynchronously; a Lab PASS or an Organize Gate PASS cannot stand in for production Memory acceptance.
- #66 Limited DOCX: the student approved the limited scope in #66; DOCX is connected to Memory (body text only, limits in shared contract §4.4); #66 is closed.
- Learning, a general analysis cache, complex or evidence-free identity inference, OCR outside Organize's approved scanned-PDF and image scope, other platforms and extension sources remain in the long-term backlog (#24/#25/#36 and others): not started now, no placeholder services, and the plans are not deleted.
- PPTX/XLSX are connected to Memory (#162, shared contract §4.4); their integration is in the scope of #37's final verification, not #26's earlier acceptance. Organize and Find read the PDF text layer (#186/#198, shared contract §4.9–§4.10), parsed in a separate child process. If Organize cannot obtain usable content, it falls back to a manual pick; unreadable body text does not prevent Find from matching file or folder names. Memory does not read PDFs. The PDF reader and its core wiring are implemented; implementation alone is not independent acceptance.
- The approved CAC scope also includes **Find** (#198: searches authorized folders by file name, folder name, readable body text and saved Organize descriptions; only the search question is sent to the model and file matching stays local); **automatic version recording** (#201: a master switch lets files Traeva has organized start being recorded, with the per-file Memory agreement, up to 50); and **the desktop pet and quick-search panel** (#208: dropping on the pet uses the Dock-drop rules; the pet itself reads state without paths). The home page also shows recently opened files and items waiting for the user. The original pet stability deadline and main-window fallback are recorded in #208 and shared contract §4.12.
- OCR and optional image input are Organize-only features (organize-spec §14.8–§14.10). On macOS 13 and later, on-device recognition reads at most the first 3 pages of PDFs without a usable text layer, and text in JPG/JPEG/PNG images. It runs in a separate child process with a soft budget based on observed memory use. When neither usable text nor separately authorized image input is available, Organize falls back to a manual pick. Recognition is on by default where supported, controlled by the core constant. Sending a reduced, metadata-stripped image copy to the model requires the separate image-sending setting and agreement; it is off by default. Under the approved v0.14 rules, OCR-based or image-assisted results may be moved automatically only when the auto-mode agreement and every condition of §13.4 hold. Memory, Find and Source do no OCR. The native verification at `137b29a` is historical evidence with its scope recorded in #211; later changes need their own verification.
- The student explicitly authorized A to merge a PR directly once its review and all checks pass (recorded in #37). This does not cover closing a Gate, closing an Issue without evidence, or using model budget that has not been approved. While the student is away, questions within the delegated scope go to A and the coordinator (#211); decisions outside that scope remain with the student.
- The latest execution plan is in [CAC parallel development setup](docs/development/cac-parallel.md) and the [CAC Sprint view](https://github.com/users/P30g53/projects/1/views/5). Dates are target checkpoints; completion is decided by real evidence.

## 2. Code boundaries (thin Tauri, thick Rust core)

| Layer | Does only | Does not |
|---|---|---|
| `src/` (React/TS) | Rendering, collecting the user's choices, polling; Organize goes through the existing `src/features/organize/api.ts`, new features call commands through their own thin APIs | Direct file/SQL/shell/model access; deciding confidence, can_confirm or whether to ask a question by itself |
| `src-tauri/` | Input parsing, window permissions, native pickers, background scheduling, holding the core | Business SQL, file moves, classification rules |
| `crates/traeva-core/` | All rules, file operations, database writes, recovery, Undo | Depending on `tauri` |

- `src-tauri/capabilities/main-window.json` lets only the main window call the listed Traeva commands; a new command must also be added to `src-tauri/src/command_names.rs`, the `src-tauri/src/lib.rs` handler and that capability file, and `build.rs` generates the permissions from command_names. No broad plugin permissions such as fs/sql/shell, no remote pages that could reach IPC, and the CSP is fixed in `tauri.conf.json`.
- IPC uses Tauri `Result<T, AppError>`. The Rust struct/enum is the source of truth; the existing Organize mirror is in `src/features/organize/types.ts`. Claude-A coordinates the incremental integration of new features' DTOs/commands and their mirrors; the frozen Organize contract is not rewritten for Memory.
- Style: plain structs, enums, functions and a few modules. No trait/service layer per module, no plugin or agent framework, no features or abstractions beyond the Issues. The student must be able to read and explain the core code.
- Database: one write connection; `foreign_keys=ON`, `journal_mode=DELETE`, `synchronous=EXTRA`, `fullfsync=ON`, read back and checked at startup. Migration files `crates/traeva-core/migrations/NNNN_*.sql` are append-only: a committed migration is never changed, a schema change goes in a new file, and both an empty database and an upgrade from an old one are tested. No error ever deletes and rebuilds the database.
- Organize's minimal initial seven tables: `authorized_roots`, `files`, `analyses`, `plans`, `actions`, `action_events`, `path_history`. If Memory/Source need new persistent structures, they use later normal migrations; Claude-A maintains the shared schema and numbering, and old migrations are not changed.

### Safety invariants that must not be weakened (details in spec §2, §6–§8)

- Before confirmation, zero writes inside authorized roots: no directories, probes, temporary files or hidden metadata. In auto mode, "confirmation" is the moment every condition of the core's check §13.4 holds and execution starts, based on a valid auto-mode agreement. Internal writes go only to the app-data the service chose; the official default is still `~/Library/Application Support/Traeva/`, and tests pass a temporary directory explicitly. App-data and every root must not overlap in either direction; Memory capture must not rewrite user files either.
- Containment uses an fd-relative walk from the authorized root's handle, component by component, never following symlinks, and is checked again after the move; canonicalize + starts_with is not the execution boundary.
- The final move is atomic no-replace (`renameatx_np` + `RENAME_EXCL`). No check-then-rename, no overwrite, no copy-delete fallback.
- `actions.plan_id NOT NULL UNIQUE`: at most one action per plan. A plan's execution content is immutable; changing the target or retrying needs a new plan and a new confirmation.
- Persist the intent first, then perform the file side effect, then record the result; no DB transaction is held during a file operation.
- Recovery only checks records: it never moves, renames, overwrites or deletes user files and never rolls back automatically. Whenever evidence is insufficient the outcome is `needs_user_decision`.
- Undo is conservative: the object, location, hash, original parent directory and original name must all match, otherwise it refuses.

### Tests and evidence

- Tests use only temporary app-data and isolated synthetic files, never personal files.
- Tests run offline by default; real model / Keychain / `.app` checks must be turned on explicitly.
- Evidence records: code version, environment, input hashes, expected/actual, PASS/FAIL/BLOCKED/DEFERRED, kind of test, evidence path; no personal text or keys. Anything not run is marked BLOCKED or "not tested", never passed off as PASS.
- O18-R is marked recovery-logic test (service rebuilt or the same database reopened), O18-C is marked abnormal-process crash test (a real aborted child process). The two are not equivalent, and neither equals a whole-machine power loss.

## 3. Sources that must not be used

- The AGPL upstream source code under `开源:可行性测试/` on the user's Desktop (`开源文件`): do not read, copy or translate it, and it never enters this repository.
- Lab R1/R2/R2.5 and the AI File Sorter audit: reference for behavior and failures only; do not copy their code, prompts, schemas or class layouts; a Lab PASS is not a production PASS. Lab and audit material stay outside the repository.
- The #22 independent seven-case evaluation is sealed material: agents do not view, generate or guess its documents or answers. At the student's submission stage only the SHA-256 is recorded; it is unsealed only after Gate 2 PASS and once the prompt/schema are frozen.

## 4. Session responsibilities and shared files

**Current Memory assignment:** The student owns Memory core implementation, rules and fixes, including version recording, family/lineage, change analysis, retrieval and source associations. Until the student explicitly reassigns this work, agents must not edit, refactor or replace Memory core logic or the student's Memory source files. This assignment applies to work from now on; earlier implementation and review contributions remain recorded in the existing commits, PRs and reports.

**Student-owned source files:** `crates/traeva-core/src/find.rs`, `crates/traeva-core/src/changes.rs`, `crates/traeva-core/src/retrieval.rs` and `crates/traeva-core/src/lineage.rs`. The same no-edit boundary covers `find.rs`, which belongs to file search rather than Version Memory. The student's contribution to the current versions of these four files and the AI assistance used are described in [README.md](README.md#my-contributions-and-ai-assistance); the broader current Memory assignment does not claim authorship of every Memory module.

Agents may inspect these files and their interfaces, identify logic, syntax and Rust coding-practice errors, suggest corrections, connect the delivered interfaces to the app, run isolated tests and report defects. The student makes changes to the protected files unless explicitly assigning a particular edit to an agent. A coordinates shared integration, including Find; B handles Memory UI wiring; C connects source collection to the delivered Memory interfaces. Wiring covers adapters, commands, DTO mirrors, events, permissions and UI calls. It must preserve the student's Find and Memory behavior, including search, association rules, snapshot handling, analysis/retrieval logic, prompts, consent and persistence rules. If wiring requires a change to that logic or its interface contract, report the exact mismatch to the student and wait for the student's change or explicit assignment. Reviewers report failures without fixing the product code themselves.

| Execution Lane | Responsible for | Boundaries |
|---|---|---|
| Student (Find and Memory) | The four source files listed above, including #198 Find; current Memory core implementation and fixes for #31/#32 recording and family/lineage, #33 changes, #34 retrieval, and #27 source association logic | Supplies the behavior and interfaces for integration; agents leave Memory core changes to the student until explicitly reassigned |
| Claude-A — Core | #65 Step 0, shared file/migration integration, #30 summary, #37 final integration; wires the student's Memory implementation into the app | Maintains shared contracts and migration numbering; Memory-related integration follows the student's supplied interfaces and changes no Memory logic |
| Claude-B — Intelligence | #35 Memory UI wiring to the student's #33/#34 interfaces; integration checks and defect reports | Does not edit Memory analysis/retrieval logic or prompts; shared wiring goes to A |
| Codex-C — Source | #27 browser source collection, receiver and UI wiring to the student's Memory interfaces | Does not change Memory source association or persistence rules; shared wiring goes to A |
| Codex-D — DOCX | #66 Limited Reader, real Word saving/parsing, conditional Word Memory verification | Does not take over core Memory; does not start PDF/PPTX/XLSX |
| Codex-E — PPTX | #28 PPTX Reader and evidence (product integration already done by #162) | Changes only its own experiments and evidence; shared files and wiring go to A |
| Codex-F — XLSX | #28 XLSX Reader and evidence (product integration already done by #162) | Same as E |
| Claude — PDF (session) | #28 PDF Reader and #186 core integration are implemented; maintains its reader modules, tests and evidence | Shared files, dependencies and wiring go to A; Memory does not read PDFs; Find uses the text layer only |
| Claude — UI polish (session) | Visual and interaction polish of the existing UI: styles, layout, copy, usability | Changes only the UI layer in `src/`, not business rules; a new small feature first gets an Issue and the student's approval; shared wiring (Tauri events, commands, permissions, file type declarations) goes to A; after Feature Freeze only fixes defects that affect the submission |
| Codex-R — Review | #19, blocker re-verification, #26 Memory independent verification, #37 final core verification (there can be several R sessions, such as R1/R2/R3) | Only reviews/tests/records; does not implement product features or fix blockers |

Codex-C/D/E/F, Claude's PDF experiment session and the UI polish session are all developers, not Codex-R; renaming a session cannot turn the same implementer into an independent reviewer. Work R did not take part in must be independently re-verified by R at the exact commit; review tests R writes do not count as product implementation. The student personally decides on materials and the final merge.

Small features from the UI polish session must not bypass Organize's rules. A file dropped on the Traeva icon or the desktop pet (#208) is handled according to organize-spec §13 (#168): with auto mode off it takes the normal flow (read and analyze → recommend, up to three clarification questions one at a time (§15) → exact preview → user confirmation); only with auto mode on and every condition of the core's check met is it moved automatically. All decisions are in the core; the UI must never decide a move by itself. When the file is not inside an authorized folder, the authorization explanation is shown first, and the flow continues only after the user agrees.

Claude-A is the single integrator of shared files: workspace/package manifests and lockfiles, core `lib.rs`/shared contracts/store/migrations, Tauri startup/command registration/permissions, top-level UI wiring, Vite/Tauri configuration. Other lanes may propose bounded diffs, but must not each upgrade the same dependency, claim a migration number or change shared configuration. Each lane maintains its assigned feature modules and tests within the current Memory boundary above; the shared integration paths are recorded in the #65 contract.

The environment follows the [run isolation plan](docs/development/cac-parallel.md#run-isolation): worktree isolation is not app-data isolation; for now, only one full desktop instance may hold the UI slot at a time, and the others do headless feature development and testing with explicit temporary app-data. Development override parameters of #65 that have not been verified must not be used as if they were existing commands; do not change `HOME`, delete the process lock or change the official release default directory to get around isolation.

Starting implementation and final acceptance are separate: once #65's shared interface PR is merged, each lane can start headless implementation as soon as its own student material and run isolation conditions are met, without waiting for #65 as a whole to close. For current Memory work, the student implements the #32 watcher/durable snapshot foundation and the #31/#33/#34 logic. Agents may prepare integration against the delivered interfaces, but this does not authorize them to implement or revise Memory logic. Family association still requires #31's accepted material, and final #33 acceptance must use real snapshots actually captured, persisted and verified by #32, never fixtures. Stage conditions and the scope of work currently allowed must be written into the Project's Next Action; a final-acceptance dependency must not be used to mark the whole implementation Blocked.

Memory's automatic watching/local capture is not permission to send to the cloud. Before #33's real AI analysis, a Memory-specific explicit authorization must be obtained that states the scope of data sent and of later automatic analysis, and it must be revocable; after revocation, no queued or new request under that authorization may start. Organize's permission must never be reused by default. The student owns Memory consent logic; B/A wire the approved consent controls and interfaces into the app without changing that logic, and R independently re-verifies the integrated behavior. The frozen Organize contract does not change.

## 5. GitHub Project process (`P30g53/Traeva`, Project "Traeva — CAC 2026 Development")

**Ownership**

- **A — Student Design Required**: before implementation the student decides the core rules, states and failure branches. A state table, flowchart, data model, decision rules or "input/output + failure states" are all fine; long pseudocode is not required.
- **B — Student Interface First**: the student defines the interface, constraints and acceptance criteria.
- **C — Opus Implementation**: keeps the historical field name and means implementation according to an approved feature contract. The actual executor follows the current assignment in §4: Memory implementation belongs to the student; other assigned work may belong to a Claude or Codex development lane. The student is responsible for understanding, testing and acceptance.
- **Codex Review**: used only for the three Gates.

**Student Material** (Not Required / Todo / Submitted / Accepted) is independent of Status. Even while an Issue is Blocked, the student can write and submit material first. Product implementation of an A/B task starts only after its material is Accepted. The corresponding development lane only checks whether the student's decisions are enough to implement within the approved slice/safety boundaries, and records the material link and the reason for accepting it; it must never answer for the student or claim on its own that material was submitted. Changing a student decision requires the student to revise/confirm it first. #10 and #11 share material; #31/#34/#27 each have a lightweight question sheet.

While material is not Accepted, a lane may read existing interfaces, inventory fixtures/evidence and dependencies, list test scenarios, and check the #65 contract and its own development environment; it must not pass off preparation as implementing ahead of time or choosing core association or retrieval rules by default. Preparation progress goes in Next Action and does not lift Blocked.

**Status**: Backlog / Blocked / Ready / In Progress / In Review / Done. Blocked means only that implementation cannot start yet; a PR waiting for the student's acceptance is In Review; a Gate waiting for fixes is Blocked and In Review during re-verification.

The Execution Lane is the person responsible for execution and Ownership is the type of student contribution; neither overrides the other. Target Date is the next checkpoint; Next Action holds the current blocker/next step and the active PR; after every real piece of progress, update the Project's actual fields, not only the Issue body. A Backlog status on the #26/#30 tracking parents does not mean Memory was downgraded to Post-MVP. Some PRs being in review does not mean the whole feature is accepted; nothing is set to Done without evidence. Maintainers move statuses after checking dependencies and materials; do not pretend there is an automatic progress bot.

**PR**

- Each implementation Issue has one or more PRs. Intermediate PRs say `Refs #N`; only the final PR that covers all of the Issue's acceptance says `Closes #N`.
- Feature-First: the owner breaks down the internal steps on their own and keeps the Issue's existing acceptance; one or two milestone PRs are a suggestion, decided by review size; no Issue for small functions.
- Intermediate PRs use GitHub's native `Refs` cross-reference, and the PR URL/review status goes into the Project's Next Action; do not add Development closing links that might auto-close the Issue just to fill the Linked pull requests column. Only the final PR covering all acceptance creates the closing link, and the native Linked pull requests column is checked. Gates never get a closing link.
- Only the student merges unless explicitly delegated in chat. Merging a PR = acceptance of that PR; merging the final PR = acceptance of the whole Issue. Agents do not accept on the student's behalf outside that delegation. A's existing merge authorization is recorded in #37, with its scope in §1; it does not authorize closing Gates or declaring untested work accepted.
- The PR description states the change, the commands run and their results, untested/BLOCKED items, and the points the student needs to understand (see `.github/pull_request_template.md`).

**The three Organize Gates** (thresholds unchanged; they do not limit the CAC product's independent Memory verification)

- #13 GATE 1 — Safe Executor
- #19 GATE 2 — AI Organize End-to-End / CAC Demo
- #23 GATE 3 — Final Organize Audit / Production Slice Complete

Gate PRs only reference the Gate and never use auto-close keywords. When Codex-R finds a blocker it creates/reuses a real `bug` Issue: as a sub-issue of that Gate, with the Gate set as blocked-by that Issue, added to the Project / `cac-sprint` with Status / Ownership / Execution Lane / Scope / Student Material / Next Action set. The implementer fixes it; Gate PASS is recorded only after every blocker is closed and re-verified by R, and a person closes the Gate by hand. #18 being done does not mean #19 PASS.

Memory's independent verification is recorded in #26 and linked to #37; final product verification is recorded in #37; no new numbered Gate is created. #19's review is bound to an exact commit/prompt/schema; later features cannot inherit PASS for parts it did not cover, and final integration still needs re-verification.

#26 is closed (R2's final re-verification at `c6a711d`). #37's final verification covers Memory after PPTX/XLSX integration and #168 on the final packaged `.app`. Scheduling checkpoints are recorded in the parallel plan; completed checks, remaining blockers and untested items are recorded in #37. Review usable increments before final integration, and re-verify the affected paths on the exact final package. A merged PR or a passed subtest does not by itself establish #37 PASS.

## 6. Common commands

See [README.md](README.md) for initial dependency installation. Build the frontend first so Tauri can find `dist/`. Before merging, run at least:

```bash
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p traeva-core
```

## 7. Communication

- Organize behavior cites sections of the frozen documents; CAC scope/scheduling cites the approved Issues and the parallel plan, clearly separating what is planned, what exists in code, and what actually passed testing.
