// One described change (#33) with its lines, read again from the two saved
// versions. The description is the model's wording; the lines are what
// Traeva checked, shown with it so the user can see that they match
// (R2-MEM-08, the student's decision of 2026-10-11). Used by the history
// and by search results.

import { useEffect, useState } from "react";
import { useShowMore } from "../../shared/ShowMore";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import { getChangeLines } from "./api";
import { ACTION_TEXT, placeText } from "./history";
import type { ChangeLinesView, ChangeView } from "./types";

/** One described change, with its lines read again from the saved
 * versions (shown unless the user hides them). */
export function ChangeItem({ change }: { change: ChangeView }) {
  const [lines, setLines] = useState<ChangeLinesView | null>(null);
  const [hidden, setHidden] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    setError(null);
    getChangeLines(change.change_id).then(setLines, (reason: unknown) => setError(isAppError(reason) ? reason : null));
  }, [change.change_id]);
  return (
    <li>
      <div className="row">
        <span className="badge">{ACTION_TEXT[change.action]}</span>
        <span className="grow">
          <strong>{change.topic}</strong>: {change.description}
        </span>
        {lines !== null && (
          <button type="button" onClick={() => setHidden(!hidden)}>
            {hidden ? "Show the lines" : "Hide the lines"}
          </button>
        )}
      </div>
      {lines && !hidden && <ChangeLines lines={lines} />}
      {error && <ErrorDetails error={error} />}
    </li>
  );
}

/** One shown line of a change: a place, a removed line or an added line. */
type LineRow = { key: string; text: string; className: string | undefined };

function ChangeLines({ lines }: { lines: ChangeLinesView }) {
  // In a slide deck or workbook, the reader's own lines (slide, role, sheet
  // and cell labels) are shown apart from the writer's text. A big change
  // shows 12 lines at first (the student's list of 10/14).
  const rows: LineRow[] = lines.hunks.flatMap((hunk) => {
    const place = placeText(hunk.place);
    return [
      ...(place ? [{ key: `${hunk.hunk_id}-p`, text: place, className: "muted" }] : []),
      ...hunk.removed.map((line, i) => ({
        key: `${hunk.hunk_id}-r${i}`,
        text: `− ${hunk.before_first_line + i}  ${line}`,
        className: hunk.removed_structure[i] ? "muted" : "warn",
      })),
      ...hunk.added.map((line, i) => ({
        key: `${hunk.hunk_id}-a${i}`,
        text: `+ ${hunk.after_first_line + i}  ${line}`,
        className: hunk.added_structure[i] ? "muted" : undefined,
      })),
    ];
  });
  const shown = useShowMore(rows, 12);
  return (
    <>
      <div className="mono small" style={{ whiteSpace: "pre-wrap", margin: "6px 0" }}>
        {shown.visible.map((row) => (
          <div key={row.key} className={row.className}>
            {row.text}
          </div>
        ))}
        {lines.shortened && <div className="muted">Some lines were cut to keep this short.</div>}
      </div>
      {shown.control}
    </>
  );
}
