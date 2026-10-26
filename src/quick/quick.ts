// What the quick search window shows and what Return does (#208). Pure:
// the search, its order and its words are the core's (contract §4.10).

import type { FindResult, FindView } from "../features/find/types.ts";
import { coverageLines, matchedText, noSearchText } from "../features/find/view.ts";

/** Results shown in the window; the rest are a search away in Traeva. */
export const MAX_QUICK_RESULTS = 6;

/** The window's height range; the core keeps it in it too. */
export const QUICK_MIN_HEIGHT = 64;
export const QUICK_MAX_HEIGHT = 480;

/** The height to ask for, whole points, for what the window shows: when
 * that changes, and each time the window is shown again, since the core
 * opens it at the search box's height (R1-QUICK224-01, #242). */
export function quickWindowHeight(contentHeight: number): number {
  if (!Number.isFinite(contentHeight)) return QUICK_MIN_HEIGHT;
  return Math.min(QUICK_MAX_HEIGHT, Math.max(QUICK_MIN_HEIGHT, Math.ceil(contentHeight)));
}

/** The first results of a finished search, in the core's order. */
export function shownResults(view: FindView | null): FindResult[] {
  return view !== null && view.state === "done" ? view.results.slice(0, MAX_QUICK_RESULTS) : [];
}

export type EnterAction = "open" | "search" | "none";

/** Return: opens the chosen result while the results are for what the box
 * says (`current`); waits while that search is under way; else searches. */
export function enterAction(view: FindView | null, current: boolean): EnterAction {
  if (current && view !== null) {
    if (shownResults(view).length > 0) return "open";
    if (view.state === "working") return "none";
  }
  return "search";
}

/** The arrow keys move the choice within the shown results. */
export function moveChoice(index: number, count: number, step: 1 | -1): number {
  return Math.min(count - 1, Math.max(0, index + step));
}

/** One line about the search: a failure, under way, cancelled, why nothing
 * was searched, nothing matched (with what was searched), or how many of
 * the matches show here. Null: nothing to add to the list. */
export function statusText(view: FindView | null, failure: string | null): string | null {
  if (failure !== null) return failure;
  if (view === null) return null;
  if (view.state === "working") return "Searching…";
  if (view.state === "cancelled") return "The search was cancelled.";
  const noSearch = noSearchText(view);
  if (noSearch !== null) return noSearch;
  if (view.results.length === 0) {
    const searched = view.coverage ? coverageLines(view.coverage, view.words)[0] : null;
    return searched ? `${matchedText(view)} ${searched}` : matchedText(view);
  }
  const shown = shownResults(view).length;
  if (view.matched > shown) return `${view.matched.toLocaleString()} files matched; the first ${shown} are here.`;
  return null;
}
