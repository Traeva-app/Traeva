# PR #102 review fixes and minimal integration plan

**Conclusion: the bounded experiment fixes can go to R1 for targeted re-verification; formal product VERIFIED is still NO-GO.**
This round did not modify the accepted #27, shared core/contracts/permissions, A/B Memory or #37, and did not merge.

Baseline main: 6f14e3a7dbdb9b60f2d7945054fc1d8556f8ccdf.
Original experiment / R1 review: dcbf13317ea729b177b7d92a502ddadc501fe445, PR #102.
The final fixed source is pinned by the new evidence package's environment.codeRevision and source file hashes; the PR's latest head
also includes a separate evidence commit. The original REPORT.txt / source-verified-evidence.zip are not overwritten.

| Item | Original FAIL (retained) | Fix and verification |
|---|---|---|
| B1 | A three-field JSON was taken as a historical acquisition success | The v1 receipt strictly checks all required fields, types/ranges, namespace/full binding, filename/ID, created and final target identity, hash, time and the historical marker. Missing fields, null, bool, and wrong identity/binding/version are all rejected; duplicate JSON keys, symlinks and FIFOs are also rejected. Old unversioned receipts are not upgraded automatically. |
| B2 | A log EIO after the receipt commit made live report a definite failure, yet after restart it read a success | File fsync + replace + app-data directory fsync is the receipt commit point. Faults in event writes/fsync/state saving after the commit only attach a warning and keep the historical success. If the replace is visible but directory persistence is not confirmed, the result is OUTCOME_UNKNOWN; a lost reply is also UNKNOWN, with no abort/re-download/receipt deletion. |
| B3 | The case-sensitivity support check passed, but filename parsing failed after taking the stream | A single parser produces the response, filename and full destination before taking the stream; length limit, parameter case, extension, field types, response format and Native prepare are all checked first. An existing target is rejected first; later races are still rejected by exclusive creation. |
| B4 | POST, Cookie, Authorization, body and query stayed in events/cache, Set-Cookie stayed in the receipt | Full events are not cached; minimal event allowlist, response header allowlist, fixed error codes; no full search of browser history. The experiment only supports synthetic URLs with no credentials/query/fragment; scans of memory, persistent records and archives from the pinned runs contain no sensitive sentinel values. |

R1's independent original FAIL archive keeps its original SHA:
`d1ff9c355755859bf35f404caa1fbd6bab337e3a6639a469d2a6d9d19b915ca3`.
C's original #102 archive is still `043334e6dfd6fa944c1c72d30eae5c7fa259eaec3f71cdfbf42f9f67acd0c2d0`.
The before-writer / before-worker reproduced in this round and the original source are also kept separately; the original FAIL is not changed to PASS.

## Measured scope

- 31 Python tests pass, including the original 22 and new receipt/commit/privacy/quota counterexamples. Multi-field tampering is expanded as
  subtests; each variant is not inflated into a separate feature acceptance. A real Native child process loses its reply with
  `os._exit(74)` after the receipt commit; the new process only accepts strict historical receipts and does not restore the live handle.
- 37 VM scenarios on the actual worker source pass. CDP/Native replies are simulated, covering valid/rejected filename
  boundaries, Native rejection, uncertain commit / lost reply, epoch change, overlapping tasks in the same tab, privacy and quota.
  These do not count as real Chrome acceptance.
- The final source completed 8 targeted download scenarios (plus 2 authorization checks) in a separate, headed real Chrome:
  normal, uppercase, too-long, private-query, redirect, stage-swap, debugger-detach, http.
  5 acquisition receipts succeeded; too-long/query/HTTP were all completed normally by Chrome; the debugger disconnect is an explicit failure with
  no receipt. Concurrently swapped temp paths still deliver each task's correct bytes and new target object. 157 record/file/
  attribute/privacy consistency checks pass. This is not 157 independent safety proofs.
- The original direct synthetic TLS method re-ran 5 scenarios: normal and two concurrent deliveries succeeded, truncation and unsupported chunking were rejected;
  strict receipt read-back keeps the history. The actual network is localhost TLS with a generated certificate, not public-CA acceptance.
- An epoch/native connection change invalidates old handlers. The experiment allows at most 4 active tasks, 32 lifetime tasks,
  80 MiB total declared receive volume; it keeps at most 64 task FDs (plus directory/log FDs). Browser events at most
  2048 entries, metadata cache 128 entries; Native log at most 16384 entries/32 MiB. Insufficient budget is rejected up front.

New evidence: `review-fixes-evidence.zip` and `review-fixes-sha256.txt`; the MANIFEST pins all files.
It contains real synthetic output, object and xattr read-backs, run records, source, before/after comparisons and R1's original archive.
The ZIP keeps no live handles and does not restore xattrs; the attribute records are historical observations.
The privacy scan only constrains the run output after the fix; the test source/input definitions and the original FAIL archives contain synthetic sentinels,
which are necessary evidence; this is not used to hide real secrets or to modify the original FAIL.

Python syntax passed an in-memory check. One py_compile was denied by the sandbox because the system tried to write an Apple external cache;
this is recorded as a tool-environment failure, a check that writes nothing to disk was used instead, and no request was made to open up personal caches. Rust /
product builds / cloud tests were not run: all changes are only in experiments and cannot inherit product or CI acceptance.

## Still to solve for formal integration

The full minimal plan is in [INTEGRATION-PROPOSAL.md](INTEGRATION-PROPOSAL.md); it is a proposal, not an approved shared interface.
[Sent to Claude-A for checking in #65](https://github.com/P30g53/Traeva/issues/65#issuecomment-6101837773):
core holds the target handle and a one-time capability, the current root/takeover-authorization generation, exact registration and
source binding in the same transaction, the commit/revocation order, and reconciliation recovery. No confirmation from A has been received yet; this must not be written up as checked and passed.

The presence of macOS quarantine/WhereFroms does not prove that Chrome Safe Browsing / dangerous-download verdicts, Gatekeeper/
XProtect and similar protections are preserved; after takeover cancels the original Chrome download, the danger verdict still lacks a trusted boundary. This is still a
blocker for enabling it in the product. The actual debugger permission prompt / user cancel / DevTools conflicts, formal installation signing and
extension ID, pairing rotation/revocation have also not been accepted; default permissions are unchanged.

Unsupported scenarios are let through before taking the stream; a failure after taking the stream must show takeover failed or uncertain, must not claim Chrome
completed normally, and must not silently re-download. The original User Confirmed is not upgraded, and an acquisition is not passed off as a Chrome ID.
This experiment rejecting query is only the research scope; it does not change #27's approved storage rule for local download links.

Public CA, real same-tab concurrency, complex non-ASCII/binary, cross-origin/iframe/worker, login/POST/blob,
HTTP2/3, cross-volume/cloud, real power loss, atomic publishing and safe leftover cleanup are still unverified. The original failure evidence and
FIRST VERIFIED DEFERRED are kept; this round's research PASS is not counted toward #27 product VERIFIED PASS.

**The 10/20 Freeze risk was flagged in advance in #65**: a new core write path, permissions/installation and safety acceptance could take up
time needed to finish Memory. Recommended: schedule it separately, and do not let #37 or the Accepted #27 depend on this research; if the conditions are not
closed, it stays off at freeze. Next step: only hand R1 the re-verification of B1–B4 on the new head; the student decides on formal integration.
