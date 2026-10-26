// Files dropped on Traeva's icon in the Dock (#168, organize-spec.md §13.7):
// each file's state as the core reports it, and the choice it waits for.
// The queue is read when the page starts (a drop can be what opened
// Traeva), whenever the window comes to the front or the page is read
// again, about every second while a file waits or is worked on, and now and
// then otherwise. A file with an action (moved, confirmed in the usual flow,
// or stopped after its move started) shows that action as saved. Whether a
// file moves is never decided here. Finished files can be cleared from the
// card for this run (#37, 10/14); Activity still lists them.
//
// The files show in Organize's groups (#37, the student's choice B, 10/14):
// those that wait for the user under Needs you, first and always open, with
// the actions that need a decision (`decisions`, from the page); those
// Traeva works on, or whose move is not settled, under In progress; the
// rest under Finished, with Clear finished.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { answerClarification, cancelOperation, getAction, getOperation, getPlan } from "../organize/api";
import { answerFor, undoNote, undoOffered } from "../organize/flow";
import { QuestionCard } from "../organize/OperationCard";
import { Group } from "../../shared/Group";
import { statusSummary } from "../../shared/status";
import { ShowMore } from "../../shared/ShowMore";
import { ErrorDetails } from "../organize/shared";
import {
  isAppError,
  type ActionView,
  type AppError,
  type ClarificationAnswer,
  type OperationView,
  type RootView,
} from "../organize/types";
import { authorizeDropSource, cancelDrop, continueDrops, getDropQueue } from "./api";
import { clearedIds, clearRows } from "./cleared";
import {
  dropBadge,
  dropPlace,
  droppedAtText,
  finishedIds,
  hasOpenAfter,
  holdsRow,
  answersShownQuestion,
  itemToShow,
  needsYou,
  needsYouCount,
  pollInterval,
  queueSignature,
  toneClass,
  validRootId,
  waitsForUser,
  type AnswerState,
  type DropAnswer,
  type DropOutcome,
} from "./queue";
import type { DropItemView, DropQueueView, DropState } from "./types";
import "./autodrop.css";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

type Props = {
  /** The authorized folders that are active now ("Into" when the mode is off). */
  roots: RootView[];
  /** Nothing else is on its way in Organize: one flow at a time. */
  idle: boolean;
  /** The dropped file now in the Organize flow (Now, above), if any. */
  activeDropId: string | null;
  /** Bumped by the page to read the queue at once. */
  refreshKey: number;
  /** Bumped whenever the page was read again: the queue and results are read again. */
  pageKey: number;
  /** What became of handed-over files in the Organize flow without a confirmed plan. */
  outcomes: Record<string, DropOutcome>;
  setOutcome: (dropId: string, outcome: DropOutcome) => void;
  /** A file changed state: the page reads its lists again. */
  onChanged: () => void;
  /** Goes on with a handed-over file's operation in the usual flow. */
  openOperation: (dropId: string, operationId: string) => void;
  /** The mode is off: starts the usual flow for the file, into this folder. */
  startOperation: (dropId: string, targetRootId: string) => void;
  /** Asks for the Undo preview of a moved file's action. */
  undo: (actionId: string) => void;
  /** Reads an unsettled action's record again (reconcile_action). */
  checkAgain: (action: ActionView) => void;
  /** The Organize tab is showing. */
  active: boolean;
  /** Asks for the Organize tab: a file to see came while another tab shows. */
  onShow?: () => void;
  /** How many dropped files wait for the user now (the sidebar's count). */
  onNeedsYou?: (count: number) => void;
  /** The actions that need a decision, shown under Needs you after the
   * dropped files that wait, and how many there are. */
  decisions: ReactNode;
  decisionCount: number;
};



export function DropQueueCard(props: Props) {
  const { onChanged, refreshKey, pageKey, outcomes, idle } = props;
  const [queue, setQueue] = useState<DropQueueView | null>(null);
  const [readError, setReadError] = useState<AppError | null>(null);
  // Kept until the next action, so a refusal is not cleared by a poll.
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  // The latest answer sent from this card per dropped file, with its question.
  const [answered, setAnswered] = useState<Record<string, DropAnswer>>({});
  const shown = useRef<string | null>(null);
  // Finished files the user cleared from this card (kept outside it, for
  // this run), and rows still reading their result or whose move is not
  // settled, which cannot be cleared yet.
  const [cleared, setCleared] = useState<ReadonlySet<string>>(clearedIds);
  const [held, setHeld] = useState<ReadonlySet<string>>(() => new Set());
  const hold = useCallback((dropId: string, holds: boolean) => {
    setHeld((current) => {
      if (current.has(dropId) === holds) return current;
      const next = new Set(current);
      if (holds) next.add(dropId);
      else next.delete(dropId);
      return next;
    });
  }, []);

  const read = useCallback(() => {
    getDropQueue().then(
      (read) => {
        setQueue(read);
        setReadError(null);
      },
      (reason: unknown) => setReadError(asError(reason)),
    );
  }, []);

  const interval = pollInterval(queue);
  useEffect(() => {
    read();
    const timer = setInterval(read, interval);
    window.addEventListener("focus", read);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", read);
    };
  }, [read, interval, refreshKey, pageKey]);

  // A moved, authorized or stopped file changes the page's own lists.
  useEffect(() => {
    if (queue === null) return;
    const signature = queueSignature(queue);
    if (shown.current !== null && shown.current !== signature) onChanged();
    shown.current = signature;
  }, [queue, onChanged]);

  const { onNeedsYou } = props;
  useEffect(() => {
    if (queue !== null) onNeedsYou?.(needsYouCount(queue.items, outcomes));
  }, [queue, outcomes, onNeedsYou]);

  // A file dropped while the page is scrolled elsewhere, or one that comes
  // to need the user, is brought into view (without motion when the system
  // asks for reduced motion). Not while a flow is open in Organize or this
  // card is hidden (another tab: no boxes). A file counts as seen only once
  // it could be shown: when the flow ends, at the first read of the queue
  // after the user is back, or when the user clicks in this card. Behind
  // another tab (Find is the first page, #198), such a file asks once for
  // the Organize tab instead; the scroll follows when the tab shows.
  const card = useRef<HTMLDivElement>(null);
  const seenStates = useRef(new Map<string, DropState>());
  const raisedStates = useRef(new Map<string, DropState>());
  const { active, onShow } = props;
  useEffect(() => {
    if (queue === null) return;
    const raise = itemToShow(queue.items, raisedStates.current);
    raisedStates.current = new Map(queue.items.map((item) => [item.drop_id, item.state]));
    const visible = active && card.current !== null && card.current.getClientRects().length > 0;
    if (!visible) {
      if (raise !== null) onShow?.();
      return;
    }
    if (!idle) return;
    const target = itemToShow(queue.items, seenStates.current);
    seenStates.current = new Map(queue.items.map((item) => [item.drop_id, item.state]));
    if (target === null) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document.getElementById(`drop-${target}`)?.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
  }, [queue, idle, active, onShow]);

  // One call at a time; each ends with the queue as it is now.
  const act = (call: () => Promise<DropQueueView>) => {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    call()
      .then(setQueue)
      .catch((reason: unknown) => setActionError(asError(reason)))
      .finally(() => setBusy(false));
  };

  // The answer starts the model's next look; the queue's next pass waits
  // for it (contract v27 §4.8). The question stays on screen until the core
  // takes the answer: a refused answer keeps it, with what was chosen or
  // typed; a question whose answer was sent is not asked for again, but the
  // next question (up to 3, organize-spec §15) is.
  const mark = (dropId: string, state: AnswerState) =>
    setAnswered((current) => {
      const now = current[dropId];
      return now === undefined ? current : { ...current, [dropId]: { ...now, state } };
    });
  const answer = (dropId: string, operationId: string, reply: ClarificationAnswer) => {
    if (answered[dropId]?.questionId === reply.question_id) return;
    setAnswered((current) => ({
      ...current,
      [dropId]: { questionId: reply.question_id, state: "sending" },
    }));
    setActionError(null);
    answerClarification(operationId, reply).then(
      () => {
        mark(dropId, "sent");
        return continueDrops().then(setQueue, (reason: unknown) => {
          setActionError(asError(reason));
          mark(dropId, "failed");
        });
      },
      (reason: unknown) => {
        setActionError(asError(reason));
        setAnswered((current) => {
          const next = { ...current };
          delete next[dropId];
          return next;
        });
      },
    );
  };

  // The answer was sent; only the queue's next pass failed to start.
  const goOn = (dropId: string) =>
    act(() =>
      continueDrops().then((next) => {
        mark(dropId, "sent");
        return next;
      }),
    );

  // A cancel also ends the files dropped with it, after it. Later drops are
  // not reached and go on (contract v29 §4.8): the queue is started again,
  // as no pass runs while it waits for this file. While a flow is open in
  // Organize, only once it ends (a call next to the page's own is turned
  // away by the core, which then looks once more).
  const resumeAfterFlow = useRef(false);
  const cancel = (dropId: string) =>
    act(async () => {
      const after = await cancelDrop(dropId);
      if (idle) return continueDrops();
      resumeAfterFlow.current = true;
      return after;
    });
  useEffect(() => {
    if (!idle || busy || !resumeAfterFlow.current) return;
    resumeAfterFlow.current = false;
    act(() => continueDrops());
    // Runs when the flow ends or this card's last call does.
  }, [idle, busy]);

  // The rows still on the card, each with its place in the queue.
  const rows = (queue?.items ?? [])
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => !cleared.has(item.drop_id));

  const readFailed = readError && (
    <section className="card card-error">
      <h2>Dropped files could not be read</h2>
      <ErrorDetails error={readError} />
    </section>
  );
  if (queue === null || rows.length === 0) {
    return (
      <div ref={card}>
        {props.decisionCount > 0 && (
          <Group id="organize-needs" title="Needs you" count={props.decisionCount} attention>
            {props.decisions}
          </Group>
        )}
        {readFailed}
      </div>
    );
  }

  // Files still wait, nothing works on them and none waits for the user:
  // the queue can be started again, once no flow is open in Organize (a
  // cancel made while one was open goes on by itself when it ends).
  const firstWaiting = queue.items.find((item) => item.state === "waiting") ?? null;
  const stalled =
    firstWaiting !== null &&
    !queue.running &&
    !waitsForUser(queue.items, outcomes) &&
    props.activeDropId === null &&
    idle;

  // The user is looking at the card: its rows as they are now count as seen,
  // so a row this click changes (the rest of a cancelled drop) is not
  // scrolled to later.
  const lookedAt = () => {
    for (const item of queue.items) seenStates.current.set(item.drop_id, item.state);
  };
  // The first file that waits for the user gets the page's one amber
  // button, unless a flow is open above (its step has the amber button).
  const primaryRow = rows.find(({ item }) => needsYou(item, outcomes))?.index ?? -1;

  // Finished rows can be cleared; the focus goes to the page's title.
  const finished = finishedIds(
    rows.map(({ item }) => item),
    outcomes,
    held,
    props.activeDropId,
  );
  const finishedSet = new Set(finished);
  const clearFinished = () => {
    setCleared(clearRows(finished));
    card.current?.closest("main")?.querySelector<HTMLElement>("h1")?.focus();
  };
  const someCleared = queue.items.some((item) => cleared.has(item.drop_id));

  // The groups, from the queue's own view only (a row's own reading would
  // move it between groups, and a moved row starts its reading over): what
  // waits for the user; what Traeva works on, or whose move is not settled;
  // the rest.
  type Placed = { item: DropItemView; index: number };
  const waiting: Placed[] = [];
  const working: Placed[] = [];
  const done: Placed[] = [];
  for (const row of rows) {
    const place = dropPlace(row.item, outcomes);
    (place === "needs" ? waiting : place === "working" ? working : done).push(row);
  }
  const list = (placed: Placed[], attention: boolean) => (
    <ul className="find-rows drop-rows" role="list">
      {placed.map(({ item, index }) => (
        <DropRow
          key={item.drop_id || `working-${index}`}
          item={item}
          attention={attention}
          primary={index === primaryRow && idle}
          laterOpen={hasOpenAfter(queue.items, index)}
          busy={busy}
          answered={answered[item.drop_id] ?? null}
          props={props}
          act={act}
          answer={answer}
          goOn={goOn}
          cancel={cancel}
          hold={hold}
        />
      ))}
    </ul>
  );
  const needs = waiting.length + props.decisionCount;

  return (
    <div ref={card} onClickCapture={lookedAt}>
      {needs > 0 && (
        <Group id="organize-needs" title="Needs you" count={needs} attention>
          {waiting.length > 0 && list(waiting, true)}
          {props.decisions}
        </Group>
      )}
      {/* Never inside a group that can close: the queue waits for this. */}
      {stalled && (
        <div className="row drop-stalled">
          <span className="muted small grow">Some files are still waiting.</span>
          <button type="button" onClick={() => act(() => continueDrops())} disabled={busy}>
            Go on with the files
          </button>
        </div>
      )}
      {working.length > 0 && (
        <Group
          id="organize-working"
          title="In progress"
          count={working.length}
          action={queue.running && <span className="muted small">Working…</span>}
        >
          {list(working, false)}
        </Group>
      )}
      {done.length > 0 && (
        <Group
          id="organize-finished"
          title="Finished"
          count={done.length}
          summary={statusSummary(
            done.map(({ item }) => ({
              badge: dropBadge(
                item,
                item.action_status === null ? null : { status: item.action_status, kind: "organize" },
                outcomes[item.drop_id] ?? null,
              ),
            })),
          )}
          note="Dropped since Traeva started. Clearing only hides them here; Activity still lists them."
          action={
            finished.length > 0 && (
              <button type="button" onClick={clearFinished}>
                Clear finished
              </button>
            )
          }
        >
          {/* Rows that cannot be cleared yet (reading their result, a move
              not settled) always show; the cleared-able ones 3 at first. */}
          <ShowMore items={done} first={3} keep={({ item }) => !finishedSet.has(item.drop_id)}>
            {(visible) => list(visible, false)}
          </ShowMore>
        </Group>
      )}
      {someCleared && done.length === 0 && (
        <p className="muted small drop-cleared">Cleared files are still listed in Activity.</p>
      )}
      {actionError && <ErrorDetails error={actionError} />}
      {readError && <p className="warn small">The list could not be read just now; it shows the last state read.</p>}
    </div>
  );
}

/** A saved action, read again whenever the page is. */
function useAction(actionId: string | null, pageKey: number): ActionView | null {
  const [action, setAction] = useState<ActionView | null>(null);
  useEffect(() => {
    if (actionId === null) {
      setAction(null);
      return;
    }
    let alive = true;
    getAction(actionId).then(
      (read) => {
        if (alive) setAction(read);
      },
      () => {
        // The row keeps what it had; Activity shows the record too.
      },
    );
    return () => {
      alive = false;
    };
  }, [actionId, pageKey]);
  return action;
}

/** The action of a plan, read from the plan: undefined while unknown. */
function usePlanAction(planId: string | null): string | null | undefined {
  const [actionId, setActionId] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    setActionId(undefined);
    if (planId === null) return;
    let alive = true;
    getPlan(planId).then(
      (plan) => {
        if (alive) setActionId(plan.action_id);
      },
      () => {
        // Unknown: the row then claims nothing about a move.
      },
    );
    return () => {
      alive = false;
    };
  }, [planId]);
  return actionId;
}

type RowArgs = {
  item: DropItemView;
  /** It waits for the user (an amber edge). */
  attention: boolean;
  /** Its next step is the page's one amber button. */
  primary: boolean;
  /** A file after this one is not finished yet. */
  laterOpen: boolean;
  busy: boolean;
  /** The latest answer sent from this card, with the question it answered. */
  answered: DropAnswer | null;
  props: Props;
  act: (call: () => Promise<DropQueueView>) => void;
  answer: (dropId: string, operationId: string, reply: ClarificationAnswer) => void;
  /** After a sent answer, starts the queue again. */
  goOn: (dropId: string) => void;
  cancel: (dropId: string) => void;
  /** Tells the card whether this row may not be cleared yet. */
  hold: (dropId: string, holds: boolean) => void;
};

function DropRow(args: RowArgs) {
  const { item, props } = args;
  // A stopped or cancelled file with a plan but no action: its plan says
  // whether a move started.
  const askPlan =
    item.action_id === null && item.plan_id !== null && (item.state === "stopped" || item.state === "cancelled");
  const planAction = usePlanAction(askPlan ? item.plan_id : null);
  const actionId = item.action_id ?? (askPlan ? (planAction ?? null) : null);
  const loaded = useAction(actionId, props.pageKey);
  // An action of another plan is not this file's move.
  const action = loaded && item.plan_id !== null && loaded.plan_id !== item.plan_id ? null : loaded;
  const notMoved = action === null && (item.plan_id === null || (askPlan && planAction === null) || loaded !== null);
  const outcome =
    item.state === "handed_over" && actionId === null ? (props.outcomes[item.drop_id] ?? null) : null;
  const badge = dropBadge(item, action, outcome);

  // Still reading its result, or its move is not settled: not cleared yet.
  const reading = (askPlan && planAction === undefined) || (actionId !== null && loaded === null);
  const holds = holdsRow(reading, action);
  const { hold } = args;
  useEffect(() => {
    if (item.drop_id !== "") hold(item.drop_id, holds);
  }, [hold, item.drop_id, holds]);

  let details: ReactNode;
  if (action !== null) {
    details = (
      <>
        {item.state === "stopped" && item.reason && <p className="small">{item.reason}</p>}
        <ActionResult
          action={action}
          automatic={item.state === "moved" && item.automatic}
          stopped={item.state === "stopped"}
          idle={props.idle}
          undo={props.undo}
          checkAgain={props.checkAgain}
        />
      </>
    );
  } else if (actionId !== null && loaded === null) {
    details = <p className="muted small">Reading its result…</p>;
  } else if (outcome !== null) {
    details = <OutcomeText outcome={outcome} />;
  } else {
    details = <DropDetails {...args} notMoved={notMoved} />;
  }

  return (
    <li
      className={args.attention ? "autodrop-item group-row-attention" : "autodrop-item"}
      id={item.drop_id ? `drop-${item.drop_id}` : undefined}
    >
      <div className="row">
        <span className="drop-name grow exact">{item.file_name || "(the file Traeva is working on)"}</span>
        <span className="muted small">dropped {droppedAtText(item.received_at_ms, Date.now())}</span>
        <span className={`badge ${toneClass(badge.tone)}`}>{badge.label}</span>
      </div>
      {details}
    </li>
  );
}

/** A file whose move started: from where to where, and its Undo or check. */
function ActionResult(props: {
  action: ActionView;
  automatic: boolean;
  /** The file stopped after its move started: the result was not recorded. */
  stopped: boolean;
  idle: boolean;
  undo: (actionId: string) => void;
  checkAgain: (action: ActionView) => void;
}) {
  const { action } = props;
  const unsettled = action.status === "prepared" || action.status === "running";
  const note = undoNote(action.undo_status);
  return (
    <>
      <dl className="small">
        <dt>From</dt>
        <dd className="mono">{action.source.display_path}</dd>
        <dt>To</dt>
        <dd className="mono">{action.target.display_path}</dd>
      </dl>
      {props.automatic && action.status === "committed" && (
        <p className="muted small">Moved automatically, without a preview, as you agreed.</p>
      )}
      {action.status === "undone" && <p className="muted small">Put back with Undo.</p>}
      {unsettled &&
        (props.stopped ? (
          <>
            <p className="small">
              The move started, but its result was not recorded, so the file may have moved. Check again
              reads its record now; it never moves the file.
            </p>
            <div className="row">
              <button type="button" onClick={() => props.checkAgain(action)}>
                Check again
              </button>
            </div>
          </>
        ) : (
          <p className="muted small">The file is being moved…</p>
        ))}
      {action.status === "needs_user_decision" && (
        <p className="small">Traeva could not confirm how this move ended. See Needs your decision.</p>
      )}
      {action.status === "failed_no_move" && <p className="muted small">The file was not moved.</p>}
      {undoOffered(action.undo_status) ? (
        <div className="row">
          <button type="button" onClick={() => props.undo(action.action_id)} disabled={!props.idle}>
            Undo…
          </button>
          {!props.idle && <span className="muted small">Finish or cancel the file in Organize first.</span>}
        </div>
      ) : (
        note && (
          <p className="muted small" title={note.full}>
            {note.short}
          </p>
        )
      )}
    </>
  );
}

function OutcomeText({ outcome }: { outcome: DropOutcome }) {
  switch (outcome.kind) {
    case "in_place":
      return <p className="muted small">It is already in that folder. Nothing was moved.</p>;
    case "given_up":
      return <p className="muted small">Not organized. Nothing was moved.</p>;
  }
}

function DropDetails(args: RowArgs & { notMoved: boolean }) {
  const { item, laterOpen, busy, props, act, notMoved } = args;
  const id = item.drop_id;
  const inFlow = props.activeDropId === id;
  const waitForFlow = !props.idle && !inFlow && (
    <p className="muted small">Finish or cancel the file in Organize first.</p>
  );
  const cancelThis = () => args.cancel(id);
  const sending = args.answered?.state === "sending";
  const cancelButton = (disabled: boolean) => (
    <button type="button" onClick={cancelThis} disabled={disabled}>
      Cancel this file
    </button>
  );
  // It does not touch the flow in Organize, so it waits only for this card.
  const cancel = cancelButton(busy || sending);
  const cancelNote = laterOpen && (
    <p className="muted small">The files dropped together with it, after it, are then not processed.</p>
  );
  const reason = item.reason && <p className="small">{item.reason}</p>;

  switch (item.state) {
    case "waiting":
      return (
        <>
          <div className="row">
            <span className="muted small grow">Waiting for its turn.</span>
            {cancel}
          </div>
          {cancelNote}
        </>
      );

    case "working":
      return <p className="muted small">Traeva is reading it and looking for its folder.</p>;

    case "needs_source_authorization":
      return (
        <>
          <p className="small">This file is in a folder Traeva may not use yet:</p>
          <p className="mono exact">{item.folder_to_authorize}</p>
          <p className="muted small">
            If you allow it, Traeva may read and organize the files in this folder and its subfolders,
            the same as for a folder you add yourself. You can remove it later in Settings, under
            Folders Traeva may organize. It is not added to the folders for automatic organizing.
            Nothing in it is read before you allow it.
          </p>
          <div className="row">
            <button
              type="button"
              className={args.primary ? "primary" : undefined}
              onClick={() => act(() => authorizeDropSource(id))}
              disabled={busy}
            >
              Allow this folder
            </button>
            {cancel}
          </div>
          {cancelNote}
        </>
      );

    case "needs_answer":
      // Only files of the automatic mode ask here (contract v27 §4.8).
      return (
        <DropQuestion
          item={item}
          answered={args.answered}
          primary={args.primary}
          onAnswer={(operationId, reply) => args.answer(id, operationId, reply)}
          sentOnly={
            <p className="muted small">
              Answer sent. Traeva asks the model again with your answers, then goes on with this file.
            </p>
          }
          failedOnly={
            <>
              <p className="warn small">Your answer was sent, but Traeva could not go on with the files.</p>
              <div className="row">
                <button type="button" onClick={() => args.goOn(id)} disabled={busy}>
                  Go on with the files
                </button>
                {cancel}
              </div>
            </>
          }
          footer={
            <>
              <div className="row">{cancel}</div>
              {cancelNote}
            </>
          }
        />
      );

    case "handed_over": {
      // With unfinished files after it, it can be skipped (its operation
      // ends and the queue goes on) or cancelled with the rest; otherwise it
      // is cancelled, which the core records (contract v27 §4.8). Nothing
      // here runs while a file is on its way in Organize.
      const locked = busy || !props.idle;
      const giveUp = laterOpen ? (
        <>
          <button
            type="button"
            disabled={locked}
            onClick={() =>
              act(async () => {
                if (item.operation_id) await cancelOperation(item.operation_id);
                props.setOutcome(id, { kind: "given_up" });
                return continueDrops();
              })
            }
          >
            Skip this file
          </button>
          <button type="button" onClick={cancelThis} disabled={locked}>
            Cancel this and the rest
          </button>
        </>
      ) : (
        cancelButton(locked)
      );
      return (
        <>
          {reason}
          {inFlow ? (
            <p className="muted small">Continue in the card at the top, under Now.</p>
          ) : item.operation_id ? (
            <div className="row">
              <button
                type="button"
                className={args.primary ? "primary" : undefined}
                onClick={() => props.openOperation(id, item.operation_id as string)}
                disabled={locked}
              >
                Organize this file
              </button>
              {giveUp}
            </div>
          ) : (
            <StartUsualFlow
              roots={props.roots}
              primary={args.primary}
              disabled={locked}
              onStart={(rootId) => props.startOperation(id, rootId)}
              giveUp={giveUp}
            />
          )}
          {waitForFlow}
        </>
      );
    }

    case "moved":
      // A moved file has its action; this shows only until it is read.
      return <p className="muted small">Reading its result…</p>;

    case "already_in_place":
      return <p className="muted small">It is already in that folder. Nothing was moved.</p>;

    case "stopped":
    case "cancelled":
      // A file whose move started is shown by ActionResult instead. A stop
      // names its next step (organize-spec §13.7), unless no operation
      // started: a folder, a link, or a folder Traeva cannot use now, would
      // stop the same way again.
      return (
        <>
          {reason}
          {notMoved && (
            <p className="muted small">
              Nothing was moved.
              {item.state === "stopped" && item.operation_id !== null && " Drop it again, or choose it in Organize."}
            </p>
          )}
        </>
      );

    case "not_processed":
      return (
        <p className="muted small">
          Not processed, because a file dropped with it, before it, stopped or was cancelled. Drop it
          again, or choose it in Organize.
        </p>
      );
  }
}

/** The automatic mode's current question for a dropped file, as in Organize.
 * Read again with every read of the queue: after an answer the core may ask
 * the next question (up to 3, organize-spec §15) while the file still waits. */
function DropQuestion(props: {
  item: DropItemView;
  /** The latest answer sent from this card, with the question it answered. */
  answered: DropAnswer | null;
  primary: boolean;
  onAnswer: (operationId: string, reply: ClarificationAnswer) => void;
  /** Shown instead of the question once its answer was sent. */
  sentOnly: ReactNode;
  /** Shown instead of the question when the answer was sent but the queue did not go on. */
  failedOnly: ReactNode;
  footer: ReactNode;
}) {
  const { item } = props;
  const operationId = item.operation_id;
  const [operation, setOperation] = useState<OperationView | null>(null);
  const [error, setError] = useState<AppError | null | undefined>(undefined);
  // Bumped to read the question again after a failed read.
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (operationId === null) return;
    let alive = true;
    setError(undefined);
    getOperation(operationId).then(
      (read) => {
        if (alive) setOperation(read);
      },
      (reason: unknown) => {
        if (alive) setError(asError(reason));
      },
    );
    return () => {
      alive = false;
    };
  }, [operationId, attempt, item]);

  const question = operation?.question ?? null;
  if (operation === null || question === null) {
    return (
      <>
        {error === undefined ? (
          <p className="muted small">Reading its question…</p>
        ) : (
          <>
            <p className="warn small">Its question could not be read.</p>
            {error && <ErrorDetails error={error} />}
            <div className="row">
              <button type="button" onClick={() => setAttempt(attempt + 1)}>
                Read it again
              </button>
            </div>
          </>
        )}
        {props.footer}
      </>
    );
  }
  const thisAnswer = answersShownQuestion(props.answered, question.question_id)
    ? props.answered?.state
    : undefined;
  if (thisAnswer === "sent") return <>{props.sentOnly}</>;
  if (thisAnswer === "failed") return <>{props.failedOnly}</>;
  return (
    <>
      <p className="muted small">
        After your answer, Traeva may move it without a preview, as you agreed for dropped files.
      </p>
      {/* The short card (#37): the file's path, typing an answer and the
          rules are in its Details. */}
      <QuestionCard
        question={question}
        answered={operation.answered}
        file={operation.file}
        compact
        onAnswer={(optionId, text) => {
          const reply = answerFor(question, optionId, text);
          if (reply) props.onAnswer(operation.operation_id, reply);
        }}
        footer={props.footer}
        sending={thisAnswer === "sending"}
        primary={props.primary}
      />
    </>
  );
}

/** The mode is off: the file takes the usual steps, into the folder chosen here. */
function StartUsualFlow(props: {
  roots: RootView[];
  primary: boolean;
  disabled: boolean;
  onStart: (rootId: string) => void;
  giveUp: ReactNode;
}) {
  const [chosen, setChosen] = useState("");
  const rootId = validRootId(chosen, props.roots.map((root) => root.root_id));
  if (props.roots.length === 0) {
    return (
      <>
        <p className="muted small">Add a folder to organize it into first.</p>
        <div className="row">{props.giveUp}</div>
      </>
    );
  }
  return (
    <>
      <label className="field">
        Into
        <select value={rootId} onChange={(event) => setChosen(event.target.value)}>
          {props.roots.map((root) => (
            <option key={root.root_id} value={root.root_id}>
              {root.display_path}
            </option>
          ))}
        </select>
      </label>
      <div className="row">
        <button
          type="button"
          className={props.primary ? "primary" : undefined}
          onClick={() => props.onStart(rootId)}
          disabled={props.disabled || rootId === ""}
        >
          Organize this file
        </button>
        {props.giveUp}
      </div>
    </>
  );
}
