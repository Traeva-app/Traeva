// What the window shows for the automatic mode and the drop queue (#168,
// organize-spec.md §13). Text and polling only: whether a file may move,
// and what happens next, is the core's decision.

import type { ActionStatus, PlanKind } from "../organize/types";
import type { AutoDropSettingsView, DropItemView, DropQueueView, DropState } from "./types";

// Tones and their badge classes are shared by every page (shared/status.ts):
// amber only while a file waits for the user, green for a done move, grey
// for the rest (a stop or a move not made is grey, #37 10/14).
import type { Tone } from "../../shared/status.ts";
export { toneClass, type Tone } from "../../shared/status.ts";

const STATE_TEXT: Record<DropState, { label: string; tone: Tone }> = {
  waiting: { label: "Waiting", tone: "neutral" },
  working: { label: "Working…", tone: "neutral" },
  needs_source_authorization: { label: "Needs your OK", tone: "warn" },
  needs_answer: { label: "A question", tone: "warn" },
  handed_over: { label: "Needs you", tone: "warn" },
  moved: { label: "Organized", tone: "ok" },
  already_in_place: { label: "Already there", tone: "neutral" },
  stopped: { label: "Stopped", tone: "neutral" },
  not_processed: { label: "Not processed", tone: "neutral" },
  cancelled: { label: "Cancelled", tone: "neutral" },
};

export function stateText(state: DropState): { label: string; tone: Tone } {
  return STATE_TEXT[state];
}

/** A saved action's status, worded as in Organize's lists. */
export function actionText(status: ActionStatus, kind: PlanKind): { label: string; tone: Tone } {
  switch (status) {
    case "prepared":
    case "running":
      return { label: "Working…", tone: "neutral" };
    case "committed":
      return { label: kind === "undo" ? "Put back" : "Organized", tone: "ok" };
    case "failed_no_move":
      return { label: "Not moved", tone: "neutral" };
    case "needs_user_decision":
      return { label: "Needs your decision", tone: "warn" };
    case "undone":
      return { label: "Undone", tone: "neutral" };
  }
}

/** What became of a handed-over file in the Organize flow when no plan
 * was confirmed, as this window saw it: the core records only a confirmed
 * plan and its action (contract v27 §4.8). */
export type DropOutcome = { kind: "in_place" } | { kind: "given_up" };

const OUTCOME_TEXT: Record<DropOutcome["kind"], { label: string; tone: Tone }> = {
  in_place: { label: "Already there", tone: "neutral" },
  given_up: { label: "Not organized", tone: "neutral" },
};

/** A dropped file's badge: its action as saved when it has one (stopped
 * while the move was not settled: Not settled), else what became of it in
 * this window's Organize flow, else its state in the queue. */
export function dropBadge(
  item: DropItemView,
  action: { status: ActionStatus; kind: PlanKind } | null,
  outcome: DropOutcome | null,
): { label: string; tone: Tone } {
  if (action !== null) {
    const unsettled = action.status === "prepared" || action.status === "running";
    if (item.state === "stopped" && unsettled) return { label: "Not settled", tone: "warn" };
    return actionText(action.status, action.kind);
  }
  if (outcome !== null) return OUTCOME_TEXT[outcome.kind];
  return stateText(item.state);
}

// While a file waits, is worked on or waits for the user, the queue is read
// about every second. Otherwise now and then, since a file dropped while
// Traeva is open arrives without any event.
const ACTIVE: DropState[] = ["waiting", "working", "needs_source_authorization", "needs_answer"];
export const FAST_POLL_MS = 1000;
export const SLOW_POLL_MS = 4000;

export function pollInterval(queue: DropQueueView | null): number {
  if (queue === null) return FAST_POLL_MS;
  const active = queue.running || queue.items.some((item) => ACTIVE.includes(item.state));
  return active ? FAST_POLL_MS : SLOW_POLL_MS;
}

/** Changes whenever a file's state or result changes, to read the page again. */
export function queueSignature(queue: DropQueueView | null): string {
  if (queue === null) return "";
  return queue.items.map((item) => `${item.drop_id}:${item.state}:${item.action_id ?? ""}`).join("|");
}

// Not finished and not being worked on: a stop or cancel of an earlier file
// of the same drop lists these as not processed (contract v29 §4.8).
const OPEN: DropState[] = ["waiting", "needs_source_authorization", "needs_answer"];

/** Files dropped together with this one, after it, that are not finished
 * yet: cancelling it would leave them unprocessed. */
export function hasOpenAfter(items: DropItemView[], index: number): boolean {
  const batch = items[index]?.batch_id;
  return items.slice(index + 1).some((item) => item.batch_id === batch && OPEN.includes(item.state));
}

/** Whether the queue waits for the user: a folder to allow, an answer, or
 * (with the automatic mode off) a handed-over file not yet finished in
 * Organize. A file the automatic mode handed over does not hold up later
 * drops (contract v29 §4.8). `outcomes`: handed-over files whose usual flow
 * ended in this window. */
export function waitsForUser(items: DropItemView[], outcomes: Record<string, DropOutcome>): boolean {
  return items.some((item) => needsYou(item, outcomes) && !(item.state === "handed_over" && item.automatic));
}

/** Whether a dropped file waits for the user, as the core works it out
 * (needs_user: a folder to allow, an answer, or a normal flow still open),
 * unless its flow just ended in this window and the core has not heard of
 * it yet. `outcomes` as for waitsForUser. */
export function needsYou(item: DropItemView, outcomes: Record<string, DropOutcome>): boolean {
  return item.needs_user && outcomes[item.drop_id] === undefined;
}

// The user no longer acts on a file in these states (moved, already there,
// stopped, not processed, cancelled; a handed-over file once its usual flow
// ended), unless it still waits for them or its move is not settled.
const FINISHED: DropState[] = ["moved", "already_in_place", "stopped", "not_processed", "cancelled", "handed_over"];
const SETTLED: ActionStatus[] = ["committed", "failed_no_move", "undone"];

/** The dropped files the user may clear from Organize's card: finished,
 * not waiting for the user, not the file open in the Organize flow, and
 * with no move still being settled. `held`: rows still reading their result,
 * or whose move is not settled, as the rows themselves found it (a stopped
 * file's plan can show a move the queue does not list). Clearing only hides
 * them there; Activity still lists them (#37, 10/14). */
export function finishedIds(
  items: DropItemView[],
  outcomes: Record<string, DropOutcome>,
  held: ReadonlySet<string>,
  activeDropId: string | null,
): string[] {
  return items
    .filter(
      (item) =>
        item.drop_id !== "" &&
        item.drop_id !== activeDropId &&
        FINISHED.includes(item.state) &&
        !needsYou(item, outcomes) &&
        (item.action_status === null || SETTLED.includes(item.action_status)) &&
        !held.has(item.drop_id),
    )
    .map((item) => item.drop_id);
}

/** Whether a dropped file's own reading of its result keeps it from being
 * cleared: still reading it, or its move is not settled (Check again). */
export function holdsRow(reading: boolean, action: { status: ActionStatus } | null): boolean {
  return reading || (action !== null && !SETTLED.includes(action.status));
}

/** Where an answer to a dropped file's question is: on its way, taken by
 * the core, or taken while the queue's next pass did not start. */
export type AnswerState = "sending" | "sent" | "failed";

/** The answer this card sent for a dropped file, and the question it answered. */
export type DropAnswer = { questionId: string; state: AnswerState };

/** Whether the card's answer belongs to the question the file shows now.
 * Only then is it on its way or sent; a newer question (up to 3 per file,
 * organize-spec §15) is asked again. */
export function answersShownQuestion(answer: DropAnswer | null, questionId: string | null): boolean {
  return answer !== null && questionId !== null && answer.questionId === questionId;
}

// A move that started and is not settled yet.
const SETTLING: ActionStatus[] = ["prepared", "running"];

/** A dropped file that stopped after its move started, its result not
 * recorded ("Not settled"): it may have moved, and only the user can look
 * again (Check again reads its record; it never moves the file). The core
 * does not count a stop as needs_user, but in Organize it waits for the
 * user all the same (R3 on #290). */
export function unsettledStop(item: DropItemView): boolean {
  return item.state === "stopped" && item.action_status !== null && SETTLING.includes(item.action_status);
}

/** Where a dropped file shows in Organize's groups, from the queue's own
 * view only (a row's own reading would move it between groups, and a moved
 * row reads again): it waits for the user, also a stop not settled (Needs
 * you, always open); Traeva works on it, or its move goes on (In progress);
 * the rest (Finished). */
export function dropPlace(item: DropItemView, outcomes: Record<string, DropOutcome>): "needs" | "working" | "done" {
  if (needsYou(item, outcomes) || unsettledStop(item)) return "needs";
  if (item.state === "waiting" || item.state === "working") return "working";
  if (item.action_status !== null && SETTLING.includes(item.action_status)) return "working";
  return "done";
}

/** How many dropped files wait for the user in Organize (its Needs you),
 * for the count next to Organize in the sidebar. Find's "Pick up where you
 * left off" lists those the core counts (needsYou); a stop not settled is
 * checked in Organize. */
export function needsYouCount(items: DropItemView[], outcomes: Record<string, DropOutcome>): number {
  return items.filter((item) => dropPlace(item, outcomes) === "needs").length;
}

/** When a file was dropped: the time today, else the date and time. */
export function droppedAtText(ms: number, now: number): string {
  const at = new Date(ms);
  const today = new Date(now).toDateString() === at.toDateString();
  return today
    ? at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
    : at.toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

/** The file the user should see now: a new one, or one that just came to
 * need the user. `seen` holds each file's state as last shown. */
export function itemToShow(items: DropItemView[], seen: Map<string, DropState>): string | null {
  for (const item of items) {
    if (item.drop_id === "") continue;
    const before = seen.get(item.drop_id);
    if (before === undefined || (item.needs_user && before !== item.state)) return item.drop_id;
  }
  return null;
}

export type ModeStatus = "on" | "not_working" | "off";

/** On: on and the agreement counts. Not working: an agreement is open but
 * does not count now (settings.problem says why). Off: no agreement. */
export function modeStatus(settings: AutoDropSettingsView): ModeStatus {
  if (settings.enabled) return "on";
  return settings.consent_id !== null ? "not_working" : "off";
}

export const MAX_SCOPES = 20;

/** Why the chosen range cannot be sent yet, or null. The core checks again. */
export function scopeChoiceProblem(count: number): string | null {
  if (count === 0) return "Choose at least one folder.";
  if (count > MAX_SCOPES) return `Choose at most ${MAX_SCOPES} folders.`;
  return null;
}

/** A folder of the target folder, as listed: "" is the folder itself. */
export function scopeLabel(relativePath: string, rootPath: string): string {
  const rootName = rootPath.split("/").filter(Boolean).pop() ?? rootPath;
  return relativePath === "" ? `${rootName} (the folder itself)` : relativePath;
}

/** The chosen folder if it is still offered, else the first one. */
export function validRootId(rootId: string, rootIds: string[]): string {
  return rootIds.includes(rootId) ? rootId : (rootIds[0] ?? "");
}
