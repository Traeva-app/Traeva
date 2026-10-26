# Word citation fields and historical opening research (#66, 2026-10-10)

This report is for the October 13 Go/No-Go. It is a Codex-D development experiment and cannot replace the student's decision or Codex-R's independent acceptance on this PR's exact commit.

| Research item | Technical recommendation | Deliverable scope | Formal product boundary |
|---|---|---|---|
| Cached display text of complex fields | **Conditional Go** | Standalone, offline, bytes-only prototype: extracts the stored display result of structurally complete complex fields, excluding codes and structured internal metadata | This PR does not change the production Reader; it does not guarantee the cache is up to date; malformed fields, fields crossing revisions and similar cases stay rejected. Wiring, permission and independent acceptance must be done separately |
| Handing historical DOCX to Word | **No-Go (formal opening)** | The experiment of a real Snapshot of an ordinary synthetic DOCX → independent read-only copy → opened in Word passed; research tools and evidence are kept | The formal open allowlist is not widened. The quarantine origin is not persisted with the Snapshot, there is no isolation proof for Word's external access, and automatic opening of arbitrary DOCX is refused |

Neither conclusion starts formal Word Memory, Semantic Change/Retrieval or cloud sending, and neither changes #103/#124. The existing Reader and #117's lifecycle evidence are kept; #66 is not closed because the research is done, and it does not block #26/#37.

## Code and experiment scope

- Separate worktree `Traeva-D-word-research`, starting from main `0314ae62d86daff056d76ed0c96a350138b493d9`.
- New standalone crate: `crates/traeva-core/tests/word_research/`. It reuses the ZIP/XML handling code that comes before the body visitor in the current `docx.rs`, and replaces the body visitor and the research output label only in its own build output. The research Reader is named `docx-cache-research-1`.
- SHA-256 of the reused production Reader: `765946eb65cf3e48f0daf90367b7a3ea37d339503a4b7d65efeaf45c49371fc2`. The production `docx.rs`, shared manifests/locks, DTOs, schema, migrations, command registration, open allowlist and dispatch are all unchanged.
- macOS 27.0.1 / 26A434, arm64, Rust/Cargo 1.98.1, Microsoft Word 16.78. The producer inside the package is Microsoft Office Word / 16.0000; it cannot stand in for the actual app version.
- The synthetic sources, the six packages saved by real Word, the predefined expected body text and the experiment app-data all stay in `/private/tmp/traeva-d-word-research-20261010/`. The repository commits only its own generator, tests and hash/status evidence, not new Word packages, extracted body text, screenshots, databases or user documents.
- #65 UI slot [claimed](https://github.com/P30g53/Traeva/issues/65#issuecomment-6105348746) / [returned](https://github.com/P30g53/Traeva/issues/65#issuecomment-6105472883). Only its own Word documents were operated on; the Traeva desktop was not started, and nothing listened on 1420. At the user's request, the open dialog and the historical document were moved to the Dell AW3418DW secondary screen, and this was checked with the reverse move menu. No macro, external content or macOS security settings were changed.
- 0 real model calls, 0 Keychain requests; the research tools have no system open command or network client. Word's own existing network behavior was not observed at the process level, so this cannot be used to claim Word made zero network access.

## 1. Cached display text of complex fields

Microsoft's definition of complex fields supports nesting; `begin` / `end` mark the range, `separate` splits the code from the saved result, and a missing separator means there is no result. The prototype reads by these structures and does not interpret or execute field code. [Microsoft FieldChar documentation](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.fieldchar?view=openxml-3.0.1)

The prototype outputs only the supported `w:t`, tab and explicit line breaks in the main body; output is allowed only when all enclosing complex fields have entered their result. While an enclosing field is still in its code area, the display results of nested fields are excluded as well. `instrText`, `delInstrText`, the instruction attribute/subtree of `fldSimple`, non-body parts and the extra data of `fldChar` never enter the body text.

Zotero stores citation information in Word field codes; EndNote's fields also contain hidden citation information. Neither one's database or plugin refresh has anything to do with this Reader. This round verified synthetic fields that match these forms after being saved by real Word; **no plugin was installed, no citation library was accessed, and no citation was generated with a real plugin**. [Zotero field codes](https://www.zotero.org/support/kb/word_field_codes), [EndNote field codes](https://docs.endnote.com/docs/endnote/2025/v1/windows/en/content/09word/field_codes.htm)

Every successful result that contains complex fields is `Partial`, and `left_out` explicitly includes the code/internal metadata exclusion, unknown cache freshness, and not evaluated/refreshed. `dirty=true` is reported separately as possibly stale; `dirty=false` or a missing attribute does not prove it is up to date either. A missing separator or an empty result does not get made-up text; when there are only fields and no result that can be kept, it returns `Unsupported` / empty body. Orphaned, duplicate or unclosed markers return `Failed` / empty body, and must not return a prefix.

When field markers are inside tables, revisions or other omitted subtrees, or when a revision crosses an open complex field, the whole body is conservatively rejected. This is not an algorithmic decision to accept/reject revisions; ordinary revisions are still omitted under the existing limits, and the omissions are listed. Field nesting is additionally limited to 32 levels.

### Real Word save matrix

The generator can exactly reproduce the hashes of the six original seeds and of the independent expected body texts; the expectations were defined before any Word operation. Each seed was opened and saved as a new `*-word.docx`, without overwriting the seed. All six seed hashes still had their original values at the end of the experiment.

| Sample | Seed result | After real Word Save As | Result check |
|---|---|---|---|
| Ordinary CITATION + Zotero/EndNote forms | Partial, 53 characters | Partial, 53 characters; all three field groups still present | Code/synthetic internal secrets not output, exact match |
| Nested fields in both the code area and the result area | Partial, 20 characters | Partial, 20 characters; three field groups, instruction split into four runs | Only visible results in the result area are kept, exact match |
| Field with no result + stale cache marked dirty | Partial, 29 characters | Partial, 29 characters; dirty changed from 1 to 0 | Stale cache kept and freshness still reported unknown; no made-up text for no result |
| Citation field inside an insertion revision | Unsupported, empty body | Unsupported, empty body; insertion revision and field kept | No revision version was chosen without authority |
| Malformed Zotero-form field missing end | Failed, empty body | Before the fix, wrongly Complete / 56 characters; after the fix, Unsupported / empty body | Original FAIL kept, new regression added |
| Two-paragraph historical body with no fields | Complete, 74 characters | Complete, 74 characters | Paragraphs match exactly, used for the historical opening experiment |

When the missing/stale sample was opened, Word asked whether to update fields that may refer to other files; **No** was explicitly chosen, and nothing was refreshed. After saving, `dirty` disappeared while the stale cache did not change. This directly shows that "Word just saved it" or "no dirty" is not evidence of freshness.

The malformed field was this round's decisive failure: when saving, Word 16.78 removed all `fldChar` / `instrText` and moved the manager code into ordinary `w:t`. Excluding by XML region alone is no longer enough. The prototype now conservatively returns Unsupported for documents whose merged display text contains the known markers `ADDIN ZOTERO_ITEM`, `ADDIN ZOTERO_BIBL`, `CSL_CITATION` or `ADDIN EN.CITE`; there are regression tests across runs, letter case and repeated whitespace.

This check will wrongly reject documents whose body legitimately discusses these codes. Once arbitrary internal data without markers has been written as ordinary body text, the Reader cannot reliably recover its origin; it cannot promise to recognize every manager, every anomaly or all metadata. A historical cache is not the latest citation, a checked citation or a citation database record.

### Safety and performance

The existing limits are kept: 20 MiB input, 512 ZIP entries, 8 MiB per entry, 32 MiB total decompressed, 200:1 compression ratio, 4 MiB per XML, XML depth 64, 500,000 events, a 1,000,000 namespace work budget and 1,000,000 output characters. Encryption/macros, malformed directories/compression, CRC mismatch, corrupt packages, DTD/external entities and similar are still rejected; external relationships are only checked, and their targets are not resolved. The prototype adds no field evaluation, file access or network access.

Four samples saved by real Word were each read 100 times in the same debug process: p50 10.97–11.03 ms, p95 11.38–11.62 ms, max 14.65 ms. They are about 12 KB each; the measurement includes the Reader's ZIP/XML/body processing and excludes file reading, Snapshot capture, Word startup and UI. This is a small-sample development measurement on this machine; release throughput, peak RSS for extreme inputs, cross-version Office and a performance SLA were not proven. [Machine evidence](../../crates/traeva-core/tests/word_research/evidence/2026-10-10.json)

## 2. Opening historical DOCX

The research CLI calls the real `read_snapshot` / `copy_for_opening`, reusing the core's hash check and isolated copy. The formal return is still an empty extension and `can_open=false`. Only the research CLI additionally creates a `.docx` file in this new random directory, using create-new to avoid overwriting, and checks the hash, 0444 and nlink=1 again; it does not call Word. The real GUI operation used the copy obtained by renaming in the first experiment; afterwards the tool was changed to keep the original core copy and additionally create-new. The latter was checked headless with the same real Snapshot (two independent inodes / same hash / core copy still present), but no Word GUI run has been done on it yet, so the two must not be described as the same verification. The new CLI copy has no quarantine and was not handed to Word.

The experiment input is the ordinary two-paragraph synthetic DOCX saved by Word in this round. The SHA-256 of both the source and the real persisted Snapshot is `3a8e3017ef4fe2408b9b7530043fa09583d6d20ec0d6db9d910e2466411facfc`. Snapshot ID `782f5312-e640-46a4-bf01-ebb47fd60e6e`, sequence 1.

| Check | Actual result |
|---|---|
| Independent copy | Source inode 170690006, experiment copy inode 170690242, nlink=1, same byte hash |
| Read-only copy | Copy mode=0444; Word title `[Read-Only]`, edit/Save controls disabled, Duplicate offered |
| Save action | The save shortcut showed selection locked; Duplicate was not clicked, and no save was needed on close |
| Original file protection | After closing Word, the original's hash / inode / mode / quarantine were all unchanged; the copy's hash/inode were also unchanged |
| Snapshot reliability | After closing, a new process reopened the real app-data, and the `read_snapshot` hash / SnapshotRef / 74-character result matched exactly |
| quarantine | The value `0083;6acb103e;TraevaWordResearch;` preset on its own synthetic source was passed to the copy unchanged; after Word opened and closed it, it still had the original value. The quarantine bits were not deleted or changed |
| Protected View | Not observed; only read-only caused by file permissions was observed. It cannot be called Protected View or sandbox verification |
| External content | The prototype rejects external relationships, any fields, embedded/linked/auto-update content and parts outside the subset; these packages were not handed to Word. Whether Word goes online under the various protection settings was not verified |

### Two unresolved problems for formal opening

1. **The historical security origin is missing.** A Snapshot stores the bytes but not the original file's quarantine xattr. The file the core copies has no marker either. This experiment took a known marker from its own current synthetic source, which only proves the passing and keeping mechanism; the product must not use the current source's attributes in place of the historical Snapshot's security attributes, let alone treat an unknown origin as safe by default. The first xattr write to a 0444 file was refused by macOS; the original failure is kept. The experiment then wrote the original value only to its own new copy, not yet opened, during the staging stage, and then sealed it as 0444. A formal version should finish the attribute write, read-back and publish through a controlled fd before the final read-only seal, and hand the needed shared changes to A to coordinate.
2. **Read-only/quarantine is not isolation of Word's external content.** 0444 protected the save behavior in this experiment, but the same user can still change the permissions or replace the copy; it does not stop content execution or external updates. Real Word already showed an update prompt for another field document. The research deny gate is only a negative filter used to pick this round's ordinary sample, not a complete validator of Office active content; an empty reason list does not authorize automatic opening. No security protection was lowered, and no document with potential external access was handed to Word for testing.

Apple's documentation says hard-quarantine can block a file from opening, but that does not prove that Word documents are isolated from network/content execution; Gatekeeper's code signing check cannot be extended into a guarantee that any DOCX opens safely either. [Apple hard quarantine](https://developer.apple.com/documentation/security/errseccsfilehardquarantined), [Apple Code Signing Guide](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)

So formal historical Word opening is still No-Go. It must not be solved by removing quarantine, enabling editing, changing macro/external content protection, or loosening the allowlist. The independent copy mechanism and this round's ordinary-sample evidence are kept.

## Development effort and next stage

The following are engineering estimates, not approval of new rules or a promise of completion dates; they will not be carried out directly in this round.

| Follow-up work | Estimate | Preconditions / uncertainty |
|---|---|---|
| Turn the cache visitor into a reviewable limited production Reader | D 1–2 person-days | Widen the prototype's security counterexamples, fix the rejection boundaries for malformed fields and revisions; samples generated by real Zotero/EndNote plugins are untested |
| A/B wiring, keeping Partial and the cache warning, re-checking the sending permission | 1–2 person-days in total | A changes the shared entry; the current permission that excludes all fields must not silently become one that includes the cache. This needs a separate explicit decision; this round does not change #103/#124 |
| R independently accepts the exact commit and expands the real Word samples | 1–2 person-days | Time depends on the UI slot and the Word/plugin environment; this round's development PASS does not replace R |
| Origin attributes, publish boundary and macOS/Word security matrix for formal historical opening | Preliminary 3–5 person-days | Covers only ordinary-content candidates; the cost of guaranteeing no external access for arbitrary DOCX is unknown and cannot be promised within these few days |
| Semantic Change / Retrieval integration itself | A/B about 0.5–1 person-day, plus 1–2 person-days of flow verification | Reuses the approved interfaces, and still needs real Snapshots, sending permission, unknowns/gaps and the cache qualification; safe historical opening and complete Word save capture are not done yet |

For 10/13 it is recommended to evaluate only the existing Reader and this round's limited cache candidate, and not to use this research to announce a full Word Memory Go ahead of time. If the full flow requires safe automatic opening of arbitrary historical DOCX, this round's evidence supports No-Go: keep the Reader and stop widening the scope.

## Checks, failures and reproduction

Final dedicated tests: **111 passed / 0 failed**: 78 regressions of the original production Reader, 30 prototype tests for cache/open filtering, and 3 real core Snapshot/copy tests. The last group covers: a copy that the test author can rewrite still does not affect the source or the Snapshot; a new copy is independent; rereading across services after the source is rewritten/moved away; and refusing to copy when the Snapshot is lost, even if the source is still there. They are headless tests and do not pretend to be the Word GUI.

Repository-level fmt / Clippy, core offline regression **791 passed / 10 ignored**, frontend build and **106 tests** passed; final results and log hashes are in the machine evidence. The core regression uses only local mock services; it ran in an execution environment that allows loopback, and read no keys and accessed no real provider.

Initial failures kept: a SHA-256 formatting compile error; a test misusing a nonexistent SnapshotMissing (actually SnapshotUnavailable); a hidden temporary root rejected by the safety rules; a truncation note assertion that did not match the existing summary behavior; Clippy's no-argument format; the malformed Word save leaking known manager markers; the Python xattr API being unavailable; and permission denied when writing an xattr to the read-only copy. What was changed was the prototype or the tests; product protections were not weakened and the initial results were not erased.

```bash
cargo test --offline --manifest-path crates/traeva-core/tests/word_research/Cargo.toml
cargo clippy --offline --manifest-path crates/traeva-core/tests/word_research/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path crates/traeva-core/tests/word_research/Cargo.toml --check
rustfmt --edition 2024 --check crates/traeva-core/tests/word_research/cached_body.rs
python3 crates/traeva-core/tests/word_research/make_seeds.py /private/tmp/traeva-word-research-new-run
```

The generator requires a brand-new directory. `word-research inspect FILE EXPECTED` prints only the hash, scope and whether it matches, not the body text; `bench FILE 100` measures Reader time. `capture FILE NEW_APP_DATA` uses only a brand-new isolated app-data; `snapshot APP_DATA ID EXPECTED` rereads the actual Snapshot. `copy APP_DATA ID` is a manual research candidate: **it does not apply quarantine, does not open Word, and must not be used in production**.

GUI re-verification must first claim the slot per #65 and use only the generator's files: open each one and Save As a new local file; choose No at the update-fields prompt, and neither accept nor reject revisions. The historical copy experiment must first check the bytes, independent inode/nlink, quarantine origin and read-back, and 0444, and only then hand the copy to Word manually; files with unknown attributes or rejected by the filter are not handed to Word. Move the window to the Dell secondary screen; at the end, close your own documents and return the slot. This step does not need the student to operate on existing real documents.

Untested: real plugin/citation library refresh, other Office/macOS versions, execution or network behavior of malicious DOCX in Word, formal window-opening wiring, UI sending permission, real model, full Word → Semantic Change → Retrieval, formal opening of old versions, and Codex-R's independent review of the new research head.
