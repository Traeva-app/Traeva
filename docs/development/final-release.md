# Final release: freeze, packaging, source submission, rehearsal (#37)

A is responsible for final integration (AGENTS.md §4). This checklist is used from the 10/20 Feature Freeze until recording on 10/22–24 and submission on 10/25. Each step records the actual commit and result; anything not done is written as “not done”, never as complete.

## 1. Freeze (10/20)

- Features merged before the freeze (updated 10/17; the evidence level of each is in [#37's status report](https://github.com/P30g53/Traeva/issues/37#issuecomment-6241872878)):
  - Organize: Gates 1–3; at most 3 clarification questions (§15), recommendation threshold 0.70; reading the PDF text layer, on-device text recognition for scanned PDFs and JPG/PNG (#186, §14, §14.8, §14.9, on by default), optional image viewing by the model (§14.10), Word/PowerPoint/Excel (v0.16); descriptions start with the subject (v0.15); folders recognized by volume UUID after a restart (#297).
  - Auto-organize by dropping on the icon or the desktop pet (#168): files whose text came from recognition or whose images the model looked at also move automatically (v0.14); the Moved to bubble after a single file is moved (#305–#308).
  - Version Memory: #26 and the PPTX/XLSX integration, automatic linking of replace-style saves (#291), automatic recording of organized files (#201).
  - Source Memory (#27).
  - Find (#198): home page, recently opened files, find by subject (#301), find by time (#303).
  - Desktop pet, quick-search panel and pet popup (#208).
  - Grouped pages and Show more, third-party license notices.
- After the freeze, only defects that affect the submission are fixed; each fix gets a PR, and the student merges it.
- Choose the **final commit**: main at the moment of the freeze, written into #37. If a fix is merged later, the final commit is updated with it, and steps 2–5 must be redone on the new commit.

## 2. Pre-packaging checks (on the final commit)

In a clean checkout (a new worktree, or a new clone), run the frontend build before Rust checks so Tauri can find the generated `dist/`, as required by the [README](../../README.md#development-and-testing):

```bash
npm ci
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/third_party_notices.py --check
```

- Privacy scan, reporting counts only: whether text files contain the local home path; the author fields, Company and Manager of Office files (docx/pptx/xlsx), and whether any of their parts contain local paths.
  Use `python3 -I scripts/release_privacy_scan.py <archive or unpacked folder>` to scan the final commit's source archive (the step 4 method; do not scan the checkout folder, because its `.git`, `target/` and `node_modules/` count as failures). Only `result: PASS` with exit code 0 counts as passing; glance over each `LIST` item one by one.
- If any item fails: fix it first, then start again from step 1's final commit.

## 3. Packaging

```bash
SYSROOT=$(rustc --print sysroot)
HASH=$(rustc -vV | awk '/^commit-hash/ {print $2}')
RUSTFLAGS="--remap-path-prefix=$HOME/.cargo/registry/src=/cargo/registry/src --remap-path-prefix=$SYSROOT/lib/rustlib/src/rust=/rustc/$HASH" npm run tauri build
```

- `--remap-path-prefix`: without it, the binary keeps the local paths of dependency sources, which reveal the user name (the 10/13 packaging rehearsal counted 781 of them on `6b00f0c`). After adding it, run a full `tauri build` and check the binary scan below.
- The second `--remap-path-prefix` covers the standard library: once the toolchain has the `rust-src` component installed, rustc writes the standard library sources as local paths (`~/.rustup/toolchains/…`) instead of `/rustc/<hash>`. When RC12 was built on 10/16 this Mac already had that component, and the scan counted 52 of them. These two commands must run in the repository folder so that the toolchain set by `rust-toolchain.toml` is used.

- Output: `target/release/bundle/macos/Traeva.app` (ad-hoc signed, not notarized).
- Checks:
  - `codesign --verify --deep --strict` passes;
  - `Contents/Resources/THIRD-PARTY-NOTICES.txt` exists and matches the one in the repository;
  - the file type declarations in `Info.plist` are txt, md, markdown, pdf, jpg, jpeg, png, docx, pptx, xlsx, role Viewer, `LSHandlerRank` None;
  - launch once with temporary app-data (`open -n --env TRAEVA_APP_DATA_DIR=<temporary folder> …`) and click **Organize** in the sidebar: the footer shows test data, and the database path is under that temporary folder. The home page's Find and the sidebar do not have this marker. Do not touch the first-time setup guide on the Find page: its completion marker is stored in the WebView under the app identifier, shared with the official App, and once it is completed or skipped it is never shown again on this Mac (found by R1 on 10/13 on the package built from `fae5b7b`, #211).
  - no local paths in the binary: `grep -ao '/Users/' Traeva.app/Contents/MacOS/traeva-app | wc -l` should be 0 (report the count only).
  - the removed verification switch is not in the binary: `strings -a Traeva.app/Contents/MacOS/traeva-app | grep -c TRAEVA_OCR_TEST` should be 0; the same command counting `TRAEVA_APP_DATA_DIR` should be greater than 0, which proves the scan itself works.
  - text recognition for scanned PDFs (§14.8): in the temporary app-data launch above, the status line under the model card on the Settings page first shows “…is getting ready (the first time can take about a minute).” and then changes to “Text recognition for scanned PDFs and images (JPG, PNG) is ready.”. The first preparation of a freshly built package takes at most about one minute (limit 90 seconds; on 10/13 R4 measured 33.6 seconds on the same Mac with a new app identity, with the core directly calling the App's preparation child process; this is not a natural cold start of the full window, and a cold start on a new device is untested); if it does not become ready, record FAIL. Once it is ready, choose a synthetic scan that has text (samples in the next item; the images in `p08-scanned-images.pdf` contain no text and cannot serve as evidence of successful recognition), and the Text line shows “text recognized from scanned pages: N characters — Read by on-device text recognition from N scanned page(s); it may contain mistakes. …” (with more than 3 pages, a note about the skipped pages follows). The item “auto mode does not move automatically” (cancelled from organize-spec v0.14: files whose text was recognized move automatically when they meet the other conditions) first needs model settings and real requests: R4 already checked it with real requests on the old layout at `137b29a`, on an uninstrumented package R4 built itself (#211 comment 6151533624; scope correction in 6152383486); on the final package it is looked at in the step 5 rehearsal, without using budget here.
  - the synthetic scan samples are not in main; they are on G-OCR's experiment branch (PR #215). Take them out pinned to a commit: `git fetch origin claude/pdf-ocr-experiment && git archive f73a06f experiments/pdf-ocr/samples | tar -x -C <temporary folder>` (after the branch is deleted, use `git fetch origin pull/215/head`).
  - images (§14.9): once ready, choose a synthetic PNG or JPG that has text (for example `crates/traeva-core/tests/fixtures/images/text-plain.png`), and the Text line shows “text recognized from the image: N characters — Read by on-device text recognition from an image; it may contain mistakes.”; choose an image without text, and it goes to manual pick.
  - run the opt-in test on the package, without opening a window: `TRAEVA_OCR_APP=<Traeva.app>/Contents/MacOS/traeva-app TRAEVA_OCR_SAMPLES=<temporary folder>/experiments/pdf-ocr/samples cargo test -p traeva-core --test ocr_app -- --ignored --nocapture`. In this step `TRAEVA_OCR_SAMPLES` must be set: without it, the test passes even when it recognizes no scan at all. It passes when the test passes and the 9 samples marked scan in truth.json (s01–s06, s11–s13) are all `recognized=true` in the output.
  - known limits, written truthfully into the README: arm64 only; ad-hoc signed and not notarized, so a copy downloaded from the web onto another Mac must be opened with right-click; every repackaging makes the Keychain ask again (remind the student before the rehearsal); text recognition for scanned PDFs needs macOS 13 or later, is prepared in the background at every launch, and may take about a minute the first time for a freshly built package, and a scan chosen before it is ready goes to manual pick; only the first 3 pages are recognized, and the recognized text may contain mistakes. OCR-based or image-assisted results may be moved automatically only when auto mode is enabled, its agreement is valid, and every condition of organize-spec §13.4 holds. With auto mode off, the normal flow still requires preview and confirmation.
- Record: the commit, the SHA-256 of the `.app` (first pack it into an archive with `ditto -c -k --keepParent Traeva.app Traeva.zip`, then compute the archive's SHA-256), and the size.

## 4. Source submission

- Generate the source archive from the final commit; **do not submit the repository with its git history** (the history still contains old files from before the cleanup, and the student decided on 10/12 not to rewrite history):

```bash
git archive --format=zip --prefix=Traeva/ -o Traeva-source.zip <final commit>
```

- If the reviewed source-submission copy contains later documentation edits, package that copy instead of exporting the earlier commit again. Exclude `.DS_Store`, `__MACOSX`, `.git`, `node_modules` and `target`. Record the baseline commit, changed files and final archive hash; do not describe the edited archive as an exact export of the earlier commit.
- Unpack the archive and run step 2's privacy scan again; confirm that it contains no `node_modules/`, no `target/` and no personal files outside any evidence folder.
  Command: `python3 -I scripts/release_privacy_scan.py Traeva-source.zip`, then run it once more on the unpacked folder; both runs must give `result: PASS`.
- Record the archive's SHA-256.

## 5. Rehearsal (the student and A, with C's Source segment merged in)

- Go through the demo steps the student wrote, from start to finish, using the `.app` built in step 3. The student writes the demo steps and the script personally, according to the competition requirements, and they are not put in the repository (the student's decision on 10/14; #159 and #160 are closed, not merged, and their branches are kept).
- Claim the UI slot in #65; use the demo folder's own app-data; use only the synthetic files in `demo/`.
- Before starting, launch step 3's `.app` once and wait until Organize's recognition status line shows ready; a scan chosen while it is still preparing goes to manual pick.
- Covers: Organize (recommendation, clarification question, preview, confirmation, Undo), auto-organize by dropping on the icon, Version Memory (descriptions, retrieval, opening an old version), Source (a separate Chrome Profile).
- Real model budget: about 10 for the rehearsal and about 3 for the demo, confirmed by the student before starting.
- The student enters the key personally; agents never handle it. The student deals with the Keychain prompts.
- Problems found in the rehearsal are recorded truthfully: those that affect the demo are fixed (with a PR), then go back to step 1.

## 6. Final verification (R)

- R2: Memory final re-verification (12 real requests approved), on step 3's `.app`.
- Auto-organize (#168), PDF and text recognition for scanned PDFs (#186, R4): if the related code changed between the final commit and the commit they were re-verified on, ask R to run a regression on the final commit; if it did not change, write “code unchanged, results of <commit> carried over”, with the diff attached as evidence.
- Word, PPT and Excel switched decompression backend (zlib-rs, #186): regression with really saved files (already included in R2's plan).
- All results are collected in #37.

## 7. Recording and submission (recording 10/22–24, submission 10/25)

- What is recorded is step 3's `.app`, following the demo steps and the script the student wrote.
- Every feature that appears in the video must have a matching independent re-verification record; anything that has not passed re-verification yet is not put in the video. The re-verification status of each feature is in #37's final summary.
- Submission materials: the video, the source archive (step 4), the description (the final version of the README).
