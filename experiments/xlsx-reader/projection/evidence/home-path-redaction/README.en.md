# Redacting machine-local paths in test evidence (Refs #28)

English translation of [the original redaction record](README.md). The original record and machine-readable evidence remain unchanged. Hashes below describe the files at the time of redaction, not this translation or later documentation edits.

Main baseline: `9b40b39481a933a558556790f84374728545ce8b`. The two directories specified by the student contained 23 machine-local home-prefix occurrences across 14 files. All were replaced with `<HOME>`: six files under projection and eight in the core test directory, including three existing DOCX/PPTX metadata records. Only paths were replaced; adjacent explanations and acceptance conclusions were unchanged.

The original files were rewritten because they contained machine-local paths; their original hashes are listed below. Original FAIL/PASS results, diagnostic meanings, test expectations and error order were retained. Redaction did not turn earlier failures into passes. Five current-file hash references across three manifests were updated to the redacted versions, and each manifest separately retained the original hashes. `redaction.json` records the full mapping, byte counts, manifest sources and validation for this round. This ordinary PR did not rewrite main history or process #137's files.

| Repository-relative file | Replacements | Original hash | Redacted hash |
|---|---:|---|---|
| `crates/traeva-core/tests/fixtures/docx/r2-review.json` | 1 | `3f6d63b4ed1348777ae008579f5685f8467646c41254c0b08f0d4efd7843abec` | `13e96f5a3963181315929c826feac757cdf0239e5412a91c32d7866b3c3609bb` |
| `crates/traeva-core/tests/fixtures/pptx/README.md` | 1 | `8c42d849dc13db0e5c290dbb14edeccc628029caaf09f2a3f913ebd441e4ed9b` | `3d3df33a394699017ab0a48d206542fe22b679de24d85665dface31aaf141b59` |
| `crates/traeva-core/tests/fixtures/pptx/validation.json` | 1 | `da9335cfec5ac341d09a85255f26ff936776c1978f7046f4d567104b801161bf` | `078e880e01616ccfce3b6003a9d48225353c9f59fde8fab3979485bd549d69f2` |
| `crates/traeva-core/tests/xlsx_reader/evidence/core-clippy.txt` | 2 | `85d0ece02ac491b5bea01fc47e60f331281953fead6358389c33e20245a6e11e` | `b2ab7eb0685929bb44518fa33e9312d73e5999cbfe2be64f5a9a73822c20a2f6` |
| `crates/traeva-core/tests/xlsx_reader/evidence/core-tests.txt` | 2 | `2de4c6427b6eda47e35715ebcb1129ebeac575d227712259bbcbeea28236e7ae` | `22ac8d6e83c36a7370598fdb9a58d9fec2dfb0a151977ec3b3c0d8a5b170a730` |
| `crates/traeva-core/tests/xlsx_reader/evidence/first-shim-compile-FAIL.txt` | 3 | `9760c1fdc5e8670439936eebfa9d695725082f8aac5eb7e92ebbf7f648dff206` | `9d3ed076e903c24102053c0d01cb7979dd5c8ca1f2cb5e739b68cef90c24c824` |
| `crates/traeva-core/tests/xlsx_reader/evidence/legacy-shim-clippy.txt` | 1 | `00bac842e5d4d22bd8dac76576ea8e66f9f9259282e35ab46618c402e0522e80` | `c49ca8edacd413a048403821ab0bef3f3e6b8e7684613c1eeb75bbe034037dea` |
| `crates/traeva-core/tests/xlsx_reader/evidence/projection-shim-fixed.txt` | 1 | `6808bbb6c6ace1e81d0a6056b8205b093a7b0f238dbc3d0113541593f8cb250a` | `5522d56d6d568f5389bc7e32f24205e0f1292593d44ba27bff4e407db6940431` |
| `experiments/xlsx-reader/projection/evidence/aligned-clippy.txt` | 1 | `89ff78e503b9ab751f12a06d705ddfaa6dd5385758b59f9111d8698de5867420` | `cce2e34f9a8b2856b697783cd7f7e3ab6f0c77150c7c4e2838c0599fa0f1378a` |
| `experiments/xlsx-reader/projection/evidence/aligned-tests.txt` | 2 | `5fed4689450b2812f355e53d835d5596eb4861cacf5441f6106ed51b6f6ad814` | `e2b0f071cdf768ac94c6c32ecf2f45ec1d096a8a76b8631f1fd8a154d470d21a` |
| `experiments/xlsx-reader/projection/evidence/binding-budget-recheck.txt` | 1 | `16c05a129327f4f60f6967bb1566668de0d7fb0734357b5c82251732db5fafce` | `a78bdb46c2c7cbd70b5bfa2060b7763afdd58a57613ac956040b666238f4c2bc` |
| `experiments/xlsx-reader/projection/evidence/first-A151-compat-FAIL.txt` | 2 | `6f5b3627bce6929272c538ccba880f9fa4d939d61f06b1dfc85bceddbc3ba2a3` | `d2b4789726ef6d0d2bc1c6a4738c55090e68f571a94fec0c83d55c8be4bdc500` |
| `experiments/xlsx-reader/projection/evidence/first-build-FAIL.txt` | 4 | `08770cc181a854cc62720e351bf82f4e1becd9deea952743ee021161bcd141e5` | `74a4e6f706692b8b50b8ff2d7e7d6f6d8a450f740036bc94eaeda948de1e1af1` |
| `experiments/xlsx-reader/projection/evidence/v23-compat-FAIL.txt` | 1 | `710c936fbd64094428abddaf0543358ee89f92594be1e2380f69fb8f66feb318` | `a0e8a8968547306b50b920f6dceb5322b95b40dd57b719dce49ce35e7bdf30ef` |


## Validation recorded in the original round

- The standalone XLSX core verification crate passed 22 tests; projection passed 16 projection tests and five shared-helper tests. All 43 tests used the lockfiles already tracked on main with offline --locked. The root and two standalone lockfiles, source code, fixture binaries and test expectations were unchanged.
- Core formatting passed. The projection formatting FAIL was retained: it identified an existing compatibility-adapter formatting problem in main's `experiments/xlsx-reader/src/lib.rs`. That source file was byte-identical to the baseline. This path-cleanup round neither fixed its formatting nor expanded its code scope. See `projection-fmt.txt` and the JSON baseline checks.
- The first test-preparation script mistakenly treated a tracked standalone lockfile as ignored. Its precondition assertion failed before Cargo tests started. The original record is `first-test-preparation-FAIL.json`; later runs used the existing lockfiles without modifying them.
- All affected JSON files were valid. Byte comparisons confirmed that only home prefixes were replaced. Current hash references and original-hash mappings were consistent. No known machine-local home path or login name remained in tracked files in the target directories or in this round's new evidence. The diff check passed.

## Rerun commands

```sh
cargo test --offline --locked --manifest-path crates/traeva-core/tests/xlsx_reader/Cargo.toml
cargo test --offline --locked --manifest-path experiments/xlsx-reader/projection/Cargo.toml
```

This round did not change the Reader, production registration/dispatch, shared DTO/schema, resource limits or safety rules. No model or GUI operations were performed, no PR was merged, and no Issue was closed. This record verifies path redaction and its targeted regressions only; it does not establish complete Reader/Memory/App acceptance or replace independent review.
