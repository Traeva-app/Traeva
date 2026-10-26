// Long lists show a first batch; "Show more" adds one batch at a time, and
// "Show less" goes back to the first batch once everything is shown (the
// student's list of 10/14, items 4 and 8). Pure functions, so the rules are
// tested without a window; ShowMore.tsx is the React side.

/** What a list shows now. */
export type Shown<T> = {
  /** The items to show, in their own order. */
  visible: T[];
  /** Items not shown yet. */
  hidden: number;
  /** Everything is shown and there were more than one batch: "Show less". */
  canShowLess: boolean;
};

/**
 * The items shown when `shown` of them may show. Items `keep` names always
 * show and do not count (a row that waits for the user, the folder chosen
 * now): only the others are cut, and the order never changes.
 */
export function shownItems<T>(
  items: readonly T[],
  shown: number,
  first: number,
  keep?: (item: T) => boolean,
): Shown<T> {
  const visible: T[] = [];
  let counted = 0;
  let hidden = 0;
  for (const item of items) {
    if (keep?.(item)) {
      visible.push(item);
    } else if (counted < shown) {
      visible.push(item);
      counted += 1;
    } else {
      hidden += 1;
    }
  }
  return { visible, hidden, canShowLess: hidden === 0 && counted > first };
}

/** How many may show after one more click: one batch more. */
export function moreShown(shown: number, batch: number): number {
  return shown + batch;
}

/** The button's text, or null when there is no button (the whole list fits
 * in the first batch). */
export function showMoreText(state: Pick<Shown<unknown>, "hidden" | "canShowLess">): string | null {
  if (state.hidden > 0) return `Show more (${state.hidden} more)`;
  if (state.canShowLess) return "Show less";
  return null;
}
