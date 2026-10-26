// The organize page: choose a file → (ask the model for a folder, answer
// its questions, up to 3) or pick the folder by hand → exact preview → confirm →
// result, plus Undo and record checks. The folders, the model and the
// automatic mode are set on the Settings page; the history of moves is on
// the Activity page, whose Undo… starts the Undo here (#248).
//
// The page only shows what the core says and collects choices. Whether to
// recommend or ask, whether a plan can be confirmed, an action's status,
// and why something failed all come from the core. A chosen file is an
// operation polled every 300 ms while it is read or analyzed; after a
// confirmation the page polls the saved action instead. A suggestion from
// the model only ever leads to a preview: the move happens after the user
// confirms the plan, through the same executor as a manual choice. A
// refresh reads the app state again and never resubmits anything.
//
// Files dropped on the Dock icon (#168) are listed by the drop queue card.
// One the core hands over goes on in this flow; when it is finished or
// given up, the queue goes on with the next file.

import { useCallback, useEffect, useRef, useState } from "react";
import { continueDrops, startDropOperation } from "../autodrop/api";
import { DropQueueCard } from "../autodrop/DropQueueCard";
import { actionText, toneClass, type DropOutcome } from "../autodrop/queue";
import { FileSourceSection } from "../source/FileSourceSection";
import { sourceFileId } from "../source/organize-source";
import {
  applyPlan,
  cancelOperation,
  cancelPlan,
  chooseFile,
  getAction,
  getAppState,
  getModelSettings,
  getOperation,
  getPlan,
  previewUndo,
  reconcileAction,
} from "./api";
import {
  FLOW_STEPS,
  actionRoots,
  answerNote,
  isBusy,
  isIdle,
  isNewer,
  isStill,
  flowStepIndex,
  pickRoots,
  undoNote,
  undoOffered,
} from "./flow";
import { FirstFolderCard } from "./FirstFolderCard";
import { Now } from "../../shared/Now";
import { OperationCard } from "./OperationCard";
import {
  ErrorDetails,
  ExtractionText,
  MoveRoute,
  failure,
  type Flow,
  type PreviewContext,
  type SetFlow,
} from "./shared";
import {
  isAppError,
  type ActionView,
  type AppError,
  type AppState,
  type ImageSending,
  type OperationView,
  type PlanView,
  type RootView,
} from "./types";

const POLL_MS = 300;

/** The setting for sending images, read when Suggest a folder is chosen for
 * an image (organize-spec §14.10): the main window asks or sends as it says. */
function readImageSending(): Promise<ImageSending> {
  return getModelSettings().then((settings) => settings.image_sending);
}
/** The card of the actions that need a decision (Review in Find and
 * Activity goes there). */
const DECISIONS_ID = "organize-decisions";
/** The flow's card an Undo asked from Activity shows: its preview or its
 * result; or the note that it did not start. */
const FLOW_CARD_ID = "organize-flow-card";
const UNDO_WAITS_ID = "organize-undo-waits";

/** Puts the focus on a card or note just brought into view, since the
 * button chosen was on another page, hidden now: on its title when it has
 * one, else on the note itself. Both take focus from code only (tabIndex
 * -1), without scrolling again. */
function focusShown(element: HTMLElement) {
  const title = element.querySelector<HTMLElement>("h2, h3");
  (title ?? element).focus({ preventScroll: true });
}

type PageState =
  | { kind: "loading" }
  | { kind: "loaded"; app: AppState }
  | { kind: "failed"; error: AppError | null; detail: string };

/** `active`: the Organize tab is showing. `onShowDrop`: asks for the tab
 * when a dropped file should be seen while another tab shows. `onNeedsYou`:
 * how many things wait for the user here (dropped files and the actions
 * that need a decision), for the sidebar. `onDropOutcomes`: how handed-over
 * files ended here without a confirmed plan, which only this window knows,
 * so Find does not list them as waiting. `showDecisions`: Find asked for the
 * actions that need a decision; `onDecisionsShown` once they are in view.
 * `undoActionId`: Activity's Undo… on this action; `onUndoStarted` once the
 * page took it. */
export function OrganizePage({
  active = true,
  onShowDrop,
  onNeedsYou,
  onDropOutcomes,
  showDecisions = false,
  onDecisionsShown,
  undoActionId = null,
  onUndoStarted,
}: {
  active?: boolean;
  onShowDrop?: () => void;
  onNeedsYou?: (count: number) => void;
  onDropOutcomes?: (outcomes: Record<string, DropOutcome>) => void;
  showDecisions?: boolean;
  onDecisionsShown?: () => void;
  undoActionId?: string | null;
  onUndoStarted?: () => void;
}) {
  const [page, setPage] = useState<PageState>({ kind: "loading" });
  const [flow, setFlow] = useState<Flow>({ step: "start" });
  // The folders picked for the next file, kept when a flow starts over.
  const [rootChoice, setRootChoice] = useState<RootChoice | null>(null);
  // Counts the page's reads, so cards that show saved records read them again.
  const [pageKey, setPageKey] = useState(0);

  const refresh = useCallback(() => {
    getAppState()
      .then((app) => {
        setPage({ kind: "loaded", app });
        setPageKey((key) => key + 1);
      })
      .catch((reason: unknown) =>
        setPage({
          kind: "failed",
          error: isAppError(reason) ? reason : null,
          detail: String(reason),
        }),
      );
  }, []);

  useEffect(refresh, [refresh]);
  // Folders and the model are set in Settings: read again when shown.
  useEffect(() => {
    if (active) refresh();
  }, [active, refresh]);

  // A result on screen is read again whenever it appears and after every
  // refresh of the page (a folder added or removed, a confirmation, the
  // Refresh button), so its Undo follows a folder that was just removed,
  // whichever answer arrived first (#79).
  const shownActionId = flow.step === "done" ? flow.action.action_id : null;
  useEffect(() => {
    if (shownActionId === null) return;
    getAction(shownActionId)
      .then((action) =>
        setFlow((current) =>
          current.step === "done" && current.action.action_id === shownActionId
            ? { ...current, action }
            : current,
        ),
      )
      .catch(() => {
        // The card keeps what it had; the lists were read on their own.
      });
  }, [shownActionId, page]);

  // Shows a view of the current operation, but never an older one.
  const showOperation = useCallback((operation: OperationView) => {
    setFlow((current) => {
      if (current.step === "operation" && !isNewer(operation, current.operation)) return current;
      return { step: "operation", operation };
    });
  }, []);

  // While the core reads the file or waits for the model, poll the
  // operation. Every other stage waits for the user.
  const pollingId =
    flow.step === "operation" && isBusy(flow.operation.stage)
      ? flow.operation.operation_id
      : null;
  useEffect(() => {
    if (pollingId === null) return;
    const timer = setInterval(() => {
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
    return () => clearInterval(timer);
  }, [pollingId]);

  // After confirming: poll the plan's saved action until the run settles.
  const applyingId = flow.step === "applying" ? flow.plan.plan_id : null;
  useEffect(() => {
    if (applyingId === null) return;
    const timer = setInterval(() => {
      getPlan(applyingId)
        .then((plan) => (plan.action_id ? getAction(plan.action_id) : null))
        .then((action) => {
          if (action === null) return;
          setFlow((current) =>
            current.step === "applying" && current.plan.plan_id === applyingId
              ? { ...current, action }
              : current,
          );
        })
        .catch(() => {
          // The run itself reports its outcome; a missed poll changes nothing.
        });
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [applyingId]);

  // An action as saved, with the error that came with it, if any.
  const showAction = (actionId: string, error: AppError) => {
    getAction(actionId)
      .then((action) => setFlow({ step: "done", action, error }))
      .catch(() => setFlow(failure("The result could not be read", error)));
  };

  const confirm = (plan: PlanView, context: PreviewContext | null) => {
    setFlow({ step: "applying", plan, action: null });
    applyPlan(plan.plan_id)
      .then((action) => setFlow({ step: "done", action }))
      .catch((reason: unknown) => {
        // An error with an action ID: the action exists; show what is saved.
        if (isAppError(reason) && reason.action_id) {
          showAction(reason.action_id, reason);
        } else if (isAppError(reason)) {
          // No action named: read the plan again. Used after all → its
          // action. Otherwise refused before anything ran: back to the same
          // preview, where the core's can_confirm says whether it can still
          // run and the way back to the operation stays open.
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
      .finally(refresh);
  };

  // Undo starts its own flow, so it waits while a file is on its way (one
  // flow at a time); nothing new starts while Undo or Check again is asked.
  const [asking, setAsking] = useState(false);
  const [checkError, setCheckError] = useState<AppError | null>(null);
  const idle = !asking && isIdle(flow.step);
  const ask = (call: () => Promise<void>) => {
    if (asking) return;
    setAsking(true);
    call().finally(() => setAsking(false));
  };

  // A late answer is shown only if the flow did not move on meanwhile.
  const showIfStill = (started: Flow, next: Flow) =>
    setFlow((current) => (isStill(started, current) ? next : current));

  const undoById = (actionId: string) => {
    if (!idle) return;
    const started = flow;
    ask(() =>
      previewUndo(actionId)
        .then((preview) =>
          showIfStill(
            started,
            preview.outcome === "planned"
              ? { step: "preview", plan: preview.plan, context: null }
              : { step: "done", action: preview.undo },
          ),
        )
        .catch((reason: unknown) => showIfStill(started, failure("This cannot be undone", reason))),
    );
  };
  const startUndo = (action: ActionView) => undoById(action.action_id);

  // Check again may run at any time (the core waits for a running action).
  // While a file is on its way, the card on screen stays and the result
  // shows only in the lists (Needs your decision, Activity): an unsettled
  // action can be what blocks it.
  const checkAgain = (action: ActionView) => {
    const started = flow;
    const show = isIdle(started.step);
    setCheckError(null);
    ask(() =>
      reconcileAction(action.action_id)
        .then((checked) => {
          if (show) showIfStill(started, { step: "done", action: checked });
        })
        .catch((reason: unknown) => {
          if (show) showIfStill(started, failure("The check did not finish", reason));
          else setCheckError(isAppError(reason) ? reason : null);
        })
        .finally(refresh),
    );
  };

  // A dropped file the core handed over (#168) goes on in this flow: its
  // own operation, or a new one when the automatic mode is off.
  const [activeDrop, setActiveDrop] = useState<string | null>(null);
  const [queueKey, setQueueKey] = useState(0);
  // Which dropped file each opened operation belongs to.
  const [dropOperations, setDropOperations] = useState<Record<string, string>>({});
  const openDrop = (dropId: string, open: () => Promise<OperationView>) => {
    if (!idle) return;
    ask(() =>
      open()
        .then((operation) => {
          setActiveDrop(dropId);
          setDropOperations((current) => ({ ...current, [operation.operation_id]: dropId }));
          setFlow({ step: "operation", operation });
        })
        .catch((reason: unknown) => setFlow(failure("Could not open the dropped file", reason))),
    );
  };
  // What became of such a file here when no plan was confirmed: the core
  // keeps it as handed over, so the queue card shows this instead.
  const [dropOutcomes, setDropOutcomes] = useState<Record<string, DropOutcome>>({});
  // Such a flow is also cancelled in the core (contract v35 §4.8), so the
  // queue, the sidebar and the desktop pet stop counting the file as
  // waiting for the user. Cancelling an operation that has ended already
  // fails, and nothing waits then.
  const operationsOf = (dropId: string) =>
    Object.keys(dropOperations).filter((operationId) => dropOperations[operationId] === dropId);
  const endDropFlow = (dropId: string) => {
    Promise.allSettled(operationsOf(dropId).map((operationId) => cancelOperation(operationId))).finally(() =>
      setQueueKey((key) => key + 1),
    );
  };
  // Already there keeps its operation while its card shows (Choose another
  // folder goes back to it): its flow ends once the user moves on.
  const leftInPlace = useRef<string | null>(null);
  const setDropOutcome = useCallback(
    (dropId: string, outcome: DropOutcome) =>
      setDropOutcomes((current) => ({ ...current, [dropId]: outcome })),
    [],
  );
  useEffect(() => {
    onDropOutcomes?.(dropOutcomes);
  }, [dropOutcomes, onDropOutcomes]);
  // Back at the start or a result: the file is finished or given up, so the
  // queue goes on with the next one. A failure before anything ran leaves
  // it handed over: the queue card still offers to continue or cancel it.
  const dropFinished = activeDrop !== null && isIdle(flow.step);
  // The queue goes on once per dropped file, even when the file comes back
  // (Choose another folder after Already there) and ends again.
  const continuedDrops = useRef(new Set<string>());
  useEffect(() => {
    if (!dropFinished || activeDrop === null) return;
    setActiveDrop(null);
    if (flow.step === "failed") return;
    const outcome = outcomeOf(flow);
    if (outcome !== null) setDropOutcome(activeDrop, outcome);
    if (outcome?.kind === "given_up") endDropFlow(activeDrop);
    if (outcome?.kind === "in_place") leftInPlace.current = activeDrop;
    if (continuedDrops.current.has(activeDrop)) return;
    continuedDrops.current.add(activeDrop);
    continueDrops()
      .catch(() => {
        // The drop queue card shows the core's own state of every file.
      })
      .finally(() => setQueueKey((key) => key + 1));
    // Runs once each time a dropped file's flow ends.
  }, [dropFinished]);
  // Back in a dropped file's operation (Choose another folder after Already
  // there): it is that file's flow again.
  const shownOperation = flow.step === "operation" ? flow.operation.operation_id : null;
  useEffect(() => {
    if (shownOperation === null || activeDrop !== null) return;
    const dropId = dropOperations[shownOperation];
    if (dropId === undefined) return;
    setActiveDrop(dropId);
    setDropOutcomes((current) => {
      const next = { ...current };
      delete next[dropId];
      return next;
    });
  }, [shownOperation, activeDrop, dropOperations]);
  // Away from Already there, other than back to that file's operation.
  useEffect(() => {
    const dropId = leftInPlace.current;
    if (dropId === null || flow.step === "in_place") return;
    leftInPlace.current = null;
    if (shownOperation !== null && dropOperations[shownOperation] === dropId) return;
    endDropFlow(dropId);
    // Runs when the step changes; the file to end is kept in the ref.
  }, [flow.step, shownOperation]);
  const activeRoots = page.kind === "loaded" ? page.app.roots.filter((root) => root.status === "active") : [];

  // Dropped files that wait for the user (the queue card counts them), plus
  // the actions the core says need a decision.
  const [dropsNeedingYou, setDropsNeedingYou] = useState(0);
  const ready = page.kind === "loaded" && page.app.storage.status === "ready";
  const needsYou = ready ? dropsNeedingYou + page.app.decisions_needed.length : 0;
  useEffect(() => {
    onNeedsYou?.(needsYou);
  }, [needsYou, onNeedsYou]);

  // Review on an action that needs a decision, in Find or Activity: that
  // card is brought into view (without motion when the system asks for
  // reduced motion) and its title takes the focus.
  useEffect(() => {
    if (!showDecisions || !active || page.kind !== "loaded") return;
    const card = document.getElementById(DECISIONS_ID);
    if (card !== null) {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      card.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
      focusShown(card);
    }
    onDecisionsShown?.();
  }, [showDecisions, active, page.kind, onDecisionsShown]);

  // Activity's Undo… starts the same Undo as a button here: a preview first.
  // One flow at a time: while a file is on its way, the page says so
  // instead, until that flow ends. A check being asked is waited for, while
  // this page shows: a request not started when the user leaves is dropped.
  // `undoWaits`: how many requests came while busy (0: no note). The note is
  // drawn anew for each, so a screen reader says it again.
  const [undoWaits, setUndoWaits] = useState(0);
  // What to bring into view for the latest request: the flow's card (its
  // preview or result) or the note. A new object each time, so a second
  // request while the note shows brings it into view again.
  const [reveal, setReveal] = useState<{ id: string } | null>(null);
  useEffect(() => {
    if (undoActionId === null) return;
    if (!active) {
      onUndoStarted?.();
      return;
    }
    if (!ready || asking) return;
    const busy = !isIdle(flow.step);
    if (busy) setUndoWaits((count) => count + 1);
    else undoById(undoActionId);
    setReveal({ id: busy ? UNDO_WAITS_ID : FLOW_CARD_ID });
    onUndoStarted?.();
    // Runs when Activity asks; the flow and undoById are the current ones.
  }, [undoActionId, active, ready, asking]);
  useEffect(() => {
    if (isIdle(flow.step)) setUndoWaits(0);
  }, [flow.step]);
  // Once the preview, result or note shows (the Undo is asked by then), it
  // is brought into view, without motion when the system asks for reduced
  // motion, and takes the focus: Undo… was on Activity, hidden now. Nothing
  // is kept for later changes of the flow.
  useEffect(() => {
    if (reveal === null || asking) return;
    setReveal(null);
    const card = document.getElementById(reveal.id);
    if (card === null) return;
    const box = card.getBoundingClientRect();
    if (box.top < 0 || box.bottom > window.innerHeight) {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      card.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
    }
    focusShown(card);
  }, [reveal, asking]);

  const flowCard = page.kind === "loaded" && (
    <FlowCard
      flow={flow}
      roots={page.app.roots}
      modelReady={page.app.model === "configured"}
      asking={asking}
      rootChoice={rootChoice}
      setRootChoice={setRootChoice}
      startUndo={startUndo}
      setFlow={setFlow}
      showOperation={showOperation}
      confirm={confirm}
      checkAgain={checkAgain}
      refresh={refresh}
      calm={dropsNeedingYou === 0}
    />
  );

  return (
    <main className="page">
      <header className="page-header">
        <h1 className="focus-target" tabIndex={-1}>
          Organize
        </h1>
        <button type="button" onClick={refresh}>
          Refresh
        </button>
      </header>

      {page.kind === "loading" && <p className="muted">Checking storage…</p>}

      {page.kind === "failed" && (
        <section className="card card-error">
          <h2>Could not read Traeva's state</h2>
          {page.error ? <ErrorDetails error={page.error} /> : <p>{page.detail}</p>}
        </section>
      )}

      {page.kind === "loaded" && page.app.storage.status === "blocked" && (
        <section className="card card-error">
          <h2>Storage blocked</h2>
          <p>Traeva will not start any operation until this is fixed. Your files were not touched.</p>
          <ErrorDetails error={page.app.storage.error} />
        </section>
      )}

      {page.kind === "loaded" && page.app.storage.status === "ready" && (
        <>
          {page.app.roots.length === 0 && <FirstFolderCard onAdded={refresh} />}
          {undoWaits > 0 && (
            <p key={undoWaits} className="warn small flow-note focus-target" id={UNDO_WAITS_ID} role="status" tabIndex={-1}>
              Undo did not start. Finish or cancel this file first, then choose Undo… in Activity again.
            </p>
          )}
          {page.app.roots.length > 0 &&
            (flow.step === "start" ? (
              <>
                <FlowSteps flow={flow} />
                {flowCard}
              </>
            ) : (
              // The file being organized: the one thing on the page now.
              <Now>
                <FlowSteps flow={flow} />
                {flowCard}
              </Now>
            ))}
          <FileSourceSection fileId={sourceFileId(flow)} />
          {checkError && (
            <section className="card card-error">
              <h2>The check did not finish</h2>
              <ErrorDetails error={checkError} />
              <button type="button" onClick={() => setCheckError(null)}>
                Close
              </button>
            </section>
          )}
          <DropQueueCard
            roots={activeRoots}
            idle={idle}
            activeDropId={activeDrop}
            refreshKey={queueKey}
            pageKey={pageKey}
            outcomes={dropOutcomes}
            setOutcome={setDropOutcome}
            onChanged={refresh}
            openOperation={(dropId, operationId) => openDrop(dropId, () => getOperation(operationId))}
            startOperation={(dropId, rootId) => openDrop(dropId, () => startDropOperation(dropId, rootId))}
            undo={undoById}
            checkAgain={checkAgain}
            active={active}
            onShow={onShowDrop}
            onNeedsYou={setDropsNeedingYou}
            decisionCount={page.app.decisions_needed.length}
            decisions={
              page.app.decisions_needed.length > 0 && (
                <Decisions
                  id={DECISIONS_ID}
                  actions={page.app.decisions_needed}
                  roots={page.app.roots}
                  idle={idle}
                  asking={asking}
                  startUndo={startUndo}
                  checkAgain={checkAgain}
                />
              )
            }
          />
        </>
      )}

      {page.kind === "loaded" && (
        <p className="muted small">
          Traeva {page.app.app_version}
          {page.app.storage.status === "ready" && (
            <>
              {" "}
              · {page.app.storage.app_data_source === "test" && <strong className="warn">test data · </strong>}
              database <span className="mono">{page.app.storage.database_path}</span>
            </>
          )}
        </p>
      )}
    </main>
  );
}

// ---------------------------------------------------------------------------
// The organize flow
// ---------------------------------------------------------------------------

type FlowProps = {
  flow: Flow;
  roots: RootView[];
  modelReady: boolean;
  setFlow: SetFlow;
  showOperation: (operation: OperationView) => void;
  confirm: (plan: PlanView, context: PreviewContext | null) => void;
  checkAgain: (action: ActionView) => void;
  refresh: () => void;
  /** Undo or Check again is being asked: nothing new starts meanwhile. */
  asking: boolean;
  rootChoice: RootChoice | null;
  setRootChoice: (choice: RootChoice) => void;
  startUndo: (action: ActionView) => void;
  /** No dropped file waits: Choose file is the page's amber button. */
  calm: boolean;
};

type RootChoice = { source: string; target: string };

/** Where the organize flow is: File · Folder · Preview · Result, the
 * current step marked (flow.ts flowStepIndex). Not shown for an Undo, a
 * failure or an operation that ended. When the flow moves on to a later
 * step while the bar is off screen (the flow card sits under the other
 * cards), the bar and its card are brought into view. */
function FlowSteps({ flow }: { flow: Flow }) {
  const at = flowStepIndex(flow);
  const bar = useRef<HTMLOListElement>(null);
  useEffect(() => {
    const box = bar.current?.getBoundingClientRect();
    if (at === null || at === 0 || box === undefined || box.height === 0) return;
    if (box.top >= 0 && box.bottom <= window.innerHeight) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    bar.current?.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
  }, [at]);
  if (at === null) return null;
  return (
    <ol className="flow-steps" aria-label="Steps" ref={bar}>
      {FLOW_STEPS.map((step, index) => (
        <li
          key={step}
          className={index < at ? "flow-step-done" : index === at ? "flow-step-now" : undefined}
          aria-current={index === at ? "step" : undefined}
        >
          {step}
        </li>
      ))}
    </ol>
  );
}

function FlowCard(props: FlowProps) {
  const { flow, roots, setFlow, confirm, checkAgain, refresh } = props;
  const restart = () => {
    setFlow({ step: "start" });
    refresh();
  };

  switch (flow.step) {
    case "start":
      return (
        <ChooseFileCard
          roots={roots}
          choice={props.rootChoice}
          setChoice={props.setRootChoice}
          disabled={props.asking}
          primary={props.calm}
          setFlow={setFlow}
        />
      );

    case "operation":
      return (
        <OperationCard
          operation={flow.operation}
          modelReady={props.modelReady}
          setFlow={setFlow}
          showOperation={props.showOperation}
          imageChoice={readImageSending}
          targetRootPath={
            roots.find((root) => root.root_id === flow.operation.target_root_id)?.display_path ??
            null
          }
        />
      );

    case "preview":
      // Keyed by plan: a new plan always gets a fresh card, so every button
      // acts on the plan on screen.
      return (
        <PreviewCard
          key={flow.plan.plan_id}
          plan={flow.plan}
          context={flow.context}
          initialError={flow.error ?? null}
          setFlow={setFlow}
          showOperation={props.showOperation}
          confirm={confirm}
        />
      );

    case "applying":
      return (
        <section className="card">
          <h2>{flow.plan.kind === "undo" ? "Putting the file back…" : "Organizing…"}</h2>
          <p>
            {flow.action
              ? `Saved status: ${statusText(flow.action)}`
              : "Confirming the plan…"}
          </p>
          <p className="muted small">
            Success is shown only after the move was checked and recorded. Closing the window does
            not cancel this.
          </p>
        </section>
      );

    case "done":
      return (
        <section className={`card ${actionTone(flow.action)}`} id={FLOW_CARD_ID}>
          <ActionDetails action={flow.action} error={flow.error ?? null} checkAgain={checkAgain} />
          <div className="row">
            <UndoButton
              action={flow.action}
              disabled={props.asking}
              startUndo={props.startUndo}
            />
            <button type="button" onClick={restart}>
              Organize another file
            </button>
          </div>
        </section>
      );

    case "in_place": {
      const operation = flow.operation;
      return (
        <section className="card card-ok">
          <h2>Already there</h2>
          <p>
            <span className="mono">{flow.file.display_path}</span> is already in that folder. Nothing
            to do.
          </p>
          <div className="row">
            {operation && (
              <button
                type="button"
                disabled={props.asking}
                onClick={() =>
                  getOperation(operation.operation_id)
                    .then(props.showOperation)
                    .catch((reason: unknown) => setFlow(failure("Could not go back", reason)))
                }
              >
                Choose another folder
              </button>
            )}
            <button type="button" onClick={restart}>
              Organize another file
            </button>
          </div>
        </section>
      );
    }

    case "failed":
      return (
        <section className="card card-error" id={FLOW_CARD_ID}>
          <h2 className="focus-target" tabIndex={-1}>
            {flow.title}
          </h2>
          {flow.error ? <ErrorDetails error={flow.error} /> : <p>{flow.detail}</p>}
          <button type="button" onClick={restart}>
            Start over
          </button>
        </section>
      );
  }
}

function ChooseFileCard(props: {
  roots: RootView[];
  choice: RootChoice | null;
  setChoice: (choice: RootChoice) => void;
  disabled: boolean;
  primary: boolean;
  setFlow: SetFlow;
}) {
  const { roots, setChoice, setFlow } = props;
  // Never null here: the card is shown only when there is a folder.
  const { source: sourceId, target: targetId } = pickRoots(roots, props.choice) ?? {
    source: "",
    target: "",
  };

  const choose = () => {
    chooseFile(sourceId, targetId)
      .then((operation) => {
        if (operation) setFlow({ step: "operation", operation });
      })
      .catch((reason: unknown) => setFlow(failure("Could not start", reason)));
  };

  // One line to start from (#37, the student's choice B): with a single
  // authorized folder there is nothing to choose, so it is only named.
  const single = roots.length === 1 ? roots[0] : null;
  return (
    <section className="card start-bar">
      <div className="start-bar-text">
        <h2>Organize a file</h2>
        <p className="muted small">Or drop files on Traeva's icon or the desktop pet.</p>
      </div>
      {single ? (
        <span className="muted small start-bar-place" title={single.display_path}>
          in <span className="exact">{lastFolder(single.display_path)}</span>
        </span>
      ) : (
        <>
          <label className="start-bar-field">
            From
            <RootSelect
              roots={roots}
              value={sourceId}
              onChange={(id) => setChoice({ source: id, target: targetId })}
            />
          </label>
          <label className="start-bar-field">
            Into
            <RootSelect
              roots={roots}
              value={targetId}
              onChange={(id) => setChoice({ source: sourceId, target: id })}
            />
          </label>
        </>
      )}
      <button
        type="button"
        className={props.primary ? "primary" : undefined}
        onClick={choose}
        disabled={props.disabled}
      >
        Choose file…
      </button>
    </section>
  );
}

/** The last folder of a path: "School" for /Users/x/School. */
function lastFolder(path: string): string {
  return path.split("/").filter((part) => part !== "").pop() ?? path;
}

function RootSelect(props: { roots: RootView[]; value: string; onChange: (id: string) => void }) {
  return (
    <select value={props.value} onChange={(event) => props.onChange(event.target.value)}>
      {props.roots.map((root) => (
        <option key={root.root_id} value={root.root_id}>
          {root.display_path}
        </option>
      ))}
    </select>
  );
}

function PreviewCard(props: {
  plan: PlanView;
  context: PreviewContext | null;
  initialError: AppError | null;
  setFlow: SetFlow;
  showOperation: (operation: OperationView) => void;
  confirm: (plan: PlanView, context: PreviewContext | null) => void;
}) {
  const { plan, context, setFlow, confirm } = props;
  const [current, setCurrent] = useState(plan);
  const [error, setError] = useState<AppError | null>(props.initialError);

  // The status is worked out by the core when read; ask again now and then.
  useEffect(() => {
    const timer = setInterval(() => {
      getPlan(plan.plan_id)
        .then(setCurrent)
        .catch(() => {});
    }, 5000);
    return () => clearInterval(timer);
  }, [plan.plan_id]);

  // Within an operation, cancelling ends the operation and its plan; a plan
  // made outside one (an Undo) is cancelled on its own.
  const cancel = () => {
    const done = context
      ? cancelOperation(context.operation.operation_id)
      : cancelPlan(plan.plan_id);
    done
      .then(() => setFlow({ step: "start" }))
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null));
  };
  // Back to the operation to choose another target. This preview's plan is
  // closed first, so only the plan on screen can ever be confirmed.
  const back = (operationId: string) => {
    setError(null);
    cancelPlan(current.plan_id)
      .then(() => getOperation(operationId))
      .then(props.showOperation)
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null));
  };

  const file = context?.operation.file ?? null;
  const answers = context?.operation.answered.map((answered) => answered.answer) ?? [];
  return (
    <section className="card" id={FLOW_CARD_ID}>
      <h2 className="focus-target" tabIndex={-1}>
        {current.kind === "undo" ? "Preview: put the file back" : "Preview"}
      </h2>
      <MoveRoute
        name={current.basename}
        from={current.source.display_path}
        to={current.target.display_path}
        newFolders={current.folders_to_create}
      />
      <dl>
        {current.user_choice && (
          <>
            <dt>Chosen</dt>
            <dd>{CHOICE_TEXT[current.user_choice]}</dd>
          </>
        )}
        {context?.candidate && (
          <>
            <dt>Why (the model)</dt>
            <dd>{context.candidate.reason}</dd>
          </>
        )}
        {answerNote(answers, current.user_choice) && (
          <>
            <dt>{answers.length === 1 ? "Your answer" : "Your answers"}</dt>
            <dd>{answerNote(answers, current.user_choice)}</dd>
          </>
        )}
        {file && (
          <>
            <dt>Text read</dt>
            <dd className="small">
              <ExtractionText file={file} />
            </dd>
          </>
        )}
      </dl>
      <details className="small">
        <summary>Details</summary>
        <dl>
          <dt>Valid until</dt>
          <dd>{new Date(current.expires_at_ms).toLocaleTimeString()}</dd>
        </dl>
      </details>
      {current.reason && <p className="warn">{current.reason}</p>}
      <p className="muted small">Nothing has changed in your folders yet.</p>
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={() => confirm(current, context)}
          disabled={!current.can_confirm}
        >
          {current.kind === "undo" ? "Put it back" : "Move file"}
        </button>
        {context && (
          <button type="button" onClick={() => back(context.operation.operation_id)}>
            Choose another folder
          </button>
        )}
        <button type="button" onClick={cancel}>
          Cancel
        </button>
      </div>
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

const CHOICE_TEXT = {
  accept: "the recommended folder",
  correct: "another folder than the recommended one",
  manual: "by hand",
} as const;

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/** The actions that need a decision, under Needs you after the dropped
 * files that wait (#37, the student's choice B): each with its folders,
 * Undo when the core offers it, and Check again, which only reads its
 * record. */
function Decisions(props: {
  id: string;
  actions: ActionView[];
  /** The authorized folders now: an action's folder missing here was removed. */
  roots: RootView[];
  /** False while a file is on its way: one flow at a time. */
  idle: boolean;
  asking: boolean;
  startUndo: (action: ActionView) => void;
  checkAgain: (action: ActionView) => void;
}) {
  return (
    <div className="decisions" id={props.id}>
      <h3 className="decisions-title focus-target" tabIndex={-1}>
        Needs your decision
      </h3>
      {/* Each waits for the user: all show, never behind Show more. */}
      <ul className="find-rows drop-rows" role="list">
            {props.actions.map((action) => (
              <li key={action.action_id} className="autodrop-item group-row-attention row">
                <span className={`badge ${toneClass(actionText(action.status, action.kind).tone)}`}>
                  {statusText(action)}
                </span>
                <span className="grow small">
                  <span className="mono">{action.source.relative_path}</span> →{" "}
                  <span className="mono">{action.target.relative_path}</span>
                  <br />
                  <span className="muted">
                    {actionRoots(action, props.roots).map((root, index) => (
                      <span key={index}>
                        {index === 0 ? "in " : " into "}
                        <span className="mono">{root.path}</span>
                        {root.removed && <strong className="warn"> (removed from Traeva)</strong>}
                      </span>
                    ))}
                  </span>
                </span>
                <UndoButton action={action} disabled={!props.idle} startUndo={props.startUndo} />
                {action.status === "needs_user_decision" && (
                  <button type="button" onClick={() => props.checkAgain(action)} disabled={props.asking}>
                    Check again
                  </button>
                )}
              </li>
            ))}
      </ul>
    </div>
  );
}

/** Undo for an organize action: offered when the core says it is
 * available; shown disabled with the reason when its folder was removed
 * (#79); otherwise not shown. Clicking only asks for a preview. */
function UndoButton(props: {
  action: ActionView;
  disabled: boolean;
  startUndo: (action: ActionView) => void;
}) {
  const { action } = props;
  const note = undoNote(action.undo_status);
  if (undoOffered(action.undo_status)) {
    return (
      <button type="button" onClick={() => props.startUndo(action)} disabled={props.disabled}>
        Undo…
      </button>
    );
  }
  if (note === null) return null;
  return (
    <span title={note.full}>
      <button type="button" disabled>
        Undo…
      </button>{" "}
      <span className="small muted">{note.short}</span>
    </span>
  );
}

function ActionDetails(props: {
  action: ActionView;
  /** The error the run ended with, if any. */
  error: AppError | null;
  checkAgain: (action: ActionView) => void;
}) {
  const { action, error } = props;
  // Still unsettled after the run ended with an error (e.g. the result could
  // not be recorded): the core can check it again.
  const unsettled = action.status === "prepared" || action.status === "running";
  return (
    <>
      <h2 className="focus-target" tabIndex={-1}>
        {statusText(action)}
      </h2>
      <dl>
        <dt>From</dt>
        <dd className="mono">{action.source.display_path}</dd>
        <dt>To</dt>
        <dd className="mono">{action.target.display_path}</dd>
        {action.folders_created.length > 0 && (
          <>
            <dt>Folders created</dt>
            <dd className="mono">{action.folders_created.join(", ")}</dd>
          </>
        )}
        {action.folders_removed.length > 0 && (
          <>
            <dt>Empty folders removed</dt>
            <dd className="mono">{action.folders_removed.join(", ")}</dd>
          </>
        )}
      </dl>
      {action.folders_kept.length > 0 && (
        <p className="warn">
          The file is back. These folders Traeva created were kept because they are not empty or
          changed: <span className="mono">{action.folders_kept.join(", ")}</span>
        </p>
      )}
      {action.folders_without_receipt.length > 0 && (
        <p className="warn">
          Traeva has no receipt for these folders, so it never touches them:{" "}
          <span className="mono">{action.folders_without_receipt.join(", ")}</span>
        </p>
      )}
      {action.problem && <ErrorDetails error={action.problem} />}
      {error && !action.problem && <ErrorDetails error={error} />}
      {(action.status === "needs_user_decision" || (error && unsettled)) && (
        <button type="button" onClick={() => props.checkAgain(action)}>
          Check again
        </button>
      )}
    </>
  );
}

function statusText(action: ActionView): string {
  switch (action.status) {
    case "prepared":
    case "running":
      return "Working…";
    case "committed":
      return action.kind === "undo" ? "Put back" : "Organized";
    case "failed_no_move":
      return "Not moved";
    case "needs_user_decision":
      return "Needs your decision";
    case "undone":
      return "Undone";
  }
}

/** A result card's tone: green for a done move or Undo; otherwise plain
 * (a status is never red; what waits for the user says so in words). */
function actionTone(action: ActionView): string {
  return action.status === "committed" || action.status === "undone" ? "card-ok" : "";
}

/** What a dropped file's flow ended with, for the drop queue card. A
 * confirmed plan is recorded by the core itself (contract v27 §4.8). */
function outcomeOf(flow: Flow): DropOutcome | null {
  switch (flow.step) {
    case "in_place":
      return { kind: "in_place" };
    case "start":
      return { kind: "given_up" };
    default:
      return null;
  }
}
