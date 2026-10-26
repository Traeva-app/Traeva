// Finding a version by a change you remember (#34): "the version before I
// fixed the citations". The core decides what matches, which side of the
// change is meant, and whether one answer is proven; this card shows its
// view and passes the user's answers back. The question typed here is kept
// in Traeva's memory only, and only until the search ends.

import { useCallback, useEffect, useRef, useState } from "react";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import {
  answerMemorySearch,
  cancelMemorySearch,
  getMemorySearch,
  retryMemorySearch,
  startMemorySearch,
} from "./api";
import {
  coverageText,
  isStructureEdit,
  isWorking,
  MODEL_WORDS_NOTE,
  noLongerCertainText,
  openActionText,
  reachedByText,
  searchFailedText,
  searchTitle,
  unsupportedText,
  versionLabel,
  versionRef,
  type LineNumbers,
} from "./history";
import { ChangeItem } from "./ChangeItem";
import type { SearchAnswer, SearchCandidate, SearchView, SnapshotRef } from "./types";
import { ShowMore } from "../../shared/ShowMore";

/** The longest question the core takes (retrieval.rs MAX_QUESTION_CHARS). */
const MAX_QUESTION_CHARS = 500;

/** `numbers`: the versions' numbers on the record's line, from the history.
 * `historyKey`: changes when the history does (history.ts). `onFound`: told
 * once per version a search finds, so the history can show it (#248). */
export function MemorySearch({
  fileId,
  numbers,
  historyKey,
  onOpen,
  onFound,
}: {
  fileId: string;
  numbers: LineNumbers;
  historyKey: string;
  onOpen: (version: SnapshotRef) => void;
  onFound?: (version: SnapshotRef) => void;
}) {
  const [question, setQuestion] = useState("");
  const [view, setView] = useState<SearchView | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  // Each action the user takes bumps this; a poll that was sent before the
  // latest action is dropped, so a late answer never replaces a newer view.
  const turn = useRef(0);
  // The view shown, for reading it again without making it a dependency.
  const shown = useRef<SearchView | null>(null);
  shown.current = view;
  // A finished search keeps its scope; when the history changes, read it
  // again so it can say new versions or descriptions arrived (R2-MEM-02).
  useEffect(() => {
    const finished = shown.current;
    if (!finished || isWorking(finished.state)) return;
    const mine = turn.current;
    getMemorySearch(finished.search_id).then(
      (next) => {
        if (turn.current === mine) setView(next);
      },
      () => {},
    );
  }, [historyKey]);

  const act = useCallback((call: Promise<SearchView>) => {
    const mine = ++turn.current;
    setError(null);
    call.then(
      (next) => {
        if (turn.current === mine) setView(next);
      },
      (reason: unknown) => setError(isAppError(reason) ? reason : null),
    );
  }, []);

  // A version found: the history shows it on its timeline. Told when the
  // found version changes, not on every poll.
  const found = view !== null && view.state.kind === "found" ? view.state.candidate.version : null;
  const foundVersion = useRef(found);
  foundVersion.current = found;
  const tellFound = useRef(onFound);
  tellFound.current = onFound;
  const foundId = found?.snapshot_id ?? null;
  useEffect(() => {
    if (foundId !== null && foundVersion.current) tellFound.current?.(foundVersion.current);
  }, [foundId]);

  const working = view !== null && isWorking(view.state);
  useEffect(() => {
    if (!view || !working) return;
    const searchId = view.search_id;
    const timer = setInterval(() => {
      const mine = turn.current;
      getMemorySearch(searchId).then(
        (next) => {
          if (turn.current === mine) setView(next);
        },
        () => {},
      );
    }, 400);
    return () => clearInterval(timer);
  }, [view, working]);

  const answer = (searchId: string, given: SearchAnswer) => act(answerMemorySearch(searchId, given));

  return (
    <section className="card">
      <h2>Find a version by a change</h2>
      <form
        className="row"
        onSubmit={(event) => {
          event.preventDefault();
          if (question.trim()) act(startMemorySearch(fileId, question.trim()));
        }}
      >
        <input
          type="text"
          className="grow"
          value={question}
          maxLength={MAX_QUESTION_CHARS}
          placeholder="the version before I fixed the citations"
          onChange={(event) => setQuestion(event.target.value)}
        />
        <button type="submit" disabled={working || question.trim() === ""}>
          Find
        </button>
      </form>
      {error && <ErrorDetails error={error} />}
      {view && (
        <div>
          <h2 style={{ marginTop: 12 }}>{searchTitle(view.state)}</h2>
          <SearchBody view={view} numbers={numbers} onAnswer={answer} onOpen={onOpen} act={act} />
          <p className="muted small">{coverageText(view.coverage)}</p>
          {view.updates_available && (
            <p className="muted small">New versions or descriptions arrived since this search started. Search again to include them.</p>
          )}
        </div>
      )}
    </section>
  );
}

function SearchBody({
  view,
  numbers,
  onAnswer,
  onOpen,
  act,
}: {
  view: SearchView;
  numbers: LineNumbers;
  onAnswer: (searchId: string, answer: SearchAnswer) => void;
  onOpen: (version: SnapshotRef) => void;
  act: (call: Promise<SearchView>) => void;
}) {
  const state = view.state;
  const id = view.search_id;
  switch (state.kind) {
    case "working":
      return (
        <div className="row">
          <span className="grow muted">Asking the model about the described changes…</span>
          <button type="button" onClick={() => act(cancelMemorySearch(id))}>
            Cancel
          </button>
        </div>
      );
    case "found": {
      const version = state.candidate.version;
      return (
        <div>
          {version && <p><strong>{versionLabel(version, numbers)}</strong></p>}
          <CandidateChanges candidate={state.candidate} numbers={numbers} />
          {state.candidate.no_longer_usable ? (
            <p className="warn small">{noLongerCertainText(state.candidate.no_longer_usable)}</p>
          ) : (
            <p className="small">{reachedByText(state.reached_by)}</p>
          )}
          {version &&
            (state.openable ? (
              <button type="button" onClick={() => onOpen(version)}>
                {openActionText(version, versionRef(version, numbers))}
              </button>
            ) : (
              <p className="warn">The saved copy of this version cannot be read now, so it cannot be opened.</p>
            ))}
        </div>
      );
    }
    case "candidates":
      return (
        <div>
          {state.question && (
            <div>
              <p>{state.question.text}</p>
              <div className="row">
                {state.question.options.map((option, index) => (
                  <button
                    key={option.text}
                    type="button"
                    title={keptText(option.candidate_ids, state.candidates, numbers)}
                    onClick={() =>
                      onAnswer(id, { kind: "option", question_id: state.question!.question_id, index })
                    }
                  >
                    {option.text}
                  </button>
                ))}
              </div>
            </div>
          )}
          <ShowMore items={state.candidates} first={5}>
            {(visible) => (
              <ul className="list">
                {visible.map((candidate) => (
                  <li key={candidate.candidate_id}>
                    <div className="row">
                      <strong className="grow">
                        {candidate.version
                          ? versionLabel(candidate.version, numbers)
                          : `Between ${versionRef(candidate.before, numbers)} and ${versionRef(candidate.after, numbers)}`}
                      </strong>
                      {candidate.maybe && <span className="badge">maybe</span>}
                      {candidate.partial && <span className="badge">described in part</span>}
                      {candidate.no_longer_usable && <span className="badge">no longer certain</span>}
                      {candidate.version && (
                        <button
                          type="button"
                          onClick={() => onAnswer(id, { kind: "pick", candidate_id: candidate.candidate_id })}
                        >
                          This one
                        </button>
                      )}
                    </div>
                    {candidate.no_longer_usable && (
                      <p className="warn small">{noLongerCertainText(candidate.no_longer_usable)}</p>
                    )}
                    <CandidateChanges candidate={candidate} numbers={numbers} />
                  </li>
                ))}
              </ul>
            )}
          </ShowMore>
          <button type="button" onClick={() => onAnswer(id, { kind: "none_of_these" })}>
            None of these
          </button>
        </div>
      );
    case "not_found":
      return <p>No change in this file's history matches your description.</p>;
    case "cannot_tell":
      return (
        <p>
          No described change matches, but part of the history is not described yet, so Memory
          cannot say it is not there.
        </p>
      );
    case "unsupported":
      return <p>{unsupportedText(state.asked_by)}</p>;
    case "unclear":
      return <p>Memory could not tell which change you mean. Try describing it another way.</p>;
    case "none_of_these":
      return <p>None of the candidates. You can look through the versions below.</p>;
    case "not_permitted":
      return (
        <p>
          Memory may not send this file's changes to the model, so it cannot search them. Agree
          under Describing changes with the model, then search again, or look through the versions.
        </p>
      );
    case "failed":
      return (
        <div>
          <p className="warn">{searchFailedText(state.error_code)}</p>
          {state.retryable && (
            <button type="button" onClick={() => act(retryMemorySearch(id))}>
              Try again
            </button>
          )}
        </div>
      );
    case "cancelled":
      return <p className="muted">The search was cancelled.</p>;
  }
}

/** Which candidates an option keeps, for its tooltip: what the model's
 * wording stands for. */
function keptText(ids: string[], candidates: SearchCandidate[], numbers: LineNumbers): string {
  const labels = ids.map((id) => {
    const candidate = candidates.find((c) => c.candidate_id === id);
    return candidate
      ? `${versionRef(candidate.before, numbers)} to ${versionRef(candidate.after, numbers)}`
      : id;
  });
  return `Keeps ${labels.join(", ")}`;
}

/** The matching changes of a candidate: why it was found. */
function CandidateChanges({ candidate, numbers }: { candidate: SearchCandidate; numbers: LineNumbers }) {
  return (
    <ul className="plain small">
      {candidate.changes.some((change) => change.position > 0) && <li className="muted">{MODEL_WORDS_NOTE}</li>}
      {candidate.changes.map((change) =>
        change.position === 0 ? (
          <li key={change.change_id} className="muted">
            {isStructureEdit(change)
              ? `${change.description} Between ${versionRef(candidate.before, numbers)} and ${versionRef(candidate.after, numbers)}.`
              : `Small edits between ${versionRef(candidate.before, numbers)} and ${versionRef(candidate.after, numbers)} (spacing, punctuation or case).`}
          </li>
        ) : (
          <ChangeItem key={change.change_id} change={change} />
        ),
      )}
    </ul>
  );
}
