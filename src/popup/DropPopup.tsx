// The pet's popup (#37, the student's request 10/14): a click on the desktop
// pet, when a dropped file waits for the user with a question or its usual
// steps, opens this small window beside the pet instead of the main window.
// It shows that one file: its question, or its usual steps with Organize's
// own card in its small form (a folder chosen by hand), and the Move card.
// A suggestion goes straight to the Move card: its plan is built at once,
// and the card shows the file and the last folders of where it goes, the
// rest in Details (organize-spec §6, v0.13). Everything goes through
// Organize's commands and the core's
// rules: nothing is decided here, and the main window shows the same file's
// state when it reads the queue. Undo, the rest of the files and anything
// else are in Traeva's main window ("Open in Traeva").
//
// The window stays when another app comes to the front (a decision may be
// half made); Esc, the close button, Open in Traeva and Close after the
// result hide it. Showing it again reads the queue again.
//
// The pet may also show it as a small "Moved to" note (MovedNote, #37
// 10/17), without the focus, after one file dropped alone on it was moved.
// A note never takes the place of a file the user is working on here.

import { useCallback, useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelDrop, continueDrops, getDropQueue, startDropOperation } from "../features/autodrop/api";
import { validRootId } from "../features/autodrop/queue";
import type { DropItemView, DropQueueView } from "../features/autodrop/types";
import {
  answerClarification,
  applyPlan,
  cancelOperation,
  cancelPlan,
  getAction,
  getAppState,
  getOperation,
  getPlan,
} from "../features/organize/api";
import { folderTail } from "../features/organize/compact";
import { Details } from "../features/organize/Details";
import { answerFor, isBusy, isIdle, isNewer } from "../features/organize/flow";
import { OperationCard, QuestionCard, RecommendationFacts, ShortPlace } from "../features/organize/OperationCard";
import {
  ErrorDetails,
  ExtractionText,
  MoveRoute,
  failure,
  type Flow,
  type PreviewContext,
} from "../features/organize/shared";
import {
  isAppError,
  type ActionView,
  type AppError,
  type AppState,
  type OperationView,
  type PlanView,
} from "../features/organize/types";
import { showMainWindow } from "../features/pet/api";
import { hideDropPopup, resizeDropPopup, takeMovedNote } from "./api";
import { MovedNote } from "./MovedNote";
import { othersWaiting, popupDrop, popupHeight } from "./popup";
import "./popup.css";

const POLL_MS = 300;
const QUEUE_POLL_MS = 1500;
// The note's width (points); the popup is wider (the core sets it back
// when the pet is clicked).
const NOTE_WIDTH = 320;

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

// The window comes in with a short fade and rise when it shows again after
// being hidden, not each time it comes back to the front.
let comeIn = true;

/** A press on the popup's top line, not on a button, drags the window. */
function dragWindow(event: PointerEvent<HTMLElement>) {
  if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
  getCurrentWindow()
    .startDragging()
    .catch(() => undefined);
}

function hide() {
  comeIn = true;
  hideDropPopup().catch(() => undefined);
}

function openTraeva() {
  showMainWindow("organize")
    .then(hide)
    .catch(() => undefined);
}

/** Which step shows: a new one comes in with a short fade (the Move card's
 * own refreshes, the queue's readings and typing do not). */
function stepKey(flow: Flow, item: DropItemView | null, choosing: boolean): string {
  if (flow.step === "operation") return `operation:${flow.operation.stage}:${choosing}`;
  if (flow.step === "preview") return `preview:${flow.plan.plan_id}`;
  if (flow.step !== "start") return flow.step;
  return item === null ? "none" : `${item.drop_id}:${item.state}`;
}

/** The window is at the note's size: the core showed it for a note
 * (show_moved_note), not as the popup (which is wider). */
function atNoteSize(): boolean {
  return window.innerWidth <= NOTE_WIDTH + 40;
}

// Motion is short and only when the system allows it (Reduce motion off).
function moving(): boolean {
  return !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function DropPopup() {
  const [queue, setQueue] = useState<DropQueueView | null>(null);
  const [app, setApp] = useState<AppState | null>(null);
  const [readError, setReadError] = useState<AppError | null>(null);
  // The file the popup works on, and its usual steps (Organize's flow).
  const [dropId, setDropId] = useState<string | null>(null);
  const [flow, setFlow] = useState<Flow>({ step: "start" });
  // A question's operation, and the answer once it is on its way or sent.
  const [asked, setAsked] = useState<OperationView | null>(null);
  const [answered, setAnswered] = useState<"sending" | "sent" | null>(null);
  // The question the answer was for: a next question (up to 3) is a new one.
  const [answeredQuestion, setAnsweredQuestion] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  // From the Move card's "Choose another folder": the folder list, not the
  // suggestion (whose plan would be built again at once).
  const [choosing, setChoosing] = useState(false);
  // Files the user cancelled here, left out until the queue says so too.
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(() => new Set());
  const root = useRef<HTMLDivElement>(null);
  const stepBox = useRef<HTMLDivElement>(null);
  // The dropped file a "Moved to" note is for, while the note shows; and
  // whether the window was just shown at the note's size, its request not
  // taken yet (nothing is drawn meanwhile).
  const [note, setNote] = useState<string | null>(null);
  const [notePending, setNotePending] = useState(atNoteSize);
  // The same, kept at once for the calls below (state lags a render).
  const noteNow = useRef<string | null>(null);
  const pendingNow = useRef(notePending);
  // The note's Undo has started: a later note is let go.
  const noteBusy = useRef(false);

  const read = useCallback(() => {
    Promise.all([getDropQueue(), getAppState()]).then(
      ([nextQueue, nextApp]) => {
        setQueue(nextQueue);
        setApp(nextApp);
        setReadError(null);
      },
      (reason: unknown) => setReadError(asError(reason)),
    );
  }, []);

  // As tall as what it shows (the core keeps it in its limits); a note is
  // narrower.
  const fit = useCallback(() => {
    const box = root.current;
    if (box === null) return;
    const height = popupHeight(box.getBoundingClientRect().height);
    if (noteNow.current !== null || pendingNow.current) {
      // Not before the note has something to show.
      if (height >= 48) resizeDropPopup(height, NOTE_WIDTH).catch(() => undefined);
    } else if (!atNoteSize()) {
      // At the note's size without a note (it is hiding), never the
      // popup's height: the core would make the window wide again.
      resizeDropPopup(height).catch(() => undefined);
    }
  }, []);
  useEffect(() => {
    const box = root.current;
    if (box === null) return;
    const observer = new ResizeObserver(fit);
    observer.observe(box);
    return () => observer.disconnect();
  }, [fit]);

  // A note the pet asked for: shown when the core showed the window for it
  // (the window was hidden, so the user was not working here; it waited at
  // the note's size), or in place of a note that shows, unless that note's
  // Undo has started. Otherwise it is let go (Activity lists the move);
  // taking it also clears the core's slot, so an old request never comes
  // up later. A window shown for a note it does not show hides again.
  const taking = useRef(false);
  const takeNote = useCallback(() => {
    if (taking.current) return;
    taking.current = true;
    const settle = (id: string | null) => {
      taking.current = false;
      const waited = pendingNow.current;
      pendingNow.current = false;
      setNotePending(false);
      const showing = noteNow.current !== null;
      if (id !== null && (waited || (showing && !noteBusy.current))) {
        noteNow.current = id;
        setNote(id);
      } else if (waited && !showing) {
        hide();
      }
    };
    takeMovedNote().then(settle, () => settle(null));
  }, []);

  // Shown (the pet was clicked, or the core showed a note) or back in
  // front: read again. While shown, the queue is read now and then, since
  // the core goes on by itself after an answer.
  useEffect(() => {
    const shown = () => {
      // The pet was clicked while a note showed (the core made the window
      // wide again): the usual popup.
      if (!atNoteSize() && noteNow.current !== null) {
        noteNow.current = null;
        setNote(null);
      }
      // Shown at the note's size (show_moved_note), not the popup's: wait
      // for the note's request before drawing anything.
      const forNote = atNoteSize() && noteNow.current === null;
      if (forNote) {
        pendingNow.current = true;
        setNotePending(true);
      }
      takeNote();
      read();
      if (!forNote) fit();
      if (!comeIn || document.hidden) return;
      comeIn = false;
      if (moving()) {
        root.current?.animate(
          [
            { opacity: 0, transform: "translateY(6px)" },
            { opacity: 1, transform: "none" },
          ],
          { duration: 180, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" },
        );
      }
    };
    const visibility = () => {
      if (document.hidden) comeIn = true;
      else shown();
    };
    shown();
    window.addEventListener("focus", shown);
    document.addEventListener("visibilitychange", visibility);
    const timer = window.setInterval(() => {
      if (document.hidden) return;
      takeNote();
      read();
    }, QUEUE_POLL_MS);
    return () => {
      window.removeEventListener("focus", shown);
      document.removeEventListener("visibilitychange", visibility);
      window.clearInterval(timer);
    };
  }, [read, fit, takeNote]);

  // The note: the page draws its own bubble on a clear window. The pet was
  // clicked while it showed (the core made the window wide again): the
  // usual popup.
  useEffect(() => {
    document.documentElement.classList.toggle("ask-note", note !== null || notePending);
    fit();
    if (note === null) return;
    const widened = () => {
      if (!atNoteSize()) {
        noteNow.current = null;
        setNote(null);
      }
    };
    window.addEventListener("resize", widened);
    return () => window.removeEventListener("resize", widened);
  }, [note, notePending, fit]);
  const noteBusyIs = useCallback((busy: boolean) => {
    noteBusy.current = busy;
  }, []);
  const closeNote = useCallback(() => {
    noteNow.current = null;
    setNote(null);
    hide();
  }, []);

  // Esc hides the popup (Organize's folder list takes Esc first to go up).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
      noteNow.current = null;
      setNote(null);
      hide();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The file: the one the user is on while its steps or answer go on, else
  // the first one the popup can take.
  const inSteps = flow.step !== "start" || answered !== null;
  const item = queue === null ? null : popupDrop(queue.items, dropId, inSteps, skipped);
  const itemId = item?.drop_id ?? null;
  useEffect(() => {
    if (itemId === dropId) return;
    setDropId(itemId);
    setFlow({ step: "start" });
    setChoosing(false);
    setAsked(null);
    setAnswered(null);
    setAnsweredQuestion(null);
    setError(null);
  }, [itemId, dropId]);

  // A question's operation, read when the file comes up.
  const questionOperation = item?.state === "needs_answer" ? item.operation_id : null;
  useEffect(() => {
    if (questionOperation === null) {
      setAsked(null);
      return;
    }
    let alive = true;
    getOperation(questionOperation).then(
      (operation) => {
        if (alive) setAsked(operation);
      },
      (reason: unknown) => {
        if (alive) setError(asError(reason));
      },
    );
    return () => {
      alive = false;
    };
  }, [questionOperation]);

  // After an answer the core goes on. It may ask a next question: the file
  // waits for an answer again, with another question, which then shows.
  const askedAgain = answered === "sent" && item?.state === "needs_answer" ? item.operation_id : null;
  useEffect(() => {
    if (askedAgain === null) return;
    let alive = true;
    getOperation(askedAgain).then(
      (operation) => {
        if (!alive || operation.question === null || operation.question.question_id === answeredQuestion) return;
        setAsked(operation);
        setAnswered(null);
      },
      () => undefined,
    );
    return () => {
      alive = false;
    };
    // Read again with each reading of the queue while it waits.
  }, [askedAgain, answeredQuestion, queue]);

  // A moved file shows its result; one that comes back for its usual steps,
  // or stops, shows that instead.
  useEffect(() => {
    if (answered !== "sent" || item === null || item.state === "needs_answer") return;
    if (item.state === "moved" && item.action_id !== null) {
      getAction(item.action_id).then(
        (action) => setFlow({ step: "done", action }),
        () => undefined,
      );
    }
    if (item.state !== "waiting" && item.state !== "working") setAnswered(null);
  }, [answered, item]);

  // A handed-over file with an operation goes on with it at once.
  const handedOperation =
    item?.state === "handed_over" && item.needs_user && flow.step === "start" ? item.operation_id : null;
  useEffect(() => {
    if (handedOperation === null) return;
    let alive = true;
    getOperation(handedOperation).then(
      (operation) => {
        if (alive) setFlow({ step: "operation", operation });
      },
      (reason: unknown) => {
        if (alive) setFlow(failure("Could not go on with this file", reason));
      },
    );
    return () => {
      alive = false;
    };
  }, [handedOperation]);

  // Shows a view of the operation, never an older one.
  const showOperation = useCallback((operation: OperationView) => {
    setFlow((current) => {
      if (current.step === "operation" && !isNewer(operation, current.operation)) return current;
      return { step: "operation", operation };
    });
  }, []);

  // While the core reads the file or waits for the model, poll the operation.
  const pollingId = flow.step === "operation" && isBusy(flow.operation.stage) ? flow.operation.operation_id : null;
  useEffect(() => {
    if (pollingId === null) return;
    const timer = window.setInterval(() => {
      getOperation(pollingId)
        .then((operation) =>
          setFlow((current) =>
            current.step === "operation" && isNewer(operation, current.operation)
              ? { step: "operation", operation }
              : current,
          ),
        )
        .catch((reason: unknown) => setFlow(failure("Could not check the file", reason)));
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [pollingId]);

  // Organize's card ends a flow with { step: "start" } (Cancel): the file is
  // let go of here, and the queue is read again.
  const setFlowFromCard = useCallback(
    (next: Flow) => {
      if (next.step === "start" && dropId !== null) {
        setSkipped((current) => new Set(current).add(dropId));
        read();
      }
      setFlow(next);
    },
    [dropId, read],
  );

  // One call at a time; each ends with the queue as it is now.
  const act = (call: () => Promise<DropQueueView>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    call()
      .then(setQueue)
      .catch((reason: unknown) => setError(asError(reason)))
      .finally(() => setBusy(false));
  };

  const answer = (operationId: string, optionId: string | null, text: string) => {
    if (asked === null || asked.question === null || answered !== null) return;
    const reply = answerFor(asked.question, optionId, text);
    if (reply === null) return;
    setAnswered("sending");
    setAnsweredQuestion(asked.question.question_id);
    setError(null);
    answerClarification(operationId, reply).then(
      () => {
        setAnswered("sent");
        continueDrops().then(setQueue, (reason: unknown) => setError(asError(reason)));
      },
      (reason: unknown) => {
        setAnswered(null);
        setError(asError(reason));
      },
    );
  };

  const cancelThis = (id: string) =>
    act(async () => {
      await cancelDrop(id);
      setSkipped((current) => new Set(current).add(id));
      return continueDrops();
    });

  const startSteps = (id: string, rootId: string) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    startDropOperation(id, rootId)
      .then((operation) => setFlow({ step: "operation", operation }))
      .catch((reason: unknown) => setError(asError(reason)))
      .finally(() => setBusy(false));
  };

  // After confirming: the plan's saved action, read until the run settles.
  const applyingId = flow.step === "applying" ? flow.plan.plan_id : null;
  useEffect(() => {
    if (applyingId === null) return;
    const timer = window.setInterval(() => {
      getPlan(applyingId)
        .then((plan) => (plan.action_id ? getAction(plan.action_id) : null))
        .then((action) => {
          if (action === null) return;
          setFlow((current) =>
            current.step === "applying" && current.plan.plan_id === applyingId ? { ...current, action } : current,
          );
        })
        .catch(() => undefined);
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [applyingId]);

  // As in Organize: an error naming an action shows that action; a refusal
  // before anything ran keeps the preview, with the core's reason.
  const showAction = (actionId: string, reason: AppError) => {
    getAction(actionId)
      .then((action) => setFlow({ step: "done", action, error: reason }))
      .catch(() => setFlow(failure("The result could not be read", reason)));
  };
  const confirm = (plan: PlanView, context: PreviewContext | null) => {
    setFlow({ step: "applying", plan, action: null });
    applyPlan(plan.plan_id)
      .then((action) => setFlow({ step: "done", action }))
      .catch((reason: unknown) => {
        if (isAppError(reason) && reason.action_id) {
          showAction(reason.action_id, reason);
        } else if (isAppError(reason)) {
          getPlan(plan.plan_id)
            .then((fresh) => {
              if (fresh.action_id) showAction(fresh.action_id, reason);
              else setFlow({ step: "preview", plan: fresh, context, error: reason });
            })
            .catch(() => setFlow(failure("This plan was not run", reason)));
        } else {
          setFlow(failure("This plan was not run", reason));
        }
      })
      .finally(read);
  };

  // Closing after a result: the next show starts with the first file again.
  const close = () => {
    if (isIdle(flow.step) && answered === null) {
      setFlow({ step: "start" });
      setChoosing(false);
      setDropId(null);
    }
    hide();
  };

  // The footer's Cancel: the file's operation (or the plan on screen) is
  // cancelled, as with Organize's Cancel, and the file is let go of here.
  const cancelSteps = (call: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    call()
      .then(() => setFlowFromCard({ step: "start" }))
      .catch((reason: unknown) => setError(asError(reason)))
      .finally(() => setBusy(false));
  };

  const others = queue === null ? 0 : othersWaiting(queue.items, itemId, skipped);
  const roots = app?.roots ?? [];
  const rootPath = (rootId: string) => roots.find((one) => one.root_id === rootId)?.display_path ?? null;
  const targetRootPath =
    flow.step === "operation"
      ? rootPath(flow.operation.target_root_id)
      : flow.step === "preview"
        ? rootPath(flow.plan.target.root_id)
        : flow.step === "done"
          ? rootPath(flow.action.target.root_id)
          : null;

  let cancel: (() => void) | null = null;
  if (flow.step === "operation") {
    const operationId = flow.operation.operation_id;
    cancel = () => cancelSteps(() => cancelOperation(operationId));
  } else if (flow.step === "preview") {
    const { plan, context } = flow;
    cancel = () =>
      cancelSteps(() => (context ? cancelOperation(context.operation.operation_id) : cancelPlan(plan.plan_id)));
  } else if (
    flow.step === "start" &&
    item !== null &&
    answered === null &&
    (item.state === "needs_answer" || (item.state === "handed_over" && item.operation_id === null))
  ) {
    const id = item.drop_id;
    cancel = () => cancelThis(id);
  }

  // A new step fades in. The cards stay mounted (no key), so what they hold,
  // such as an error or typed text, stays.
  const step = stepKey(flow, item, choosing);
  const shownStep = useRef(step);
  useEffect(() => {
    if (shownStep.current === step) return;
    shownStep.current = step;
    if (moving()) {
      stepBox.current?.animate(
        [
          { opacity: 0, transform: "translateY(4px)" },
          { opacity: 1, transform: "none" },
        ],
        { duration: 160, easing: "ease-out" },
      );
    }
  }, [step]);

  let body: ReactNode;
  if (queue === null) {
    body = readError ? <ErrorDetails error={readError} /> : <p className="muted">Reading…</p>;
  } else if (flow.step !== "start") {
    body = (
      <StepCard
        flow={flow}
        modelReady={app?.model === "configured"}
        setFlow={setFlowFromCard}
        showOperation={showOperation}
        targetRootPath={targetRootPath}
        choosing={choosing}
        setChoosing={setChoosing}
        confirm={confirm}
        close={close}
      />
    );
  } else if (item === null && others > 0) {
    // Files wait, but not for anything this window does (a folder to allow,
    // whose explanation is in Organize).
    body = (
      <section className="card">
        <h2>Go on in Traeva</h2>
        <p className="muted small">
          {others === 1 ? "The file that waits needs" : "The files that wait need"} Traeva's main window, for
          example to allow a folder.
        </p>
        <div className="row">
          <button type="button" className="primary" onClick={openTraeva}>
            Open in Traeva
          </button>
          <button type="button" onClick={close}>
            Close
          </button>
        </div>
      </section>
    );
  } else if (item === null) {
    body = (
      <section className="card">
        <h2>Nothing waits for you now</h2>
        <p className="muted small">The files you dropped on the pet are listed in Traeva, under Activity.</p>
        <div className="row">
          <button type="button" className="primary" onClick={close}>
            Close
          </button>
        </div>
      </section>
    );
  } else if (item.state === "needs_answer" || answered !== null) {
    // Its question, or the answer on its way while the core works on it.
    body = (
      <QuestionStep
        item={item}
        operation={asked}
        answered={answered}
        onAnswer={answer}
      />
    );
  } else if (item.state === "handed_over" && item.operation_id === null) {
    body = (
      <StartStep
        item={item}
        roots={roots}
        busy={busy}
        onStart={(rootId) => startSteps(item.drop_id, rootId)}
      />
    );
  } else if (item.state === "handed_over") {
    body = <p className="muted">Opening its steps…</p>;
  } else {
    // The file went on without the user (moved, stopped…): the queue's word.
    body = (
      <section className="card">
        <h2>{item.reason ?? "Traeva went on with this file."}</h2>
        <div className="row">
          <button type="button" className="primary" onClick={close}>
            Close
          </button>
        </div>
      </section>
    );
  }

  return (
    // One box for both, so its size is followed (fit) whichever shows.
    <div className={note === null && !notePending ? "ask" : "ask ask-note-box"} ref={root}>
      {note !== null || notePending ? (
        note !== null && (
          <MovedNote
            key={note}
            dropId={note}
            onClose={closeNote}
            onChange={fit}
            onBusy={noteBusyIs}
          />
        )
      ) : (
        <>
          {/* The top line moves the window (it has no title bar): a press
              anywhere on it but the close button. */}
          <header className="ask-header" onPointerDown={dragWindow}>
            <div className="ask-file">
              <span className="muted small">Dropped on Traeva</span>
              <span className="ask-name exact">{item?.file_name || "(a dropped file)"}</span>
            </div>
            <button type="button" className="ask-close" aria-label="Close" onClick={close}>
              ×
            </button>
          </header>
          <div className="ask-step" ref={stepBox}>
            {body}
          </div>
          {error && <ErrorDetails error={error} />}
          <footer className="ask-footer">
            {others > 0 && item !== null && (
              <span className="muted small">
                {others === 1 ? "1 more file waits in Traeva." : `${others} more files wait in Traeva.`}
              </span>
            )}
            <span className="ask-links">
              {cancel && (
                <button type="button" className="ask-link" onClick={cancel} disabled={busy}>
                  Cancel this file
                </button>
              )}
              <button type="button" className="ask-link" onClick={openTraeva}>
                Open in Traeva
              </button>
            </span>
          </footer>
        </>
      )}
    </div>
  );
}

/** A dropped file's question, as in Organize's drop card. */
function QuestionStep(props: {
  item: DropItemView;
  operation: OperationView | null;
  answered: "sending" | "sent" | null;
  onAnswer: (operationId: string, optionId: string | null, text: string) => void;
}) {
  const { item, operation, answered } = props;
  if (answered === "sent") {
    return (
      <section className="card">
        <h2>Answer sent</h2>
        <p className="muted small">Traeva asks the model once more, then goes on with this file.</p>
      </section>
    );
  }
  if (operation === null || operation.question === null) return <p className="muted">Reading the question…</p>;
  return (
    <>
      {item.automatic && (
        <p className="muted small">
          After your answer, Traeva may move it without a preview, as you agreed for dropped files.
        </p>
      )}
      <QuestionCard
        question={operation.question}
        answered={operation.answered}
        file={operation.file}
        sending={answered === "sending"}
        primary
        compact
        onAnswer={(optionId, text) => props.onAnswer(operation.operation_id, optionId, text)}
        footer={null}
      />
    </>
  );
}

/** A file handed over with the automatic mode off: its usual steps start
 * here, into the folder chosen (as in Organize's drop card). */
function StartStep(props: {
  item: DropItemView;
  roots: AppState["roots"];
  busy: boolean;
  onStart: (rootId: string) => void;
}) {
  const [chosen, setChosen] = useState("");
  const rootId = validRootId(chosen, props.roots.map((one) => one.root_id));
  return (
    <section className="card">
      <h2>Organize this file?</h2>
      {props.item.reason && <p className="small">{props.item.reason}</p>}
      {props.roots.length === 0 ? (
        <p className="muted small">Add a folder to organize it into first, in Traeva's Settings.</p>
      ) : (
        props.roots.length > 1 && (
          <label className="field">
            Into
            <select value={rootId} onChange={(event) => setChosen(event.target.value)}>
              {props.roots.map((one) => (
                <option key={one.root_id} value={one.root_id}>
                  {one.display_path}
                </option>
              ))}
            </select>
          </label>
        )
      )}
      {props.roots.length === 1 && (
        <p className="muted small" title={props.roots[0].display_path}>
          Into <span className="mono exact">{folderTail(props.roots[0].display_path, "")}</span>
        </p>
      )}
      <div className="row">
        <button
          type="button"
          className="primary"
          disabled={props.busy || rootId === ""}
          onClick={() => props.onStart(rootId)}
        >
          Organize it
        </button>
        {/* Handed over because the automatic mode is off (`automatic`
            false): Settings shows its card, where the user reads what it
            does and turns it on. The popup never agrees to anything. */}
        {!props.item.automatic && (
          <button type="button" onClick={turnOnAutomatic}>
            Turn on automatic mode…
          </button>
        )}
      </div>
    </section>
  );
}

/** Shows Traeva with the automatic mode's card in Settings. */
function turnOnAutomatic() {
  showMainWindow("automatic").catch(() => undefined);
}

/** The usual steps once they started: Organize's own card for the
 * operation, then the preview, the run and its result. */
function StepCard(props: {
  flow: Flow;
  modelReady: boolean;
  setFlow: (flow: Flow) => void;
  showOperation: (operation: OperationView) => void;
  targetRootPath: string | null;
  choosing: boolean;
  setChoosing: (choosing: boolean) => void;
  confirm: (plan: PlanView, context: PreviewContext | null) => void;
  close: () => void;
}) {
  const { flow } = props;
  switch (flow.step) {
    case "start":
      return null;
    case "operation":
      return (
        <OperationCard
          // Back from the folder list is a new card: the suggestion's plan
          // is built again at once.
          key={props.choosing ? "choose" : "suggest"}
          operation={flow.operation}
          modelReady={props.modelReady}
          setFlow={props.setFlow}
          showOperation={props.showOperation}
          targetRootPath={props.targetRootPath}
          compact
          chooseFolder={props.choosing}
          onBack={() => props.setChoosing(false)}
        />
      );
    case "preview":
      return (
        <MoveStep
          key={flow.plan.plan_id}
          plan={flow.plan}
          context={flow.context}
          targetRootPath={props.targetRootPath}
          initialError={flow.error ?? null}
          showOperation={props.showOperation}
          chooseAnother={() => props.setChoosing(true)}
          confirm={props.confirm}
        />
      );
    case "applying":
      return (
        <section className="card">
          <h2>Organizing…</h2>
          <p className="muted small">
            Success is shown only after the move was checked and recorded. Closing this window does not cancel it.
          </p>
        </section>
      );
    case "done":
      return (
        <DoneStep
          action={flow.action}
          targetRootPath={props.targetRootPath}
          error={flow.error ?? null}
          close={props.close}
        />
      );
    case "in_place":
      return (
        <section className="card card-ok">
          <h2>Already there</h2>
          <p>It is already in that folder. Nothing to do.</p>
          <div className="row">
            <button type="button" className="primary" onClick={props.close}>
              Close
            </button>
          </div>
        </section>
      );
    case "failed":
      return (
        <section className="card card-error">
          <h2>{flow.title}</h2>
          {flow.error ? <ErrorDetails error={flow.error} /> : <p>{flow.detail}</p>}
          <div className="row">
            <button type="button" className="primary" onClick={openTraeva}>
              Open in Traeva
            </button>
            <button type="button" onClick={props.close}>
              Close
            </button>
          </div>
        </section>
      );
  }
}

/** The Move card: the preview in its small form (organize-spec §6, v0.13).
 * The file is named above it; the card shows the last folders of where it
 * goes and Move file, only when the core says it can run. The whole route
 * (From, the full To, new folders), why and what was read are in Details. */
function MoveStep(props: {
  plan: PlanView;
  context: PreviewContext | null;
  targetRootPath: string | null;
  initialError: AppError | null;
  showOperation: (operation: OperationView) => void;
  chooseAnother: () => void;
  confirm: (plan: PlanView, context: PreviewContext | null) => void;
}) {
  const { plan, context, confirm } = props;
  const [current, setCurrent] = useState(plan);
  const [error, setError] = useState<AppError | null>(props.initialError);

  // The status is worked out by the core when read; ask again now and then.
  useEffect(() => {
    const timer = window.setInterval(() => {
      getPlan(plan.plan_id)
        .then(setCurrent)
        .catch(() => undefined);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [plan.plan_id]);

  // Another folder: this plan is closed first, so only the plan on screen
  // can ever be confirmed; then the folder list.
  const another = (operationId: string) => {
    setError(null);
    cancelPlan(current.plan_id)
      .then(() => getOperation(operationId))
      .then((operation) => {
        props.chooseAnother();
        props.showOperation(operation);
      })
      .catch((reason: unknown) => setError(asError(reason)));
  };

  const parts = current.target.relative_path.split("/");
  const folder = parts.slice(0, -1).join("/");
  const file = context?.operation.file ?? null;
  const suggestion = context?.operation.suggestion ?? null;
  return (
    <section className="card">
      <h2>Move it here?</h2>
      <ShortPlace
        label="To"
        short={folderTail(props.targetRootPath, folder)}
        full={current.target.display_path}
        made={current.folders_to_create.length}
      />
      {current.reason && <p className="warn">{current.reason}</p>}
      <div className="row">
        <button type="button" className="primary" onClick={() => confirm(current, context)} disabled={!current.can_confirm}>
          Move file
        </button>
        {context && (
          <button type="button" onClick={() => another(context.operation.operation_id)}>
            Choose another folder
          </button>
        )}
      </div>
      <Details>
        <MoveRoute
          name={current.basename}
          from={current.source.display_path}
          to={current.target.display_path}
          newFolders={current.folders_to_create}
        />
        {context?.candidate && suggestion && file ? (
          <RecommendationFacts candidate={context.candidate} suggestion={suggestion} file={file} />
        ) : (
          file && (
            <dl>
              <dt>Text read</dt>
              <dd className="small">
                <ExtractionText file={file} />
              </dd>
            </dl>
          )
        )}
        <p className="muted small">Nothing has changed in your folders yet.</p>
      </Details>
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** The result as saved, in the small form: where it went, the whole route
 * in Details. Undo is in Traeva's Activity (the footer's Open in Traeva). */
function DoneStep(props: {
  action: ActionView;
  targetRootPath: string | null;
  error: AppError | null;
  close: () => void;
}) {
  const { action } = props;
  const moved = action.status === "committed";
  const folder = action.target.relative_path.split("/").slice(0, -1).join("/");
  return (
    <section className={`card ${moved ? "card-ok" : "card-error"}`}>
      <h2>{moved ? "Moved" : "Not moved"}</h2>
      {moved && (
        <ShortPlace label="To" short={folderTail(props.targetRootPath, folder)} full={action.target.display_path} />
      )}
      {props.error && <ErrorDetails error={props.error} />}
      {!moved && action.problem && <ErrorDetails error={action.problem} />}
      {moved && <p className="muted small">To undo it, open Traeva and choose Undo… in Activity.</p>}
      <div className="row">
        <button type="button" className="primary" onClick={props.close}>
          Close
        </button>
      </div>
      <Details>
        <MoveRoute
          name={action.target.relative_path.split("/").pop() ?? ""}
          from={action.source.display_path}
          to={action.target.display_path}
          newFolders={action.folders_created}
        />
      </Details>
    </section>
  );
}
