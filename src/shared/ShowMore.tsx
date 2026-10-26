// A list that shows a first batch, then one more batch per "Show more"
// (rules in listBatch.ts). It wraps the list without changing it: the caller
// renders the visible items as before. The button is one element whose text
// changes, so keyboard focus stays on it after each click.

import { useState, type ReactNode } from "react";
import { moreShown, showMoreText, shownItems } from "./listBatch";

/** The items to show now, and the button to place after the list (null
 * when the whole list fits in the first batch). */
export function useShowMore<T>(
  items: readonly T[],
  first: number,
  keep?: (item: T) => boolean,
): { visible: T[]; control: ReactNode } {
  const [shown, setShown] = useState(first);
  const state = shownItems(items, shown, first, keep);
  const text = showMoreText(state);
  const control =
    text === null ? null : (
      <button
        type="button"
        className="show-more"
        onClick={() => setShown(state.hidden > 0 ? moreShown(shown, first) : first)}
      >
        {text}
      </button>
    );
  return { visible: state.visible, control };
}

/** The same as a wrapper: renders `children(visible)`, then the button. */
export function ShowMore<T>(props: {
  items: readonly T[];
  /** How many show at first; each "Show more" adds as many again. */
  first: number;
  /** Items that always show and are not counted (never hidden). */
  keep?: (item: T) => boolean;
  children: (visible: T[]) => ReactNode;
}) {
  const { visible, control } = useShowMore(props.items, props.first, props.keep);
  return (
    <>
      {props.children(visible)}
      {control}
    </>
  );
}
