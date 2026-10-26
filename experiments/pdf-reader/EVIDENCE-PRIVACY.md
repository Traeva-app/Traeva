# Privacy rewrite of the PDF Reader run reports

Refs #37 / #28

The basis is [R3's privacy scan](https://github.com/P30g53/Traeva/issues/37#issuecomment-6141263203): FAIL at `ef16c1c2b1b21af6e63480e0ee781f217e243d1d`. See also [R3's supplementary original hashes and counting rules](https://github.com/P30g53/Traeva/issues/37#issuecomment-6141313861).

This round cleans the seven committed run reports `run-15` … `run-21` in `evidence/runs/`:
- local account name: 90 found / replaced / 0 remaining;
- home directory path: 16 found / replaced / 0 remaining.

The two counts overlap and are not added together: of the 90 account-name hits, 16 are inside home directory paths, and the other 74 are in temp directory paths, in the dash-encoded form `-Users-<name>-`.

The baseline is main at the start of this round, `6dd58482de2e22fffb92960770efdd4c166a14bd`. The bytes of these seven files at this baseline are identical to those at R3's endpoint `ef16c1c` (7/7 blobs match). This result is only G's privacy cleanup and self-check; it waits for R3's re-verification on the new commit.

## How it was changed

- the home directory prefix becomes `<HOME>`;
- the account name in temp directory paths becomes `<user>`;
- nothing else changes: the first FAIL's results, statuses, counts, durations and expected values are byte-for-byte unchanged except for the replaced substrings;
- the Reader check matrix was not re-run, old FAILs were not deleted, and git history was not rewritten.

When the account name appears in any other position, the sanitizer refuses to write; that did not happen this time. All hits are in the `command` strings and in the generator output's `tail`. The reports have no internal hash covering these fields, so no other field needs changing.

**The committed reports are rewritten copies.** As before, the originals exist only on this machine:
- 7/7 are preserved byte for byte in an old local worktree;
- the earlier commits in git history also still contain the originals; as requested, history is not rewritten, and the repository is PRIVATE.

No manifest, README or script in the repository pins the hashes of these seven reports:
- `validate.py` scans `runs/` only to get the next number;
- `relocate.py --check` compares the core's `pdf*.rs` with the experiment source and does not involve run reports;
- no Rust code reads these reports.

So apart from the record below, there are no other hashes to update.

The earlier `run-01` … `run-14` contain 20 occurrences of `/home/user`. They come from the generic account of the cloud container, not this Mac's account, and as requested they are not changed. The scan counts them as generic paths, not as hits.

## Original and rewritten files

The original SHA-256 values were checked one by one against R3's table: 7/7 match; the original git blobs also match 7/7. The [machine-readable record](evidence-privacy-validation.json) lists both kinds of hash and the replacement counts.

| Report | Original SHA-256 | Rewritten SHA-256 | Home directory paths | Account names outside home paths |
|---|---|---|---|---|
| `run-15-368aa5e.json` | `e919acc55957d922f1bbd454af6464d5d93553b0f4275f8025a514daf7bed9e3` | `c74e06c4939cc00a53492aea923c55b83fa3a2d77395cb38e70933b38655056e` | 3 | 10 |
| `run-16-dba679a.json` | `d3438e8abf05cfd43da2cec8c97edc43c95307a967d90ba9b3f3ada3ddcc4053` | `771665640105efd72c9f644880a0fabdbca1e03acafda119acfcdd425992ca32` | 4 | 10 |
| `run-17-dd70a72.json` | `bebd122e2aa8ed4c84771e6cc08f1a21871fe662d2bc680c4b95beff4262fd62` | `9fc6b181b25270d5fc7a1f4c4e757ade8cf66fa0d346cadb413e091231403128` | 1 | 10 |
| `run-18-c10d23a.json` | `15e8bfd7d1beedb6c31a29c2316fb75e0b3e805108f85e39f1fdae08b8311ec1` | `945f91b1e018360c286ff3841d1932271ea884fdf9be3fbf16b21fcb419c9726` | 1 | 10 |
| `run-19-17a1616.json` | `75e6a32d7e2f061ec1a384dc96842e1be62db76d37ccea83024b4a537b8ef51e` | `72e4471bbd7454a5b29514b36edf13beaf11381acdd36480057395702cc8cf9a` | 2 | 12 |
| `run-20-0c72d05.json` | `5c366e824c9eb1fd2d501b0ccdab0054a63425da3b249264174a6270d6eaf53c` | `fc33ef405143cfcb082c186eae80e6d444a706c9153c63037ad964ac7920847e` | 3 | 10 |
| `run-21-4b3d28f.json` | `7d30a1a3272a19b413b7b3058d5a970441cd95de414ba470bb82be4d754df4c0` | `1042f2789d10c61525517e9a58867f4fc9ef981aa6a89ed02e6ca4c1dadb7ad3` | 2 | 12 |

| Report | Original git blob | Rewritten git blob |
|---|---|---|
| `run-15-368aa5e.json` | `43b11bd7d98ba2f3e24f4614f552202035fb49df` | `6f36086d80cdc9dee387587ef264c89260a46b0f` |
| `run-16-dba679a.json` | `43b0e6bd2bad6a6573cd5a0303f76baec2032c97` | `5442fce36cc7d8483cb2d8220ff57b78b481f578` |
| `run-17-dd70a72.json` | `253b9ed35ec77a9952d4d126877337d3dc08c160` | `bcd114a67d889cd29003c5373a7220d2a306705c` |
| `run-18-c10d23a.json` | `1d1b2e6d60936561952f8e88bcc6aa6acff02458` | `08045aa9ccc50b2f24e5679f48f4ac36bcd87252` |
| `run-19-17a1616.json` | `de3294c730128feb1d4d86af9d0e83dc5415e1e6` | `581c9e4fdd112cb124c389307faea92d45f8cc0c` |
| `run-20-0c72d05.json` | `c1e5c1574a9718b1927c2ccbe09f6e2fb7da4daa` | `5b375aba268aaceaeb045fe2c18f2e123d6b519f` |
| `run-21-4b3d28f.json` | `419e6b076674ca3ade3f6292f66c44ef0b573e79` | `f3dadc26e0285b15e73f90829d29358d00fe9f33` |

## Checks this round

Environment: macOS, Python 3.9.6. Only the seven reports in the baseline and this branch's rewritten versions were read; the Reader, the App and models were not run.

```sh
python3 -B experiments/pdf-reader/test_evidence_privacy.py
python3 -B experiments/pdf-reader/sanitize_evidence.py verify --baseline-ref 6dd58482de2e22fffb92960770efdd4c166a14bd
python3 -B experiments/pdf-reader/sanitize_evidence.py scan
```

- **Synthetic checks**: 8 passed. They cover:
  - only the two forms are changed;
  - a longer name is not truncated;
  - it refuses when the name appears elsewhere (including with different case);
  - content already rewritten is not changed again;
  - any other byte change (status, numbers, trailing newline) is detected;
  - the tokens can stand only for the same one account.
- **`verify` does not need to know the account name**: on the rewritten copy, it puts the same home directory path back for every `<HOME>` and the same name back for every `<user>`, and the rebuilt content must be byte-for-byte identical to the baseline. It also checks:
  - both kinds of hash and the replacement counts match the record;
  - none of the 15 FAIL markers is missing;
  - all seven JSON files parse.

  This is a text-preservation check; it does not represent a new test or experiment PASS.
- **`scan`**: under `experiments/pdf-reader/`, a case-insensitive search for the local account name, the home directory path, and other accounts' home directory paths apart from generic paths: all 0.
- **Another, independent counting script** (not in the repository) scanned the whole repository, 779 files: 0 account names remaining.

To regenerate, first restore these seven files to the baseline version, then replace `verify` with `rewrite`. The sanitizer handles only the seven reports listed here and does not start any experiment.

## Not done

- git history was not rewritten; old commits and GitHub caches were not handled;
- if `validate.py` is run again later, new reports will still record local paths, but this experiment has ended;
- this round changed no product, shared core, dependencies or permissions;
- no claim that the whole repository's privacy check or #37 has passed.
