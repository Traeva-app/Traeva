# DOCX Memory integration checks for A/B

> Historical pre-integration checklist for the baseline named below. Pending Go decisions, unmerged PRs and remaining wiring tasks refer to that handoff, not the current package. See the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md) for current DOCX support. Retain each original evidence boundary.

Baseline: main `069208aa37f07ba6a1539b188c94b67ab975ba60` for the original wiring checklist. #128/#129/#133 are merged with R1 LIMITED PASS. #136's tool hardening has R1 LIMITED PASS at `f0e4a420bf59dc4cc4815156856c25a6e0d99506` and remains unmerged at this handoff. D's [actual Word save increment](docx-word-save-increment.md) used that exact head, not main; it adds evidence, not requirements or production wiring. Reader/component/fault matrices are reused. Formal enablement still needs the October 13 Go/No-Go.

| Owner | Check | Required evidence |
|---|---|---|
| A | Local preview IPC uses the historical Snapshot ID and existing verified `read_snapshot`; no current-source fallback | Real SnapshotRef/Hash → bounded local response; unknown, missing, corrupt and revoked-root errors remain typed; no body in diagnostics |
| A | DOCX defaults and permissions remain gated | Preview does not grant AI send consent, select cached fields as the default model input, or expand `copy_for_opening`; no Word launch |
| B | Memory history passes the actual version to the stable injected preview adapter | Version ID, file ID, snapshot name, capture time, byte count and Hash match Core; original file can be changed/deleted |
| A/B | Carry observed Word replacement through App choice | D now supplies actual Word no-change/edit/replacement, opt-in and Core-choice evidence. Remaining: production GUI presents the real candidate and fresh page, makes the explicit choice, and shows User Confirmed; no automatic continuity inferred |
| A/B | Confirmed continuation survives App restart | D now supplies pending/confirmed identity across normal Core processes. Remaining: same chain/family, actual historical versions, chosen parent relation and `user` evidence after a packaged App quit/reopen |
| B | History status and warnings cross real IPC | Complete/Partial/Unsupported/Failed; cached citation warning and flattened-metadata limit; all extracted text is inert plain text |
| A/B | Failure and late-result paths use real IPC | Missing/corrupt old blob, source deleted, revoked authorization, rapid version switch and error/retry; no stale body, silent fallback or Word auto-open |
| A/B | History gaps remain visible | D now supplies two real Word saves while Core is absent: only final observed bytes captured, `watcher_stopped` retained. Remaining: carry actual Core gap/state through IPC/UI without “every save”/unique first or last claims |
| R1 | Review final integrated head after A/B wiring | Separate packaged App test on #65's UI slot, Dell secondary screen, isolated app-data/root; no production PASS inherited from this Core driver |

All rows involving IPC or the Memory page are **BLOCKED on the tested head**. D's actual Word increment also verifies old Snapshot text after source rename/deletion and normal Core restart. This closes D's missing GUI-source observation; it does not close production IPC/UI, App restart, failure/late-result or R1 integration checks. The driver exercises existing Core APIs and local preview through the standalone validation package; it is not a Tauri command or a production adapter. #136's R1 result does not independently accept the new Word GUI evidence.

Use D's [reproduction guide](docx-memory-e2e.md) and `run_memory_e2e.py`; keep its independent expected-text Hashes. The source deletion and blob fault injections are confined to a fresh, disposable synthetic run. Do not apply those steps to a user's app-data or original documents. Ordinary component safety and Reader ZIP/XML cases remain covered by #128/#129 rather than duplicated here.

Estimated remaining work after a Go: A local IPC/public export and bounded response 0.5–1 day; B Memory entry/adapter and gap/error display 0.5–1 day; A/B/R1 integrated Word matrix 0.5–1 day. These are estimates, not completed work or a promise that Semantic Change/Retrieval accept DOCX. Their model-facing consent/default-extraction choices need their own approved integration and acceptance.
