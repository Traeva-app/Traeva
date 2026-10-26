# Mechanical relocation protocol

Expected behavior is unchanged from reviewed #155 head
`8f274c186a24cfaa4902fced09e8b4edb818969f`, actual helper dependency
`3d1a4a98b3afaa51e66ea1a276aa83f3778f385c`. The 22 existing assertions and their
oracles are reused, not redesigned. Source namespace/module paths may change;
parsing/projection logic, profile v1, shared Reader names and revision stay fixed.

Before validation, compare the relocated raw parser/package/XML and projection
bodies mechanically against that head, normalizing only the declared imports and
module declarations. Existing original nine FAILs and R3 matrices are retained;
the 122-input matrix is not repeated.

Build the exact core files with #[path], no production mod/dispatch registration.
Run the relocated 22 checks. For the 24 already-captured F-owned #155 synthetic
inputs, verify their recorded input hashes and both modes' expected text hash,
reader/status/scalar count/actual omissions against the new core probe. This is
developer migration parity, not additional independent PASS counts or Excel saves.
Do not read R3 test inputs or use developer evidence to close independent X15.

Root manifests/lock, shared DTO/schema/helper source, Tauri, Memory UI/consent and
main/R2 remain unchanged by F. Independent validation locks stay root-aligned.
Retain first build/test errors. Changes in output or safety rules require diagnosis
and a separate explicit decision rather than moving the expected hashes.
