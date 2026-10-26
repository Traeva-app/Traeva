// Tests for view.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  actionName,
  actionsToRead,
  actionWhen,
  activityLists,
  byDay,
  dayLabel,
  dropRows,
  folderNote,
  moveText,
  namesFor,
  placeText,
  rootNames,
  stillWorking,
} from "./view.ts";
import type { DropItemView, DropState } from "../autodrop/types.ts";
import type { ActionView, PlanEnd, RootView } from "../organize/types.ts";

const ROOT = "/Users/x/School";

function end(relative: string, root = ROOT, rootId = "r1"): PlanEnd {
  return { root_id: rootId, relative_path: relative, display_path: `${root}/${relative}` };
}

function action(id: string, extra: Partial<ActionView> = {}): ActionView {
  return {
    action_id: id,
    plan_id: `p-${id}`,
    kind: "organize",
    file_id: "f1",
    status: "committed",
    source: end("Inbox/notes.md"),
    target: end("History/Primary Sources/notes.md"),
    folders_created: [],
    folders_removed: [],
    folders_without_receipt: [],
    folders_kept: [],
    created_at_ms: 1000,
    committed_at_ms: 2000,
    recovered_at_ms: null,
    problem: null,
    undo_of_action_id: null,
    undo_status: "available",
    undo_action_id: null,
    ...extra,
  };
}

function drop(id: string, state: DropState, extra: Partial<DropItemView> = {}): DropItemView {
  return {
    drop_id: id,
    file_name: `${id}.md`,
    batch_id: "b1",
    received_at_ms: 0,
    state,
    automatic: true,
    folder_to_authorize: null,
    operation_id: null,
    plan_id: null,
    action_id: null,
    moved_to: null,
    reason: null,
    needs_user: state === "needs_answer" || state === "needs_source_authorization" || state === "handed_over",
    action_status: null,
    ...extra,
  };
}

function root(path: string, rootId: string): RootView {
  return { root_id: rootId, display_path: path, status: "active", granted_at_ms: 0, revoked_at_ms: null, problem: null };
}

test("a move shows its folders from the authorized folder's name, never more", () => {
  const names = namesFor([], [root(ROOT, "r1")]);
  assert.equal(placeText(end("Inbox/notes.md"), names), "School/Inbox");
  assert.equal(placeText(end("notes.md"), names), "School");
  assert.equal(moveText(action("a"), names), "School/Inbox → School/History/Primary Sources");
  // Into another authorized folder: each side names its own.
  const across = action("b", { target: end("Labs/notes.md", "/Users/x/Clubs", "r2") });
  assert.equal(moveText(across, namesFor([across], [])), "School/Inbox → Clubs/Labs");
  assert.equal(actionName(action("a")), "notes.md");
});

test("two authorized folders with the same name are told apart by the folders above them", () => {
  const names = rootNames(["/Users/x/Documents/School", "/Users/x/Desktop/School", "/Users/x/Clubs"]);
  assert.equal(names.get("/Users/x/Documents/School"), "Documents/School");
  assert.equal(names.get("/Users/x/Desktop/School"), "Desktop/School");
  assert.equal(names.get("/Users/x/Clubs"), "Clubs");
  // Nothing shorter tells them apart: the full path.
  const nested = rootNames(["/a/School", "/b/a/School"]);
  assert.equal(nested.get("/a/School"), "/a/School");
  assert.equal(nested.get("/b/a/School"), "b/a/School");
  // A folder Traeva has now counts even when no listed move used it, so a
  // move's name does not change when another folder's moves come and go.
  const across = action("c", { target: end("Labs/notes.md", "/Users/y/School", "r2") });
  const both = namesFor([across], [root(ROOT, "r1"), root("/Users/y/School", "r2")]);
  assert.equal(moveText(across, both), "x/School/Inbox → y/School/Labs");
  assert.equal(moveText(action("a"), namesFor([], [root(ROOT, "r1"), root("/Users/y/School", "r2")])), "x/School/Inbox → x/School/History/Primary Sources");
});

test("a folder removed from Traeva is named next to the status", () => {
  const roots = [root("/Users/x/Clubs", "r2")];
  const names = namesFor([action("a")], roots);
  // Its Undo was refused for that: the full reason is the tooltip.
  const revoked = folderNote(action("a", { undo_status: "root_revoked" }), roots, names);
  assert.equal(revoked?.short, "School removed from Traeva");
  assert.match(revoked?.full ?? "", /^Can't be undone: a folder it used was removed from Traeva\./);
  // Undone already, say: only that the folder was removed.
  assert.deepEqual(folderNote(action("b", { undo_status: "undone" }), roots, names), {
    short: "School removed from Traeva",
    full: "School removed from Traeva.",
  });
  // Both folders removed.
  const across = action("c", { target: end("Labs/notes.md", "/Users/x/Clubs", "r2") });
  assert.equal(folderNote(across, [], namesFor([across], []))?.short, "School and Clubs removed from Traeva");
  // Folders still there: nothing to say.
  assert.equal(folderNote(action("d"), [root(ROOT, "r1")], names), null);
});

test("an action's time is when it was done, else when it started", () => {
  const now = new Date(2026, 9, 13, 18, 0).getTime();
  const done = new Date(2026, 9, 13, 15, 4).getTime();
  assert.match(actionWhen(action("a", { created_at_ms: done - 1000, committed_at_ms: done }), now), /^today, /);
  const failed = action("b", { status: "failed_no_move", created_at_ms: new Date(2026, 9, 12, 9, 0).getTime(), committed_at_ms: null });
  assert.match(actionWhen(failed, now), /^yesterday, /);
});

test("dropped files: newest first, each worded as the drop queue words it", () => {
  const now = new Date(2026, 9, 13, 18, 0).getTime();
  const at = (minutes: number) => now - minutes * 60_000;
  const moved = action("m", { target: end("Labs/m.md") });
  const rows = dropRows(
    [
      drop("old", "not_processed", { received_at_ms: at(30), reason: "Not processed: an earlier file stopped." }),
      drop("moved", "moved", { received_at_ms: at(20), action_id: "m", action_status: "committed" }),
      drop("ask", "needs_answer", { received_at_ms: at(10), operation_id: "op" }),
      drop("handed", "handed_over", { received_at_ms: at(5), reason: "No clear folder." }),
      drop("gave-up", "handed_over", { received_at_ms: at(5), reason: "No clear folder." }),
    ],
    { "gave-up": { kind: "given_up" } },
    new Map([["m", moved]]),
    now,
  );
  // Same time: the later one in the queue first.
  assert.deepEqual(
    rows.map((row) => row.key),
    ["gave-up", "handed", "ask", "moved", "old"],
  );
  const byKey = new Map(rows.map((row) => [row.key, row]));
  assert.equal(byKey.get("moved")?.badge.label, "Organized");
  assert.equal(byKey.get("moved")?.action, moved);
  assert.equal(byKey.get("moved")?.open, null);
  assert.equal(byKey.get("ask")?.open, "Answer");
  assert.equal(byKey.get("handed")?.open, "Review");
  assert.equal(byKey.get("handed")?.reason, "No clear folder.");
  // Its usual flow ended here without a plan: it waits for nothing.
  assert.equal(byKey.get("gave-up")?.badge.label, "Not organized");
  assert.equal(byKey.get("gave-up")?.open, null);
  assert.equal(byKey.get("gave-up")?.reason, null);
  assert.equal(byKey.get("old")?.badge.label, "Not processed");
  assert.equal(byKey.get("old")?.reason, null);
  assert.match(byKey.get("old")?.dropped ?? "", /^dropped /);
});

test("a moved file whose action was not read shows the queue's status, without Undo", () => {
  const rows = dropRows(
    [drop("d", "moved", { action_id: "gone", action_status: "undone" })],
    {},
    new Map(),
    0,
  );
  assert.equal(rows[0].badge.label, "Undone");
  assert.equal(rows[0].action, null);
});

test("the file Traeva is working on has no ID or name yet", () => {
  const rows = dropRows([drop("", "working", { file_name: "" })], {}, new Map(), 0);
  assert.equal(rows[0].key, "working-0");
  assert.equal(rows[0].name, "(the file Traeva is working on)");
  assert.equal(rows[0].badge.label, "Working…");
});

test("only dropped files' actions missing from the recent ones are read on their own", () => {
  const items = [
    drop("a", "moved", { action_id: "known" }),
    drop("b", "moved", { action_id: "older" }),
    drop("c", "stopped", { action_id: "older" }),
    drop("d", "cancelled"),
  ];
  assert.deepEqual(actionsToRead(items, [action("known")]), ["older"]);
});

test("the page reads again only while something is still being worked on", () => {
  const queue = (items: DropItemView[], running = false) => ({ items, running });
  // Finished files, files that wait for the user, settled actions: no.
  const settled = queue([
    drop("a", "moved", { action_id: "a1", action_status: "committed" }),
    drop("b", "needs_answer"),
    drop("c", "handed_over"),
    drop("d", "cancelled"),
  ]);
  assert.equal(stillWorking(settled, [action("x"), action("y", { status: "needs_user_decision" })]), false);
  assert.equal(stillWorking(null, []), false);
  // A file waiting or being organized, a pass running, a move not settled.
  assert.equal(stillWorking(queue([drop("e", "waiting")]), []), true);
  assert.equal(stillWorking(queue([drop("", "working", { file_name: "" })]), []), true);
  assert.equal(stillWorking(queue([], true), []), true);
  assert.equal(stillWorking(queue([drop("f", "stopped", { action_id: "f1", action_status: "running" })]), []), true);
  assert.equal(stillWorking(null, [action("z", { status: "prepared", committed_at_ms: null })]), true);
  // Files waiting behind one that waits for the user: the core's pass has
  // stopped there, so nothing changes until the user acts.
  const held = queue([drop("q", "needs_answer", { needs_user: true }), drop("w", "waiting")]);
  assert.equal(stillWorking(held, []), false);
  assert.equal(stillWorking({ ...held, running: true }, []), true);
});

test("each file shows once: what waits, what is worked on, the moves, what ended", () => {
  const now = new Date(2026, 9, 14, 20, 0).getTime();
  const at = (minutes: number) => now - minutes * 60_000;
  const recent = [
    action("new", { committed_at_ms: at(5) }),
    action("decide", { status: "needs_user_decision", committed_at_ms: null, created_at_ms: at(10) }),
    action("dropped", { committed_at_ms: at(30) }),
    action("old", { committed_at_ms: at(60 * 30) }),
  ];
  const decisions = [recent[1]];
  const older = action("older-drop", { committed_at_ms: at(45) });
  const actions = new Map([...recent, older].map((one) => [one.action_id, one]));
  const rows = dropRows(
    [
      drop("ask", "needs_answer", { received_at_ms: at(2), operation_id: "op" }),
      drop("busy", "working", { received_at_ms: at(3) }),
      drop("settling", "stopped", { received_at_ms: at(4), action_id: "run", action_status: "running" }),
      drop("d1", "moved", { received_at_ms: at(31), action_id: "dropped", action_status: "committed" }),
      drop("d2", "moved", { received_at_ms: at(46), action_id: "older-drop", action_status: "committed" }),
      drop("d3", "moved", { received_at_ms: at(50), action_id: "unread", action_status: "committed" }),
      drop("there", "already_in_place", { received_at_ms: at(70) }),
      drop("stop", "stopped", { received_at_ms: at(80), reason: "No clear folder." }),
    ],
    {},
    actions,
    now,
  );
  const lists = activityLists(decisions, recent, rows);
  // A stop whose move is not settled waits for the user too (R3 on #290).
  assert.deepEqual(lists.waiting.map((row) => row.key), ["ask", "settling"]);
  assert.equal(lists.waiting[1].open, "Review");
  assert.equal(lists.waiting[1].badge.label, "Not settled");
  assert.deepEqual(lists.decisions.map((one) => one.action_id), ["decide"]);
  assert.deepEqual(lists.working.map((row) => row.key), ["busy"]);
  // Newest first; the decision is not among the moves; a dropped file's
  // move shows once, with the file; one whose action was not read too.
  assert.deepEqual(
    lists.moves.map((move) => move.key),
    ["new", "dropped", "drop-d2", "drop-d3", "old"],
  );
  assert.equal(lists.moves[1].drop?.key, "d1");
  assert.equal(lists.moves[2].action, older);
  assert.equal(lists.moves[3].action, null);
  assert.deepEqual(lists.ended.map((row) => row.key), ["there", "stop"]);
});

test("moves are cut by day: today, yesterday, earlier", () => {
  const now = new Date(2026, 9, 14, 9, 0).getTime();
  const day = (d: number, h: number) => new Date(2026, 9, d, h, 0).getTime();
  assert.equal(dayLabel(day(14, 1), now), "Today");
  assert.equal(dayLabel(day(13, 23), now), "Yesterday");
  assert.equal(dayLabel(day(12, 23), now), "Earlier");
  const move = (key: string, ms: number) => ({ key, at: ms, action: null, drop: null });
  const days = byDay([move("a", day(14, 8)), move("b", day(14, 2)), move("c", day(13, 9)), move("d", day(1, 9))], now);
  assert.deepEqual(
    days.map((one) => [one.label, one.moves.map((m) => m.key)]),
    [
      ["Today", ["a", "b"]],
      ["Yesterday", ["c"]],
      ["Earlier", ["d"]],
    ],
  );
});
