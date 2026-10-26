# Conditional Source acquisition experiment

Independent, opt-in macOS research for #27. **No production VERIFIED flag, migration,
Tauri command, accepted Source behavior, or shared permission is changed.**

Read [REPORT.txt](REPORT.txt) (the frozen original, in Chinese; English translation in [REPORT.en.txt](REPORT.en.txt)) before running. The result is GO for the bounded acquisition
mechanism, NO-GO for production integration pending the listed boundaries and Codex-R.

This reuses C-owned M27 receive primitives, removing their old name-based delivery and
HTTP server. It does not import TraevaLab R1/R2/R2.5 or upstream AGPL implementation.

The evidence packages were rewritten because they contained local machine paths; for the original and new hashes and how historical FAILs are kept, see the [privacy rewrite note](EVIDENCE-PRIVACY.md). This cleanup does not mean the frozen experiment was re-run or approved.

## Reproduce on macOS

Requirements: system Python 3, OpenSSL, installed Google Chrome, Node and Playwright.
First set `TRAEVA_PLAYWRIGHT_MODULE`; the operator specifies the installed Playwright module.
The script has no default local directory; when it is not set, it stops before creating the experiment directory or starting the browser.
This configuration fix does not mean approval to re-run the frozen experiment; the existing opt-in and isolation boundaries are unchanged.

```sh
python3 -B -m unittest discover -s experiments/source-verified -p 'test_*.py' -v
node --check experiments/source-verified/worker.js
node --check experiments/source-verified/run_chrome.cjs
python3 -B experiments/source-verified/direct_probe.py
node experiments/source-verified/run_chrome.cjs
python3 -B experiments/source-verified/verify_artifacts.py /private/tmp/traeva-c-verified-chrome-RUN
```

The browser harness creates a fresh headed Chrome user-data directory, a native host
manifest **only inside that directory**, synthetic download directories and isolated
app-data. The lab clicks its explicit consent checkbox and displayed save-location
authorization. This is synthetic test authorization, not a user's production consent.
Chrome writes under the user's home are denied by the lab launcher; the browser is
closed at the end. Existing profiles, cookies and formal host registration are untouched.

HTTPS uses a generated localhost certificate with an exact fixture SPKI exception in
Chrome; the direct client trusts that fixture certificate with hostname checks. Neither
run establishes normal public-CA HTTPS acceptance. No personal credentials are used.

`TRAEVA_LAB_SCENARIOS` selects comma-separated cases. `TRAEVA_LAB_HOLD_UI=1` pauses after
attaching an isolated fixture tab for UI inspection; create `continue-ui` in the printed
run directory to continue. The pause expires rather than keeping Chrome open indefinitely.

## Frozen evidence

`source-verified-evidence.zip` contains final Chrome/direct records, actual synthetic
output bytes, test-created failure/crash evidence, source copies and readback attributes.
`archive-sha256.txt` pins it. Its `evidence/MANIFEST.json` pins every artifact and the
source revision; no browser profile, host token/config or TLS private key is included.
The previous comparison archive is embedded unchanged so earlier failures remain available.

```sh
unzip source-verified-evidence.zip -d /private/tmp/traeva-c-verified-review
```

File attributes are separately recorded in `file-attributes.json`: extracting a ZIP does
not restore macOS quarantine attributes. The output object records and sequence logs are
historical evidence, not proof of a live descriptor after extraction or restart.

Do not install this host in your normal Chrome directory or copy these lab permissions
into the product. The manifest token/caller-origin checks reject ordinary unauthorized
messages; they do not defend against arbitrary malicious code running as the same OS user.

## PR #102 review fixes

Read [REVIEW-FIXES.md](REVIEW-FIXES.md) and the proposed
[minimum integration boundary](INTEGRATION-PROPOSAL.md). The original evidence/report
above are frozen. `review-fixes-evidence.zip` is a separate before/after archive.

```sh
python3 -B -m unittest discover -s experiments/source-verified -p 'test_*.py' -v
node experiments/source-verified/test_worker.cjs
TRAEVA_LAB_SCENARIOS=normal,uppercase,too-long,private-query,redirect,stage-swap,debugger-detach,http node experiments/source-verified/run_chrome.cjs
```

VM tests mock CDP/native replies. Historical schema v1 is strict, but does not authenticate
imported facts or restore current VERIFIED after restart. URLs containing query/fragment or
credentials are unsupported in this isolated takeover lab; this does not change #27's
approved source storage. Neither the repaired prototype nor the proposal enables product
VERIFIED. Run only synthetic fixtures; formal Chrome security/installation remains deferred.
