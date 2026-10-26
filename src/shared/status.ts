// How a status looks, on every page (#37, the student's choice 10/14): amber
// only while it waits for the user (a question, a folder to allow, a
// decision, a move not settled); green for a move that was done (Organized,
// Put back); grey for everything else, going on or ended (Working, Already
// there, Stopped, Not moved, Cancelled, Undone). Never red for a status.

export type Tone = "warn" | "ok" | "neutral";

/** The badge class for a tone (styles.css .badge). */
export function toneClass(tone: Tone): string {
  return tone === "ok" ? "card-ok" : tone === "warn" ? "badge-warn" : "";
}

/** What a closed list of rows holds, by status: "2 already there · 1
 * stopped", most first. */
export function statusSummary(rows: { badge: { label: string } }[]): string {
  const counts = new Map<string, number>();
  for (const row of rows) counts.set(row.badge.label, (counts.get(row.badge.label) ?? 0) + 1);
  return [...counts]
    .sort((a, b) => b[1] - a[1])
    .map(([label, count]) => `${count} ${label.charAt(0).toLowerCase()}${label.slice(1)}`)
    .join(" · ");
}
