# CAC demo files 2 (#37, for rehearsal and recording after the freeze)

`demo/cac/` (#18) has only Organize's three files, and its expectation for `field-trip-form.pdf` is still "PDF goes straight to a manual pick"; since Organize reads the PDF text layer (#186), that no longer holds. This set covers the current four demo segments: **Find, Organize, drop on the icon, Memory**, plus a few distractor files so that search has something to choose from.

- **Content**: all synthetic content newly written for this demo (Claude-G, 2026-10-12); no personal information.
- **Not used to tune the prompt, not given to the classification model**: these files were not used to tune the prompt, and before the demo they were not given to the model Traeva uses for classification; on 10/13 R3 read their full text to check whether they overlap with #22 (read only, no classification request sent).
- **#22 sealed material**: G has not seen the #22 sealed material; **the student needs to check that these files do not overlap with #22**.
- **10/13 replacement**: R3's check found a topic-boundary doubt between the original `Inbox/enzyme-lab.txt` and #22, so it was replaced with `Inbox/fried-rice-recipe.md` (a recipe card for vegetable and egg fried rice); the check record is in #211.
- **Byte protection**: `demo/cac2/** -text` in `.gitattributes` ensures git does not change these bytes; `SHA256SUMS.txt` holds the hashes from when the files were finalized.
- **Everything in the table below is an expectation, not a guarantee**: the model's recommendations, clarification questions and descriptions may differ each time. When a real run differs from the expectation, **record it truthfully** in the last section and do not change these files; to start over, write new files.

## Files and expectations

`setup.py` puts every file under `files/` unchanged at the same location in the demo folder.

| Location | Used in which segment | Expectation |
|---|---|---|
| `Inbox/fried-rice-recipe.md` | Organize: clear recommendation | Recommends `Personal/Recipes` directly, no clarification question |
| `Inbox/carbon-tax-points.md` | Organize: one clarification question | Unsure between economics class (`School/Economics`) and debate club (`Clubs/Debate`), asks one clarification question; after the answer "debate club" it recommends `Clubs/Debate` |
| `Inbox/museum-visit.pdf` | Organize: PDF with a text layer | Reads the text layer (the core reader reads it as complete, 538 characters), recommends `School/History` |
| `Inbox/sheet-0912.txt` | Find: found by Traeva's description | Organize it once first; the expected recommendation is `Clubs/Robotics`, and Traeva saves a one-sentence description (something like "shift and price sheet for a fundraising sale to pay the robotics competition entry fee"); after that, searching with the meaning of the description finds it by the description. Neither the name nor the text contains words like "义卖" (fundraising sale) or "fundraiser" |
| `Notes/untitled-2.md` | Find: the name does not say what is inside; found by the text | Search `price elasticity notes`; it matches this file by its text; the file name itself is unrelated |
| `Notes/week6.txt` | Find: ask in Chinese, find English content | Search `光合作用和呼吸作用的复习笔记` (review notes on photosynthesis and respiration); it matches this English note by its text. **Precondition**: Find's agreement and the model are both in place, and the model gives English wordings; without a model only the Chinese words of the question itself are used, and it is expected not to be found |
| `Downloads/binary-search-notes.md` | Drop on the icon | Choose `School` as the auto mode target scope; expected to move automatically to `School/Computer Science` |
| `Downloads/gdp-worksheet.txt` | Drop on the icon | Same as above; expected to move automatically to `School/Economics` |
| `Writing/scholarship-essay.md` | Memory: several saves | Save three times following the steps below to get four versions; each change can be seen |
| `School/Biology/photosynthesis-quiz.txt` | Distractor | When asking about photosynthesis in Chinese, it may also be found, as another choice |
| `School/Economics/market-structures.md` | Distractor | Another economics file |
| `Clubs/Debate/motion-ideas.md` | Distractor | A file the debate club already has |
| `Personal/Travel/trip-checklist.md` | Distractor | Unrelated to school work |
| `Personal/Recipes/grocery-list.txt` | Distractor | Somewhat similar to the fundraising sale (lemons, flour, butter); may also appear when searching for the sale by description |
| `Personal/weekend-plan.txt` | Distractor | Chinese content |

Empty folders: `School/History`, `School/Computer Science` and `Clubs/Robotics` are the recommended targets and are empty at the start.

## Setup

```bash
python3 demo/cac2/setup.py
```

- It creates `~/Traeva-CAC-Demo-2` (refuses if it already exists), copies the 15 files byte for byte to their locations, and checks the hashes before and after copying.
- Open the packaged `.app`, and with Add a folder… (the first-launch card, or Add folder… in Settings) choose `~/Traeva-CAC-Demo-2`.
- A fresh app-data is recommended, so that only the demo's records are on screen.

## Demo steps

**Organize** (normal flow):

1. For "File from" choose `Inbox`; for "Into" choose `~/Traeva-CAC-Demo-2`.
2. After the model is set up and sending has been agreed to, choose in order:
   - `fried-rice-recipe.md`: Suggest a folder → preview → confirm;
   - `carbon-tax-points.md`: Suggest a folder → answer one question (choose debate club) → preview → confirm;
   - `museum-visit.pdf`: same as above, with no clarification question.
3. Then organize `sheet-0912.txt`, to prepare for Find's "found by description".

**Find**:

1. Turn on Find's agreement on the home page.
2. Search in order:
   - `price elasticity notes`: the name does not say what is inside; found by the text;
   - `光合作用和呼吸作用的复习笔记` (review notes on photosynthesis and respiration): asked in Chinese, finds English content;
   - `义卖的值班表` (fundraising sale shift schedule) (or `bake sale volunteer schedule`): found by Traeva's description. This step must come after `sheet-0912.txt` has been organized.
3. Click a result, "Show in Finder".

**Drop on the icon**:

1. Turn on auto mode in Settings, choose `School` as the target scope, agree to the explanation.
2. Open `Downloads` in Finder and drag both files together onto the Traeva icon in the Dock.
3. Expected: both move automatically, with no confirmation page. Undo is possible afterwards.

**Memory** (the versions must be saved live in the real App):

1. Turn on version recording for `Writing/scholarship-essay.md` the way the App does it at demo time:
   - when using #201's automatic recording, organize this file once first;
   - otherwise turn this file on in Memory.
2. Open it in a text editor and make the three changes below in order. **After each save, wait until the App shows the new version before making the next one**:
   - **1st change**: after the first paragraph (the kitchen timer sentence), start a new paragraph and add:
     `Last spring I joined the robotics club and built a gripper that could pick up a tennis ball.`
   - **2nd change**: replace the last sentence with:
     `I want to study mechanical engineering so that I can design tools that people can repair themselves.`
   - **3rd change**: delete the sentence `I did not fix it that day, but I wanted to know how things work.`, and add on a new line at the end:
     `That kitchen timer still sits on my desk.`
3. On the Memory page, look at the four versions and what changed each time; old versions can be opened.

## Real run record

After each rehearsal or recording, write down here: the date, the `main` commit, the prompt version, the model, and the actual result for each file; write down truthfully wherever it differs from the expectation.

| Date | Commit | File | Expected | Actual |
|---|---|---|---|---|
| | | | | |

## Rules

- These files are only for the demo; they must not be used to tune the prompt, and must not be used for #22. If the prompt is changed in the future, new demo files must be written.
- `files/Inbox/museum-visit.pdf` is generated by `build_pdf.py` (reportlab 5.0.1, invariant mode), with the same bytes every time. It is not needed when preparing the demo.
