# #66 DOCX Memory end-to-end preparation

> Historical preparation and test-tool report for the commits named below. Its pending review and wiring statements are not the current product status. See the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md) for current DOCX support. Replaying this standalone tool does not establish packaged-app acceptance.

The original **test-only Core driver and replay** was delivered in #133, based on main `069208aa37f07ba6a1539b188c94b67ab975ba60`. It reuses the actual Memory capture/candidate/confirmation/history APIs and #128's local Snapshot preview. R1 gave #133 LIMITED PASS at `553778b7075fdced13e25249994c34a527202387`; the student merged it. #128/#129/#133 acceptance remains confined to their reviewed scopes. The following hardening is based on main `dd8702338cb12f7c82a1506fb105dbb23d02b388` and needs its own R1 review.

## Reproduction

```sh
cargo build --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --bin docx-probe --offline --locked
python3 crates/traeva-core/tests/docx_reader/run_memory_e2e.py
PYTHONPYCACHEPREFIX=/private/tmp/traeva-d-e2e-pycache python3 -m unittest discover -s crates/traeva-core/tests/docx_reader -p test_memory_e2e_oracles.py
cargo test --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml memory_e2e::tests --offline --locked
cargo clippy --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --all-targets --offline --locked -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --check
```

The replay creates a fresh `traeva-d-memory-e2e-*` directory in `/private/tmp`, a synthetic source root and separate app-data. `--output FRESH_DIRECTORY` selects another fresh run location. It refuses an existing output directory. New CLI sessions explicitly create app-data with mode 0700, including under a shared temporary parent. It never opens Word, starts a Traeva App, requests a provider, or enables sending. Do not use personal documents. `protocol.json` freezes the expected text Hashes and actions before capture; `transcript.jsonl` and `report.json` retain full version IDs, Hashes, choices, expected/actual results, process exits, source hashes and environment. `base_revision` identifies the branch base; `tested_tree` separately records HEAD, whether the working tree was dirty, and source Hashes. A pre-commit run is bound to those source Hashes, not described as testing a bare base that lacks the changes. Logs contain metadata and text Hashes, not extracted body text. Machine-specific raw paths stay local; checked-in reports are sanitized.

The two `word-layout` fixtures are existing synthetic packages saved by actual Word 16.78 in the accepted lifecycle work. The replay uses those bytes, plus the existing `word_research/make_seeds.py` generator. Its `os.replace` operations are **synthetic physical replacements, not new Word GUI saves**. Replaying them verifies the new Core chain; it does not repeat accepted parser/preview tests or claim a production scheduler rate.

## Core matrix

| Scenario | Required result |
|---|---|
| First actual immutable Snapshot | Core captures fixture bytes; local history preview text Hash matches independently specified expected text |
| Object replaced while candidate capture is off | New inode; unread occupant; original Snapshot retained; no implicit relation |
| Candidate capture explicitly enabled | One pending candidate; no usable pair before confirmation |
| Explicit test-author continuation | Current page token and shown parent sent to existing Core; watch moves; relation evidence is `user`; replacement gap remains |
| Process exits with Memory still active | App-data survives; a subsequent Core process can recover candidates and history |
| Two saves during watcher absence | Only observed final candidate is retained; intermediate bytes are not invented; `watcher_stopped` survives on the eventual pair |
| Restart with a candidate still pending | Candidate and fresh confirmation page survive; explicit confirmation completes the chain |
| Original source deleted; process restarted | All saved versions remain readable with the same IDs, Hashes and text Hashes; source-loss gap remains |
| One historical blob corrupted, then removed | That version gives `snapshot_corrupt`, then `snapshot_unavailable`; other versions remain readable; no source fallback |

The driver does not determine whether a real user's new object continues a work. The synthetic test author explicitly selects its known saved copy using the approved #31 interface. This is **Core confirmation evidence, not a human GUI confirmation or an algorithm policy**. Snapshots are read through Core, with Hash checking; storage layout is used only by the replay's isolated fault injection. The unchanged Reader/preview security suites, Word close/reopen, fast-save observation and post-capture SIGKILL evidence are reused from [#117's report](docx-word-lifecycle.md), [#128's report](docx-local-preview.md), and #129/R1. No kill-during-commit or power-loss result is claimed.

## Real Word replay protocol

Claim #65's desktop slot after its previous holder returns it. Operate only the synthetic source copy and put every test window on Dell's secondary display. Do not open any Snapshot or copied historical DOCX in Word.

1. Copy the existing `word-layout-v1.docx` into a fresh synthetic root. Write independent v2/v3 expected text files **before** editing. Start `docx-probe memory-e2e SOURCE.docx FRESH_APP_DATA` and retain JSONL output.
2. Open only that source in Microsoft Word. Replace its body with known v2 text and save. Send `{"op":"check"}` and `{"op":"pending"}` to the driver; record physical identity and refusal/retained initial Snapshot.
3. Send `{"op":"enable_candidates"}`, then `{"op":"pending"}`. Send `{"op":"decide","request":...}` using that exact chain/page/candidate/shown-parent and explicit `continue` choice. Do not derive continuity from a name, Hash or inode.
4. Edit/save v3 in Word with candidate capture on, check and confirm its pending candidate using another fresh page. Obtain `{"op":"history"}` and compare every historical text Hash with its prewritten expectation.
5. Close only the synthetic Word document. Quit the driver without stopping Memory, delete/move only this disposable original, and restart with `docx-probe memory-e2e SOURCE.docx APP_DATA CHAIN_ID`. Check the retained history and gaps. Optional corrupted/missing-blob steps must target only this run's own blob, after confirming its expected bytes/Hash.
6. Retain actual file IDs, version IDs, package/text Hashes, before/after device/inode, expected/actual results and process exit statuses. Return #65's slot. A full Tauri confirmation/page route is still BLOCKED until A/B connect it.

`memory-e2e` supports JSON-line `history`, `check`, `enable_candidates`, `pending`, `decide` and `quit`. It never chooses a relation automatically. Resume requires this driver's own session marker with matching chain/source, so it will not silently create/open default app-data. Exiting ends the test process without invoking `stop_memory`; it therefore models a watcher interruption, not a user switching Memory off. History output includes each typed preview error instead of dropping the failed version.

## Status and boundaries

The initial synthetic replay passed 18 assertions. Two new isolation/identity checks passed. Initial development failures are retained in the machine report: SHA2 formatting compile error, hidden temporary-root test rejection, and test-module ordering Clippy error; each was corrected without changing Core/Reader behavior. Python bytecode compilation initially hit the sandboxed system cache; selecting a temporary cache resolved that environment check.

### R1's three follow-up recommendations

The [independent #133 review](https://github.com/P30g53/Traeva/pull/133#pullrequestreview-5484417617) found no blocker and suggested these improvements:

- The replay now retains exact SnapshotRefs and candidate/file/snapshot IDs before restart or confirmation, requires the expected package-Hash set, and checks exact pair endpoints/evidence. Deletion/recovery compares complete stored previews and pairs. Fault injection requires the precise victim version and Memory error, with every unaffected preview identical to its baseline. Five test-oracle regressions reject changed IDs behind known Hashes, duplicate/missing packages, recreated candidates, wrong fault victims, altered previews or changed relations.
- New app-data is created atomically with explicit 0700 permissions. A regression uses a direct child of shared `/private/tmp`; the replay also checks permissions with inherited umask 000. Marker and Core-managed file modes are unchanged, protected by that directory. Older sessions are not modified; retain their private parent as in the original replay.
- The historical report's `code_revision` is clarified as `base_revision` plus a dirty `tested_tree` and source Hashes. Its original 18 results/log Hashes remain intact. New runs use the same explicit distinction.

The initial permission regression reproduced 0755 and failed before the fix. The first stronger pair comparison also failed: only `watcher_stopped` had been added to an existing pair. Existing Core keeps conservative downtime from the previous run's last persisted heartbeat, so restarting can extend a gap into an earlier captured interval. The corrected oracle preserves exact pair metadata and existing gap kinds, permitting only this additional `watcher_stopped` after an interruption; it rejects dropped gaps or changed relation/evidence. Later source-deletion/recovery comparisons use the complete post-interruption baseline. The failed run and its diff remain in the [hardening evidence](docx-memory-e2e-hardening-validation.json). This changes the test expectation, not Core coverage policy or student-owned rules.

Final hardening checks: **25 Core replay assertions, 3 Rust driver tests, and 5 Python oracle tests passed**. The 127 accepted Reader/Snapshot/preview cases remain filtered out. Standalone build/Clippy, workspace/standalone fmt, Python syntax and diff checks passed. The final replay used the clean committed source tree `67e17a579ca26be714c2f438398c2654cb4e0686`, with inherited umask 000. This is local validation; new independent acceptance is pending. No CI or real Word GUI result is claimed.

Real Word GUI increment: **not yet executed**. R2 has [returned #65's slot](https://github.com/P30g53/Traeva/issues/65#issuecomment-6112458474); D's next Word test must claim it afresh and keep windows on Dell. This review-follow-up remained headless. Full DOCX → Tauri IPC → Memory page → Semantic Change/Retrieval: **BLOCKED on this main**. `reader::read_text` still returns `none/Unsupported` for DOCX and the merged component is not wired into Memory history; the standalone driver deliberately calls the accepted local preview function. No Reader, schema, shared DTO, migration, command, capability, Memory page, #103 or #124 change is included. Formal Word open remains No-Go; no cache is sent to a model. No #26/#37 acceptance is changed.

The [A/B integration checklist](docx-memory-integration-checklist.md) names the remaining wiring and acceptance work. Formal Word Memory remains subject to the October 13 Go/No-Go; this increment prepares evidence and does not enable it.
