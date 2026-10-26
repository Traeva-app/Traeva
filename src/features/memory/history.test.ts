// Tests for history.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  canExport,
  coverageText,
  exportName,
  exportRefusal,
  historyKey,
  isSettling,
  lineNumbers,
  lineVersions,
  pairGapText,
  pairsAround,
  stopLabel,
  stopNotes,
  timelineLabel,
  minorText,
  MODEL_WORDS_NOTE,
  DOCX_PREVIEW_NOTE,
  DECK_PREVIEW_NOTE,
  WORKBOOK_PREVIEW_NOTE,
  isDocxVersion,
  isStructureEdit,
  previewKind,
  previewStatusText,
  structureLabel,
  noLongerCertainText,
  openActionText,
  openedCopyRef,
  pairStateText,
  placeText,
  reachedByText,
  searchFailedText,
  searchTitle,
  unsupportedText,
  versionLabel,
  versionRef,
} from "./history.ts";
import type { Coverage, PairState, SnapshotRef, VersionPairView } from "./types";

const ALL_STATES: PairState[] = [
  { kind: "not_compared" },
  { kind: "no_text_change" },
  { kind: "unreadable", note: "The later version has no readable text." },
  { kind: "not_permitted" },
  { kind: "queued" },
  { kind: "analyzing" },
  { kind: "analyzed", complete: true, hunks: 2, left_out_hunks: 0, minor_hunks: [] },
  { kind: "analyzed", complete: false, hunks: 3, left_out_hunks: 1, minor_hunks: [] },
  { kind: "failed", error_code: "model_timeout", will_retry: true },
  { kind: "invalid_output", will_retry: false, reason: null },
];

function version(sequence: number): SnapshotRef {
  return {
    snapshot_id: `s${sequence}`,
    file_id: "f",
    sequence,
    file_revision: 1,
    root_id: "r",
    relative_path: "essay.md",
    sha256: "0".repeat(64),
    byte_length: 1,
    captured_at_ms: 0,
  };
}

function pair(state: PairState, gap = false): VersionPairView {
  return {
    before: version(1),
    after: version(2),
    pair_id: null,
    state,
    changes: [],
    basis: "same_file",
    evidence: "system",
    gap,
    gap_kinds: gap ? ["watcher_stopped"] : [],
    unplaced: false,
    permission_file_ids: ["f"],
    complete: true,
    edge_id: null,
    linked_automatically: false,
  };
}

function coverage(fields: Partial<Coverage>): Coverage {
  return {
    pairs: 3,
    described: 3,
    described_in_part: 0,
    waiting: 0,
    not_permitted: 0,
    failed: 0,
    unreadable: 0,
    gaps: 0,
    gap_after_latest: false,
    changes_left_out: 0,
    complete: true,
    ...fields,
  };
}

test("only a described pair or an unchanged text is shown as settled and fine", () => {
  for (const state of ALL_STATES) {
    const { tone, label } = pairStateText(state);
    const known = state.kind === "no_text_change" || (state.kind === "analyzed" && state.complete);
    assert.equal(tone === "ok", known, `${state.kind}: ${label}`);
    // "Not described" never reads as "nothing changed".
    if (!known) assert.ok(!/did not change/.test(label), label);
  }
});

test("the same bytes saved again are never said to differ (R2-MEM-04)", () => {
  const same = pairStateText({ kind: "no_text_change" }, true).detail ?? "";
  assert.match(same, /exactly the bytes of the one before it/);
  assert.doesNotMatch(same, /differ/);
  assert.match(pairStateText({ kind: "no_text_change" }, false).detail ?? "", /bytes differ/);
});

test("a pair described in part says what was left out", () => {
  const { detail } = pairStateText({
    kind: "analyzed",
    complete: false,
    hunks: 3,
    left_out_hunks: 1,
    minor_hunks: [],
  });
  assert.match(detail ?? "", /1 of 3 changed places/);
});

test("a failure says whether Traeva tries again by itself", () => {
  assert.match(
    pairStateText({ kind: "failed", error_code: "model_timeout", will_retry: true }).detail ?? "",
    /tries again by itself/,
  );
  assert.match(
    pairStateText({ kind: "invalid_output", will_retry: false, reason: null }).detail ?? "",
    /does not try again/,
  );
});

test("the page keeps asking only while a pair is on its way", () => {
  assert.ok(isSettling([pair({ kind: "queued" })]));
  assert.ok(isSettling([pair({ kind: "no_text_change" }), pair({ kind: "analyzing" })]));
  assert.ok(isSettling([pair({ kind: "not_compared" })]));
  assert.ok(!isSettling([pair({ kind: "not_permitted" }), pair({ kind: "no_text_change" })]));
  assert.ok(!isSettling([pair({ kind: "failed", error_code: "model_timeout", will_retry: true })]));
});

test("coverage says when a match may lie in the part not searched", () => {
  assert.match(coverageText(coverage({})), /every change in all 3 pairs/);
  const partial = coverageText(
    coverage({ complete: false, described: 1, waiting: 1, gaps: 1, changes_left_out: 4 }),
  );
  assert.match(partial, /1 of 3 pairs/);
  assert.match(partial, /1 not described yet/);
  assert.match(partial, /1 where Traeva may have missed saves/);
  assert.match(partial, /oldest 4 changes were left out/);
  assert.match(partial, /may also be in the part not searched/);
  assert.match(coverageText(coverage({ pairs: 0, described: 0 })), /only one saved version/);
});

test("a stop after the newest version is said, and a pair with a gap is a difference, not one save", () => {
  const text = coverageText(coverage({ complete: false, gap_after_latest: true }));
  assert.match(text, /missed saves after the newest version/);
  assert.match(text, /may also be in the part not searched/);
  assert.equal(pairGapText(pair({ kind: "queued" })), null);
  assert.match(pairGapText(pair({ kind: "queued" }, true)) ?? "", /not one save/);
});

test("a picked version is never presented as proven", () => {
  assert.match(reachedByText("pick"), /did not check/);
  assert.match(reachedByText("search"), /only match/);
});

test("unsupported questions suggest describing a change, never 'not found'", () => {
  for (const askedBy of ["state", "time", "name"] as const) {
    const text = unsupportedText(askedBy);
    assert.match(text, /not supported yet/);
    assert.ok(!/not found/i.test(text), text);
  }
});

test("titles", () => {
  assert.equal(searchTitle({ kind: "candidates", candidates: [], question: null }), "Several matches");
  const one = {
    candidate_id: "K1",
    before: version(1),
    after: version(2),
    version: version(1),
    changes: [],
    maybe: false,
    partial: false,
    no_longer_usable: null,
  };
  assert.equal(searchTitle({ kind: "candidates", candidates: [one], question: null }), "One possible match");
  assert.equal(
    searchTitle({
      kind: "candidates",
      candidates: [],
      question: { question_id: "q", text: "Which?", options: [] },
    }),
    "One question",
  );
  assert.equal(minorText([]), null);
  assert.match(minorText(["H1", "H2"]) ?? "", /2 places/);
});

test("an answer whose pair the core no longer lists is never shown as found", () => {
  const one = {
    candidate_id: "K1",
    before: version(1),
    after: version(2),
    version: version(1),
    changes: [],
    maybe: false,
    partial: false,
    no_longer_usable: null,
  };
  assert.equal(searchTitle({ kind: "found", candidate: one, reached_by: "search", openable: true }), "Found");
  const revoked = { ...one, no_longer_usable: "relation_superseded" as const };
  assert.equal(
    searchTitle({ kind: "found", candidate: revoked, reached_by: "search", openable: true }),
    "No longer certain",
  );
  assert.match(noLongerCertainText("relation_superseded"), /^No longer certain: .*revoked or replaced/);
  assert.match(noLongerCertainText("relation_disputed"), /disputed/);
  assert.match(searchFailedText("page_outdated"), /history changed .* nothing more was sent/);
  assert.equal(searchFailedText("model_timeout"), "The model could not be asked (model_timeout).");
  for (const reason of [
    "not_a_member",
    "candidate_version",
    "no_confirmed_relation",
    "not_adjacent",
  ] as const) {
    assert.match(noLongerCertainText(reason), /^No longer certain: .+ Search again/);
  }
});

test("versions are numbered along the record's line, not by each file's own count", () => {
  // Old file v1, v2, v3, then the file the record went on as (its own v1).
  const on = (file: string, sequence: number): SnapshotRef => ({
    ...version(sequence),
    file_id: file,
    snapshot_id: `${file}${sequence}`,
  });
  const line = [on("a", 1), on("a", 2), on("a", 3), on("b", 1)];
  const pairs = line.slice(1).map((after, i) => ({ ...pair({ kind: "no_text_change" }), before: line[i], after }));
  const numbers = lineNumbers(pairs);
  assert.deepEqual([...numbers.entries()], [["a1", 1], ["a2", 2], ["a3", 3], ["b1", 4]]);
  assert.match(versionLabel(on("b", 1), numbers), /^Version 4 · /);
  assert.equal(versionRef(on("a", 2), numbers), "version 2");
  assert.match(versionRef(on("c", 1), numbers), /^the version saved /, "not on the line: by time, never a number");
  assert.match(versionLabel(on("c", 1), numbers), /^Saved /);
});

test("an export starts with the version's line number and keeps its extension", () => {
  const essay = { ...version(1), relative_path: "Drafts/essay.md", snapshot_id: "e1" };
  assert.equal(exportName(essay, new Map([["e1", 3]])), "essay (version 3).md");
  assert.equal(exportName(essay, new Map()), "essay (saved copy).md");
  assert.equal(exportName({ ...essay, relative_path: "notes" }, new Map([["e1", 2]])), "notes (version 2)");
  assert.ok(canExport(essay));
  assert.ok(canExport({ ...essay, relative_path: "a/B.TXT" }));
  assert.ok(!canExport({ ...essay, relative_path: "setup.terminal" }));
});

test("a refused export always says nothing was written", () => {
  for (const code of [
    "target_conflict",
    "outside_root",
    "unsupported_location",
    "protected_location",
    "unresolved_action",
    "page_outdated",
  ] as const) {
    assert.match(exportRefusal(code) ?? "", /Nothing was written/, code);
  }
  assert.match(exportRefusal("target_conflict") ?? "", /never replaces a file/);
  assert.equal(exportRefusal("result_unconfirmed"), null, "not a failure: shown on its own");
  assert.equal(exportRefusal("invalid_input"), null, "the core says which rule the name breaks");
});

test("one saved version without a pair is version 1, so it can be opened (R2-MEM-01)", () => {
  const only = { ...version(1), snapshot_id: "only" };
  const numbers = lineNumbers([], only);
  assert.equal(numbers.get("only"), 1);
  assert.match(versionLabel(only, numbers), /^Version 1 · /);
  assert.equal(lineNumbers([], null).size, 0);
  // With pairs, the line decides; the newest version is not put first.
  const pairs = [pair({ kind: "no_text_change" })];
  assert.deepEqual([...lineNumbers(pairs, { ...version(2), snapshot_id: "s2" }).entries()], [["s1", 1], ["s2", 2]]);
});

test("the history key changes with a new version or a new state, not on a repeat (R2-MEM-02)", () => {
  const four = [pair({ kind: "no_text_change" })];
  assert.equal(historyKey(four), historyKey([pair({ kind: "no_text_change" })]));
  assert.notEqual(historyKey(four), historyKey([...four, pair({ kind: "queued" })]));
  assert.notEqual(historyKey([pair({ kind: "queued" })]), historyKey([pair({ kind: "no_text_change" })]));
  assert.equal(historyKey(null), historyKey([]));
});

test("a dropped answer says which rule it broke, or that it was not recorded", () => {
  const rule = "change 1's after_quote is not in the added lines of its hunks";
  assert.match(
    pairStateText({ kind: "invalid_output", will_retry: true, reason: rule }).detail ?? "",
    /after_quote is not in the added lines/,
  );
  assert.match(
    pairStateText({ kind: "invalid_output", will_retry: true, reason: null }).detail ?? "",
    /which rule was not recorded: an earlier analysis/,
  );
});

test("descriptions are said to be the model's words, checked against the lines (R2-MEM-08)", () => {
  assert.match(MODEL_WORDS_NOTE, /model's wording/);
  assert.match(MODEL_WORDS_NOTE, /read again from your saved versions/);
});

test("an opened copy's receipt keeps the number in its name (R2-MEM-11)", () => {
  assert.equal(openedCopyRef({ number: 4 }), "version 4");
  assert.equal(openedCopyRef({ number: null }), "an older version");
});

test("a Word version is previewed on the page, never opened (#66)", () => {
  const docx = { ...version(1), relative_path: "Drafts/Report.DOCX" };
  const md = { ...version(1), relative_path: "Drafts/essay.md" };
  assert.equal(isDocxVersion(docx), true);
  assert.equal(isDocxVersion(md), false);
  assert.equal(isDocxVersion({ ...version(1), relative_path: "Drafts/report.docx.md" }), false);
  assert.equal(openActionText(docx), "Preview text");
  assert.equal(openActionText(docx, "version 2"), "Preview the text of version 2");
  assert.equal(openActionText(md), "Open a copy");
  assert.equal(openActionText(md, "version 2"), "Open a copy of version 2");
  assert.match(DOCX_PREVIEW_NOTE, /citation fields, which Memory does not compare or send/);
  assert.match(DOCX_PREVIEW_NOTE, /citations that are plain text are part of the body/);
  assert.match(DOCX_PREVIEW_NOTE, /Word is not opened, and showing it sends nothing\.$/);
});


test("a deck or workbook whose structure only changed says so, and sent nothing (v23)", () => {
  for (const [change, label] of [
    ["slide_order", "Only the order of the slides changed"],
    ["sheet_order", "Only the order of the sheets changed"],
    ["sheet_renamed", "A sheet was renamed; no cell changed"],
    ["labels", "Only the lines Traeva adds changed (such as slide numbers)"],
  ] as const) {
    const text = pairStateText({ kind: "structure_only", change });
    assert.equal(text.label, label);
    assert.match(text.detail ?? "", /nothing was sent/);
  }
  // A workbook's labels are its cells' addresses and types: the writer's.
  const cells = pairStateText({ kind: "structure_only", change: "labels" }, false, true, "workbook");
  assert.equal(cells.label, "Only cell addresses or value types changed");
  assert.match(cells.detail ?? "", /in other cells or stored as another type/);
  assert.match(cells.detail ?? "", /nothing was sent/);
  assert.equal(
    pairStateText({ kind: "structure_only", change: "labels" }, false, true, "deck").label,
    "Only the lines Traeva adds changed (such as slide numbers)",
  );
  assert.equal(
    pairStateText({ kind: "structure_only", change: "sheet_order" }, false, true, "workbook").label,
    "Only the order of the sheets changed",
  );
  assert.equal(isStructureEdit({ change_id: "structure-p1", position: 0 }), true);
  assert.equal(isStructureEdit({ change_id: "minor-p1", position: 0 }), false);
});


test("a change in a deck or workbook says where it is (v23)", () => {
  const none = { slide: null, role: null, sheet: null, cell: null, field: null };
  assert.equal(placeText(null), null);
  assert.equal(placeText(none), null);
  assert.equal(placeText({ ...none, slide: 2, role: "notes" }), "Slide 2 · speaker notes");
  assert.equal(placeText({ ...none, slide: 3, role: "title" }), "Slide 3 · title");
  assert.equal(
    placeText({ ...none, sheet: "Data", cell: "C2", field: "cache" }),
    "Sheet “Data” · C2 · value the file saved (not verified)",
  );
  assert.equal(placeText({ ...none, sheet: "Data", cell: "C2", field: "formula" }), "Sheet “Data” · C2 · formula");
});

test("a PowerPoint or Excel version is previewed on the page, never opened (#28)", () => {
  const deck = { ...version(1), relative_path: "Talks/heat.PPTX" };
  const book = { ...version(1), relative_path: "Data/readings.xlsx" };
  assert.equal(previewKind(deck), "deck");
  assert.equal(previewKind(book), "workbook");
  assert.equal(previewKind({ ...version(1), relative_path: "notes.md" }), null);
  assert.equal(openActionText(deck), "Preview text");
  assert.equal(openActionText(book, "version 3"), "Preview the text of version 3");
  assert.match(DECK_PREVIEW_NOTE, /Hidden slides are shown here and marked; Memory leaves them out and never sends them/);
  assert.match(WORKBOOK_PREVIEW_NOTE, /nobody checked it is current/);
  assert.match(WORKBOOK_PREVIEW_NOTE, /Rows or columns that moved show as many cells changing/);
});

test("a preview labels the reader's lines and says what it could not show", () => {
  assert.equal(structureLabel({ kind: "slide", number: 4, id: 260, hidden: true }), "Slide 4 (hidden)");
  assert.equal(structureLabel({ kind: "role", role: "notes" }), "Speaker notes");
  assert.equal(
    structureLabel({ kind: "sheet", number: 2, id: 7, name: "Raw", visibility: "very_hidden" }),
    "Sheet “Raw” (very hidden)",
  );
  assert.equal(structureLabel({ kind: "cell", address: "C2", field: "formula", valueType: null }), "C2 · formula");
  assert.equal(
    structureLabel({ kind: "cell", address: "C2", field: "cache", valueType: "number" }),
    "C2 · saved value (not verified)",
  );
  assert.equal(structureLabel({ kind: "cell", address: "C2", field: "cache", valueType: null }), "C2 · no saved value");
  assert.equal(
    structureLabel({ kind: "cell", address: "C2", field: "cache", valueType: "blank" }),
    "C2 · saved value: empty (not verified)",
  );
  assert.equal(previewStatusText({ status: "complete", left_out: [], note: null }), null);
  assert.equal(
    previewStatusText({ status: "partial", left_out: ["pictures"], note: null }),
    "Part of the text: left out are pictures.",
  );
  assert.match(previewStatusText({ status: "unsupported", left_out: [], note: "No reader yet." }) ?? "", /cannot be shown: No reader yet\./);
  assert.match(previewStatusText({ status: "failed", left_out: [], note: null }) ?? "", /could not be read\./);
});

test("a pair not read whole never says the whole text did not change", () => {
  const partial = pairStateText({ kind: "no_text_change" }, false, false);
  assert.equal(partial.label, "No change in the text Memory reads");
  assert.match(partial.detail ?? "", /hidden slides or sheets, formatting, tables, fields/);
  assert.equal(pairStateText({ kind: "no_text_change" }, false, true).label, "The text did not change");
  const inPart = pairStateText({ kind: "analyzed", complete: false, hunks: 2, left_out_hunks: 0, minor_hunks: [] });
  assert.match(inPart.detail ?? "", /does not read every part/);
});

test("the timeline lists a line's versions oldest first, numbered as the history numbers them", () => {
  const step = (before: number, after: number): VersionPairView => ({ ...pair({ kind: "queued" }), before: version(before), after: version(after) });
  const pairs = [step(1, 2), step(2, 3), step(3, 4)];
  assert.deepEqual(
    lineVersions(pairs).map((v) => v.snapshot_id),
    ["s1", "s2", "s3", "s4"],
  );
  // One saved version and no pair yet: that version alone (R2-MEM-01).
  assert.deepEqual(lineVersions([], version(1)).map((v) => v.snapshot_id), ["s1"]);
  assert.deepEqual(lineVersions([]), []);
  const numbers = lineNumbers(pairs);
  assert.match(timelineLabel(version(1), numbers), /^v1 · /);
  assert.match(timelineLabel(version(4), numbers), /^v4 · /);
  // A version not on the line is named by when it was saved only.
  assert.doesNotMatch(timelineLabel(version(9), numbers), /^v/);
  // The pair that made a version, and the one going on from it.
  assert.equal(pairsAround(pairs, version(1)).made, null);
  assert.equal(pairsAround(pairs, version(1)).next?.after.snapshot_id, "s2");
  assert.equal(pairsAround(pairs, version(3)).made?.before.snapshot_id, "s2");
  assert.equal(pairsAround(pairs, version(4)).next, null);
});

test("the timeline marks a stretch Traeva may have missed and a change not fully described", () => {
  const made = (state: PairState, gap = false): VersionPairView => ({ ...pair(state, gap), before: version(1), after: { ...version(2), sha256: "1".repeat(64) } });
  assert.deepEqual(stopNotes(null), { gap: false, unclear: false });
  assert.deepEqual(stopNotes(made({ kind: "analyzed", complete: true, hunks: 1, left_out_hunks: 0, minor_hunks: [] })), { gap: false, unclear: false });
  // The core's gap, never the page's own guess.
  assert.deepEqual(stopNotes(made({ kind: "queued" }, true)), { gap: true, unclear: false });
  // Not described in full: as pairStateText says it needs a look.
  for (const state of [
    { kind: "analyzed", complete: false, hunks: 3, left_out_hunks: 1, minor_hunks: [] },
    { kind: "unreadable", note: "No readable text." },
    { kind: "failed", error_code: "model_timeout", will_retry: false },
  ] as PairState[]) {
    assert.equal(stopNotes(made(state)).unclear, true, state.kind);
  }
  // Waiting, not permitted, or a try that comes again by itself is not a
  // mark: nothing is settled yet.
  assert.equal(stopNotes(made({ kind: "queued" })).unclear, false);
  assert.equal(stopNotes(made({ kind: "not_permitted" })).unclear, false);
  assert.equal(stopNotes(made({ kind: "failed", error_code: "model_timeout", will_retry: true })).unclear, false);
  assert.equal(stopNotes(made({ kind: "invalid_output", will_retry: true, reason: null })).unclear, false);
  // Only part of each version read, that part the same, the bytes not.
  assert.equal(stopNotes({ ...made({ kind: "no_text_change" }), complete: false }).unclear, true);
  // The same bytes: nothing changed at all.
  const same = { ...made({ kind: "no_text_change" }), complete: false };
  assert.equal(stopNotes({ ...same, after: version(2) }).unclear, false);
  const numbers = lineNumbers([made({ kind: "queued" })]);
  assert.match(stopLabel(version(2), numbers, { gap: true, unclear: true }), /^Version 2 · .+\. Traeva may have missed saves before this version\. Memory could not fully say what changed$/);
  assert.match(stopLabel(version(2), numbers, { gap: false, unclear: false }), /^Version 2 · [^.]+$/);
});

test("the not-permitted note does not say where the agreement is", () => {
  const text = pairStateText({ kind: "not_permitted" }, false, true, null);
  assert.match(text.detail ?? "", /Agree under Describing changes with the model/);
  assert.doesNotMatch(text.detail ?? "", /above|below/);
});
