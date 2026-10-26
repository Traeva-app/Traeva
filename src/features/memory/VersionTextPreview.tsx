// A saved slide deck's or workbook's text on the page (#28, contract v23
// §4.7): the core reads the saved version's checked bytes with the preview
// reader (hidden slides and sheets included and marked) and returns its
// text; nothing is opened, copied, stored or sent. The reader's structure
// lines are shown as labels and the writer's lines without the reader's
// escape. A late result, or one for another version, is dropped.

import { useEffect, useState } from "react";
import { checkedPreview, snapshotKey } from "../docx/preview";
import { previewVersionText, type VersionPreview } from "../preview/api";
import { structureLine, unescapeSourceLine } from "../preview/structure";
import type { SnapshotRef } from "../family/types";
import { previewStatusText, structureLabel } from "./history";
import { ShowMore } from "../../shared/ShowMore";

export function VersionTextPreview({ snapshot, note }: { snapshot: SnapshotRef; note: string }) {
  const key = snapshotKey(snapshot);
  const [attempt, setAttempt] = useState(0);
  // A result is kept with the version and the try it was read for, and is
  // shown only while both are still the chosen ones: from the first render
  // after another version is chosen, nothing of the one before shows
  // (R1-M28-O3). `preview` null: it could not be read.
  const [result, setResult] = useState<{ key: string; attempt: number; preview: VersionPreview | null } | null>(
    null,
  );
  const current = result !== null && result.key === key && result.attempt === attempt ? result : null;

  useEffect(() => {
    let live = true;
    previewVersionText(snapshot.snapshot_id).then(
      (value) => {
        if (!live) return;
        let preview: VersionPreview | null;
        try {
          preview = checkedPreview(snapshot, value);
        } catch {
          preview = null;
        }
        setResult({ key, attempt, preview });
      },
      // An error can hold a path or text: say only that it failed.
      () => {
        if (live) setResult({ key, attempt, preview: null });
      },
    );
    return () => {
      live = false;
    };
    // `key` stands for every field of `snapshot`.
  }, [key, attempt]);

  if (current === null) return <p className="muted">Reading the saved version…</p>;
  if (current.preview === null) {
    return (
      <div>
        <p className="warn">This saved version could not be read. No text is shown.</p>
        <button type="button" onClick={() => setAttempt((n) => n + 1)}>
          Try again
        </button>
      </div>
    );
  }
  const { summary, text } = current.preview.extraction;
  const status = previewStatusText(summary);
  return (
    <div>
      {status && <p className={summary.status === "complete" || summary.status === "partial" ? "muted" : "warn"}>{status}</p>}
      {text !== "" && (
        <ShowMore items={text.split("\n")} first={60}>
          {(visible) => (
            <div className="mono small" style={{ whiteSpace: "pre-wrap" }}>
              {visible.map((line, i) => {
                const structure = structureLine(summary.reader, line);
                return structure ? (
                  <div key={i} className="muted">
                    {structureLabel(structure)}
                  </div>
                ) : (
                  <div key={i}>{unescapeSourceLine(summary.reader, line)}</div>
                );
              })}
            </div>
          )}
        </ShowMore>
      )}
      <p className="muted small">{note}</p>
    </div>
  );
}
