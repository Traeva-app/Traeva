// Candidate capture after a replacement, and the decisions on waiting
// candidates (#31 material §3, §5 and the supplement). The core lists what
// may be decided and checks every decision again; these cards show its
// views, collect the user's choices and send them with the page's token.

import { useCallback, useEffect, useState } from "react";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import { familyApi } from "../family/api";
import type {
  CandidateCaptureInfo,
  CandidateChoice,
  CandidateDecisionOutcome,
  CurrentObjectChoice,
  PendingCandidate,
  PendingCandidatesView,
} from "../family/types";
import {
  BRANCH_HINT,
  CHOICE_TEXT,
  CURRENT_OBJECT_TEXT,
  REASON_TEXT,
  branchParents,
  candidateNotice,
  captureText,
  continueAllInOrder,
  continueChoice,
  currentObjectKinds,
  currentObjectNote,
  decisionProblems,
  decisionRequest,
  emptyDraft,
  hasDecisions,
  isAtLimit,
  leaveAsItIs,
  namedPlace,
  pendingText,
  savedText,
  skipNote,
  snapshotPhrase,
  waitingLabel,
  type ChoiceKind,
  type DecisionDraft,
} from "./records";

function capitalizeFirst(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** Turning candidate capture on or off for one recording. On needs the
 * notice first; the notice's version and the object at the place go back
 * with the request, so a changed place is refused (page_outdated). */
export function CandidateCaptureCard({
  chainId,
  changed,
  onChange,
}: {
  chainId: string;
  /** Changes when the recording does (recordSignature): read again. */
  changed: string;
  onChange: () => void;
}) {
  const [info, setInfo] = useState<CandidateCaptureInfo | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [outdated, setOutdated] = useState(false);

  const load = useCallback(() => {
    familyApi.candidateCapture(chainId).then(setInfo, (reason: unknown) => setError(asError(reason)));
  }, [chainId]);
  useEffect(load, [load, changed]);

  if (!info) return error ? <ErrorDetails error={error} /> : null;
  const set = (on: boolean) => {
    setError(null);
    setOutdated(false);
    familyApi.setCandidateCapture({
      chain_id: chainId,
      on,
      notice_version: on ? info.notice_version : null,
      occupant_token: on ? (info.unread_occupant?.occupant_token ?? null) : null,
      place_token: on ? info.place_token : null,
    }).then(
      (next) => {
        setInfo(next);
        onChange();
      },
      (reason: unknown) => {
        const failure = asError(reason);
        if (failure?.code === "page_outdated") {
          setOutdated(true);
          load();
        } else {
          setError(failure);
        }
      },
    );
  };
  const canTurnOn = info.state === "active" && info.root_active;
  return (
    <section className="card">
      <h2>When an app replaces the file</h2>
      <p>{captureText(info.candidate_capture, info.pending)}</p>
      {info.candidate_capture === "off" ? (
        <>
          <p>Turning this on keeps a copy of a file at its place that Traeva cannot follow by itself, as a candidate:</p>
          <ul>
            {candidateNotice(info).map((point) => (
              <li key={point}>{point}</li>
            ))}
          </ul>
          {info.place_problem && <ErrorDetails error={info.place_problem} />}
          <button type="button" disabled={!canTurnOn} onClick={() => set(true)}>
            Turn on for this file
          </button>
          {!canTurnOn && <p className="muted small">Only a recording that is on, in an authorized folder, can turn this on.</p>}
        </>
      ) : (
        <>
          {(info.pending.objects > 0 || info.pending.snapshots > 0) && (
            <p className="muted small">{pendingText(info.pending)}</p>
          )}
          <button type="button" onClick={() => set(false)}>
            Turn off
          </button>
        </>
      )}
      {outdated && (
        <p className="warn">The place changed since this notice was shown. Nothing was changed; read it again.</p>
      )}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** The waiting candidates of one recording, and the user's decisions.
 * Only what this page lists is decided; a page that changed meanwhile is
 * refused by the core and shown again. */
export function PendingCard({
  chainId,
  changed,
  onDecided,
}: {
  chainId: string;
  /** Changes when the recording does (recordSignature): read again. */
  changed: string;
  onDecided: () => void;
}) {
  const [view, setView] = useState<PendingCandidatesView | null>(null);
  const [draft, setDraft] = useState<DecisionDraft>({});
  const [current, setCurrent] = useState<CurrentObjectChoice | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  const show = useCallback((next: PendingCandidatesView) => {
    setView(next);
    setDraft(emptyDraft());
    setCurrent(null);
  }, []);
  const load = useCallback(() => {
    familyApi.pendingCandidates(chainId).then(show, (reason: unknown) => setError(asError(reason)));
  }, [chainId, show]);
  useEffect(load, [load, changed]);

  if (!view) return error ? <ErrorDetails error={error} /> : null;
  const currentKinds = currentObjectKinds(view);
  if (view.items.length === 0 && currentKinds.length === 0) return null;

  const problems = decisionProblems(view, draft);
  const choose = (id: string, choice: CandidateChoice | null) =>
    setDraft(choice ? { ...draft, [id]: choice } : leaveAsItIs(draft, id));
  const submit = () => {
    setSending(true);
    setError(null);
    setNotice(null);
    familyApi.decide(decisionRequest(view, draft, current)).then(
      (outcome: CandidateDecisionOutcome) => {
        setSending(false);
        setNotice(savedText(outcome.history_only));
        show(outcome.page);
        onDecided();
      },
      (reason: unknown) => {
        setSending(false);
        const failure = asError(reason);
        if (failure?.code === "page_outdated") {
          setNotice("The list changed since it was shown, perhaps by a new save. Nothing was changed. Review it again.");
          load();
        } else {
          setError(failure);
        }
      },
    );
  };

  return (
    <section className="card">
      <h2>Files waiting for your decision</h2>
      {view.items.length > 0 && <p className="small">{pendingText(view.pending)}</p>}
      {isAtLimit(view.pending) && (
        <p className="warn">Waiting records reached the limit, so candidate capture is paused. Review them first.</p>
      )}
      <p className="muted small">
        Stop editing the file before you decide: each new save at this place changes this list. Files you leave as
        they are stay waiting, and nothing is recorded about them. "Not sure yet" records that you are not sure; it
        also keeps the file waiting and does not free the limit.
      </p>
      <ul className="list">
        {view.items.map((item) => (
          <li key={item.candidate_id}>
            <CandidateRow
              item={item}
              items={view.items}
              forkPoints={view.fork_points}
              choice={draft[item.candidate_id] ?? null}
              onChoose={(choice) => choose(item.candidate_id, choice)}
            />
          </li>
        ))}
      </ul>
      {currentKinds.length > 0 && (
        <div>
          <p>
            {view.unread_occupant
              ? `A file at this place has not been read yet (${view.unread_occupant.occupant.byte_length.toLocaleString()} bytes). Saying it continues the work reads it now.`
              : "The file at this place now is one you decided on earlier, or one another recording keeps. Traeva does not read it."}
          </p>
          <div className="row">
            {currentKinds.map((kind) => {
              const from = view.unread_occupant?.continue_from ?? null;
              const choice: CurrentObjectChoice | null =
                kind === "continue" ? (from ? { kind, parent_snapshot_id: from.snapshot_id } : null) : { kind };
              return (
                <label key={kind}>
                  <input
                    type="radio"
                    name="current-object"
                    disabled={choice === null}
                    checked={current?.kind === kind}
                    onChange={() => setCurrent(choice)}
                  />{" "}
                  {CURRENT_OBJECT_TEXT[kind]}
                </label>
              );
            })}
            <label>
              <input type="radio" name="current-object" checked={current === null} onChange={() => setCurrent(null)} />{" "}
              Leave it for now
            </label>
          </div>
          {current && <p className="muted small">{currentObjectNote(current.kind)}</p>}
          {current?.kind === "continue" && view.unread_occupant?.continue_from && (
            <p className="muted small">
              It would continue from {snapshotPhrase(view.unread_occupant.continue_from, view.items)}.
              {view.unread_occupant.continue_skips > 0 && " That is a new relation past files you marked unrelated."}
            </p>
          )}
          {view.unread_occupant ? (
            <p className="muted small">{BRANCH_HINT}</p>
          ) : (
            view.place_problem && <ErrorDetails error={view.place_problem} />
          )}
        </div>
      )}
      <div className="row">
        <button type="button" onClick={() => setDraft(continueAllInOrder(view, draft))}>
          Mark all that can as continuing, in order
        </button>
        <button
          type="button"
          disabled={sending || problems.length > 0 || !hasDecisions(draft, current)}
          onClick={submit}
        >
          Save these decisions
        </button>
      </div>
      {problems.length > 0 && (
        <ul className="warn small">
          {problems.map((problem) => (
            <li key={problem}>{problem}</li>
          ))}
        </ul>
      )}
      {notice && <p>{notice}</p>}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

function CandidateRow({
  item,
  items,
  forkPoints,
  choice,
  onChoose,
}: {
  item: PendingCandidate;
  items: PendingCandidate[];
  forkPoints: PendingCandidatesView["fork_points"];
  choice: CandidateChoice | null;
  onChoose: (choice: CandidateChoice | null) => void;
}) {
  const asContinue = continueChoice(item);
  const pick = (kind: ChoiceKind) => {
    switch (kind) {
      case "continue":
        if (asContinue) onChoose(asContinue);
        return;
      case "branch":
        onChoose({ kind, parent_snapshot_id: null });
        return;
      default:
        onChoose({ kind });
    }
  };
  const parent = item.continue_from ? snapshotPhrase(item.continue_from, items) : null;
  const skip = skipNote(item, parent);
  const place = namedPlace(item.display_path);
  return (
    <div>
      <div className="row">
        <span className="grow">
          <strong>
            {capitalizeFirst(waitingLabel(item, items))}: {place.name}
          </strong>
          {place.folder && (
            <>
              <br />
              <span className="mono small muted">{place.folder}</span>
            </>
          )}
        </span>
        {item.at_place && <span className="badge">at the place now</span>}
        {!item.present && <span className="badge">no longer there</span>}
        {item.deferred && <span className="badge">marked not sure earlier</span>}
      </div>
      <p className="small muted">
        {REASON_TEXT[item.reason]} Found {new Date(item.found_at_ms).toLocaleString()}
        {item.found_after && `, after ${snapshotPhrase(item.found_after, items)}`}.{" "}
        {item.snapshots.length === 1 ? "1 copy saved." : `${item.snapshots.length} copies saved.`}
      </p>
      {parent && (
        <p className="small">
          If it continues the work, it follows {parent}.
          {skip && <span className="warn"> {skip}</span>}
        </p>
      )}
      <div className="row small">
        <label>
          <input
            type="radio"
            name={`choice-${item.candidate_id}`}
            checked={choice === null}
            onChange={() => onChoose(null)}
          />{" "}
          Leave as it is
        </label>
        {(Object.keys(CHOICE_TEXT) as ChoiceKind[]).map((kind) => (
          <label key={kind}>
            <input
              type="radio"
              name={`choice-${item.candidate_id}`}
              disabled={kind === "continue" && !asContinue}
              checked={choice?.kind === kind}
              onChange={() => pick(kind)}
            />{" "}
            {CHOICE_TEXT[kind]}
          </label>
        ))}
      </div>
      {choice?.kind === "branch" && (
        <label className="small">
          Branches from{" "}
          <select
            value={choice.parent_snapshot_id ?? ""}
            onChange={(event) => onChoose({ kind: "branch", parent_snapshot_id: event.target.value || null })}
          >
            <option value="">not known</option>
            {branchParents(forkPoints, item).map((point) => (
              <option key={point.snapshot_id} value={point.snapshot_id}>
                {snapshotPhrase(point, items)}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}
