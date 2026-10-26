# #66 Word save lifecycle follow-up

> Historical Word lifecycle report for the commit named below. Statements that production dispatch or model sending are not enabled apply to this test harness and review stage. Current DOCX integration is described in the [README](../../README.md#supported-content) and [shared contract](../development/shared-contract.md); it does not expand the scope of these recorded results.

This follow-up uses main `6cc3a60742c54cac6e3e056eb422cf2af36d2bd2` and its real Rust Snapshot APIs. The bytes-only Reader is called on `read_snapshot` output through the existing standalone D harness. Production dispatch, Semantic Change, Retrieval and cloud sending are not enabled. The October 13 Go/No-Go and R1 acceptance remain pending.

## Reader explanations

- Unsupported documents with no supported body text now name the actual omitted categories in `note`. The names are static Reader categories; neither document text nor field instructions enter the note. Empty documents remain Complete; Partial still requires retained text.
- The 200:1 note now explains that a highly repetitive long document can exceed the limit and becomes Unsupported with no retained prefix. The ratio and parsing behavior are unchanged. A regression uses a valid repetitive main story below the other relevant size/text limits.
- These follow the three student decisions recorded in [A's reply](https://github.com/P30g53/Traeva/pull/100#issuecomment-6101881983), including renewed Memory consent v2 before future DOCX sending. `docx-2` remains unchanged because extraction and status decisions did not change.

## Isolation and evidence boundaries

The test used a separate D-owned worktree; its machine-local location is omitted from this source package. Only D-created synthetic files under `/private/tmp/traeva-d-word-lifecycle-20261010/root` are opened or changed. Every capture gets explicit, separate app-data outside that root. No default Traeva database, personal document, Keychain, provider or external relationship is used. Newly saved Word packages and their text remain local; published evidence contains hashes, statuses, counts and test instructions.

`docx-probe watch FILE NEW_APP_DATA` binds to the initial physical file object. Its stdin commands are `check`, `poll SECONDS` (1–60) and `stop`. Poll uses 100ms between completed checks, plus actual capture time. This is active test polling, not evidence of the production scheduler's approximately two-second capture rate. No replacement is automatically registered or joined to a family. A test-author capture of the current object in fresh app-data is a separate observation, not lineage proof.

## Actual Microsoft Word 16.78 lifecycle

The [claim](https://github.com/P30g53/Traeva/issues/65#issuecomment-6102899503) followed C's return and a check for no Traeva process/port 1420 listener. The [return](https://github.com/P30g53/Traeva/issues/65#issuecomment-6102920320) followed closing the synthetic document. No full Traeva desktop instance was started.

| Case | Observation | Result |
|---|---|---|
| W1 ordinary edited save | Two known paragraphs, Chinese and emoji: 62 Unicode scalars; Complete; exact independent expected text | PASS for supported text |
| W2 Cmd-S without editing | Same inode and SHA-256; Unchanged; one version | PASS, no duplicate |
| W3 edited save v1 → v2 | Word replaced inode `169718612` with `169721742`; old watch returned FileChanged, retained v1; explicit separate v2 capture matched expected text | PASS for refusal/immutability; replacement continuity not enabled |
| W4 successive v3/v4 saves | Cmd-S events 753ms apart; observed inodes `169723009` then `169723289`; old watch retained v2; explicit v4 capture matched expected text | GAP: v3 was not captured or retained |
| W5 save during active polling | Both edited-save sequences occurred within polling intervals. One check observed changed source metadata across its call; replacement errors did not add mixed text or overwrite the previous snapshot | PASS for observed refusal; no proof of a Word write inside the internal 350ms read window |
| W6 close/reopen in Word | Word reopened v4 with the expected two paragraphs and was then closed; final source hash remained unchanged | PASS |
| W7 source rename and move | Four snapshots (seed, v1, v2, v4), each checked after close, after rename and after move out of the original authorized root, using a fresh process each time | 12/12 exact SnapshotRef, byte hash and text checks |
| W8 actual v4 capture process terminated | New isolated app-data captured actual Word v4, then the owned child was killed with SIGKILL; fresh process recovered the same snapshot and exact expected text | PASS after durable capture; not mid-commit/power loss |

The observed path inode sequence was `169555696` (copied seed) → `169718612` (v1) → `169721742` (v2) → `169723009` (v3) → `169723289` (v4), on device `16777232`. These are observation evidence, not a family or lineage decision. Every new object capture was an explicit test-author session with fresh app-data; candidate capture was not enabled.

The original-object watches each retained exactly one snapshot. **This does not deliver uninterrupted Word save history.** The actual rapid-save gap is retained as a limitation, rather than marking the whole capture matrix PASS. New Word packages, independent expected-text files and full local watch logs remain at `/private/tmp/traeva-d-word-lifecycle-20261010/`; no new Word package or body is published in this PR. The final source is `moved-source/lifecycle.docx`.

## Headless cases

| Case | Expected behavior |
|---|---|
| Same bytes rewritten | SameBytes, one immutable version, no source writes by core |
| Atomic replacement at the same path | FileChanged; the original watch does not adopt the replacement; original snapshot remains readable |
| Source renamed and service reopened | Original snapshot bytes/hash remain readable independently of the source |
| Stable truncated Word ZIP | Stable bytes may be snapshotted, but Reader returns Failed with no text; this is not a complete save |
| Concurrent repeated prefix writes | Observed writes overlap capture; FileChanging; no new version and original snapshot unchanged |
| Corrupt/missing snapshot blob | Existing regressions require SnapshotCorrupt/SnapshotUnavailable, without fallback to the source |

The concurrency case is a scheduling-dependent stress replay using bytes from an earlier actual Word fixture. It is not an actual Word GUI save, a deterministic interleaving proof or a guarantee that every application save can be observed. File stability cannot prove that a writer has finished a logical transaction.

## Completed local checks

- Reader/fixture/Snapshot suite: **84 passed, 0 failed** (78 Reader and 6 Snapshot tests).
- Core regression: **769 passed, 0 failed, 9 ignored** with approved local mock-server access. The initial sandbox run had **588 passed / 82 failed / 4 ignored**; all 82 failures were loopback `PermissionDenied`. That failed run is retained, not replaced in the record.
- Workspace and standalone fmt/Clippy: passed. Frontend build and **95 tests**: passed.
- A real child process was killed with SIGKILL **after durable capture completed**; exit status was -9. A fresh process recovered the same SnapshotRef, input hash and exact expected text. This uses an existing actual-Word synthetic fixture, independently of the new GUI run. It is an O18-C-style process termination check, not a kill during commit or a power-loss test.
- [Development review](../../crates/traeva-core/tests/fixtures/docx/lifecycle-development-review.json) found a test enum typo, now corrected, and verified the final four source hashes. It is separate from R1 acceptance.

[Machine evidence and local raw-log hashes](../../crates/traeva-core/tests/fixtures/docx/word-lifecycle-validation.json) retain the initial assertion failures, compile failure, successful reruns and crash check.

## Reproduction

Use a new temporary root and app-data for each run. Build and test without production wiring:

```sh
cargo test --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --offline --locked
cargo clippy --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --all-targets --offline --locked -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/docx_reader/Cargo.toml --check
```

For the Word GUI matrix, claim #65's UI slot first. Copy the existing synthetic `word-layout-v1.docx` to the temporary root. In Word, replace only that copy's body with two known paragraphs and save. Start `docx-probe watch` against that saved file and fresh app-data. Save without editing, then edit/save successive versions while polling. Record physical identity and snapshot outcomes after each step. Capture replacement objects only by an explicit new test-author session. Close the synthetic document, rename/move it, then read every saved Snapshot ID in a fresh process and compare with the independently written expected-text file. Return #65's UI slot after closing the test document.

Do not infer full Word Memory acceptance from these checks. A owns the production wiring and any shared changes; B owns consent and Semantic Change/Retrieval; R1 independently verifies the exact proposed commit. No new format or shared file is changed here.

The responsibilities in the preceding paragraph describe the stage covered by this report. Current Find and Memory work follows [AGENTS.md §4](../../AGENTS.md#4-session-responsibilities-and-shared-files): the student owns the core implementation, agents handle permitted wiring and review, and R independently verifies changes. This clarification does not change the historical tests, results or implementation attribution.

## Integration implications and October 13 boundary

The supported stored-text checks passed and the Reader explanations now follow the student's approved policy. The narrow changes do not alter ZIP/XML limits, failure discarding, external-relationship handling or the extraction algorithm. The earlier security regressions remain in the 84-test suite. Full OOXML interpretation, Word layout fidelity and worst-case/concurrent performance remain outside the claims.

Performance: this run retains per-call debug timings in the machine evidence. The earlier release measurements in [the Reader report](docx-reader.md#performance-and-remaining-validation) are historical selected-input results from the previous source hash, not a new performance or capture-rate guarantee.

A's #103 still needs an updated main base, current Reader/tests and the prepared test migration. B's consent v2 must invalidate prior DOCX send permission before any later sending. The observed replacement gap must be handled through the approved #31 flow and then independently tested with actual snapshots; D does not invent automatic continuity. Safe opening of old Word copies and actual Semantic Change/Retrieval also remain integration work. This follow-up changes no shared manifests, lockfiles, DTOs, schemas, migrations, command registration or production dispatch.

Reader-only delivery can proceed to **R1 review**. Formal Word Memory enablement and the October 13 Go/No-Go remain pending. No merge, Issue closure or change to #26/#37 acceptance is implied.
