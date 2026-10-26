// What the Activity page shows (#248): Traeva's recent moves and Undos, the
// actions that need a decision, and the files dropped since Traeva started.
// Text and order only: every status, and whether Undo is offered, comes from
// the core's views (get_app_state, get_drop_queue, get_action).

import { dropBadge, droppedAtText, needsYou, unsettledStop, type DropOutcome, type Tone } from "../autodrop/queue.ts";
import type { DropItemView, DropQueueView, DropState } from "../autodrop/types.ts";
import { lastSegment, parentPath, whenText } from "../find/view.ts";
import { actionRoots, rootPathOf, undoNote } from "../organize/flow.ts";
import type { ActionStatus, ActionView, PlanEnd, RootView } from "../organize/types.ts";

/** get_app_state lists at most this many recent actions (AppState). */
export const RECENT_LIMIT = 50;
/** The drop queue keeps about this many files; older finished ones go
 * (MAX_QUEUE_ITEMS in auto_drop.rs). */
export const DROP_LIMIT = 50;

/** The file's name; a move never changes it. */
export function actionName(action: ActionView): string {
  return lastSegment(action.target.relative_path);
}

/** How the page names authorized folders, by their full paths: the last
 * folder's name, with as many folders above it as it takes to tell it from
 * the others ("Documents/School" and "Desktop/School"); the full path when
 * nothing shorter does. */
export function rootNames(paths: string[]): Map<string, string> {
  const segments = new Map(paths.map((path) => [path, path.split("/").filter(Boolean)]));
  const names = new Map<string, string>();
  for (const [path, parts] of segments) {
    for (let count = 1; ; count++) {
      const name = parts.slice(-count).join("/");
      const shared = [...segments].some(
        ([other, otherParts]) => other !== path && otherParts.slice(-count).join("/") === name,
      );
      if (!shared) {
        names.set(path, name);
        break;
      }
      if (count >= parts.length) {
        names.set(path, path);
        break;
      }
    }
  }
  return names;
}

/** The authorized folders a list of actions uses and the ones Traeva has
 * now, named so that none two look alike (rootNames). */
export function namesFor(actions: ActionView[], roots: RootView[]): Map<string, string> {
  const paths = [
    ...roots.map((root) => root.display_path),
    ...actions.flatMap((action) => [rootPathOf(action.source), rootPathOf(action.target)]),
  ];
  return rootNames([...new Set(paths)]);
}

/** The folder a plan end lies in, starting at its authorized folder's name
 * (`names`, from namesFor): "School/History" for
 * /Users/x/School/History/notes.md. */
export function placeText(end: PlanEnd, names: Map<string, string>): string {
  const rootPath = rootPathOf(end);
  const root = names.get(rootPath) ?? lastSegment(rootPath);
  const folder = parentPath(end.relative_path);
  return folder === "" ? root : `${root}/${folder}`;
}

/** Where an action took the file, folders only. */
export function moveText(action: ActionView, names: Map<string, string>): string {
  return `${placeText(action.source, names)} → ${placeText(action.target, names)}`;
}

/** What a row says next to its status about the action's folders: which
 * were removed from Traeva since (#80), with why there is no Undo as its
 * full text when that is the reason; else only why there is no Undo
 * (undoNote). Null when there is nothing to say. */
export function folderNote(
  action: ActionView,
  roots: RootView[],
  names: Map<string, string>,
): { short: string; full: string } | null {
  const why = undoNote(action.undo_status);
  const removed = actionRoots(action, roots)
    .filter((root) => root.removed)
    .map((root) => names.get(root.path) ?? lastSegment(root.path));
  if (removed.length === 0) return why;
  const short = `${removed.join(" and ")} removed from Traeva`;
  return { short, full: why?.full ?? `${short}.` };
}

/** When the action happened: when it was recorded as done, else when it
 * started. */
export function actionWhen(action: ActionView, now: number): string {
  return whenText(action.committed_at_ms ?? action.created_at_ms, now);
}

/** Where a dropped file shows on the page (#37, layout B): it waits for
 * the user ("needs"); Traeva works on it ("working"); it has a move, which
 * shows among the moves ("moved"); it ended without a move ("ended"). */
export type DropGroup = "needs" | "working" | "moved" | "ended";

/** One dropped file's row. */
export type DropRow = {
  key: string;
  group: DropGroup;
  /** Its action's ID, also when the page could not read the action. */
  actionId: string | null;
  /** When it was dropped (ms since 1970). */
  receivedAt: number;
  name: string;
  badge: { label: string; tone: Tone };
  dropped: string;
  /** Why it stopped or was handed over, as the core wrote it; never file text. */
  reason: string | null;
  /** Its action as saved, when the page has it: the move's folders and Undo. */
  action: ActionView | null;
  /** It waits for the user in Organize: the label of the button there. */
  open: "Answer" | "Review" | null;
};

// The core's reason says why for these; for the other states it only
// repeats the badge ("Cancelled.", "Moved.").
const REASON_STATES: DropItemView["state"][] = ["stopped", "handed_over"];

/** The dropped files, newest first. `actions`: the saved actions the page
 * has, by ID. `outcomes`: handed-over files whose usual flow ended in
 * Organize without a plan (this window only). */
export function dropRows(
  items: DropItemView[],
  outcomes: Record<string, DropOutcome>,
  actions: Map<string, ActionView>,
  now: number,
): DropRow[] {
  const order = items.map((item, index) => ({ item, index }));
  order.sort((a, b) => b.item.received_at_ms - a.item.received_at_ms || b.index - a.index);
  return order.map(({ item, index }) => {
    const action = item.action_id === null ? null : (actions.get(item.action_id) ?? null);
    // Without the action itself, its status as the queue gives it (a
    // dropped file's action always organizes).
    const fromQueue = item.action_status === null ? null : { status: item.action_status, kind: "organize" as const };
    const saved = action ?? (item.action_id === null ? null : fromQueue);
    const outcome = item.state === "handed_over" && item.action_id === null ? (outcomes[item.drop_id] ?? null) : null;
    const waits = needsYou(item, outcomes);
    // Stopped after its move started, not settled: checked in Organize.
    const unsettled = unsettledStop(item);
    const settling = saved !== null && ACTION_WORKING.includes(saved.status);
    const group: DropGroup = waits || unsettled
      ? "needs"
      : settling || DROP_WORKING.includes(item.state)
        ? "working"
        : item.action_id !== null
          ? "moved"
          : "ended";
    return {
      key: item.drop_id || `working-${index}`,
      group,
      actionId: item.action_id,
      receivedAt: item.received_at_ms,
      name: item.file_name || "(the file Traeva is working on)",
      badge: dropBadge(item, saved, outcome),
      dropped: `dropped ${droppedAtText(item.received_at_ms, now)}`,
      reason: REASON_STATES.includes(item.state) && outcome === null ? item.reason : null,
      action,
      open: waits ? (item.state === "needs_answer" ? "Answer" : "Review") : unsettled ? "Review" : null,
    };
  });
}

/** The dropped files' actions that are not among the recent ones, to read
 * on their own (get_action) for their Undo. */
export function actionsToRead(items: DropItemView[], recent: ActionView[]): string[] {
  const known = new Set(recent.map((action) => action.action_id));
  const ids = items.map((item) => item.action_id).filter((id): id is string => id !== null && !known.has(id));
  return [...new Set(ids)];
}

// A dropped file not yet done that needs nothing from the user, and an
// action whose move is not settled yet: these change on their own.
const DROP_WORKING: DropState[] = ["waiting", "working"];
const ACTION_WORKING: ActionStatus[] = ["prepared", "running"];

/** Whether something the page lists is still being worked on, so it reads
 * again now and then until nothing is. What waits for the user changes only
 * when the user acts in Organize, which the page reads when it shows again;
 * so do the files waiting behind it once the core's pass has stopped there
 * (it stops at a file that waits for the user). */
export function stillWorking(queue: DropQueueView | null, recent: ActionView[]): boolean {
  const heldByUser = queue !== null && !queue.running && queue.items.some((item) => item.needs_user);
  const dropWorking =
    queue !== null &&
    (queue.running ||
      queue.items.some(
        (item) =>
          (DROP_WORKING.includes(item.state) && !heldByUser) ||
          (item.action_status !== null && ACTION_WORKING.includes(item.action_status)),
      ));
  return dropWorking || recent.some((action) => ACTION_WORKING.includes(action.status));
}

/** One move in the list of moves: a saved action, the dropped file it came
 * from if any, or a dropped file whose action could not be read. */
export type MoveEntry = { key: string; at: number; action: ActionView | null; drop: DropRow | null };

/** When an action happened, in ms (actionWhen's time). */
function actionAt(action: ActionView): number {
  return action.committed_at_ms ?? action.created_at_ms;
}

/** The page's lists (#37, the student's choice B, 10/14), each file once:
 * what waits for the user (the dropped files that wait, newest first, then
 * the actions that need a decision); dropped files Traeva works on; the
 * moves, newest first, a dropped file's move with it; and dropped files
 * that ended without a move. An action that needs a decision shows only
 * among what waits. */
export function activityLists(
  decisions: ActionView[],
  recent: ActionView[],
  drops: DropRow[],
): { waiting: DropRow[]; decisions: ActionView[]; working: DropRow[]; moves: MoveEntry[]; ended: DropRow[] } {
  const deciding = new Set(decisions.map((action) => action.action_id));
  const fromDrop = new Map(
    drops.flatMap((row) => (row.group === "moved" && row.actionId !== null ? [[row.actionId, row] as const] : [])),
  );
  const moves: MoveEntry[] = recent
    .filter((action) => !deciding.has(action.action_id))
    .map((action) => ({ key: action.action_id, at: actionAt(action), action, drop: fromDrop.get(action.action_id) ?? null }));
  const listed = new Set(recent.map((action) => action.action_id));
  for (const [actionId, row] of fromDrop) {
    if (listed.has(actionId) || deciding.has(actionId)) continue;
    const action = row.action;
    moves.push({ key: `drop-${row.key}`, at: action === null ? row.receivedAt : actionAt(action), action, drop: row });
  }
  moves.sort((a, b) => b.at - a.at);
  return {
    waiting: drops.filter((row) => row.group === "needs"),
    decisions,
    working: drops.filter((row) => row.group === "working"),
    moves,
    ended: drops.filter((row) => row.group === "ended"),
  };
}

/** "Today", "Yesterday" or "Earlier", by the calendar day. */
export function dayLabel(ms: number, now: number): "Today" | "Yesterday" | "Earlier" {
  const at = new Date(ms);
  const today = new Date(now);
  if (at.toDateString() === today.toDateString()) return "Today";
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  return at.toDateString() === yesterday.toDateString() ? "Yesterday" : "Earlier";
}

/** The moves cut by day, in their order (newest first). */
export function byDay(moves: MoveEntry[], now: number): { label: string; moves: MoveEntry[] }[] {
  const days: { label: string; moves: MoveEntry[] }[] = [];
  for (const move of moves) {
    const label = dayLabel(move.at, now);
    const last = days[days.length - 1];
    if (last?.label === label) last.moves.push(move);
    else days.push({ label, moves: [move] });
  }
  return days;
}
