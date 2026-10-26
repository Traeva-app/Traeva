// "Moved to …" (#37, the student's request 10/17): after one file dropped
// alone on the pet was moved by the automatic mode, the popup shows as a
// small note beside the pet, without taking the focus: where the file went
// (its last three folders; the whole route in the tooltip) and Undo. It
// says "Moved" only when the core has the move as done (committed), and
// offers Undo only when the core does.
//
// Undo is the usual one (organize-spec §8, §13.6): first its preview, which
// shows the exact place the file goes back to, then Put it back; nothing
// moves on the first click. Keep it here lets the preview's plan go. The
// note goes after a few seconds when it is left alone; not while the
// pointer or the keyboard is on it, nor once Undo started.

import { useEffect, useRef, useState } from "react";
import { getDropQueue } from "../features/autodrop/api";
import { applyPlan, cancelPlan, getAction, getAppState, getPlan, previewUndo } from "../features/organize/api";
import { folderTail } from "../features/organize/compact";
import { undoOffered } from "../features/organize/flow";
import { ErrorDetails } from "../features/organize/shared";
import { isAppError, type ActionView, type AppError, type PlanView, type RootView } from "../features/organize/types";
import { showMainWindow } from "../features/pet/api";
import { noteItem, noteTail } from "./popup";

// How long a note left alone stays, and a "Put back" after Undo.
const NOTE_MS = 8000;
const DONE_MS = 4000;

type Phase =
  | { step: "reading" }
  | { step: "moved"; action: ActionView }
  | { step: "previewing"; action: ActionView }
  | { step: "confirm"; action: ActionView; plan: PlanView }
  | { step: "putting"; plan: PlanView }
  | { step: "put"; action: ActionView }
  | { step: "said"; text: string }
  // The core has an Undo of this file going on, or waiting for the user's
  // decision (undo_in_progress): checked in Traeva.
  | { step: "elsewhere" }
  // Nothing moved (the core refused, or the run stopped before the move).
  | { step: "failed"; error: AppError | null; detail: string }
  // The run started but the core could not confirm where the file is now
  // (needs your decision, or not settled): never "nothing changed".
  | { step: "unsure"; action: ActionView };

/** The folder a plan end lies in, as its last three folders from the
 * authorized folder's name ("…/课件/Chem/AP"). */
function placeTail(end: { root_id: string; relative_path: string }, roots: RootView[]): string {
  const root = roots.find((one) => one.root_id === end.root_id)?.display_path ?? null;
  return folderTail(root, end.relative_path.split("/").slice(0, -1).join("/"));
}

/** The folder of a full path, for the tooltip and the Undo preview. */
function folderOf(path: string): string {
  const at = path.lastIndexOf("/");
  return at > 0 ? path.slice(0, at) : path;
}

export function MovedNote(props: {
  dropId: string;
  onClose: () => void;
  /** What it shows changed: the window takes its height. */
  onChange: () => void;
  /** Its Undo started (a later note must not take its place) or ended. */
  onBusy: (busy: boolean) => void;
}) {
  const { dropId, onClose, onChange, onBusy } = props;
  const [phase, setPhase] = useState<Phase>({ step: "reading" });
  useEffect(onChange, [phase, onChange]);
  const undoing = phase.step === "previewing" || phase.step === "confirm" || phase.step === "putting";
  useEffect(() => onBusy(undoing), [undoing, onBusy]);
  // The Undo preview's plan while it waits for Put it back: let go when the
  // note goes another way (closed, timed out, the pet clicked). Only when
  // the note goes, not when the popup draws again.
  const openPlan = useRef<string | null>(null);
  const busyNow = useRef(onBusy);
  busyNow.current = onBusy;
  useEffect(
    () => () => {
      busyNow.current(false);
      if (openPlan.current !== null) cancelPlan(openPlan.current).catch(() => undefined);
    },
    [],
  );
  const [roots, setRoots] = useState<RootView[]>([]);
  const [held, setHeld] = useState(false);
  // The side toward the pet, from where the core put the window.
  const [tail] = useState(() =>
    noteTail(window.screenX, (window.screen as Screen & { availLeft?: number }).availLeft ?? 0),
  );
  const failed = (reason: unknown) =>
    setPhase({ step: "failed", error: isAppError(reason) ? reason : null, detail: String(reason) });

  // The queue says it again: the only file of its drop, moved; the core's
  // action says the move is done. Otherwise nothing is said.
  useEffect(() => {
    let alive = true;
    Promise.all([getDropQueue(), getAppState()])
      .then(async ([queue, app]) => {
        const item = noteItem(queue.items, dropId);
        if (item === null || item.action_id === null) return null;
        const action = await getAction(item.action_id);
        return { action, roots: app.roots };
      })
      .then(
        (read) => {
          if (!alive) return;
          if (read === null || read.action.status !== "committed") {
            onClose();
            return;
          }
          setRoots(read.roots);
          setPhase({ step: "moved", action: read.action });
        },
        () => {
          if (alive) onClose();
        },
      );
    return () => {
      alive = false;
    };
  }, [dropId, onClose]);

  // Left alone, it goes; a "Put back" goes sooner. Not while it asks or
  // after a problem (the user reads it and closes it).
  const lasts = phase.step === "moved" ? NOTE_MS : phase.step === "put" || phase.step === "said" ? DONE_MS : null;
  useEffect(() => {
    if (lasts === null || held) return;
    const timer = window.setTimeout(onClose, lasts);
    return () => window.clearTimeout(timer);
  }, [lasts, held, onClose]);

  const undo = (action: ActionView) => {
    setPhase({ step: "previewing", action });
    previewUndo(action.action_id).then((preview) => {
      if (preview.outcome === "planned") {
        openPlan.current = preview.plan.plan_id;
        setPhase({ step: "confirm", action, plan: preview.plan });
      } else if (preview.outcome === "already_undone") {
        setPhase({ step: "said", text: "It was put back already." });
      } else {
        setPhase({ step: "elsewhere" });
      }
    }, failed);
  };
  const keep = (plan: PlanView) => {
    openPlan.current = null;
    cancelPlan(plan.plan_id).catch(() => undefined);
    onClose();
  };
  // The Undo's action as saved: done, not moved, or not confirmed (as the
  // main window's Organize reads it).
  const settled = (action: ActionView) => {
    if (action.status === "committed") setPhase({ step: "put", action });
    else if (action.status === "failed_no_move") failed(action.problem ?? "It was not put back.");
    else setPhase({ step: "unsure", action });
  };
  const putBack = (plan: PlanView) => {
    openPlan.current = null;
    setPhase({ step: "putting", plan });
    applyPlan(plan.plan_id).then(settled, (reason: unknown) => {
      // An error that names an action: that action says what happened.
      // Otherwise the plan says whether a run started.
      if (isAppError(reason) && reason.action_id) {
        getAction(reason.action_id).then(settled, () => failed(reason));
        return;
      }
      getPlan(plan.plan_id)
        .then((fresh) => (fresh.action_id ? getAction(fresh.action_id).then(settled) : failed(reason)))
        .catch(() => failed(reason));
    });
  };

  if (phase.step === "reading") return null;
  return (
    <div
      className={tail === null ? "note" : `note note-tail-${tail}`}
      role="status"
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setHeld(false);
      }}
    >
      <NoteBody phase={phase} roots={roots} undo={undo} keep={keep} putBack={putBack} />
      <button type="button" className="note-close" aria-label="Close" onClick={onClose}>
        ×
      </button>
    </div>
  );
}

function NoteBody(props: {
  phase: Phase;
  roots: RootView[];
  undo: (action: ActionView) => void;
  keep: (plan: PlanView) => void;
  putBack: (plan: PlanView) => void;
}) {
  const { phase, roots } = props;
  switch (phase.step) {
    case "reading":
      return null;
    case "moved":
    case "previewing": {
      const { action } = phase;
      return (
        <div className="note-row">
          <MovedIcon />
          <p
            className="note-text"
            title={`${action.source.display_path} → ${action.target.display_path}`}
          >
            <span className="note-label">Moved to</span>
            <span className="note-place mono exact">{placeTail(action.target, roots)}</span>
          </p>
          {undoOffered(action.undo_status) && (
            <button type="button" onClick={() => props.undo(action)} disabled={phase.step === "previewing"}>
              {phase.step === "previewing" ? "Undo…" : "Undo"}
            </button>
          )}
        </div>
      );
    }
    case "confirm":
    case "putting": {
      const plan = phase.plan;
      const busy = phase.step === "putting";
      return (
        <div className="note-confirm">
          <p className="note-label">Put it back?</p>
          {/* The exact place it goes back to (organize-spec §8). */}
          <p className="mono exact note-back">{folderOf(plan.target.display_path)}</p>
          {plan.reason && <p className="warn small">{plan.reason}</p>}
          <div className="row">
            <button
              type="button"
              className="primary"
              onClick={() => props.putBack(plan)}
              disabled={busy || !plan.can_confirm}
            >
              {busy ? "Putting it back…" : "Put it back"}
            </button>
            {!busy && (
              <button type="button" onClick={() => props.keep(plan)}>
                Keep it here
              </button>
            )}
          </div>
        </div>
      );
    }
    case "put":
      return (
        <div className="note-row">
          <MovedIcon back />
          <p className="note-text" title={phase.action.target.display_path}>
            <span className="note-label">Put back in</span>
            <span className="note-place mono exact">{placeTail(phase.action.target, roots)}</span>
          </p>
        </div>
      );
    case "elsewhere":
      return (
        <div className="note-confirm">
          <p className="note-label">An Undo of this file is under way or waits for you</p>
          <div className="row">
            <button type="button" onClick={() => showMainWindow("organize").catch(() => undefined)}>
              Check it in Traeva
            </button>
          </div>
        </div>
      );
    case "said":
      return <p className="note-label note-said">{phase.text}</p>;
    case "failed":
      return (
        <div className="note-confirm">
          <p className="note-label">Not put back</p>
          {phase.error ? <ErrorDetails error={phase.error} /> : <p className="small">{phase.detail}</p>}
          <p className="muted small">Nothing was moved. Traeva's Activity lists the move.</p>
        </div>
      );
    case "unsure":
      return (
        <div className="note-confirm">
          <p className="note-label">Traeva could not confirm where the file is now</p>
          {phase.action.problem && <ErrorDetails error={phase.action.problem} />}
          <div className="row">
            <button type="button" onClick={() => showMainWindow("organize").catch(() => undefined)}>
              Check it in Traeva
            </button>
          </div>
        </div>
      );
  }
}

/** A folder with a check (or an arrow back): the note's small icon. */
function MovedIcon({ back = false }: { back?: boolean }) {
  return (
    <svg className="note-icon" width="28" height="28" viewBox="0 0 28 28" aria-hidden="true">
      <path
        className="note-icon-folder"
        d="M3 8.5A2.5 2.5 0 0 1 5.5 6h5l2 2.2h10A2.5 2.5 0 0 1 25 10.7v9.8a2.5 2.5 0 0 1-2.5 2.5h-17A2.5 2.5 0 0 1 3 20.5z"
      />
      <circle className="note-icon-badge" cx="21" cy="20" r="6" />
      {back ? (
        <path className="note-icon-mark" d="M23.5 20h-5m0 0 2-2m-2 2 2 2" />
      ) : (
        <path className="note-icon-mark" d="m18.4 20.1 1.8 1.8 3.4-3.6" />
      )}
    </svg>
  );
}
