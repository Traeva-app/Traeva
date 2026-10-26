// The dropped files the user cleared from Organize's card in this run
// (#37, 10/14). Kept in memory here, outside the card, so they stay cleared
// when the user switches pages or the card is drawn again; gone when
// Traeva starts again, as the queue itself is. Activity still lists them.

const cleared = new Set<string>();

/** The cleared files' drop IDs. */
export function clearedIds(): ReadonlySet<string> {
  return new Set(cleared);
}

/** Clears these files from the card; returns all cleared IDs. */
export function clearRows(dropIds: string[]): ReadonlySet<string> {
  for (const id of dropIds) cleared.add(id);
  return clearedIds();
}
