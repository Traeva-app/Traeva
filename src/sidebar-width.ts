// The sidebar's width (App shell). The user drags the line between the
// sidebar and the page: narrower than SNAP_BELOW it becomes the icon rail,
// otherwise it stays between MIN_FULL and MAX_FULL. The width is kept in
// this window's localStorage; without it the sidebar starts at DEFAULT. In
// a window narrower than NARROW_WINDOW the rail shows, whatever was kept.

export const RAIL = 64;
export const MIN_FULL = 184;
export const MAX_FULL = 280;
export const DEFAULT = 200;
export const SNAP_BELOW = 120;
/** One arrow key press on the line. */
export const STEP = 16;
/** A window narrower than this shows the rail, to leave the page room. */
export const NARROW_WINDOW = 640;

const KEY = "traeva.sidebarWidth";

export function isRail(width: number): boolean {
  return width < SNAP_BELOW;
}

/** The width shown: the kept width, or the rail in a narrow window (the
 * kept width does not change). */
export function shownWidth(width: number, windowWidth: number): number {
  return windowWidth < NARROW_WINDOW ? RAIL : width;
}

/** The width for a dragged width: the rail, or a full width in range. */
export function snapWidth(raw: number): number {
  if (!Number.isFinite(raw) || isRail(raw)) return RAIL;
  return Math.min(MAX_FULL, Math.max(MIN_FULL, Math.round(raw)));
}

/** Left (-1) or Right (+1) on the line. Left from the narrowest full width
 * goes to the rail; Right from the rail to the narrowest full width. */
export function stepWidth(width: number, direction: -1 | 1): number {
  if (isRail(width)) return direction > 0 ? MIN_FULL : RAIL;
  if (direction < 0 && width <= MIN_FULL) return RAIL;
  return snapWidth(width + direction * STEP);
}

/** Double-click or Enter: the rail, or back to the last full width. */
export function toggleWidth(width: number, lastFull: number): number {
  return isRail(width) ? snapWidth(Math.max(lastFull, MIN_FULL)) : RAIL;
}

/** A saved width as read from storage; anything else is the default. */
export function parseWidth(saved: string | null): number {
  if (saved === null || saved.trim() === "") return DEFAULT;
  const width = Number(saved);
  return Number.isFinite(width) ? snapWidth(width) : DEFAULT;
}

export function loadWidth(): number {
  try {
    return parseWidth(window.localStorage.getItem(KEY));
  } catch {
    return DEFAULT;
  }
}

export function saveWidth(width: number): void {
  try {
    window.localStorage.setItem(KEY, String(width));
  } catch {
    // Not kept: the next start uses the default width.
  }
}
