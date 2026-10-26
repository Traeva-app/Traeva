# #66 actual Word save increment

> Historical evidence increment. PR states, the scheduled Go decision and integration gaps below describe the original handoff. Current DOCX support is recorded in the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md); the Word observations here retain their original commit and scope.

Date: 2026-10-11. This is **D's development evidence from actual Word saves and the existing Core tool**, not independent R1 acceptance, main validation, an App confirmation flow, or approval of the October 13 Go/No-Go. There are no Reader, preview, driver or production changes in this evidence increment.

## Reviewed tool handoff

[#136 remains open and unmerged](https://github.com/P30g53/Traeva/pull/136), at reviewed head `f0e4a420bf59dc4cc4815156856c25a6e0d99506` and base `dd8702338cb12f7c82a1506fb105dbb23d02b388`. [R1's latest review](https://github.com/P30g53/Traeva/pull/136#pullrequestreview-5484869302) gives LIMITED PASS for all three tool/oracle/evidence recommendations, with no mandatory blocker. That branch/head was not changed. This new evidence is on a separate branch. Latest main observed before execution was `eb98835e712d371dff1a3c52d8ea7e67664e1e58`; it is not the tested tree.

The existing standalone `docx-probe` was built offline at the clean reviewed head. Its seven source Hashes, binary Hash, protocol and actual observations are recorded in [the sanitized machine report](docx-word-save-increment-validation.json). #128/#129 Reader/component acceptance and #133/#136 headless fault matrices were reused, not redeveloped or rerun. In particular, damaged/missing blobs and no current-source fallback retain their separately reviewed evidence; no new blob fault was injected here.

## Actual Word operations and results

Microsoft Word **16.78 / 16.78.23100802**, macOS 27.0.1, arm64. Only the previously prepared synthetic original was opened. Its v1 package and v1/v2/v3 text expectations match the pre-existing frozen preparation. v4/v5 extend the missing-save scenario; their intended content was written before editing, with a disclosed terminal-separator expectation correction below. Office package Hashes were recorded after each real save and compared with the actual captured Snapshot; byte-for-byte Office package reproducibility was not required.

| Operation | Observed result |
|---|---|
| Open v1, click Save without editing | Same device/inode, bytes, package Hash and Snapshot; no duplicate version |
| Replace body with v2 in Word, Cmd-S | Actual inode and package changed; exact prewritten text matched. Ordinary Word save was a physical replacement on this machine |
| Core checks with candidate capture off | `file_changed` and unread occupant; original Snapshot retained, one confirmed version, no candidate/pair or automatic continuation |
| Explicitly enable candidate capture | v2 captured as one pending candidate; its real package Hash and shown parent matched; confirmed versions/relations did not increase |
| Tool selects `continue` with the fresh Core page | Exact v1→v2 SnapshotRefs persisted, `user` evidence, replacement gap retained; this is **synthetic test-author/Core confirmation**, not App GUI confirmation |
| Edit/save v3 in Word with capture on | New object first became pending; exact v3 Snapshot/package/text and v2 parent retained |
| Normally exit/restart Core with v3 pending | Same candidate ID, file ID, SnapshotRef/Hash and shown parent; prior confirmed history and relations retained |
| Explicitly confirm v3, normally exit/restart Core | Exact three-version history and chosen user relations survived |
| Core absent, edit/save v4 then v5 in Word | Both actual saved packages observed locally, with new inodes and different bytes; only v5 was subsequently captured |
| Restart, capture/confirm v5 | Exact v3→v5 user relation with `watcher_stopped`; v4 Hash absent from captured history. The intermediate save was not fabricated |
| Close own Word document; rename original | All four historical previews and relations remained unchanged, read from immutable Snapshots |
| Delete only that renamed disposable source; normally restart Core | Same chain/family, all four exact SnapshotRefs/package/text Hashes and user relations; missing source did not prevent local history reads |

The same existing `pairs_after_interruption` oracle permits only conservative additions of `watcher_stopped` after an interruption; it cannot alter endpoints, evidence, edge identity or remove an existing gap. This is the reviewed #136 interpretation of Core coverage, not a new continuity or capture policy. This run's final pair gaps were `replaced_uncaptured`, none, and `watcher_stopped`, respectively.

All saves used device `16777232`. Observed inodes: v1 and no-change save `171393246`; v2 `171579277`; v3 `171579326`; v4 `171579392`; v5 `171579592`. These are observations of this run, not portable IDs or a promise about every Word save.

| Version | Snapshot ID | Actual saved package SHA-256 |
|---|---|---|
| v1 | `bac265e3-cffe-4db3-8fbe-16c0791d0a0d` | `27195e89f2cb810d28374b5d8f9183caef7dfbe6e584a5d5fa9a94d3c3480c41` |
| v2 | `6ccc8f06-d3e5-435e-9f0e-010e3c41949b` | `b83b686e6b614716488d42928b0ef8782fa22f8ee71ba81aac6f1642fdd6700b` |
| v3 | `ca27c397-4c1e-48ce-b49c-aaca67255c79` | `337db5305a81605b4fd84bd59e0b31761ff3ebf4f42e05ae805c55351dfb915a` |
| v4 | **Not captured** | `97943d69154865596b6c09219ff774f8d68ed65acf5bca152c9384760bb5c94f` |
| v5 | `b6cc5f07-54a8-4398-ba25-27c369581e09` | `10a8905ef7a20c144424b9b6fd73db0c543472715c3188a02a7d28cc2380f0f2` |

The machine report also retains full file IDs, chain/family IDs, Snapshot metadata, candidate identities before/after restart, edge IDs and independently specified text Hashes. Extracted text, Word packages, database and screenshots were not uploaded. Raw artifacts remain in D's private local run.

## Retained failures and limits

The raw run has **23 PASS and 3 FAIL rows**, all preserved. The required scoped behaviors have final passing checks, with no product defect established. This is not reported as a flawless 26/26 first run:

- Row 17: the newly written v4 expected output accidentally ended with LF. Existing v1–v3 expectations and Reader paragraph output have no implicit final separator. Word's saved XML contains exactly the intended two paragraphs. The original protocol and expected files remain unchanged; a separate amendment fixes only the output separator, before v5 editing. Row 19 verifies the same saved v4 package against the corrected expectation; there was no new save or product change.
- Row 18: an interactive receipt-helper loop lacked its terminating blank line, so the corrected file had not yet been created. The SyntaxError and failed check are retained. The helper was corrected, not the Reader/Core.
- Row 21: the check incorrectly expected a pending candidate Snapshot to be absent from all history previews. The existing tool includes pending family members, as already explained in #133's independent review. Row 23 checks the retained observation: three confirmed versions and two unchanged relations, plus exactly the final pending candidate Snapshot; no v4 package. No new capture or product change was made to satisfy that check.

An earlier local receipt-helper `KeyError` read `chain_id` at the wrong JSON nesting; the same running Core session was retained and its existing `ready.history.chain_id` read correctly. This is recorded separately and does not change the tested code. The amended expected output and first failed observations are included in the machine evidence.

This is one Word/macOS build, small synthetic paragraphs and explicit sequential Core checks. It does not establish every-save capture, writes during the stable-read window, all Office save modes, abnormal crashes, kill-during-commit, power loss, plugin behavior, or production performance. Normal Core subprocess restarts are not packaged App quit/reopen. Cached fields were not retested; their accepted stale-result and flattened-metadata restrictions remain.

## Isolation, reproduction and remaining wiring

D [claimed #65](https://github.com/P30g53/Traeva/issues/65#issuecomment-6113960372) before Word operations and [returned it](https://github.com/P30g53/Traeva/issues/65#issuecomment-6114031963) after teardown. Word PID `80738`, own document window `64941`, exact source URL and bounds were checked. The document and open dialog were on Dell AW3418DW; the document bounds stayed `x=-2302, y=104, width=1096, height=858`. Only that document was closed. Word was left running; other Office files were not closed. All five Core processes exited normally, own DB/lock had no openers, and port 1420 had no listener. The source root ended empty. App-data was outside the source root and mode 0700. No Traeva App, default database content, Keychain, external document resources, model call or historical Word copy was used; no security setting was changed.

To reproduce the bounded Core/Word sequence, build the existing tool at the exact reviewed head:

```sh
cargo build --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --bin docx-probe --offline --locked
```

Follow [the existing real Word replay protocol](docx-memory-e2e.md#real-word-replay-protocol) with a new private synthetic root and fresh separate app-data. Prewrite expected paragraph output without an implicit final separator; freeze its Hashes before editing. Use the existing JSON-line `check`, `pending`, `enable_candidates`, `decide`, `history` and `quit` operations. Record actual post-save package Hashes and exact IDs. Before the final save, normally exit the driver, save two distinct contents in Word, then resume using the same source/app-data/chain marker. Confirm only the final observed candidate through Core. Close only the owned Word document before source rename/deletion and another Core restart. Preserve failures rather than replacing initial records. The local receipt script merely recorded these operations; no second framework is added to the repository.

Default DOCX dispatch, formal IPC/permissions, Memory entry, actual App confirmation/restart and model-facing Semantic Change/Retrieval remain **BLOCKED on the tested head**. A/B own their existing minimal wiring checklist; this evidence removes the missing D Word-save observation, not their App acceptance obligations. No shared DTO/schema/registration/capability, #103/#124, Memory page or open whitelist changed. Historical DOCX in Word remains **No-Go**. No citation cache is made a default model input. Student Go/No-Go on October 13 and subsequent A/B integration are still pending; #66 remains open and this does not expand #26/#37 acceptance.
