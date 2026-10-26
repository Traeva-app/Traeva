// Where the main window's focus goes after the pet is shown again (#259).
// Showing the pet takes the focus and set_pet_visible gives it back to the
// main window, but the page's focused element is lost: the focus sits on
// the page itself. Find puts its search box back (MAIN_FOCUS_BACK); any
// other page gets the focus back where it was when the switch was used
// (a text field, or the switch itself).

/** The element to focus again: the one focused before, when the focus is
 * now on nothing in particular (the body) and that element is still on the
 * page. Null when the focus is somewhere already, or there is nothing to go
 * back to. */
export function focusBackTarget<T extends { isConnected: boolean }>(
  before: T | null,
  now: unknown,
  body: unknown,
): T | null {
  if (now !== null && now !== body) return null;
  if (before === null || before === body || !before.isConnected) return null;
  return before;
}
