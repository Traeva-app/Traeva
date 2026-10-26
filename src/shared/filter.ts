// The rule for a page's filter (Filter.tsx): it never keeps something new
// that waits for the user out of sight.

/** The filter to show after a reading: back to `all` when more things wait
 * for the user than before and the current filter would hide them. */
export function filterAfter<K extends string>(
  current: K,
  all: K,
  hidesWaiting: (key: K) => boolean,
  waitingBefore: number,
  waitingNow: number,
): K {
  return waitingNow > waitingBefore && hidesWaiting(current) ? all : current;
}
