# OCR core port and signed bundle verification report

2026-10-13. This document records developer experiments and in-process development review; it is not R4 independent acceptance. The test platform is macOS 27.0.1 / arm64. Only the frozen synthetic ordinary samples were used; there were no real classification model requests, file uploads or personal file reads.

| Issue | Current conclusion | Measured basis and boundaries |
|---|---|---|
| Conflict between first preparation and the 15-second read | Partly solved | Synthetic preparation uses a separate 90-second budget; everyday reads keep 15 seconds. In the latest signed bundle, first preparation took 70.891 seconds, and the next two took 0.260 / 0.266 seconds. The budgets and entries are now separate, but the first wait is still there; there is no guarantee of permanent initialization or that every device finishes preparing within 90 seconds. |
| Vision memory control | Partly solved | Page-by-page processing, pixel/page limits, actual process footprint monitoring and reaping all have code and experiment results. Under the latest 64 MiB stress threshold it still reached 119.266 MiB, 55.266 MiB over the limit; the old #215 signed bundle was already seen 77.563 MiB over the limit. No strict hard cap on physical memory is implemented. |
| Missing text in the signed bundle context | Still unsolved | The English first page of the same synthetic PDF is recognized as only 16 and 190 non-whitespace characters, respectively, in the two bundle contexts; the three pages' pixel hashes, sizes and recorded actual request options are the same. Agreement between the old and new implementations in the same context only shows the migration is consistent; it cannot remove this quality problem. |

What can be handed to A now is the unregistered core module, the strict child-process interface and the failure evidence. There is enough material to continue integration development, but no basis for enabling by default or for passing product acceptance. When A wires it in, OCR should stay off by default per the agreed rules; after the formal App and R4 re-check, the student decides when and in which version to enable it. This change did not alter the production 15-second and 512 MiB rules, and did not on its own wire into Organize, Auto Drop, Find or Memory.

The additional lifecycle tests for the latest narrowed entry are complete. The re-test with the formal product metadata after the script fix is PASS for 15/15 reads and both preparation verdicts; the first round's 14/15 reads and the gap in its preparation summary verdict remain recorded. The later pass does not cover over the first round's quality fluctuation, and does not change the status that R4 and the formal App have not yet been accepted.

## Versions and scope

| Point | Exact version | Purpose |
|---|---|---|
| Original G branch check starting point | `205801147c85e386101050c2e85dfc50d753535d` | Keeps the original branch, samples and first failures |
| Original G prototype implementation | `6793096907407eb38d29b16df3175f70fae6b027` | Source of the Rust calls into Apple Vision |
| #215 experiment handoff point | `f7923f8f1e0c5055741182e4dd756330cbe7ff72` | Source of this round's port; its main initialization and memory implementation is `c2854919e7d75d8d27a08bbc2c3871f36fcb20e2` |
| Core port baseline | `f0f96eee7b7e9b953937be17421ddcafbfd2849d` | Uses `pdf_child::set_heap_cap` as opened up by A; keeps main's existing reader and interface fixes |
| First-round core matrix | `9751706272151f4d8e86b4d8d190e5d2516a38f6` | Old output contract; 74 PASS / 5 FAIL out of 79 items |
| Full matrix code | `e09e764df408bfbb72dd98f65a6f5acb64a7a7aa` | Narrowed entry and output; 93 PASS / 1 FAIL out of 94 items |
| Final output hardening | `caa5bc3cbbf44a97893a597b053d2ba359f13668` | Rejects abnormal page numbers / text-boundary ambiguity; 13 pure runtime tests; formal metadata reads 14/15, prepares 2/2 |
| metadata verdict script fix | `cbb74e1861495a24601d01921d3a4baee087f149` | Adds the aggregate verdict for the preparation stage; the re-test still uses the `e09e764` core binary |
| #215 stopped-state fix | `98be7d0747a9060d2d063e0ddc08b839778aea69` | Fixes the original experiment's R4-OCR-01; the old-implementation comparison in the final 94-item matrix already includes this fix; the recognition algorithm is unchanged |

The current branch is `codex-g/ocr-core-module`. Core changes are limited to `crates/traeva-core/src/pdf_ocr/`, and verification material is limited to this directory; product module registration, shared Cargo/lock, the Tauri entry, the parent supervisor and the UI are A's responsibility. The old #215 experiment and its evidence are not overwritten by this port.

The verdict criteria were recorded first in the [port protocol](PROTOCOL.md), and the additional comparisons for the text difference were recorded first in the [diagnostic protocol](DIAGNOSTIC-PROTOCOL.md). The first-version and narrowed contracts are counted separately and must not be merged into one pass rate. For the API, dependencies and reproduction commands see the [README](README.md).

## Ordinary samples and the new output

The 15 PDFs and truth are carried over from #215. The script first checks the complete file set and each file's SHA-256, then runs; the ordinary samples, frozen texts and original scoring were not changed. The s09/s10 source-label failures in the old scoring are kept as historical failures; a migration or output pass here does not change the old verdicts.

| Evidence | Result | Conclusion it can support |
|---|---|---|
| [First core matrix](evidence/20261013T072424216549Z.json) | 79 items: 74 PASS / 5 FAIL | Detailed parity fails for s05, s06, s13; the s06/s13 output text also differs from the old implementation outside the bundle. The 15 strict JSON checks pass. |
| [First context pairing](evidence/same-bundle-20261013T080000917414Z.json) | 15/15 exact parity PASS | Under the same Info.plist, the old/new detailed reads are equal; s06's first page has only 16 characters on both sides. |
| [BaselineDiagnostic context pairing](evidence/same-bundle-20261013T080028700387Z.json) | 15/15 exact parity PASS | Under another identical Info.plist, the old/new reads are equal; s06's first page has 190 characters on both sides. |
| [Narrowed full matrix](evidence/20261013T080237189662Z.json) | 94 items: 93 PASS / 1 FAIL | Schema, status/text mapping and page-section checks pass for all 15 samples; the detailed migration comparison is 14/15. |
| [Formal metadata first-round comparison](evidence/product-metadata-20261013T080758123061Z.json) | 14 PASS / 1 FAIL out of 15 reads; no overall pass verdict | The experiment CLI bundle with the formal metadata did not reproduce the s06/s13 truncation to the title; s05 still has a one-character difference. The script gap in the preparation summary verdict was fixed separately. |
| [Formal metadata re-test after the script fix](evidence/product-metadata-20261013T081252375848Z.json) | 15/15 reads, 2/2 preparation verdicts PASS | The re-test with the same plist and signed binary passes; this run's s05 equality must not be used to cover over the first round's one-character difference. |

Both pairing records have source_sha `9751706`; they verify the migration consistency of the detailed diagnostic path and must not pass as verification of the `e09e764` new product output. They also do not wash away the missing text in the First scenario.

The only failure in the narrowed version is `s05-zh-lowres.pdf detailed parity`: that detailed recognition dropped one "的" (a Chinese particle) from "平面图形的面积" ("the area of a plane figure"). Another independent wire call in the same matrix recognized that character again, so the wire mapping passes; the detailed comparison still keeps the FAIL, and the better result is not chosen to cover it.

The narrowed version only accepts a PDF with no usable text layer anywhere in the document. If any page has `Text` or `TextWithOmissions`, it is rejected first, with no rendering or Vision call. Successful text is rebuilt only from the lines of at most three actual OCR pages, keeping the PDF's original page numbers, with no empty heading output for omitted pages. If any attempted page fails, the whole text is discarded; output over 1,000,000 Unicode scalars is also rejected. The success status is always `partial`, with a note about possible recognition errors and omissions.

So the new output for `s08-mixed-layer-scan.pdf` is unsupported/empty text, which is the approved scope narrowing; `s03-mixed-clear.pdf` is a single scanned page with mixed Chinese and English, with no usable text layer, and can still be used for the OCR recovery test. `--inspect` keeps the original detailed diagnostic behavior and must not be used as product output.

Timing and footprint examples from the same narrowed matrix:

| Sample | Old implementation outside bundle: time / peak | New entry inside bundle: time / peak |
|---|---:|---:|
| s01 English single page | 324 ms / 168.720 MiB | 361.022 ms / 169.142 MiB |
| s06 four-page scan, at most three pages recognized | 614 ms / 200.595 MiB | 665.480 ms / 200.845 MiB |
| s13 long scan, at most three pages recognized | 648 ms / 204.782 MiB | 708.202 ms / 205.688 MiB |
| s08 text layer mixed with scanned pages | 368 ms / 175.938 MiB | 14.663 ms / 6.766 MiB, rejected under the new scope |

Data from the [narrowed matrix](evidence/20261013T080237189662Z.json). The outside-bundle/inside-bundle contexts differ, and s08's scope of work changed; this table is not used to prove a single-variable performance improvement from the port.

## Localizing the text difference in signed bundles

The original First bundle repeatedly left only the title `Organic Chemistry` on s06's first page, 16 non-whitespace characters; the comparison context reads five lines, 190 characters. The old binary placed in a copy with the same metadata as First also gives 16 characters, and the new binary placed in another verified context recovers 190 characters. See [old implementation, same ID](evidence/same-id-original-diagnostic-01.json), [new implementation, comparison ID](evidence/same-good-id-core-diagnostic-01.json), [First repeat](evidence/signed-core-repeat-diagnostic-01.json).

In the [metadata single-variable record](evidence/bundle-factor-diagnostic-01.json), keeping the original ID and changing only `CFBundleName` or `CFBundleExecutable` still gives 16 characters on the first page; switching `CFBundleIdentifier` and re-signing gives 190 characters. Signing itself also changes the Mach-O file hash, so the conclusion is stated as an identifier-related bundle and signing context; it cannot claim all internal system factors have been isolated.

The [pixel and request diagnosis in a normal macOS environment](evidence/pixel-request-diagnostic-02.json) reproduces the 16/190 difference, while the three traces on each side are equal one for one: the three pages' grayscale pixel SHA-256, the 1654×2338 size, revision 3, minimumTextHeight 0, the actual language list `zh-Hans/en-US`, automatic language on, language correction on and the Accurate level are all the same. This rules out this render output and these recorded explicit options as the source of the difference; it does not yet explain Vision's internal behavior in different contexts. The good ID must not be treated as a fix, and this must not be used to assert a cache fault or an engine limit.

The diagnostic copy only adds a stderr trace and does not change the formal output schema; for the build identity see [trace-build](evidence/trace-build.json), and for the instrumentation diff see [vision-trace.patch](evidence/vision-trace.patch). The [first trace](evidence/pixel-request-diagnostic-01.json) failed on both sides with `Failed to create CVPixelBuffer` in a restricted execution environment; the original record is kept. It did not reproduce the 16/190 difference with successful recognition and cannot be used for that causal judgment.

The [CPU-only diagnosis](evidence/cpu-same-id-diagnostic-01.json) did not improve the First text; after [old prepare plus language query](evidence/language-query-diagnostic-01.json) it was also still 16 characters. These experiments were not adopted into the formal core, and no system cache was deleted. The one-character change in s05 is kept separately and not inferred to share the same mechanism as s06.

The [first-round comparison](evidence/product-metadata-20261013T080758123061Z.json) with the formal Traeva metadata assembled the experiment CLI bundle using `io.github.p30g53.traeva` and the formal plist contents: 14 PASS / 1 FAIL out of 15 reads. The expected OCR text for s06/s13 is complete, and the title-only problem was not reproduced; s05 low-resolution Chinese still shows a single-character change. The two original preparation records are 34.757 / 0.169 seconds, both finished, exit 0, exact ready and reaped.

That first-round script did not include the preparation results in the summary verdict, so it cannot be called an overall pass. After `cbb74e1` completed the verdict, the [re-test](evidence/product-metadata-20261013T081252375848Z.json) is 15/15 reads and 2/2 preparations PASS; the two preparations took 0.276 / 0.151 seconds, both exact ready, exit 0 and reaped. The re-test used the same plist, signed binary and `e09e764` core code; this is an already-prepared context and cannot be treated as new first-preparation data.

The first-round raw record and s05's one-character change are still kept; getting more complete text from a repeated call is not interpreted as the problem being fixed. The metadata comparison is still an experiment CLI bundle and cannot replace acceptance of A's final Tauri App. The current results do not support changing any shared Info.plist key to solve the OCR problem.

## First wait and recovery

The prepare entry does not read stdin or user files and uses a fixed 960×240 synthetic bitmap. A successful prepare only means that request finished; the parent program must not save a permanent Ready. Product wiring still needs A to implement session state, main-window responsiveness during preparation, falling back to manual pick on failure, and the single task slot.

| Run | Preparation result | What followed |
|---|---:|---|
| New ID in the first core matrix | 34.121 s | Two new-process preparations at 0.148 / 0.146 s |
| New ID in the `e09e764` matrix | 70.891 s | Two new-process preparations at 0.266 / 0.260 s |
| `e09e764` after recognition cancel | Re-prepare 0.284 s | Recovery recognition 0.428 s, reaped |
| `e09e764` after recognition timeout | Re-prepare 0.235 s | Recovery recognition 0.405 s, reaped |
| `e09e764` after memory stop | Re-prepare 0.234 s | Recovery recognition 0.704 s, reaped |

Sources: [first matrix](evidence/20261013T072424216549Z.json), [narrowed matrix](evidence/20261013T080237189662Z.json). In the latter, the 1 ms recognition cancel/timeout happened before the first memory sample; it only proves early stop and recovery and cannot replace a stress test of interrupting recognition.

The [additional prepare and parent-exit tests](evidence/lifecycle-extra-20261013T074059025557Z.json) on an earlier core version are 16/16 PASS: kill→exit for prepare cancel and timeout about 3.018 / 3.024 ms; re-prepare about 31.645 / 31.990 seconds respectively. After force-ending the parent process, the child was observed to disappear within about 188.551 ms, and the following re-prepare and read succeeded. This is evidence for `22c69b4` and cannot simply be stamped onto the later version that moved the watchdog into the core entry.

The [new lifecycle record](evidence/lifecycle-extra-20261013T080456270812Z.json) for `e09e764` ends with `complete=true`, 16/16 PASS, covering three new signed bundles:

| Scenario | Stop or exit observation | Re-prepare with the same ID | Recovery read |
|---|---:|---:|---:|
| Cancel about 1 second into prepare | kill→exit 18.961 ms, empty answer and reaped | 61.969 s | 0.553 s |
| Prepare 1-second budget timeout | kill→exit 30.879 ms, empty answer and reaped | 46.268 s | 0.652 s |
| Force-end the parent process | Child observed to disappear within 156.099 ms | 35.729 s | 0.414 s |

All three re-prepares are exact ready, exit 0 and reaped; all three recovery reads are partial. Preparation after cancel/timeout still takes tens of seconds; an interruption must not be interpreted as system initialization already being complete. The parent-exit observation proves that the watchdog did end the child this time; 50 ms is the thread's target check interval, not an upper bound on the actual exit latency.

The original #215 experiment had already separated the initialization budget from everyday reads; in the signed experiment parent session, the first full preparation took 62.563 seconds and later ones 0.171 to 0.266 seconds, and the conclusion there was also partly solved. The historical source is the [#215 pinned-version engineering report](https://github.com/P30g53/Traeva/blob/f7923f8f1e0c5055741182e4dd756330cbe7ff72/experiments/pdf-ocr/ENGINEERING-REPORT.md). This round's new 70.891 seconds shows the historical maximum must not be taken as an upper bound. The new bundle IDs were all created on this Mac, which already has a Vision usage history, and cannot be called evidence of a freshly installed Mac.

## Memory coverage and brief overruns

| Check stage | Limit or action | Boundary not yet covered |
|---|---|---|
| Before and while reading | stdin at most 20 MiB; exactly one valid parent PID; only approved resource arguments accepted | PDF decoding and Vision have not run yet, so their peaks cannot be inferred from this |
| Before rendering | At most three qualifying scanned pages, 200 dpi, at most 8,000,000 output pixels per page, edge-length planning | Does not limit source image decode size, PDF content caches or Vision internal objects |
| Per-page lifecycle | render→recognize→release, an independent RenderCache per page, Vision autoreleasepool | The PDF's own caches may still persist across pages |
| While running | Target 5 ms sampling, 384 MiB actual-footprint soft threshold; Rust uses the product's existing 512 MiB counting allocator | The scheduling interval may be longer; the Rust allocator does not cover Vision; other system-service processes are not covered |
| Before return and after stop | Reject when a sample is missing; running peak across samples; final sample before reap; drop the result on overrun, confirm wait reaping | Cannot undo a peak that already happened, and cannot prove a strict hard cap on physical memory |

The 64 MiB stress case in the latest full matrix recorded a peak of 119.266 MiB, 55.266 MiB over the limit, kill→exit 18.919 ms, `answer=null` and `child_reaped=true`; preparation and recognition then recovered. The stop latency here is measured from sending the stop action, not from when real memory first crossed the line. The largest recorded observation interval in this case is 22.977 ms; the 5 ms target is not an actual fixed interval. [Raw record](evidence/20261013T080237189662Z.json)

The same kind of case in the first-round core reached 141.423 MiB, 77.423 MiB over the limit, kill→exit 3.033 ms; the #215 signed bundle once reached 141.563 MiB, 77.563 MiB over the limit. The smaller number from the later run does not cover over these overruns that already happened, and is not a future upper bound. The headroom from the 384 MiB threshold to 512 MiB cannot be proven sufficient for arbitrary input from this. [Core first record](evidence/20261013T072424216549Z.json), [#215 pinned-version report](https://github.com/P30g53/Traeva/blob/f7923f8f1e0c5055741182e4dd756330cbe7ff72/experiments/pdf-ocr/ENGINEERING-REPORT.md)

The first real core memory test found the kernel lifetime peak smaller than the current footprint in the same sample; the original assertion failure is kept in [first-memory-test-failure.txt](evidence/first-memory-test-failure.txt). The API returns both the raw kernel value and `max(kernel,current)`; the parent side then takes the maximum across samples. The original failure did not save the numbers, and the report does not guess the difference. The corrected metric is still an observed lower bound and cannot guarantee catching every brief spike.

## Build, signing and handoff boundaries

The [development log](evidence/dev-log.md) records that the narrowed version's 19 core tests, Clippy and the release build pass; the driver's 3 pipe/JSON unit tests also have a pass record. [build-validation.json](evidence/build-validation.json) is a structured transcription of observed tool output, not a raw command log; it records `e09e764`, the built binary hash, the 19+3 tests, and the three PDF fixture filter items that were not run. The log keeps the first memory assertion failure and the Clippy fix process. A normal root build does not compile the unregistered OCR module, so a standalone harness compiles the real core source files directly; the harness extracts the original allocator implementation from the product pdf_child and does not write a separate counting algorithm.

R4-OCR-01 in the original [#215](https://github.com/P30g53/Traeva/pull/215) is the problem of treating `CLD_STOPPED` as an exit by mistake; the fix commit is `98be7d0747a9060d2d063e0ddc08b839778aea69`, the pre-registered protocol commit is `773b818`, and this evidence archive's head is `23caaf5d3034b15eb91ae38d8f81cc10c2db6888`. Its standalone experiment completed developer verification of SIGSTOP→timeout/cancel→reap→re-prepare/recognize in the same parent session, in a real signed session; 34/34 checks pass. kill→exit for timeout/cancel is 8.809 / 2.522 ms respectively, recovery reads in the same parent session are 0.458 / 0.331 seconds, and first preparation is 60.493 seconds; see the [#215 pinned-version raw record](https://github.com/P30g53/Traeva/blob/23caaf5d3034b15eb91ae38d8f81cc10c2db6888/experiments/pdf-ocr/evidence/engineering/20261013T081052830031Z-sigstop.json). This result is not an R4 independent re-verification; R4-OCR-01 must not be marked as passed by R4, and the old session supervisor's results must not replace acceptance of A's formal supervisor.

The latest matrix records the experiment App's ad hoc signing, hardened runtime, strict signature verification and system dynamic library check; the recognition child process clears its environment variables. This bundle is an experiment CLI bundle, not the formal Tauri GUI, and not Developer ID distribution, notarization or App Store sandbox acceptance. The signed bundle hash and the unsigned build hash are saved separately in the [matrix](evidence/20261013T080237189662Z.json).

A still needs to complete module/dependency registration, the real parent supervisor, the 90-second prepare and 15-second read states, 384 MiB soft-threshold monitoring, strict JSON intake, revoking Ready after an anomaly, releasing the task slot only after reaping completes, and the explanations about user waiting and possible OCR errors. Formal source metadata must come from the successful OCR path and must not be guessed from the note text. The agreement and manual-handling boundaries for OCR in Organize are still wired by A per the approved contract; this module does not move files automatically and does not connect to Find or Memory.

R4 still needs to independently re-check on the exact final SHA and the signed App: text quality in the formal bundle context, prepare/cancel/stop/parent exit, memory pressure and malicious compressed images, main-window recovery, and falling back to manual pick on failure. Passing developer scripts, same-context migration equality and reviews by collaborating agents cannot replace this acceptance.

Untested scope: real scanner output approved for use, phone photos or real course materials; new devices, freshly installed systems, after an OS restart, older macOS, Intel; system-service memory, the formal product GUI and release distribution. No system cache was deleted to create first-run conditions. All first-round failures, later re-tests and version records are kept separately.

To reproduce, run the offline build, full matrix and lifecycle commands in the [README](README.md). Use the synthetic samples from the pinned #215 checkout, and save new timestamped evidence each time; known FAILs must be kept. The pairing diagnosis uses [parity_same_bundle.py](parity_same_bundle.py): first confirm both plists are byte-identical and the signatures valid, then compare per-page source, text and omissions. After a change in dependencies, code or bundle identity, evidence may only be re-collected for the affected scope; the old bundle's pass conclusion must not be carried over.

## Final output hardening and re-tests of affected items

The final implementation `caa5bc3cbbf44a97893a597b053d2ba359f13668` only tightens the output on top of e09e764: actual OCR page numbers must be positive and strictly increasing; every actual page section must have non-empty text. If a line of the OCR text, after stripping leading whitespace, starts with the reserved marker `[Page `, the whole document is failed/empty text, so that recognized text is not misread as an extra page header; the recognized text is not modified to manufacture a success. A page marker mentioned in the middle of a normal line is still kept as is. 13 pure runtime tests, fmt, Clippy and release pass. The prepare, monitoring and watchdog source is unchanged; their full matrix was not mechanically re-run.

In the [final signed formal-metadata record](evidence/product-metadata-20261013T082053845902Z.json), both preparations PASS, and the 15 reads are 14 PASS / 1 FAIL, again the single-character difference of s05 missing "的" (a Chinese particle); the s06/s13 English first pages are complete, and the strict page-section constraints are met. The earlier 15/15 re-test was not used to cover this FAIL, so the final result is not called a full quality pass. The full first-version 79 items, the new output's 94 items, the 16 preparation lifecycle items and the latest output re-test are each bound to their own SHA and must not be merged into one pass rate for the current head.

For the evidence hash list see the [manifest](evidence/manifest.json). Two personal home paths in the new evidence were normalized to `<HOME>`; the recognized text, expectations, timings and failure statuses were not changed. The historical records of the original G and #215 were not overwritten. The original #215's current remote head is `39212db6d279c7df9a7ea62995848fafdc6923c7`, which relative to 23caaf5 only adds the SIGSTOP evidence index; R4 independent re-verification is still pending.
