# Memory: semantic changes (#33) and memory-style retrieval (#34) acceptance evidence index

The basis for Closes #33 and #34 is all here: which evidence each acceptance condition maps to, at which commit it was tested, and what boundaries remain. The implementation is all in merged PRs; this file changes no code. The student merging this PR equals acceptance of these two Issues. #35 (the Memory page) and the final integration acceptance of #26 / #37 are not here.

## #33 Semantic changes

| Acceptance condition | Evidence | Result |
|---|---|---|
| 1. One document with 5–10 real versions, with citations and conclusions added and changed; using snapshots that #32 actually captured, persisted and hash-verified | R2: real TextEdit saves, 7 snapshots, 6 real analyses ([#33 R2](https://github.com/P30g53/Traeva/issues/33#issuecomment-6105830857), [re-verification](https://github.com/P30g53/Traeva/issues/33#issuecomment-6106280124)); B: real VS Code saves, 7 snapshots, all 6 pairs `Analyzed`, with the snapshot IDs and hashes listed ([#33 B round 2](https://github.com/P30g53/Traeva/issues/33#issuecomment-6104926285)) | R2 PASS |
| 2. Descriptions have grounds that can be checked; a structurally valid but ungrounded description still counts as a failure | Judged by the student's decision of 2026-10-11 ([decision](https://github.com/P30g53/Traeva/issues/33#issuecomment-6105894275)): a description must name at least one word from the changed lines, otherwise it is recorded as `invalid_output` (#125); the page shows, under each description, the changed lines read again from the snapshot (#127). R2 re-verified by this standard ([MEM-08 re-verification](https://github.com/P30g53/Traeva/issues/33#issuecomment-6106280124)) | R2 PASS (by the student's standard) |
| 3. When the same kind of change happens twice, both boundaries are kept | The two citation fixes are in different version pairs and were not merged into one sentence (R2; the Oke and Bowler fixes in B round 2) | R2 PASS |
| 4. Missing model, API failure, analysis not finished and no change all have explicit states and do not affect saved snapshots | R2's independent failure tests; B round 1 really hit `model_key_rejected`: recorded as Failed, no further automatic sending, the other pairs stayed Queued ([round 1](https://github.com/P30g53/Traeva/issues/33#issuecomment-6104856923)) | R2 PASS |
| 5. Instructions in the document are only data; the full text, raw prompt and key are not additionally written to logs; the execution lock is not held while waiting on the network | R2: instructions in a real document were described as added content; offline tests cover injection, the diagnostics switch, and concurrency while waiting | R2 PASS |
| Memory-specific cloud sending authorization | R2: with no authorization, or an authorization for another file, zero HTTP requests; queued requests are checked again before sending; after revocation no new requests are sent and late replies are dropped; all agreements are revoked at the end | R2 PASS |

## #34 Memory-style retrieval

R2's independent matrix ([#34 R2](https://github.com/P30g53/Traeva/issues/34#issuecomment-6106300416)): 10 first-time real retrievals, with the questions fixed and their hashes recorded before the requests were sent; the files are synthetic files R2 wrote itself, saved through real editors.

| Acceptance condition | Evidence | Result |
|---|---|---|
| 1. The two product examples and new phrasings not used for tuning all return real snapshots and explain the matching change and boundary | R2 Q1–Q4, Q7; plus the held-out question evaluation whose SHA the student sealed, 4/4 (local `RESULTS-eval-run1.md`, not included in this source package; the reported held-out result cannot be independently checked from this package alone) | R2 PASS |
| 2. When the same kind of change appears twice, give candidates or a necessary clarification question; never force a pick of one version | Q1 first asks a clarification question distinguishing Neri and Osei, then picks the version by the user's answer; offline tests cover narrowing candidates over several rounds | R2 PASS |
| 3. A change that never happened clearly shows “not found”; analysis not finished clearly shows “cannot tell yet” | Q5 Not found, Q9 Cannot tell yet, Q6 time-based question Not supported, Q10 Not permitted without permission and zero requests | R2 PASS |
| 4. Results use only real IDs, hashes can be checked, and they can be opened; not relying on file names | The IDs, byte counts and hashes of the 7 snapshots were checked one by one; Q7 opens a 0444 read-only copy whose hash matches the snapshot; all versions use the same file name | R2 PASS |
| 5. First FAILs are kept; old queries, new changes and late results after a cancel do not leak into the wrong history | Offline compound tests cover late replies, IDs of old questions and old choices, and re-checking after a relation is revoked (#116) | R2 PASS |

## Boundaries (what has not been proven)
- Whether a description's wording is right cannot be proven mechanically by rules: a description that shares one real word and is otherwise made up may still be accepted. The user has to judge by comparing it with the original text shown on the page (the student's decision). Word comparison does no lemmatization (bus and buses count as different words).
- Only local TXT / Markdown was tested. Word (#66) waits for the 10/13 decision; notice version 2 is in #124.
- The model's answers differ each time. These are samples from a few real runs, not an accuracy rate.
- #35's window acceptance and the final integration acceptance of #26 / #37 are done separately.
