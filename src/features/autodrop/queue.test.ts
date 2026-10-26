// Tests for queue.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  answersShownQuestion,
  FAST_POLL_MS,
  SLOW_POLL_MS,
  modeStatus,
  pollInterval,
  queueSignature,
  scopeChoiceProblem,
  scopeLabel,
  stateText,
  toneClass,
  actionText,
  dropBadge,
  finishedIds,
  hasOpenAfter,
  holdsRow,
  itemToShow,
  needsYouCount,
  dropPlace,
  unsettledStop,
  validRootId,
  waitsForUser,
} from "./queue.ts";
import type { AutoDropSettingsView, DropItemView, DropQueueView, DropState } from "./types.ts";

function item(state: DropState, extra: Partial<DropItemView> = {}): DropItemView {
  return {
    drop_id: `drop-${state}`,
    batch_id: "batch-1",
    received_at_ms: 0,
    file_name: "notes.md",
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

function queue(states: DropState[], running = false): DropQueueView {
  return { items: states.map((state) => item(state)), running };
}

function settings(extra: Partial<AutoDropSettingsView> = {}): AutoDropSettingsView {
  return {
    enabled: false,
    notice: "notice",
    notice_version: 1,
    consent_id: null,
    agreed_at_ms: null,
    target_root_id: null,
    scopes: [],
    problem: null,
    ...extra,
  };
}

test("only a moved file reads as organized; a stop is never a success", () => {
  assert.equal(stateText("moved").label, "Organized");
  assert.equal(stateText("moved").tone, "ok");
  for (const state of ["stopped", "handed_over", "needs_answer", "needs_source_authorization"] as const) {
    assert.notEqual(stateText(state).tone, "ok", state);
    assert.notEqual(stateText(state).label, "Organized", state);
  }
  for (const state of ["already_in_place", "not_processed", "cancelled", "waiting", "stopped"] as const) {
    assert.equal(stateText(state).tone, "neutral", state);
  }
  // Amber only while a file waits for the user (#37, 10/14).
  for (const state of ["needs_answer", "needs_source_authorization", "handed_over"] as const) {
    assert.equal(stateText(state).tone, "warn", state);
  }
  assert.equal(toneClass("ok"), "card-ok");
  assert.equal(toneClass("warn"), "badge-warn");
  assert.equal(toneClass("neutral"), "");
});

test("the queue is read every second while a file waits or is worked on", () => {
  assert.equal(pollInterval(null), FAST_POLL_MS);
  for (const state of ["waiting", "working", "needs_source_authorization", "needs_answer"] as const) {
    assert.equal(pollInterval(queue([state])), FAST_POLL_MS, state);
  }
  assert.equal(pollInterval(queue(["moved"], true)), FAST_POLL_MS);
  assert.equal(pollInterval(queue(["moved", "stopped", "not_processed"])), SLOW_POLL_MS);
  assert.equal(pollInterval(queue([])), SLOW_POLL_MS);
});

test("the signature changes with a state or an action", () => {
  const a = queue(["working"]);
  const b = queue(["moved"]);
  assert.notEqual(queueSignature(a), queueSignature(b));
  const withAction = { items: [item("moved", { action_id: "act-1" })], running: false };
  assert.notEqual(queueSignature(b), queueSignature(withAction));
  assert.equal(queueSignature(queue(["moved"])), queueSignature(b));
  assert.equal(queueSignature(null), "");
});

test("the mode is on only when the core says the agreement counts", () => {
  assert.equal(modeStatus(settings({ enabled: true, consent_id: "c1" })), "on");
  assert.equal(modeStatus(settings({ enabled: false, consent_id: "c1", problem: "gone" })), "not_working");
  assert.equal(modeStatus(settings()), "off");
});

test("a range has 1 to 20 folders", () => {
  assert.equal(scopeChoiceProblem(0), "Choose at least one folder.");
  assert.equal(scopeChoiceProblem(1), null);
  assert.equal(scopeChoiceProblem(20), null);
  assert.equal(scopeChoiceProblem(21), "Choose at most 20 folders.");
});

test("the folder itself is named after the target folder", () => {
  assert.equal(scopeLabel("", "/Users/me/Documents/School"), "School (the folder itself)");
  assert.equal(scopeLabel("BIO 201/Week 3", "/Users/me/Documents/School"), "BIO 201/Week 3");
});

test("a saved action is worded as in Organize; only a committed move is a success", () => {
  assert.deepEqual(actionText("committed", "organize"), { label: "Organized", tone: "ok" });
  assert.deepEqual(actionText("committed", "undo"), { label: "Put back", tone: "ok" });
  assert.equal(actionText("undone", "organize").label, "Undone");
  assert.equal(actionText("failed_no_move", "organize").tone, "neutral");
  assert.equal(actionText("needs_user_decision", "organize").tone, "warn");
  assert.equal(actionText("running", "organize").tone, "neutral");
});

test("cancelling warns while a file dropped with it, after it, is not finished", () => {
  const items = [item("handed_over"), item("waiting")];
  assert.equal(hasOpenAfter(items, 0), true);
  assert.equal(hasOpenAfter(items, 1), false);
  assert.equal(hasOpenAfter([item("handed_over"), item("needs_answer")], 0), true);
  assert.equal(hasOpenAfter([item("handed_over"), item("not_processed"), item("moved")], 0), false);
  // Another drop is not reached by this file's cancel (contract v29).
  assert.equal(hasOpenAfter([item("handed_over"), item("waiting", { batch_id: "batch-2" })], 0), false);
});

test("the queue waits for the user only where the core does", () => {
  assert.equal(waitsForUser([item("needs_answer"), item("waiting")], {}), true);
  assert.equal(waitsForUser([item("needs_source_authorization")], {}), true);
  // With the automatic mode off, a handed-over file waits for Organize until it ends there.
  const manual = item("handed_over", { automatic: false });
  assert.equal(waitsForUser([manual, item("waiting")], {}), true);
  assert.equal(waitsForUser([manual], { [manual.drop_id]: { kind: "given_up" } }), false);
  assert.equal(waitsForUser([item("handed_over", { automatic: false, action_id: "a1" })], {}), false);
  // A file the automatic mode handed over does not hold up later drops (contract v29).
  assert.equal(waitsForUser([item("handed_over"), item("waiting", { batch_id: "batch-2" })], {}), false);
});

test("the sidebar counts the dropped files that wait for the user", () => {
  assert.equal(needsYouCount([], {}), 0);
  const items = [
    item("needs_answer", { drop_id: "a" }),
    item("needs_source_authorization", { drop_id: "b" }),
    // Handed over by either mode and not finished in Organize.
    item("handed_over", { drop_id: "c" }),
    item("handed_over", { drop_id: "d", automatic: false }),
    // Confirmed in the usual flow, or ended there: not counted.
    item("handed_over", { drop_id: "e", action_id: "act-1" }),
    item("handed_over", { drop_id: "f" }),
    // Its flow was cancelled when the window left it: the core says so.
    item("handed_over", { drop_id: "g", operation_id: "op-1", needs_user: false }),
  ];
  assert.equal(needsYouCount(items, { f: { kind: "in_place" } }), 4);
  for (const state of ["waiting", "working", "moved", "already_in_place", "stopped", "not_processed", "cancelled"] as const) {
    assert.equal(needsYouCount([item(state)], {}), 0, state);
  }
});

test("a new file, or one that comes to need the user, is shown", () => {
  const a = item("moved", { drop_id: "a" });
  const b = item("waiting", { drop_id: "b" });
  const seen = new Map([["a", "moved" as const]]);
  assert.equal(itemToShow([a, b], seen), "b");
  const seen2 = new Map<string, DropState>([["a", "moved"], ["b", "working"]]);
  assert.equal(itemToShow([a, item("needs_answer", { drop_id: "b" })], seen2), "b");
  assert.equal(itemToShow([a, item("working", { drop_id: "b" })], seen2), null);
  const seen3 = new Map<string, DropState>([["a", "moved"], ["b", "needs_answer"]]);
  assert.equal(itemToShow([a, item("needs_answer", { drop_id: "b" })], seen3), null);
});

test("a removed folder falls back to the first one still offered", () => {
  assert.equal(validRootId("b", ["a", "b"]), "b");
  assert.equal(validRootId("gone", ["a", "b"]), "a");
  assert.equal(validRootId("", []), "");
});

test("a dropped file's badge: its action first, then this window's outcome, then its state", () => {
  const moved = item("moved", { action_id: "a1", action_status: "committed" });
  assert.deepEqual(dropBadge(moved, { status: "committed", kind: "organize" }, null), {
    label: "Organized",
    tone: "ok",
  });
  // Undone later: the action as saved now, not the queue's state.
  assert.equal(dropBadge(moved, { status: "undone", kind: "organize" }, null).label, "Undone");
  // Stopped while its move was not settled: the file may have moved.
  const stopped = item("stopped", { action_id: "a2" });
  assert.deepEqual(dropBadge(stopped, { status: "running", kind: "organize" }, null), {
    label: "Not settled",
    tone: "warn",
  });
  assert.equal(dropBadge(item("handed_over"), null, { kind: "given_up" }).label, "Not organized");
  assert.equal(dropBadge(item("handed_over"), null, { kind: "in_place" }).label, "Already there");
  assert.equal(dropBadge(item("handed_over"), null, null).label, "Needs you");
  assert.equal(dropBadge(item("not_processed"), null, null).label, "Not processed");
});

test("only finished files that wait for nothing can be cleared from the card", () => {
  const none = new Set<string>();
  const all: DropState[] = [
    "waiting",
    "working",
    "needs_source_authorization",
    "needs_answer",
    "handed_over",
    "moved",
    "already_in_place",
    "stopped",
    "not_processed",
    "cancelled",
  ];
  const items = all.map((state) => item(state));
  // Waiting, working, a folder to allow, a question and an open usual flow stay.
  assert.deepEqual(finishedIds(items, {}, none, null), [
    "drop-moved",
    "drop-already_in_place",
    "drop-stopped",
    "drop-not_processed",
    "drop-cancelled",
  ]);
  // A handed-over file whose usual flow ended (in this window, or as the core says).
  assert.deepEqual(finishedIds([item("handed_over")], { "drop-handed_over": { kind: "given_up" } }, none, null), [
    "drop-handed_over",
  ]);
  assert.deepEqual(
    finishedIds([item("handed_over", { action_id: "a1", action_status: "committed" })], {}, none, null),
    ["drop-handed_over"],
  );
  // Its move confirmed in the usual flow but not settled yet; or one that needs a decision.
  assert.deepEqual(finishedIds([item("moved", { action_id: "a1", action_status: "running" })], {}, none, null), []);
  assert.deepEqual(
    finishedIds([item("stopped", { action_id: "a1", action_status: "needs_user_decision" })], {}, none, null),
    [],
  );
  // Undone or not moved: finished.
  assert.deepEqual(finishedIds([item("moved", { action_id: "a1", action_status: "undone" })], {}, none, null), [
    "drop-moved",
  ]);
  // A row still reading its result, or whose move it found unsettled.
  assert.deepEqual(finishedIds([item("stopped")], {}, new Set(["drop-stopped"]), null), []);
  // The file open in the Organize flow, and the file being worked on (no ID yet).
  assert.deepEqual(finishedIds([item("cancelled")], {}, none, "drop-cancelled"), []);
  assert.deepEqual(finishedIds([item("moved", { drop_id: "" })], {}, none, null), []);
});

test("a row holds while it reads its result or its move is not settled", () => {
  assert.equal(holdsRow(true, null), true);
  assert.equal(holdsRow(false, null), false);
  assert.equal(holdsRow(false, { status: "committed" }), false);
  assert.equal(holdsRow(false, { status: "failed_no_move" }), false);
  assert.equal(holdsRow(false, { status: "prepared" }), true);
  assert.equal(holdsRow(false, { status: "running" }), true);
  assert.equal(holdsRow(false, { status: "needs_user_decision" }), true);
});

test("an answer counts only for the question it answered; the next one is asked again", () => {
  const sent = { questionId: "q1", state: "sent" as const };
  assert.equal(answersShownQuestion(sent, "q1"), true, "the answered question: Answer sent");
  // Up to 3 questions per file (organize-spec §15): question 2 is asked again.
  assert.equal(answersShownQuestion(sent, "q2"), false);
  assert.equal(answersShownQuestion(null, "q1"), false, "nothing sent from this card");
  assert.equal(answersShownQuestion(sent, null), false, "the question is not read yet");
});

test("a stop whose move is not settled waits for the user; the groups follow the queue only", () => {
  // As the core reports it (auto_drop_tests.rs: a move stopped before its
  // result was recorded): stopped, the action running, needs_user false.
  for (const status of ["running", "prepared"] as const) {
    const stopped = item("stopped", { action_id: "a1", action_status: status, needs_user: false });
    assert.equal(unsettledStop(stopped), true, status);
    assert.equal(dropPlace(stopped, {}), "needs", status);
    assert.equal(needsYouCount([stopped], {}), 1, status);
  }
  for (const status of ["committed", "failed_no_move", "undone"] as const) {
    const settled = item("stopped", { action_id: "a1", action_status: status, needs_user: false });
    assert.equal(unsettledStop(settled), false, status);
    assert.equal(dropPlace(settled, {}), "done", status);
  }
  assert.equal(dropPlace(item("stopped"), {}), "done");
  assert.equal(dropPlace(item("needs_answer"), {}), "needs");
  assert.equal(dropPlace(item("waiting"), {}), "working");
  assert.equal(dropPlace(item("working"), {}), "working");
  // A move going on that did not stop: In progress.
  assert.equal(dropPlace(item("moved", { action_id: "a1", action_status: "running" }), {}), "working");
  assert.equal(dropPlace(item("moved", { action_id: "a1", action_status: "committed" }), {}), "done");
  // A handed-over file whose flow ended here: Finished.
  const ended = item("handed_over", { drop_id: "h" });
  assert.equal(dropPlace(ended, { h: { kind: "given_up" } }), "done");
});
