// Tests for flow.ts. Run with `npm test` (Node's own test runner; Node
// strips the TypeScript types, so no extra package is needed).

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  answerFor,
  basename,
  endedHint,
  evidenceText,
  hasText,
  inRoot,
  isCurrentPlace,
  manualReason,
  pickRoots,
  canRetry,
  candidateFolderName,
  candidatePath,
  cardFor,
  isBusy,
  isIdle,
  isNewer,
  isStill,
  likelyCandidate,
  offeredCandidates,
  recommendedCandidate,
  scoreText,
  singleCandidate,
  undoNote,
  undoOffered,
  rootPathOf,
  actionRoots,
  answerNote,
  currentFolderIn,
  chosenTargetPath,
  typedNewFolders,
  extractionText,
  flowStepIndex,
  ocrStateMayChange,
  ocrStateText,
} from "./flow.ts";
import type {
  ActionView,
  AppError,
  CandidateView,
  FileView,
  OperationStage,
  OperationView,
  PlanView,
  QuestionView,
  RootView,
  SuggestionView,
} from "./types.ts";

const STAGES: OperationStage[] = [
  "observing",
  "ready_for_analysis",
  "analyzing",
  "needs_clarification",
  "recommendation_ready",
  "needs_manual_location",
  "analysis_failed",
  "invalid_model_output",
  "waiting_for_file",
  "stale",
  "blocked",
  "cancelled",
];

function view(generation: number, operationId = "op-1"): OperationView {
  return {
    operation_id: operationId,
    generation,
    stage: "analyzing",
    source_root_id: "root",
    target_root_id: "root",
    file: null,
    error: null,
    question: null,
    answer: null,
    answered: [],
    suggestion: null,
    model_requests: 0,
    plan_id: null,
    action_id: null,
  };
}

function error(allowedNext: AppError["allowed_next"]): AppError {
  return {
    code: "model_timeout",
    message: "No answer within 60 seconds.",
    stage: "analysis",
    retryable: allowedNext.includes("try_again"),
    file_id: null,
    plan_id: null,
    action_id: null,
    allowed_next: allowedNext,
  };
}

const travel: CandidateView = {
  candidate_id: "C1",
  folder_path: "Work/Travel",
  new_segments: [],
  reason: "A work trip.",
  evidence_ids: ["E1"],
};

const atTheRoot: CandidateView = {
  candidate_id: "C2",
  folder_path: "",
  new_segments: ["Trips", "2026"],
  reason: "A trip.",
  evidence_ids: [],
};

function suggestion(recommendedId: string | null): SuggestionView {
  return {
    request_id: "req-1",
    summary: "A trip.",
    confidence: 0.9,
    recommended_id: recommendedId,
    candidates: [travel, atTheRoot],
  };
}

const question: QuestionView = {
  question_id: "q-1",
  uncertain_field: "purpose",
  question: "Is this for work or for a holiday?",
  options: [
    { option_id: "O1", text: "Work" },
    { option_id: "O2", text: "Holiday" },
  ],
  number: 1,
  max_questions: 3,
};

test("only reading and analyzing keep the window polling", () => {
  const busy = STAGES.filter(isBusy);
  assert.deepEqual(busy, ["observing", "analyzing"]);
});

test("Undo and Check again wait while a file is on its way or a preview is shown", () => {
  const steps = ["start", "operation", "preview", "applying", "done", "in_place", "failed"] as const;
  assert.deepEqual(steps.filter(isIdle), ["start", "done", "in_place", "failed"]);
});

test("a result read again is still the flow an Undo or a check started from (#168)", () => {
  const action = { action_id: "act-1", status: "committed" } as ActionView;
  const shown = { step: "done", action } as const;
  assert.equal(isStill(shown, shown), true);
  // Read again in the background: a new object for the same action.
  assert.equal(isStill(shown, { step: "done", action: { ...action, undo_status: "available" } }), true);
  assert.equal(isStill(shown, { step: "done", action: { ...action, action_id: "act-2" } }), false);
  assert.equal(isStill(shown, { step: "start" }), false);
  // Any other step counts only as the very same object.
  assert.equal(isStill({ step: "start" }, { step: "start" }), false);
});

test("every stage has a card, and every final stage ends the flow", () => {
  for (const stage of STAGES) assert.ok(cardFor(stage));
  assert.equal(cardFor("ready_for_analysis"), "ready");
  assert.equal(cardFor("needs_clarification"), "question");
  assert.equal(cardFor("recommendation_ready"), "recommendation");
  assert.equal(cardFor("needs_manual_location"), "manual");
  assert.equal(cardFor("invalid_model_output"), "analysis_failed");
  for (const stage of ["waiting_for_file", "stale", "blocked", "cancelled"] as const) {
    assert.equal(cardFor(stage), "ended");
  }
});

test("an older or foreign view never replaces the shown one", () => {
  assert.equal(isNewer(view(3), view(2)), true);
  assert.equal(isNewer(view(2), view(2)), true);
  assert.equal(isNewer(view(1), view(2)), false);
  assert.equal(isNewer(view(9, "op-2"), view(2)), false);
});

test("a retry is offered only when the core's error says so", () => {
  assert.equal(canRetry(null), false);
  assert.equal(canRetry(error(["try_again", "choose_folder"])), true);
  assert.equal(canRetry(error(["choose_folder"])), false);
});

test("a candidate's place is its folder, its new folders and the unchanged name", () => {
  const file = { relative_path: "Inbox/march.txt" } as FileView;
  assert.equal(basename(file), "march.txt");
  assert.equal(candidatePath(travel, "march.txt"), "Work/Travel/march.txt");
  assert.equal(candidatePath(atTheRoot, "march.txt"), "Trips/2026/march.txt");
});

test("only a recommendation has a recommended candidate; offers have none", () => {
  assert.equal(recommendedCandidate(suggestion("C1")), travel);
  assert.equal(recommendedCandidate(suggestion(null)), null);
  assert.equal(recommendedCandidate(null), null);
  // In a manual choice the candidates are offered, none preselected.
  assert.deepEqual(offeredCandidates(suggestion(null)), [travel, atTheRoot]);
  assert.deepEqual(offeredCandidates(suggestion("C1")), []);
});

test("one place named and none recommended: that place is the likely folder", () => {
  const one = (candidates: CandidateView[], recommendedId: string | null = null): SuggestionView => ({
    ...suggestion(recommendedId),
    candidates,
  });
  // Only one: the card shows it first (Probably …, Use …).
  assert.equal(singleCandidate(one([travel])), travel);
  // None, two or three: the full card, as before.
  assert.equal(singleCandidate(one([])), null);
  assert.equal(singleCandidate(one([travel, atTheRoot])), null);
  assert.equal(singleCandidate(null), null);
  // A recommendation is not this card (it has its own).
  assert.equal(singleCandidate(one([travel], "C1")), null);
  // The folder's name: its last folder, a new one included; the target folder itself by name.
  assert.equal(candidateFolderName(travel, "School"), "Travel");
  assert.equal(candidateFolderName(atTheRoot, "School"), "2026");
  assert.equal(candidateFolderName({ ...travel, folder_path: "" }, "School"), "School");
});

test("the likely place is not shown when the user passed it by at an open question", () => {
  const oneOffered: SuggestionView = { ...suggestion(null), candidates: [travel] };
  const manual = (extra: Partial<OperationView>): OperationView => ({
    ...view(3),
    stage: "needs_manual_location",
    suggestion: oneOffered,
    ...extra,
  });
  const second: QuestionView = { ...question, question_id: "q-2", number: 2 };
  const answeredFirst = { question_id: "q-1", number: 1, question: question.question, answer: "Work" };
  // No question: the merged card.
  assert.equal(likelyCandidate(manual({})), travel);
  // "None of these" at the first question: that round's place is kept on offer, but the full card shows.
  assert.equal(likelyCandidate(manual({ question })), null);
  // At the second question, with the first one answered: still open, still the full card.
  assert.equal(likelyCandidate(manual({ question: second, answer: answeredFirst.answer, answered: [answeredFirst] })), null);
  // Every question answered, and the core turned to picking by hand: the merged card.
  assert.equal(likelyCandidate(manual({ question, answer: "Work", answered: [answeredFirst] })), travel);
});

test("an answer is one option or the user's own words, never both", () => {
  assert.deepEqual(answerFor(question, "O1", "ignored"), { question_id: "q-1", option_id: "O1" });
  assert.deepEqual(answerFor(question, null, "For a client visit"), {
    question_id: "q-1",
    text: "For a client visit",
  });
  assert.equal(answerFor(question, null, "   "), null);
});

test("a confidence is shown as a score, or a dash without one", () => {
  assert.equal(scoreText(0.8), "80%");
  assert.equal(scoreText(null), "—");
});

function root(id: string, grantedAt: number): RootView {
  return { root_id: id, granted_at_ms: grantedAt } as RootView;
}

test("the next file's folders: the user's choice while valid, else the newest folder", () => {
  const roots = [root("old", 1), root("new", 2)];
  assert.deepEqual(pickRoots(roots, null), { source: "new", target: "new" });
  assert.deepEqual(pickRoots(roots, { source: "old", target: "new" }), {
    source: "old",
    target: "new",
  });
  // A removed folder is never still chosen.
  assert.deepEqual(pickRoots([root("new", 2)], { source: "old", target: "new" }), {
    source: "new",
    target: "new",
  });
  assert.equal(pickRoots([], null), null);
});

test("a path inside a folder is written out in full", () => {
  assert.equal(inRoot("/Users/u/Demo", "Work/Travel/a.md"), "/Users/u/Demo/Work/Travel/a.md");
  assert.equal(inRoot("/Users/u/Demo", ""), "/Users/u/Demo");
});

function withText(usable: boolean): FileView {
  return {
    relative_path: "Inbox/a.txt",
    extraction: {
      status: "ok",
      total_chars: 10,
      truncated: false,
      blank: !usable,
      usable_text: usable,
      note: null,
      recognized: false,
    },
  } as FileView;
}

test("a manual choice says why there is no recommendation", () => {
  const manual = { ...view(4), stage: "needs_manual_location" as const, file: withText(true) };
  // The user chose to pick the folder: nothing to explain.
  assert.equal(manualReason(manual), null);
  assert.equal(
    manualReason({ ...manual, suggestion: { ...suggestion(null), candidates: [] } }),
    "The model did not suggest a folder for this file.",
  );
  assert.equal(
    manualReason({ ...manual, suggestion: suggestion(null) }),
    "Traeva did not find one folder the text clearly supports, so none is recommended.",
  );
  assert.equal(
    manualReason({ ...manual, file: withText(false) }),
    "This file has no text the model may read, so the model was not asked.",
  );
  assert.equal(
    manualReason({ ...manual, error: error(["choose_folder"]) }),
    "No suggestion: No answer within 60 seconds.",
  );
  assert.equal(hasText(withText(true)), true);
  assert.equal(hasText(withText(false)), false);
});

test("a file outside the chosen folder points to “File from”", () => {
  const outside = { ...error(["choose_file"]), code: "outside_root" as const };
  const blocked = { ...view(2), stage: "blocked" as const, error: outside };
  assert.match(endedHint(blocked) ?? "", /File from/);
  assert.match(endedHint(blocked) ?? "", /in Settings first/);
  assert.equal(endedHint({ ...blocked, error: error(["choose_file"]) }), null);
});

test("a reason's evidence is counted, not shown by its internal ID", () => {
  assert.equal(evidenceText([]), "no part of the text");
  assert.equal(evidenceText(["E1"]), "1 part of the text");
  assert.equal(evidenceText(["E1", "E3"]), "2 parts of the text");
});

test("a candidate that is the file's current folder is pointed out", () => {
  const file = { root_id: "root", relative_path: "Work/Travel/march.txt" } as FileView;
  assert.equal(isCurrentPlace(travel, file, "root"), true);
  assert.equal(isCurrentPlace(travel, file, "other-root"), false);
  assert.equal(isCurrentPlace(atTheRoot, file, "root"), false);
});

test("Undo is offered only when the core says it is available", () => {
  assert.equal(undoOffered("available"), true);
  for (const status of ["in_progress", "undone", "not_available", "root_revoked"] as const) {
    assert.equal(undoOffered(status), false, status);
  }
});

test("only a removed folder's actions say why there is no Undo (#79)", () => {
  const note = undoNote("root_revoked");
  assert.equal(note?.short, "Folder removed");
  assert.match(note?.full ?? "", /removed from Traeva.*does not bring Undo back/);
  for (const status of ["available", "in_progress", "undone", "not_available"] as const) {
    assert.equal(undoNote(status), null, status);
  }
});

test("an action names its authorized folders and marks removed ones (#80)", () => {
  const end = (rootId: string, rootPath: string, relative: string) => ({
    root_id: rootId,
    relative_path: relative,
    display_path: `${rootPath}/${relative}`,
  });
  assert.equal(rootPathOf(end("a", "/Users/x/School", "Notes/todo.txt")), "/Users/x/School");
  const roots = [{ root_id: "a" }, { root_id: "b" }] as RootView[];
  const within = {
    source: end("a", "/Users/x/School", "todo.txt"),
    target: end("a", "/Users/x/School", "Taxes/todo.txt"),
  } as ActionView;
  assert.deepEqual(actionRoots(within, roots), [{ path: "/Users/x/School", removed: false }]);
  const across = { ...within, target: end("c", "/Users/x/Old", "todo.txt") } as ActionView;
  assert.deepEqual(actionRoots(across, roots), [
    { path: "/Users/x/School", removed: false },
    { path: "/Users/x/Old", removed: true },
  ]);
  // The same file name under two folders is told apart by the folder.
  const twin = { ...within, source: end("b", "/Users/x/Club", "todo.txt"), target: end("b", "/Users/x/Club", "Taxes/todo.txt") } as ActionView;
  assert.notDeepEqual(actionRoots(twin, roots), actionRoots(within, roots));
});

test("the preview mentions the answers only for the model's suggestion (#80)", () => {
  const one = ["For the book club"];
  assert.match(answerNote(one, "accept") ?? "", /your answer into.*For the book club/);
  // Up to three answers, oldest first (organize-spec §15).
  assert.equal(
    answerNote(["Travel", "2026"], "accept"),
    "The recommended folder took your answers into account: “Travel”, “2026”.",
  );
  // "correct" is a folder picked by hand while a recommendation was shown.
  assert.equal(answerNote(one, "correct"), null, "picked by hand");
  assert.equal(answerNote(one, "manual"), null, "picked by hand");
  assert.equal(answerNote([], "accept"), null, "no question was answered");
  assert.equal(answerNote(one, null), null, "an undo plan");
});

test("a folder picked by hand shows the full target, and the file's own folder is known (#80)", () => {
  const file = { root_id: "root", relative_path: "Inbox/march.txt" } as FileView;
  assert.equal(currentFolderIn(file, "root"), "Inbox");
  assert.equal(currentFolderIn({ ...file, relative_path: "march.txt" }, "root"), "");
  assert.equal(currentFolderIn(file, "other"), null);
  assert.equal(chosenTargetPath("/R", "School", ["Kyoto"], "march.txt"), "/R/School/Kyoto/march.txt");
  assert.equal(chosenTargetPath("/R", "", [], "march.txt"), "/R/march.txt");
});

test("new folder names are sent and shown as typed, never trimmed (#93)", () => {
  // R's case: the core must see the spaces to refuse the names (spec §4.2).
  const typed = typedNewFolders(["  Amber  ", " Winter", ""]);
  assert.deepEqual(typed, ["  Amber  ", " Winter"]);
  assert.equal(
    chosenTargetPath("/R", "Archive", typed, "march.txt"),
    "/R/Archive/  Amber  / Winter/march.txt",
  );
  assert.deepEqual(typedNewFolders(["2026 taxes", "Q1", ""]), ["2026 taxes", "Q1"], "inner spaces kept");
  assert.deepEqual(typedNewFolders(["", "", ""]), [], "no new folders");
  assert.deepEqual(typedNewFolders(["   ", "", ""]), ["   "], "only spaces is a name the core refuses");
  assert.deepEqual(typedNewFolders(["", "Winter", ""]), ["", "Winter"], "a gap is not closed");
  assert.equal(chosenTargetPath("/R", "Archive", ["", "Winter"], "march.txt"), "/R/Archive//Winter/march.txt");
});

test("what was read says so, and recognized text says it was recognized", () => {
  const read = (extra: Partial<FileView["extraction"]>) => ({
    status: "ok" as const,
    total_chars: 1234,
    truncated: false,
    blank: false,
    usable_text: true,
    note: null,
    recognized: false,
    ...extra,
  });
  assert.equal(extractionText(read({})), "whole text (1,234 characters)");
  assert.equal(
    extractionText(read({ status: "partial", truncated: true, total_chars: 20000 })),
    "only the first 6,000 and last 2,000 of 20,000 characters; the middle was not read",
  );
  assert.equal(extractionText(read({ status: "partial" })), "part of the file: 1,234 characters of text");
  assert.equal(
    extractionText(read({ status: "partial", recognized: true })),
    "text recognized from scanned pages: 1,234 characters",
  );
  assert.equal(
    extractionText(read({ status: "partial", recognized: true, truncated: true, total_chars: 20000 })),
    "text recognized from scanned pages: only the first 6,000 and last 2,000 of 20,000 characters; the middle was not read",
  );
  // An image (JPG, PNG): its recognized text says it came from the image.
  assert.equal(
    extractionText(read({ status: "partial", recognized: true }), true),
    "text recognized from the image: 1,234 characters",
  );
  assert.equal(extractionText(read({}), true), "whole text (1,234 characters)", "only recognized text says where it came from");
  assert.equal(extractionText(read({ blank: true, total_chars: 0 })), "blank: no text to read");
  assert.equal(extractionText(read({ status: "unsupported", usable_text: false })), "unsupported: no text the model may read");
});

test("the text recognition line shows only when recognition is not off", () => {
  assert.equal(ocrStateText("off"), null);
  assert.equal(
    ocrStateText("preparing"),
    "Text recognition for scanned PDFs and images (JPG, PNG) is getting ready (the first time can take about a minute).",
  );
  assert.equal(ocrStateText("ready"), "Text recognition for scanned PDFs and images (JPG, PNG) is ready.");
  assert.equal(
    ocrStateText("failed"),
    "Text recognition for scanned PDFs and images (JPG, PNG) stopped working; it is off until Traeva starts again.",
  );
  assert.equal(
    ocrStateText("timed_out"),
    "Text recognition for scanned PDFs and images (JPG, PNG) could not get ready; it is off until Traeva starts again.",
  );
});

test("the recognition line is read again while the state can still change (R4-OCR-06)", () => {
  for (const state of ["not_prepared", "preparing", "ready"] as const) {
    assert.equal(ocrStateMayChange(state), true, state);
  }
  for (const state of ["off", "failed", "timed_out", "cancelled"] as const) {
    assert.equal(ocrStateMayChange(state), false, state);
  }
});

test("the step bar follows the flow's step and the operation's stage only", () => {
  assert.equal(flowStepIndex({ step: "start" }), 0);
  const at = (stage: OperationStage) => flowStepIndex({ step: "operation", operation: { ...view(1), stage } });
  assert.equal(at("observing"), 0);
  for (const stage of [
    "ready_for_analysis",
    "analyzing",
    "needs_clarification",
    "recommendation_ready",
    "needs_manual_location",
    "analysis_failed",
  ] as const) {
    assert.equal(at(stage), 1, stage);
  }
  assert.equal(at("cancelled"), null);
  const plan = (kind: "organize" | "undo") => ({ kind }) as PlanView;
  assert.equal(flowStepIndex({ step: "preview", plan: plan("organize"), context: null }), 2);
  assert.equal(flowStepIndex({ step: "preview", plan: plan("undo"), context: null }), null);
  assert.equal(flowStepIndex({ step: "applying", plan: plan("organize"), action: null }), 3);
  assert.equal(flowStepIndex({ step: "done", action: { kind: "organize" } as ActionView }), 3);
  assert.equal(flowStepIndex({ step: "done", action: { kind: "undo" } as ActionView }), null);
  assert.equal(flowStepIndex({ step: "failed", title: "x", error: null, detail: "x" }), null);
});
