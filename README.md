# Traeva

Traeva is a Mac file app designed for students with ADHD (attention deficit hyperactivity disorder). It helps you find study materials, organize files, and get back old versions of your assignments.

I want to cut down the steps of looking for materials while studying. For example, you get interrupted while writing a report, and when you come back you have to figure out which file and which version you were on; or you download a pile of materials and never get around to organizing them. Traeva brings file organization and search together in one app, using file content to help you get back to the assignment in front of you.

## Main features

1. **Context aware search (Find)**

   Even if you don't remember the full file name, so long as you can describe some of its contents, you can find it. Traeva compares file names, folder names, readable body text and the descriptions left when files were organized. With the model, it also understands time conditions like "last week".

2. **Find a place for your study materials (Organize)**

   For downloaded handouts, assignments and references, you can let AI recommend a folder based on the content. You see where the file will move first, then confirm; if it is unsure, you can answer follow-up questions (up to 3) or pick the folder yourself. With Organize dropped files automatically turned on in Settings, drag files onto the Traeva icon or the desktop pet, and files that meet the conditions are moved automatically. Moves are listed on the Activity page and can be undone after a check passes.

3. **Get back an assignment version by the change you remember (Version Memory)**

   After you turn on recording for a file, Traeva saves the stable versions it sees while it is running. With your permission, AI describes what changed between neighboring versions, with the original lines next to it so you can check. You can ask for "the version before I rewrote the conclusion" to find a saved old version; when it is ambiguous it asks a follow-up question, and when the record is incomplete it says it cannot tell.

4. **See where a download came from (Source Memory)**

   With the Chrome extension, Traeva receives records such as the download address, the source page and the time. After you confirm which file a record belongs to, you can look the source up again while writing a report. Information the browser did not provide is shown as unknown. The extension has to be paired with Traeva on this Mac first; see the [setup guide](extensions/chrome/README.md).

When studying gets interrupted, the home page shows recently opened files and items waiting for you. The desktop pet is also a shortcut: it takes dropped files, shows items waiting for you, and opens quick search. You can turn it off any time you don't want it shown.

## Try it first

Use the [demo files](demo/cac2/README.md) or copies of your files.

AI features currently use OpenAI. First, in the Model card in Settings, enter the model name and your own API key and click Save; before using AI organizing, also click Allow sending to the model. Find asks for its own agreement once, the first time you use its AI search; Memory asks for each file before AI describes that file's changes.

1. In Settings, add a folder and allow Traeva to access it.
2. In Organize, choose a file and let AI analyze the content, write a description and recommend a location; confirm to move it.
3. In Find, type what you remember about that file's content and look at the results.
4. In Memory, turn on recording for a file, save a few versions, then look at its history. With AI permission, you can read the change descriptions and then search by change.

Files that were not organized can still be searched by file name, folder name and readable body text. Running Organize's AI analysis first lets Find also match the content description it writes; moving a file by hand does not create this description.

Without a key, keyword search, manual organizing, local version recording and source memory still work. Results in the demo files are only examples; actual results depend on the files and the model's replies.

## Supported content

| Feature | Supported content |
|---|---|
| Find body-text search | TXT, Markdown, Word (.docx), PowerPoint (.pptx), Excel (.xlsx) and the PDF text layer |
| AI organizing | The formats above, plus JPG and PNG images; unsupported formats can be organized by hand |
| Version memory | TXT, Markdown, Word, PowerPoint, Excel; not PDF or images |
| Opening and exporting old versions | TXT, Markdown; Office files currently only preview the extracted text on the page |

- Word, PowerPoint and Excel are read only in part. For Word, mainly the body text; for PowerPoint, the text and notes of visible slides; for Excel, the cell values, formulas and saved calculated results of visible sheets, without recalculating. Changes to images, layout and the like are outside the text comparison; see the [format notes](docs/development/shared-contract.md).
- Text recognition for scanned PDFs and JPG and PNG images is used only for organizing (at most the first 3 pages of a PDF) and needs macOS 13 or later; on earlier systems these files fall back to picking a folder yourself. Find and Memory do not recognize scans or images.
- Find does not read the body text of files over 20 MB or of iCloud files not yet downloaded to this Mac, and never triggers a download; these files can still be found by name.
- Files over 20 MB cannot be organized (not even by picking the folder yourself), and Memory does not record them.
- Version memory does not promise to record every save. There can be gaps in the record, for example while the app is not running; old-version search currently works by change, not by date or file name.

For what was tested and the open issues, see the [acceptance record](https://github.com/P30g53/Traeva/issues/37). Each test result there applies to the version recorded with it.

## Files and privacy

Files, version copies and records are stored on this Mac. Before using AI, each feature explains what it sends, and permission can be revoked.

| Feature | What is sent to the model |
|---|---|
| Finding files | Only the search question; matching files happens on this Mac |
| Organizing files | File name, current location, candidate destination folders, a text excerpt, and the questions and answers of a follow-up; for a short file the excerpt may be the whole text |
| Describing changes | The changed lines between neighboring versions, each place with at most two lines before and after |
| Finding old versions | The search question and the saved change descriptions |

Images are not sent by default. In the Model card in Settings, first click Allow sending to the model, then set Let the model look at images: Off sends nothing, Ask each time asks before each send, Don't ask stops asking every time. The same setting covers scanned PDFs: a smaller copy of their first 3 pages may be sent. An image copy that is allowed to be sent is made smaller and has its location and camera information removed. The API key is stored in the macOS Keychain. Source memory does not call the model.

Normal organizing moves a file only after you confirm; Organize dropped files automatically has to be turned on and agreed to separately. A move never overwrites a file with the same name; before an undo, Traeva checks that the file still meets the conditions, and stops when it cannot be sure. Search and version recording do not change the original files.

## Running the project

Development needs macOS, the Xcode build tools, Node.js 22.18 or later with npm, and Rust. The Rust version is pinned by `rust-toolchain.toml`. Put the source in a local folder that iCloud, Dropbox or OneDrive does not sync.

Unzip the submitted `Traeva-source.zip` to get the `Traeva` folder. In Terminal, go into this folder and run:

```bash
npm ci
npm run build
npm run tauri build
```

After the build, the app is at `target/release/bundle/macos/Traeva.app`. When trying it, give it a new absolute path as its test data folder:

```bash
open -n --env TRAEVA_APP_DATA_DIR=/absolute/path/to/empty-folder target/release/bundle/macos/Traeva.app
```

While changing code, you can use the development mode with hot reload:

```bash
TRAEVA_APP_DATA_DIR=/absolute/path/to/empty-folder npm run tauri dev
```

Replace the example path with your own empty folder, kept apart from the demo folder. Only one instance can run on the same data folder. If none is given, both the development build and the packaged app use `~/Library/Application Support/Traeva/`; switching branches does not separate data. The detailed rules are in the [run isolation notes](docs/development/cac-parallel.md#run-isolation).

Closing the main window keeps the desktop pet; press Cmd+Q to quit the app.

## My contributions and AI assistance

I designed Traeva's product rules and wrote the current versions of four core source files for Find and Memory: `find.rs`, `changes.rs`, `retrieval.rs`, and `lineage.rs`.

For these four files, I used AI to review the logic and identify logical errors, then revised the code myself. Finally, AI helped check syntax and Rust coding practices.

For the rest of the application, Claude Code and Codex generated most of the code.

## Development and testing

The interface uses React, TypeScript and Tauri 2, file handling and search logic use Rust, and records are stored in SQLite.

| Location | Contents |
|---|---|
| `src/` | Interface and interaction |
| `src-tauri/` | The connection between the desktop window and the Rust core |
| `crates/traeva-core/` | File operations, search, version memory and the database |
| `extensions/chrome/` | The download source extension |
| `demo/`, `docs/` | Demo files, feature rules and development notes |

| Check | Command |
|---|---|
| Rust tests | `cargo test --workspace` |
| Frontend and extension tests | `npm test` |
| Rust formatting | `cargo fmt --all --check` |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| Frontend types | `npm run typecheck` |

Run `npm run build` before a Rust build or Clippy, because Tauri needs the generated `dist/`. Tests run offline by default, use temporary data and do not call a real model.

Detailed docs: [Architecture](docs/architecture.md) · [Organize rules](docs/features/organize-spec.md) · [Shared interfaces and format limits](docs/development/shared-contract.md) · [Collaboration rules](AGENTS.md) · [Development progress](https://github.com/users/P30g53/projects/1/views/5)
