# CAC demo files (#18)

Three synthetic files, used in a real-window demo of "clear recommendation", "one necessary clarification question" and "unsupported format goes straight to a manual pick". All of them are newly written, **were never given to the model before the demo**, and were not used in any prompt tuning; the student checked that they do not overlap with the sealed #22 material. The content is fictional (*Frankenstein* was published in 1818 and is in the public domain).

| File | Expectation |
|---|---|
| `titration-lab.txt` | Recommends `School/Chemistry` directly, no clarification question |
| `frankenstein-notes.md` | One clarification question (for class or for the book club); answer "book club" → `Book Club` |
| `field-trip-form.pdf` | Not TXT/Markdown: not sent to the model, goes straight to a manual pick (the demo picks `School/History`) |

`SHA256SUMS.txt` holds the hashes from when the files were finalized. `demo/cac/** -text` in `.gitattributes` ensures git does not change these bytes.

## How to reproduce

```bash
python3 demo/cac/setup.py
```

It creates `~/Traeva-CAC-Demo` (refuses if it already exists), containing `School/Chemistry`, `School/English`, `School/History`, `Book Club`, `Robotics Club`, `Personal/Health` and `Inbox`, copies the three files byte for byte into `Inbox`, and checks the hashes. Then:

1. Open the packaged `.app`, and with Add a folder… (the first-launch card, or Add folder… in Settings) choose `~/Traeva-CAC-Demo`; choose it for both "File from" and "Into".
2. After the model is set up and sending has been agreed to, choose the three files in order: Suggest a folder → (answer one question) → preview → confirm.
3. Quit with Cmd+Q and reopen; on the Activity page, for each of the three entries in order: Undo… → (goes to Organize) preview → Put it back.

When recording the actual CAC video, a fresh app-data is recommended, so that only the demo's records are on screen.

## Rules

- These files are only for the demo; they must not be used to tune the prompt, and must not be used for #22. If the prompt is changed in the future, new demo files must be written.
- When the demo does not ask the clarification question as expected, record it truthfully and do not change these files; to start over, write new files.

Results of the first real run on 2026-10-08 (`main` `85ed78d`, `classify-3`, `gpt-6-luna`): titration recommended `School/Chemistry` directly (0.99); frankenstein asked one clarification question and then recommended `Book Club` (0.99); the PDF sent no request and went straight to a manual pick; all three were undone successfully after a restart, with bytes, hash, inode and file_id unchanged. The full evidence is outside the repository (`traeva-evidence/m18/`).
