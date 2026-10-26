// Activity (#248): what Traeva did lately, in one place, in groups (#37,
// the student's choice B, 10/14). First what needs you (dropped files that
// wait, and actions that need a decision), always open; then dropped files
// Traeva works on; then the moves and Undos (the core's last 50 and the
// dropped files' moves, newest first, by day), each file once; then the
// dropped files that ended without a move, closed at first. Each group
// shows only when it has something.
//
// Nothing runs here. Review goes to Organize's card of actions that need a
// decision; Undo… goes to Organize, which starts its usual Undo (a preview
// first, then Put it back). Whether Undo is offered, and every status, come
// from the core. The page reads Traeva's state when it shows and when the
// window comes back; while a file or a move is still being worked on, it
// reads again every few seconds until nothing is.

import { useCallback, useEffect, useRef, useState } from "react";
import { getDropQueue } from "../autodrop/api";
import { SLOW_POLL_MS, actionText, type DropOutcome } from "../autodrop/queue";
import type { DropItemView } from "../autodrop/types";
import { FileIconSvg } from "../find/icons";
import { fileIcon } from "../find/view";
import { getAction, getAppState } from "../organize/api";
import { undoOffered } from "../organize/flow";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type ActionView, type AppError, type AppState, type RootView } from "../organize/types";
import { Group, Row, Rows, StatusLine } from "../../shared/Group";
import { ShowMore } from "../../shared/ShowMore";
import {
  DROP_LIMIT,
  RECENT_LIMIT,
  actionName,
  actionWhen,
  actionsToRead,
  activityLists,
  byDay,
  dropRows,
  folderNote,
  moveText,
  namesFor,
  stillWorking,
  type DropRow,
} from "./view";
import { statusSummary } from "../../shared/status";
import "../find/find.css";
import "./activity.css";

type PageState =
  | { kind: "loading" }
  | { kind: "loaded"; app: AppState }
  | { kind: "failed"; error: AppError | null; detail: string };

type Props = {
  /** The Activity tab is showing. */
  active: boolean;
  /** Switches to Organize; "decisions": to its actions that need a decision. */
  onOrganize: (to?: "decisions") => void;
  /** Switches to Organize and starts its Undo of this action there. */
  onUndo: (actionId: string) => void;
  /** Handed-over dropped files that ended in Organize without a plan. */
  dropOutcomes: Record<string, DropOutcome>;
};

export function ActivityPage({ active, onOrganize, onUndo, dropOutcomes }: Props) {
  const [page, setPage] = useState<PageState>({ kind: "loading" });
  const [drops, setDrops] = useState<DropItemView[]>([]);
  const [dropsFailed, setDropsFailed] = useState(false);
  // Dropped files' actions that are not among the recent ones.
  const [otherActions, setOtherActions] = useState<ActionView[]>([]);
  // Something listed is still being worked on: read again now and then.
  const [working, setWorking] = useState(false);
  // Only the latest reading is shown.
  const turn = useRef(0);

  const read = useCallback(() => {
    const mine = ++turn.current;
    const current = () => mine === turn.current;
    const appRead = getAppState();
    const queueRead = getDropQueue();
    appRead.then(
      (app) => {
        if (current()) setPage({ kind: "loaded", app });
      },
      (reason: unknown) => {
        if (current()) setPage({ kind: "failed", error: isAppError(reason) ? reason : null, detail: String(reason) });
      },
    );
    queueRead.then(
      (queue) => {
        if (!current()) return;
        setDrops(queue.items);
        setDropsFailed(false);
      },
      () => {
        if (current()) setDropsFailed(true);
      },
    );
    // Whether to read again. A reading that failed (it shows above) keeps
    // the page reading while something was being worked on, so a move still
    // going on shows once a reading works again.
    Promise.allSettled([appRead, queueRead]).then(([app, queue]) => {
      if (!current() || app.status === "rejected") return;
      const recent = app.value.recent_actions;
      if (queue.status === "rejected") {
        setWorking((was) => was || stillWorking(null, recent));
        return;
      }
      setWorking(stillWorking(queue.value, recent));
      // A moved file's action for its Undo: among the recent ones, else read
      // on its own. One that cannot be read shows the queue's status only.
      Promise.allSettled(actionsToRead(queue.value.items, recent).map(getAction)).then((results) => {
        if (!current()) return;
        setOtherActions(results.flatMap((result) => (result.status === "fulfilled" ? [result.value] : [])));
      });
    });
  }, []);

  useEffect(() => {
    if (!active) return;
    read();
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [active, read]);
  useEffect(() => {
    if (!active || !working) return;
    const timer = window.setInterval(read, SLOW_POLL_MS);
    return () => window.clearInterval(timer);
  }, [active, working, read]);

  return (
    <main className="page activity-page">
      <header className="page-header">
        <h1>Activity</h1>
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
          <p>Traeva cannot show its records until this is fixed. Your files were not touched.</p>
          <ErrorDetails error={page.app.storage.error} />
        </section>
      )}

      {page.kind === "loaded" && page.app.storage.status === "ready" && (
        <Sections
          app={page.app}
          drops={drops}
          dropsFailed={dropsFailed}
          otherActions={otherActions}
          dropOutcomes={dropOutcomes}
          onOrganize={onOrganize}
          onUndo={onUndo}
        />
      )}
    </main>
  );
}

function Sections(props: {
  app: AppState;
  drops: DropItemView[];
  dropsFailed: boolean;
  otherActions: ActionView[];
  dropOutcomes: Record<string, DropOutcome>;
  onOrganize: (to?: "decisions") => void;
  onUndo: (actionId: string) => void;
}) {
  const { app, onOrganize, onUndo } = props;
  const now = Date.now();
  const actions = new Map([...props.otherActions, ...app.recent_actions].map((action) => [action.action_id, action]));
  const rows = dropRows(props.drops, props.dropOutcomes, actions, now);
  const lists = activityLists(app.decisions_needed, app.recent_actions, rows);
  const review = () => onOrganize("decisions");
  // Two authorized folders with the same name are told apart.
  const places: Places = {
    roots: app.roots,
    names: namesFor([...app.decisions_needed, ...actions.values()], app.roots),
  };
  const waiting = lists.waiting.length + lists.decisions.length;

  if (waiting === 0 && lists.working.length === 0 && lists.moves.length === 0 && lists.ended.length === 0) {
    return (
      <>
        <p className="find-line">Nothing yet. Moves and dropped files show here.</p>
        {props.dropsFailed && <p className="find-row-error">Dropped files could not be read just now.</p>}
      </>
    );
  }

  return (
    <>
      {waiting > 0 && (
        // First and always open; its first button is the page's one main button.
        <Group id="activity-needs" title="Needs you" count={waiting} attention>
          <Rows>
            {lists.waiting.map((row, index) => (
              <DropItem
                key={row.key}
                row={row}
                places={places}
                primary={index === 0}
                onOrganize={() => onOrganize()}
                onUndo={onUndo}
              />
            ))}
            {lists.decisions.map((action, index) => (
              <ActionRow
                key={action.action_id}
                action={action}
                places={places}
                now={now}
                primary={lists.waiting.length === 0 && index === 0}
                onReview={review}
              />
            ))}
          </Rows>
        </Group>
      )}

      {lists.working.length > 0 && (
        <Group id="activity-working" title="In progress" count={lists.working.length}>
          <Rows>
            {lists.working.map((row) => (
              <DropItem key={row.key} row={row} places={places} primary={false} onOrganize={() => onOrganize()} onUndo={onUndo} />
            ))}
          </Rows>
        </Group>
      )}

      {lists.moves.length > 0 && (
        <Group
          id="activity-moves"
          title="Moves"
          count={lists.moves.length}
          note={
            <>
              Undo… shows a preview in Organize first.
              {app.recent_actions.length >= RECENT_LIMIT && ` Only the last ${RECENT_LIMIT} are listed.`}
            </>
          }
        >
          <ShowMore items={lists.moves} first={10}>
            {(visible) =>
              byDay(visible, now).map((day) => (
                <div key={day.label} className="activity-day">
                  <h3 className="activity-day-title">{day.label}</h3>
                  <Rows>
                    {day.moves.map((move) =>
                      move.action !== null ? (
                        <ActionRow
                          key={move.key}
                          action={move.action}
                          places={places}
                          now={now}
                          primary={false}
                          dropped={move.drop !== null}
                          onReview={review}
                          onUndo={onUndo}
                        />
                      ) : (
                        move.drop !== null && (
                          <DropItem
                            key={move.key}
                            row={move.drop}
                            places={places}
                            primary={false}
                            onOrganize={() => onOrganize()}
                            onUndo={onUndo}
                          />
                        )
                      ),
                    )}
                  </Rows>
                </div>
              ))
            }
          </ShowMore>
        </Group>
      )}

      {(lists.ended.length > 0 || props.dropsFailed) && (
        <Group
          id="activity-ended"
          title="Dropped, not moved"
          count={lists.ended.length}
          defaultOpen={false}
          forceOpen={props.dropsFailed && lists.ended.length === 0}
          summary={statusSummary(lists.ended)}
          note={
            <>
              Files dropped since Traeva started that were not moved.
              {props.drops.length >= DROP_LIMIT && ` Only the last ${DROP_LIMIT} are kept.`}
            </>
          }
        >
          {lists.ended.length > 0 && (
            <ShowMore items={lists.ended} first={10}>
              {(visible) => (
                <Rows>
                  {visible.map((row) => (
                    <DropItem key={row.key} row={row} places={places} primary={false} onOrganize={() => onOrganize()} onUndo={onUndo} />
                  ))}
                </Rows>
              )}
            </ShowMore>
          )}
          {props.dropsFailed && <p className="find-row-error">Dropped files could not be read just now.</p>}
        </Group>
      )}
    </>
  );
}

/** The authorized folders Traeva has now, and how the page names the ones
 * it shows (namesFor). */
type Places = { roots: RootView[]; names: Map<string, string> };

/** Where an action took the file, its folders named as the page names
 * them. The full paths are its tooltip. */
function MoveLine(props: { action: ActionView; places: Places }) {
  const { action } = props;
  return (
    <div className="find-row-line" title={`${action.source.display_path} → ${action.target.display_path}`}>
      {moveText(action, props.places.names)}
    </div>
  );
}

/** A saved action: the file, where it went, its status and when (and when
 * it was dropped, for a dropped file's move). Review when it needs a
 * decision; Undo… when the core offers it. */
function ActionRow(props: {
  action: ActionView;
  places: Places;
  now: number;
  primary: boolean;
  /** A dropped file's move. */
  dropped?: boolean;
  onReview: () => void;
  /** Not given: no Undo here (what needs a decision). */
  onUndo?: (actionId: string) => void;
}) {
  const { action, onUndo } = props;
  const name = actionName(action);
  const canUndo = onUndo !== undefined && undoOffered(action.undo_status);
  const decide = action.status === "needs_user_decision";
  return (
    <Row
      icon={<FileIconSvg icon={fileIcon(null, name)} />}
      name={name}
      attention={decide}
      lines={<MoveLine action={action} places={props.places} />}
      status={
        <StatusLine
          badge={actionText(action.status, action.kind)}
          when={props.dropped ? `${actionWhen(action, props.now)} · dropped` : actionWhen(action, props.now)}
          note={folderNote(action, props.places.roots, props.places.names)}
        />
      }
      actions={
        (decide || canUndo) && (
          <>
            {decide && (
              <button
                type="button"
                className={props.primary ? "primary" : undefined}
                aria-label={`Review: ${name}`}
                onClick={props.onReview}
              >
                Review
              </button>
            )}
            {canUndo && (
              <button type="button" aria-label={`Undo…: ${name}`} onClick={() => onUndo(action.action_id)}>
                Undo…
              </button>
            )}
          </>
        )
      }
    />
  );
}

/** A dropped file: its state as the drop queue words it, when it was
 * dropped, and its move once it has one. One that waits for the user goes
 * to Organize, where the drop queue has its question or next step. */
function DropItem(props: {
  row: DropRow;
  places: Places;
  /** Its button is the page's one main button (the first that waits). */
  primary: boolean;
  onOrganize: () => void;
  onUndo: (actionId: string) => void;
}) {
  const { row } = props;
  const action = row.action;
  const canUndo = action !== null && undoOffered(action.undo_status);
  return (
    <Row
      icon={<FileIconSvg icon={fileIcon(null, row.name)} />}
      name={row.name}
      attention={row.open !== null}
      lines={
        <>
          {action !== null && <MoveLine action={action} places={props.places} />}
          {row.reason && <div className="find-row-line">{row.reason}</div>}
        </>
      }
      status={
        <StatusLine
          badge={row.badge}
          when={row.dropped}
          note={action === null ? null : folderNote(action, props.places.roots, props.places.names)}
        />
      }
      actions={
        (row.open !== null || canUndo) && (
          <>
            {row.open !== null && (
              <button
                type="button"
                className={props.primary ? "primary" : undefined}
                aria-label={`${row.open}: ${row.name}`}
                onClick={props.onOrganize}
              >
                {row.open}
              </button>
            )}
            {canUndo && (
              <button type="button" aria-label={`Undo…: ${row.name}`} onClick={() => props.onUndo(action.action_id)}>
                Undo…
              </button>
            )}
          </>
        )
      }
    />
  );
}
