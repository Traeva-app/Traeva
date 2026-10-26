# Conditional acquisition integration — proposed, not approved

This proposal follows R1's review of PR #102 at dcbf133 and the current shared contract
at 6f14e3a (§4.1 and §4.5). It changes no shared interface or accepted #27 behavior.
The experiment fix may proceed independently. Production VERIFIED remains NO-GO until
A reviews the interface, the student approves integration, and R1 verifies the new boundary.

## Minimum trustworthy path

Chrome's paired extension supplies one specific Fetch response stream to a **core-owned
Rust acquisition writer**. The Python lab and its receipts are never an import authority.
The extension/bridge are part of the explicitly authorized trusted computing base; CDP IDs,
pathnames, supplied file_id, hashes and caller argv are not standalone authentication.
A compromised trusted extension or arbitrary malicious same-user code is outside the
current lab's security claim and must be addressed in the production threat model.

Proposed private core operations, with names and storage owned by A:

1. `reserve_acquisition`: check separate takeover consent, currently active root grant and
   its generation, validated response/filename/support plan, directory chain, quota and
   no-replace destination. Issue a short-lived one-use opaque lease in a separate
   `TRAEVA_CHROME_FETCH_ACQUISITION` namespace. Preflight does not promise that the
   destination cannot later become occupied. Its actual exclusive create must still fail.
2. `begin_acquisition` consumes that lease and attaches an authenticated, ordered stream
   to the writer it creates. Core owns the receive and destination file handles from birth.
   JSON cannot mint a trusted acquisition or choose an existing file_id. With a separate
   native process, transferring an actual FD needs an authenticated local IPC design
   (e.g. SCM_RIGHTS); sending an integer FD or serialized object evidence is insufficient.
   Prefer the writer living in core so this extra transfer is unnecessary.
3. `commit_acquisition` accepts only an internal capability referencing the live writer and
   its destination handle. Under the existing execution/revocation lock order, revalidate
   root generation and every directory, fixed byte count, stable readback, safety attributes,
   and that the authorized destination currently names **that exact live target object**.
   Build `SeenFile` from this retained handle. Reuse `observe_in` inside the same database
   transaction as acquisition/source association and capability consumption. Do not call
   `register_file(path)` later and then attach an old receipt to whatever object it finds.
4. `acquisition_outcome(id)` reconciles the authoritative committed row. Repeating the same
   authenticated request returns its existing historical outcome without creating another
   file or association. Conflicting namespace/request/capability tuples are rejected.

The FileRecord refers to the newly created target object, not the staging object. Physical
fields and hash substantiate that live observation; they do not merge file_id or families.
The observation has an exact time. A replacement after it invalidates the current-location
claim without rewriting history or binding the replacement. Current display/actions must
re-observe; neither a receipt nor database `present` means the path is currently verified.
Existing User Confirmed records, copies and family members are never upgraded by this path.

## Commit, restart and revocation

Use one authoritative core transaction for FileRecord + source/acquisition association.
The experiment's receipt is only historical research evidence. File-system delivery and
SQLite commit cannot be treated as one atomic action: journal intent before delivery,
retain precise partial/delivered/commit-unknown facts, and reconcile from authoritative
state. A lost reply or post-commit diagnostic failure cannot turn commit into definite
failure. Do not re-download, delete by old pathname, or erase history to repair ambiguity.

Takeover/root revocation must share a serialized generation check with reserve/create/
write/commit. A revoke ordered first rejects subsequent work; a commit ordered first remains
historical. Stop receiving after revoke, retain a failure/uncertain artifact notification,
and allow no further user-directory writes. A defines exact locks and transaction ordering.
A restart loses descriptor continuity: a valid historical row/receipt alone cannot restore
current VERIFIED or resume a stream. Reacquiring permission does not upgrade old history.

Proposed states distinguish preflight refusal, receiving, delivery-uncertain,
commit-outcome-unknown, and historically committed acquisition. Only the approved complete
product path may expose VERIFIED; source-history display and current-location validity
remain separate. This PR adds no SourceStatus variant or migration.

## Permissions, installation and pairing

Source Memory's existing read/collect consent is insufficient for creating downloads and
writing quarantine attributes. Add separate opt-in takeover mode and native-selected save
root; both are off by default. Core checks them per task and supports revoke. A owns Tauri
commands, capability registration, DB changes, installer and shared configuration.

Use a stable extension ID and signed native installation with exact `allowed_origins`, a
narrow RPC allowlist, bounded framing, authenticated pairing and rotation/revocation.
Keep pair secrets out of argv, URLs, logs and evidence; choose private credential storage
and installer paths with A/R1. Runtime permissions must be explained before opt-in:
Chrome debugger can inspect/change tab activity; a save-root grant does not narrow that
browser permission. Do not suppress Chrome's debugging warning. Actual install warning,
attach banner, user cancellation, DevTools conflict, update, uninstall and revoke require
headed packaged-install acceptance; this fix's VM tests do not verify them.

## Safety and fallback

Setting and reading back quarantine/WhereFroms on the held FD **before writing content**
is necessary but does not establish Chrome Safe Browsing/dangerous-download verdicts or
Gatekeeper/XProtect equivalence. The current takeover aborts Chrome's original download
and has no authenticated dangerous-download verdict. This remains a production blocker.
A/R1 must approve a protection mechanism demonstrably preserving these layers before any
normal public-site takeover. Do not run dangerous downloads or weaken system protection.
Text extensions/MIME/permissions and content hashes are not a malware safety verdict.

Experiment scope is explicit allowlisted synthetic HTTPS GET/200, one positive
Content-Length ≤20 MiB, no transfer framing/compression, simple ASCII txt/md filename,
query-free credential-free URL and an authorized synthetic location. Unsupported static
cases, consent/prepare refusal, budget exhaustion or session loss before take continue the
original Chrome request. A race discovered after take is a reported failure/uncertainty;
Chrome cannot simply resume the consumed original response. Never silently create a new
request. Keep the accepted candidate/User Confirmed/UNKNOWN path for normal browser-owned
downloads. Login/POST/blob, encoded/incomplete responses, unsupported names and unverified
network/file-system contexts are not automatically VERIFIED.

Queries are refused only by this isolated acquisition experiment. This is **not** a change
to #27's approved private signed-download URL storage. Any future authenticated source URL
storage must be separate from shareable diagnostics, redacted by default and not exported
without explicit scope. No Cookie/Authorization/POST body/Set-Cookie is needed as evidence.

## Review request to Claude-A and freeze risk

Please confirm or revise: (1) core-owned live target capability + `observe_in` atomic source
binding; (2) root/takeover grant generation and revoke lock order; (3) authoritative journal/
DB result, idempotency and restart semantics; (4) installer/pairing ownership and safety
prerequisites. These are proposed interfaces, not implemented shared-contract changes.

The 10 October repair is isolated. Formal integration adds a new core write path, native
installation/pairing, permission UI, safety controls and independent acceptance. It could
consume A/B's remaining Memory time before **20 October Feature Freeze**. Do not make #37
or accepted CAC #27 depend on this research. Unless the new safety/interfaces can be approved
and independently verified in time, keep takeover disabled/deferred at freeze; ship the
already accepted Source Memory behavior. This is a scheduling recommendation for approval.

Primary references checked 2026-10-10:
- [CDP Fetch](https://chromedevtools.github.io/devtools-protocol/tot/Fetch/)
- [Chrome debugger](https://developer.chrome.com/docs/extensions/reference/api/debugger)
- [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
- [Apple malware protection](https://support.apple.com/guide/security/protecting-against-malware-sec469d47bd8/web)
