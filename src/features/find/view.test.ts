// Tests for view.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  LEFT_OFF_MS,
  MAX_PICK_UP,
  accepts,
  coverageLines,
  fileIcon,
  firstWaiting,
  kindLabel,
  lastSegment,
  matchedText,
  noSearchText,
  matchLines,
  modelUse,
  parentPath,
  pickUpRows,
  recentFolders,
  resultTimeText,
  timeText,
  whenText,
  wordsByText,
  wordsText,
} from "./view.ts";
import type { DropItemView } from "../autodrop/types.ts";
import type { MemoryRecordView } from "../family/types.ts";
import type { ActionView } from "../organize/types.ts";
import type { FindCoverage, FindResult, FindSetupView, FindView, FindWords } from "./types.ts";

function coverage(extra: Partial<FindCoverage> = {}): FindCoverage {
  return {
    folders: 1,
    folders_unavailable: 0,
    folders_left_out: false,
    files: 12,
    file_limit_reached: false,
    described: 0,
    texts_searched: 0,
    texts_too_large: 0,
    texts_in_cloud: 0,
    texts_unreadable: 0,
    texts_not_reached: 0,
    ...extra,
  };
}

function result(extra: Partial<FindResult> = {}): FindResult {
  return {
    result_id: "R1",
    root_id: "root-1",
    folder: "/Users/test/Synthetic/Trips",
    relative_path: "Trips/kyoto.md",
    name: "kyoto.md",
    kind: "markdown",
    size: 100,
    modified_at_ms: 0,
    matches: [],
    described: false,
    description_outdated: false,
    memory_chain_id: null,
    opened_at_ms: null,
    added_at_ms: null,
    time_found: null,
    ...extra,
  };
}

function view(extra: Partial<FindView> = {}): FindView {
  return {
    find_id: "find-1",
    state: "done",
    words: null,
    words_by: null,
    results: [],
    matched: 0,
    coverage: null,
    problem: null,
    ...extra,
  };
}

function setup(extra: Partial<FindSetupView> = {}): FindSetupView {
  return {
    model_ready: true,
    provider: "openai",
    model: "synthetic-model",
    consent_agreed_at_ms: null,
    notice: "notice",
    ...extra,
  };
}

test("kinds Traeva reads have their names; others show the extension or nothing", () => {
  assert.equal(kindLabel("powerpoint", "deck.pptx"), "PowerPoint");
  assert.equal(kindLabel("pdf", "scan.pdf"), "PDF");
  assert.equal(kindLabel(null, "Budget.numbers"), "NUMBERS");
  assert.equal(kindLabel(null, "archive.tar.gz"), "GZ");
  // Images Organize reads by text recognition say they are images.
  assert.equal(kindLabel(null, "whiteboard.JPG"), "JPG image");
  assert.equal(kindLabel(null, "notes.png"), "PNG image");
  assert.equal(kindLabel(null, "scan.jpeg"), "JPEG image");
  assert.equal(kindLabel(null, "photo.heic"), "HEIC");
  assert.equal(kindLabel(null, "Makefile"), null);
  assert.equal(kindLabel(null, "trailing."), null);
});

test("today and yesterday show the time; other days the date, with the year only when it differs", () => {
  const now = new Date(2026, 9, 12, 15, 0).getTime();
  assert.match(whenText(new Date(2026, 9, 12, 9, 5).getTime(), now), /^today, /);
  assert.match(whenText(new Date(2026, 9, 11, 23, 59).getTime(), now), /^yesterday, /);
  // Across a month's start.
  assert.match(whenText(new Date(2026, 9, 31, 8, 0).getTime(), new Date(2026, 10, 1, 9, 0).getTime()), /^yesterday, /);
  const thisYear = whenText(new Date(2026, 2, 3).getTime(), now);
  assert.doesNotMatch(thisYear, /today|yesterday|2026/);
  assert.match(whenText(new Date(2024, 2, 3).getTime(), now), /2024/);
  assert.equal(whenText(Number.NaN, now), "unknown");
});

test("a file's icon: its kind, else one from the extension, else the extension as the label", () => {
  assert.deepEqual(fileIcon("word", "essay.docx"), { kind: "word", label: "W" });
  assert.deepEqual(fileIcon("pdf", "scan.PDF"), { kind: "pdf", label: "PDF" });
  // The core's kind wins over the name.
  assert.deepEqual(fileIcon("markdown", "notes.txt"), { kind: "markdown", label: "MD" });
  // Names only (a dropped file, an action): the colour from the extension.
  assert.deepEqual(fileIcon(null, "field-trip.md"), { kind: "markdown", label: "MD" });
  assert.deepEqual(fileIcon(null, "Budget.XLSX"), { kind: "excel", label: "XLS" });
  assert.deepEqual(fileIcon(null, "deck.ppt"), { kind: "powerpoint", label: "PPT" });
  // Images: their own icon, labelled with the extension.
  assert.deepEqual(fileIcon(null, "photo.jpeg"), { kind: "image", label: "JPEG" });
  assert.deepEqual(fileIcon(null, "board.PNG"), { kind: "image", label: "PNG" });
  assert.deepEqual(fileIcon(null, "photo.heic"), { kind: "other", label: "HEIC" });
  assert.deepEqual(fileIcon(null, "sheet.numbers"), { kind: "other", label: "" });
  assert.deepEqual(fileIcon(null, "README"), { kind: "other", label: "" });
  // Not a key of Object.prototype.
  assert.deepEqual(fileIcon(null, "x.constructor"), { kind: "other", label: "" });
});

test("paths: the last part and the folder it is in", () => {
  assert.equal(lastSegment("/Users/test/School/English"), "English");
  assert.equal(lastSegment("/Users/test/School/English/"), "English");
  assert.equal(lastSegment("essay.docx"), "essay.docx");
  assert.equal(lastSegment(""), "");
  assert.equal(parentPath("/Users/test/School/essay.docx"), "/Users/test/School");
  assert.equal(parentPath("/essay.docx"), "/");
  assert.equal(parentPath("essay.docx"), "");
});

test("recent folders: each once, the one with the newest opened file first, with that file", () => {
  const files = [
    result({ result_id: "R1", folder: "/S/English", name: "a.docx", opened_at_ms: 500 }),
    result({ result_id: "R2", folder: "/S/Physics", name: "b.pdf", opened_at_ms: 400 }),
    result({ result_id: "R3", folder: "/S/English", name: "c.docx", opened_at_ms: 300 }),
    // Not in the core's order: still counted by its time.
    result({ result_id: "R4", folder: "/S/Physics", name: "d.pdf", opened_at_ms: 900 }),
    result({ result_id: "R5", folder: "/S/Art", name: "e.md", opened_at_ms: null }),
  ];
  assert.deepEqual(recentFolders(files), [
    { folder: "/S/Physics", name: "Physics", resultId: "R4" },
    { folder: "/S/English", name: "English", resultId: "R1" },
    { folder: "/S/Art", name: "Art", resultId: "R5" },
  ]);
  assert.deepEqual(recentFolders([]), []);
});

const NOW = new Date(2026, 9, 12, 15, 0).getTime();

function drop(state: DropItemView["state"], extra: Partial<DropItemView> = {}): DropItemView {
  return {
    drop_id: `drop-${state}`,
    file_name: "field-trip.md",
    batch_id: "batch-1",
    received_at_ms: 0,
    state,
    automatic: true,
    folder_to_authorize: null,
    operation_id: null,
    plan_id: null,
    action_id: null,
    moved_to: null,
    reason: null,
    // As the core works it out for a flow nobody has left yet.
    needs_user:
      state === "needs_source_authorization" ||
      state === "needs_answer" ||
      (state === "handed_over" && (extra.action_id ?? null) === null),
    action_status: null,
    ...extra,
  };
}

function action(extra: Partial<ActionView> = {}): ActionView {
  return {
    action_id: "act-1",
    plan_id: "plan-1",
    kind: "organize",
    file_id: "file-1",
    status: "committed",
    source: { root_id: "r1", relative_path: "Inbox/notes.md", display_path: "/S/Inbox/notes.md" },
    target: { root_id: "r1", relative_path: "School/English/notes.md", display_path: "/S/School/English/notes.md" },
    folders_created: [],
    folders_removed: [],
    folders_without_receipt: [],
    folders_kept: [],
    created_at_ms: NOW - 60_000,
    committed_at_ms: NOW - 50_000,
    recovered_at_ms: null,
    problem: null,
    undo_of_action_id: null,
    undo_status: "available",
    undo_action_id: null,
    ...extra,
  };
}

function record(capturedAt: number | null, extra: Partial<MemoryRecordView> = {}): MemoryRecordView {
  const latest =
    capturedAt === null
      ? null
      : {
          snapshot_id: "snap-1",
          file_id: "file-2",
          sequence: 14,
          file_revision: 1,
          root_id: "r1",
          relative_path: "School/Climate.docx",
          sha256: "0".repeat(64),
          byte_length: 10,
          captured_at_ms: capturedAt,
        };
  return {
    chain_id: "chain-1",
    family_id: "family-1",
    origin_file_id: "file-2",
    permission_file_id: "file-2",
    state: "active",
    watch: {
      watch_id: "watch-1",
      file_id: "file-2",
      status: "active",
      root_id: "r1",
      relative_path: "School/Climate.docx",
      started_at_ms: 0,
      stopped_at_ms: null,
      versions: 14,
      latest,
      checked_at_ms: null,
      problem: null,
    },
    line_versions: 14,
    place: { root_id: "r1", relative_path: "School/Climate.docx", display_path: "/S/School/Climate.docx" },
    root_active: true,
    waiting_for_relocation: false,
    candidate_capture: "off",
    pending: { objects: 0, snapshots: 0, limit: 20 },
    open_gaps: [],
    same_content: [],
    hold: null,
    ...extra,
  };
}

const NOTHING = { drops: [], outcomes: {}, decisions: [], lastAction: null, records: [], now: NOW };

test("pick up: nothing to show when nothing waits and nothing happened lately", () => {
  assert.deepEqual(pickUpRows(NOTHING), { rows: [], more: 0 });
  // Old enough not to be where the user left off.
  const old = NOW - LEFT_OFF_MS - 1;
  const stale = pickUpRows({
    ...NOTHING,
    lastAction: action({ committed_at_ms: old }),
    records: [record(old)],
  });
  assert.deepEqual(stale.rows, []);
});

test("pick up: dropped files that wait for the user, each with its one action", () => {
  const { rows } = pickUpRows({
    ...NOTHING,
    drops: [
      drop("needs_answer", { drop_id: "a", file_name: "field-trip.md" }),
      drop("needs_source_authorization", { drop_id: "b", file_name: "lab.pdf" }),
      drop("handed_over", { drop_id: "c", file_name: "essay.docx" }),
    ],
  });
  assert.deepEqual(
    rows.map((row) => [row.name, row.detail, row.action]),
    [
      ["field-trip.md", "Traeva has a question about it", { kind: "organize", label: "Answer" }],
      ["lab.pdf", "Needs your OK for its folder", { kind: "organize", label: "Review" }],
      ["essay.docx", "Needs you to finish organizing", { kind: "organize", label: "Review" }],
    ],
  );
});

test("pick up: finished or busy dropped files are left out", () => {
  const drops = [
    drop("waiting", { drop_id: "a" }),
    drop("working", { drop_id: "b" }),
    drop("moved", { drop_id: "c" }),
    // Confirmed in the usual flow.
    drop("handed_over", { drop_id: "d", action_id: "act-9" }),
    // Ended in Organize without a plan (this window knows).
    drop("handed_over", { drop_id: "e" }),
  ];
  assert.deepEqual(pickUpRows({ ...NOTHING, drops, outcomes: { e: { kind: "given_up" } } }).rows, []);
});

test("pick up: a decision, then the newest version, then the last move", () => {
  const { rows, more } = pickUpRows({
    ...NOTHING,
    decisions: [action({ action_id: "act-2", status: "needs_user_decision" })],
    lastAction: action(),
    records: [
      record(NOW - 3_600_000, { chain_id: "older" }),
      record(NOW - 60_000, { chain_id: "newer" }),
      // Not recording now: left out however new.
      record(NOW, { chain_id: "stopped", state: "stopped" }),
      record(NOW, { chain_id: "elsewhere", root_active: false }),
      record(NOW, { chain_id: "missing", waiting_for_relocation: true }),
      record(null, { chain_id: "none-yet" }),
    ],
  });
  assert.equal(more, 0);
  assert.deepEqual(rows[0], {
    key: "decision:act-2",
    name: "notes.md",
    detail: "Needs your decision",
    action: { kind: "decision", label: "Review" },
  });
  assert.equal(rows[1].name, "Climate.docx");
  assert.match(rows[1].detail, /^Last version saved today, .* · 14 versions$/);
  assert.deepEqual(rows[1].action, { kind: "history", chainId: "newer" });
  assert.equal(rows[2].name, "notes.md");
  assert.match(rows[2].detail, /^Moved to English · today, /);
  assert.equal(rows[2].action, null);
});

test("pick up: only a finished move is shown; an undo says where the file went back", () => {
  for (const status of ["prepared", "running", "failed_no_move", "needs_user_decision", "undone"] as const) {
    assert.deepEqual(pickUpRows({ ...NOTHING, lastAction: action({ status }) }).rows, [], status);
  }
  const undo = action({
    kind: "undo",
    target: { root_id: "r1", relative_path: "Inbox/notes.md", display_path: "/S/Inbox/notes.md" },
  });
  assert.match(pickUpRows({ ...NOTHING, lastAction: undo }).rows[0].detail, /^Put back in Inbox · /);
  // A removed folder: only the relative path is known.
  const bare = action({ target: { root_id: "gone", relative_path: "notes.md", display_path: "notes.md" } });
  assert.match(pickUpRows({ ...NOTHING, lastAction: bare }).rows[0].detail, /^Moved to the folder you added · /);
});

test("pick up: at most three rows, waiting things first, and how many more wait", () => {
  const drops = ["a", "b", "c", "d"].map((id) => drop("needs_answer", { drop_id: id, file_name: `${id}.md` }));
  const { rows, more } = pickUpRows({
    ...NOTHING,
    drops,
    decisions: [action({ action_id: "act-2", status: "needs_user_decision" })],
    lastAction: action(),
    records: [record(NOW)],
  });
  assert.equal(rows.length, MAX_PICK_UP);
  assert.deepEqual(
    rows.map((row) => row.name),
    ["a.md", "b.md", "c.md"],
  );
  assert.equal(more, 2);
  // Room left after the waiting things goes to the version, then the move.
  const two = pickUpRows({ ...NOTHING, drops: drops.slice(0, 2), lastAction: action(), records: [record(NOW)] });
  assert.deepEqual(
    two.rows.map((row) => row.key),
    ["drop:a", "drop:b", "record:chain-1"],
  );
  assert.equal(two.more, 0);
});

test("pick up: the filled button is the first thing that waits, never View history", () => {
  const decision = pickUpRows({
    ...NOTHING,
    decisions: [action({ action_id: "act-2", status: "needs_user_decision" })],
    records: [record(NOW)],
  });
  assert.equal(firstWaiting(decision.rows), 0);
  // Only a version and a move: none (the newest recent file's Open is then).
  const calm = pickUpRows({ ...NOTHING, lastAction: action(), records: [record(NOW)] });
  assert.equal(calm.rows[0].action?.kind, "history");
  assert.equal(firstWaiting(calm.rows), -1);
  assert.equal(firstWaiting([]), -1);
});

test("each match is one line in the core's order, with the wording and the snippet", () => {
  const lines = matchLines(
    result({
      described: true,
      description_outdated: true,
      matches: [
        { thing: 1, wording: "Kyoto", place: "name", snippet: null, related_words: [] },
        { thing: 0, wording: "budget", place: "description", snippet: "…the trip budget…", related_words: [] },
        { thing: 2, wording: "march", place: "text", snippet: "…in March we…", related_words: [] },
        { thing: 3, wording: "Trips", place: "folder", snippet: null, related_words: [] },
      ],
    }),
  );
  assert.deepEqual(
    lines.map((line) => [line.label, line.wording, line.snippet, line.outdated]),
    [
      ["In its name", "Kyoto", null, false],
      ["From Traeva's description", "budget", "…the trip budget…", true],
      ["From its text", "march", "…in March we…", false],
      ["In its folder's name", "Trips", null, false],
    ],
  );
});

test("a time range reads as minutes to years, and a result shows the time that matched", () => {
  assert.equal(timeText({ newest: 15, oldest: 60, unit: "minute" }), "changed, added or opened 15 to 60 minutes ago");
  assert.equal(timeText({ newest: 0, oldest: 15, unit: "minute" }), "changed, added or opened in the last 15 minutes");
  assert.equal(timeText({ newest: 1, oldest: 3, unit: "day" }), "changed, added or opened 1 to 3 days ago");
  assert.equal(timeText({ newest: 0, oldest: 1, unit: "year" }), "changed, added or opened in the last year");
  const now = new Date(2026, 9, 17, 12, 0).getTime();
  const day = 86_400_000;
  const base = { modified_at_ms: now - 40 * day, added_at_ms: now - 2 * day, opened_at_ms: now - 15 * day };
  assert.match(resultTimeText(result({ ...base, time_found: "added" }), now), /^added to its folder /);
  assert.match(resultTimeText(result({ ...base, time_found: "opened" }), now), /^opened /);
  assert.match(resultTimeText(result({ ...base, time_found: null }), now), /^modified /);
  assert.match(resultTimeText(result({ ...base, time_found: "added", added_at_ms: null }), now), /^modified /);
});

test("a match through related words lists them under one label", () => {
  const lines = matchLines(
    result({
      matches: [
        { thing: 0, wording: "reaction", place: "text", snippet: "…the reaction…", related_words: ["reaction", "mole"] },
      ],
    }),
  );
  assert.deepEqual(
    lines.map((line) => [line.label, line.wording, line.snippet]),
    [["Related words it uses", "reaction, mole", "…the reaction…"]],
  );
  assert.equal(
    wordsText({ things: [["chemistry", "化学"]], related: [["reaction", "mole"]], kinds: [], when: null }),
    "Looked for “chemistry” or “化学” (or 2 of reaction, mole).",
  );
});

test("an outdated description marks only the description line", () => {
  const lines = matchLines(
    result({ description_outdated: true, matches: [{ thing: 0, wording: "kyoto", place: "text", snippet: "kyoto", related_words: [] }] }),
  );
  assert.equal(lines[0].outdated, false);
});

test("the words line lists each thing's wordings, then the kinds and the time range", () => {
  assert.equal(
    wordsText({ things: [["京都", "Kyoto"], ["budget"]], related: [], kinds: [], when: null }),
    "Looked for “京都” or “Kyoto”; “budget”.",
  );
  assert.equal(
    wordsText({ things: [["budget"]], related: [], kinds: ["word", "excel", "pdf"], when: { newest: 0, oldest: 1, unit: "week" } }),
    "Looked for “budget” · only Word, Excel and PDF files · changed, added or opened in the last week.",
  );
  assert.equal(wordsText({ things: [], related: [], kinds: ["pdf"], when: { newest: 0, oldest: 1, unit: "day" } }), "Looked for any file · only PDF files · changed, added or opened in the last day.");
});

test("the words say whether the model made them, and why not", () => {
  assert.equal(wordsByText({ kind: "model" }), "Searched with the model.");
  assert.equal(
    wordsByText({ kind: "question", reason: "Searching with the model is not turned on." }),
    "Searched with your words: Searching with the model is not turned on.",
  );
});

test("the count says when only the first results are shown", () => {
  assert.equal(matchedText(view({ matched: 0 })), "No files matched.");
  assert.equal(matchedText(view({ matched: 1, results: [result()] })), "1 file matched.");
  const thirty = Array.from({ length: 30 }, (_, i) => result({ result_id: `R${i + 1}` }));
  assert.equal(matchedText(view({ matched: 45, results: thirty })), "45 files matched, showing the first 30.");
});

test("a search that could not look at the files says why, instead of a count", () => {
  assert.equal(noSearchText(view({ problem: "Traeva could not read its records." })), "Traeva could not read its records.");
  const nothing: FindWords = { things: [], related: [], kinds: [], when: null };
  assert.equal(noSearchText(view({ words: nothing })), "Type a word about the file.");
  // A kind or a time range is something to look for.
  assert.equal(noSearchText(view({ words: { things: [], related: [], kinds: ["pdf"], when: null } })), null);
  assert.equal(noSearchText(view({ words: { things: [], related: [], kinds: [], when: { newest: 0, oldest: 1, unit: "week" } } })), null);
  assert.equal(noSearchText(view({ words: { things: [["kyoto"]], related: [], kinds: [], when: null } })), null);
});

test("coverage mentions only what happened", () => {
  const kyoto: FindWords = { things: [["kyoto"]], related: [], kinds: [], when: null };
  assert.deepEqual(coverageLines(coverage(), kyoto), ["Searched 12 files in the folder you added."]);
  assert.deepEqual(
    coverageLines(
      coverage({
        folders: 2,
        folders_unavailable: 1,
        folders_left_out: true,
        file_limit_reached: true,
        described: 3,
        texts_searched: 8,
        texts_too_large: 1,
        texts_in_cloud: 2,
        texts_unreadable: 1,
        texts_not_reached: 4,
      }),
      kyoto,
    ),
    [
      "Searched 12 files in the 2 folders you added.",
      "Searched Traeva's descriptions of 3 files.",
      "Searched the text of 8 files.",
      "Text not searched: 1 over 20 MB, searched by name only; 2 only in iCloud, not downloaded; 1 unreadable; 4 not reached within 10 seconds.",
      "1 folder you added was unavailable and not searched.",
      "Some subfolders were left out: the search stops at 5,000 folders or 12 levels deep.",
      "The search stopped at 20,000 files.",
    ],
  );
  assert.deepEqual(coverageLines(coverage({ folders: 0, files: 0, folders_unavailable: 2 }), kyoto), [
    "No folder could be searched.",
    "2 folders you added were unavailable and not searched.",
  ]);
});

test("without words to look for, no description was compared", () => {
  const any: FindWords = { things: [], related: [], kinds: [], when: null };
  assert.deepEqual(coverageLines(coverage({ described: 3 }), any), ["Searched 12 files in the folder you added."]);
  assert.deepEqual(coverageLines(coverage({ described: 3 }), null), ["Searched 12 files in the folder you added."]);
});

test("the model is offered only when it is set up, and asks for agreement first", () => {
  assert.equal(modelUse(setup({ model_ready: false })), "no_model");
  assert.equal(modelUse(setup({ model_ready: false, consent_agreed_at_ms: 5 })), "no_model");
  assert.equal(modelUse(setup()), "ask");
  assert.equal(modelUse(setup({ consent_agreed_at_ms: 5 })), "agreed");
});

test("only the latest search is shown, and a late poll never moves it back", () => {
  const working = view({ state: "working" });
  const withWords = view({ state: "working", words: { things: [["kyoto"]], related: [], kinds: [], when: null } });
  const done = view({ state: "done", coverage: coverage() });
  const cancelled = view({ state: "cancelled" });
  // Nothing accepted before the core named the search, nor an older one.
  assert.equal(accepts(null, null, working), false);
  assert.equal(accepts("find-2", working, done), false);
  assert.equal(accepts("find-2", working, view({ find_id: "find-2", state: "working" })), true);
  // The same search goes forward only.
  assert.equal(accepts("find-1", working, withWords), true);
  assert.equal(accepts("find-1", withWords, working), false);
  assert.equal(accepts("find-1", withWords, done), true);
  assert.equal(accepts("find-1", done, withWords), false);
  assert.equal(accepts("find-1", cancelled, working), false);
  assert.equal(accepts("find-1", working, cancelled), true);
});
