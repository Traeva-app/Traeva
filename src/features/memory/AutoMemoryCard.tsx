// The switch "Keep versions of files Traeva organizes" (#201): when on,
// files Traeva organizes start keeping their versions, and Memory may send
// their changes to be described (Memory's notice, version 4). The core keeps
// the agreement, starts the records and withdraws them when it is turned
// off; this card shows the notice and sends the user's choice.

import { useState } from "react";
import { familyApi } from "../family/api";
import type { AutoMemoryView } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import {
  AUTO_MEMORY_OFF_TEXT,
  AUTO_MEMORY_TITLE,
  autoMemoryLimitText,
  autoMemoryRefusal,
  autoMemoryState,
  autoMemoryText,
} from "./records";

/** `onChange`: the switch as the core says it is now; `onReload`: read it
 * again (after a refusal). */
export function AutoMemoryCard({
  view,
  onChange,
  onReload,
}: {
  view: AutoMemoryView;
  onChange: (view: AutoMemoryView) => void;
  onReload: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const state = autoMemoryState(view);
  const since = view.agreed_at_ms === null ? null : new Date(view.agreed_at_ms).toLocaleString();
  const limit = autoMemoryLimitText(view);

  const turn = (enable: boolean) => {
    setBusy(true);
    setNotice(null);
    setError(null);
    familyApi.setAutoMemory({ enable, notice_version: view.notice_version }).then(
      (next) => {
        setBusy(false);
        onChange(next);
      },
      (reason: unknown) => {
        setBusy(false);
        const failure = isAppError(reason) ? reason : null;
        const text = failure ? autoMemoryRefusal(failure.code) : null;
        if (text) {
          setNotice(text);
          onReload();
        } else {
          setError(failure);
        }
      },
    );
  };

  return (
    <section className="card">
      <div className="row">
        <h2 className="grow">{AUTO_MEMORY_TITLE}</h2>
        <span className={`badge ${state === "on" ? "card-ok" : ""}`}>{state === "on" ? "On" : "Off"}</span>
      </div>
      <p className={state === "outdated" ? "warn" : undefined}>{autoMemoryText(view, since)}</p>
      {(state === "off" || state === "outdated") && (
        <>
          <div className="exact small">{view.notice}</div>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => turn(true)}>
              Turn on
            </button>
          </div>
        </>
      )}
      {state === "on" && (
        <>
          {limit && <p className="warn small">{limit}</p>}
          <details>
            <summary className="small">What it sends</summary>
            <div className="exact small">{view.notice}</div>
          </details>
          <p className="muted small">{AUTO_MEMORY_OFF_TEXT}</p>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => turn(false)}>
              Turn off
            </button>
          </div>
        </>
      )}
      {notice && <p className="warn small">{notice}</p>}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}
