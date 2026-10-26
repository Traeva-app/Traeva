# Source evidence package privacy rewrite

Refs #27 / #37

This change cleans up two committed Source evidence packages in response to [R3's scan report](https://github.com/P30g53/Traeva/issues/37#issuecomment-6137181271). Found 16 / replaced / 0 remaining. Reason for the change: **rewritten because it contained local machine paths**.

The baseline is main `2e90cf82e2be3b8bd14e50b9f2e1fe968e47bbf7` at the start of this round. R3's original FAIL report is bound to `762839dde16f4e4902a79942f85c7c3ceaa541cf`; this result belongs only to C's privacy cleanup and evidence consistency check, and awaits R3's independent re-verification on the new commit.

## Original and derived packages

All paths are relative locations within the repository or package. Both outer ZIPs and both nested ZIPs were read recursively and repacked; `r-original-evidence.zip` needed no replacement and is byte-identical after repacking.

| Package | Original SHA-256 | Derived SHA-256 |
| --- | --- | --- |
| `review-fixes-evidence.zip!r-original-evidence.zip` | `d1ff9c355755859bf35f404caa1fbd6bab337e3a6639a469d2a6d9d19b915ca3` | `d1ff9c355755859bf35f404caa1fbd6bab337e3a6639a469d2a6d9d19b915ca3` |
| `review-fixes-evidence.zip` | `9a15451e9734ebe52540dd0f33556329a50ece5c05370132535bde2e619990ff` | `66c47f168b50c9cca23fba7565e2a58c4dc30d05e5835f424f326574d829a3c8` |
| `source-verified-evidence.zip!evidence/legacy-comparison-evidence.zip` | `b44db618a174f7c07c11be32a22c5b2702dae5111e539f394cd2bb8a84d555fd` | `13363fbdaeb4b6cdc81603e3bcffb7be86f288f771ff7dc3092311da6bba42d9` |
| `source-verified-evidence.zip` | `043334e6dfd6fa944c1c72d30eae5c7fa259eaec3f71cdfbf42f9f67acd0c2d0` | `76faabc7cbbf0629beff3dea44603b290f18847996425dce6a4c77b7a96a9210` |

`archive-sha256.txt` and `review-fixes-sha256.txt` are updated to the checksums of the current derived packages. The [machine-readable record](evidence-privacy-validation.json) lists the original and new hash of every rewritten file and package.

## Preserved evidence

- Of the 618 non-ZIP inner files, 11 only had the local home prefix replaced with `<HOME>`; the other 607 files are byte-for-byte unchanged. Member order, filenames, times, permissions, compression type, comments and extra metadata are kept the same.
- A per-file comparison independent of the sanitizer confirms that every rewritten file equals the original content with only the replacement above applied. 246 historical status markers keep their original order and values, including 44 retained FAIL markers. This is a text-preservation check and must not be read as 246 tests or a new experiment PASS.
- The old `MANIFEST`, `sourceRevision` and the original hashes in the failure records inside the packages still point to the historical evidence and were not rewritten to the derived files' hashes. To check the current derived bytes, use this note and `evidence-privacy-validation.json`; do not treat the old manifest as the checksum list for the current packages.
- The scripts inside the packages are historical copies of the frozen experiment; the path placeholders do not represent a directly runnable configuration. The original FAIL, BLOCKED, unverified boundaries and the frozen Auto VERIFIED status are kept.

## Checks in this round

Environment: Python 3.9.6, zlib 1.2.12. Only the two packages in the Git baseline and this branch's derived packages were read; no browser, App, download takeover, model or existing acquisition experiment was run.

```sh
python3 -B experiments/source-verified/test_evidence_privacy.py
python3 -B experiments/source-verified/sanitize_evidence.py verify --baseline-ref 2e90cf82e2be3b8bd14e50b9f2e1fe968e47bbf7
python3 -B experiments/source-verified/sanitize_evidence.py scan
```

- 7 synthetic checks pass, covering UTF-8, UTF-16 in both byte orders and offsets, escaping/URL encoding, package and member comments, member names, local/central extra, nested packages, FAIL preservation, duplicate-name conflicts, unreadable nested packages, trailing data and malformed extra. Unknown trailing data is not silently dropped by the rewriter.
- Both actual packages and their nested packages pass CRC; a recursive re-scan of all inner payloads, filenames, comments, local/central extra and raw container bytes finds 0 remaining.
- A separate read-only check uses independent path-matching, decoding and URL-decoding logic over 4 ZIPs, 618 inner files and known local identifiers, finding 0 remaining; the per-file content and historical-status preservation checks pass.
- `verify` regenerates the expected derived packages from the baseline and compares all bytes and provenance records. The repacked bytes depend on the Python/zlib versions; R3 may check the content independently with its own scan tool, and compressed-byte differences across versions are not used in place of a content judgment.

To rebuild, `verify` may be changed to `rewrite`; it only processes the two frozen packages listed here and does not start the experiment. The sanitizer is only for these ordinary, parseable ZIPs and does not promise to handle arbitrary encrypted or special ZIP variants; when it cannot parse, it refuses to pass.

This round has no product, shared core, permission or communication architecture changes; Git history was not rewritten, old commits and GitHub caches were not handled, and it does not claim privacy for the whole repository or #37 acceptance.
