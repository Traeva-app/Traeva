// Where text recognition for scanned PDFs and images stands (#186 OCR;
// images #37, 10/14), under the model on the Settings page. Off, nothing
// shows.

import { useEffect, useRef, useState } from "react";
import { getOcrState } from "./api";
import { ocrStateMayChange, ocrStateText } from "./flow";
import type { OcrState } from "./types";

/** Read again every few seconds while the state can still change
 * (R4-OCR-06: ready can fall to failed). */
export function OcrStateLine({
  pageKey,
  onState,
}: {
  pageKey: number;
  /** Told each state read, e.g. so Settings can move a failure to Needs you. */
  onState?: (state: OcrState) => void;
}) {
  const [state, setState] = useState<OcrState>("off");
  const report = useRef(onState);
  report.current = onState;
  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    const read = () => {
      getOcrState()
        .then((next) => {
          if (stopped) return;
          setState(next);
          report.current?.(next);
          if (ocrStateMayChange(next)) timer = window.setTimeout(read, 3000);
        })
        .catch(() => undefined);
    };
    read();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [pageKey]);
  const text = ocrStateText(state);
  return text === null ? null : <p className="muted small">{text}</p>;
}
