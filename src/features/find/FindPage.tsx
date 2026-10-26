// Find (#198, contract §4.10): the app's first page. The user types what
// they remember about a file; the core searches the authorized folders on
// this Mac and returns its results, best first. The page shows them in
// that order and never decides what matches. A search is polled every
// 300 ms until it is done or cancelled; only the latest one is shown. The
// question stays in the search box; Traeva never writes it to disk. Showing
// or opening a result names the search and the result, never a path.
//
// While no search is shown, the page is a home: what waits for the user and
// what they worked on last, the files opened lately and their folders. Each
// part shows only when it has something. ⌘K puts the cursor in the search
// box; Esc clears it and brings the home back. On the first start the page
// is the setup guide instead (onboarding/OnboardingGuide.tsx, #248).

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { getDropQueue } from "../autodrop/api";
import type { DropOutcome } from "../autodrop/queue";
import type { DropItemView } from "../autodrop/types";
import { familyApi } from "../family/api";
import type { MemoryRecordView } from "../family/types";
import { OnboardingGuide, useGuide } from "../onboarding/OnboardingGuide";
import { getAppState } from "../organize/api";
import { FirstFolderCard } from "../organize/FirstFolderCard";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type AppState } from "../organize/types";
import {
  cancelFind,
  getFind,
  getFindSetup,
  getRecentFiles,
  openFindResult,
  revealFindResult,
  setFindConsent,
  startFind,
} from "./api";
import { MAIN_FOCUS_BACK } from "../pet/api";
import { MAX_FIND_QUESTION_CHARS, type FindResult, type FindSetupView, type FindView } from "./types";
import {
  accepts,
  coverageLines,
  fileIcon,
  firstWaiting,
  kindLabel,
  matchedText,
  matchLines,
  resultTimeText,
  modelUse,
  noSearchText,
  pickUpRows,
  recentFolders,
  whenText,
  wordsByText,
  wordsText,
  type PickUpRow,
  type RecentFolder,
} from "./view";
import { FileIconSvg, FolderIconSvg, SearchGlyph } from "./icons";
import "./find.css";
import { ShowMore } from "../../shared/ShowMore";

const POLL_MS = 300;
/** How many recent files and folders the home lists: few, so the home
 * stays calm (the student, 10/14: it piled up at 5 and 3). */
const MAX_RECENT_FILES = 3;
const MAX_RECENT_FOLDERS = 2;
const NO_OUTCOMES: Record<string, DropOutcome> = {};
// The opened times are macOS's own record (#198, D's check on real apps):
// some ways of opening a file leave no record, so this is not every file
// the user opened.
const RECENT_NOTE = "As macOS recorded them. Some ways of opening a file, such as Quick Look, are not recorded.";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** A failed call: the core's error, or what was thrown when it is not one. */
type Failure = { error: AppError | null; detail: string };

function failureOf(reason: unknown): Failure {
  return { error: asError(reason), detail: String(reason) };
}

function FailureText({ failure }: { failure: Failure }) {
  return failure.error ? <ErrorDetails error={failure.error} /> : <p>{failure.detail}</p>;
}

function failureMessage(failure: Failure): string {
  return failure.error ? failure.error.message : failure.detail;
}

type Props = {
  /** The Find tab is showing. */
  active: boolean;
  /** Switches to the Organize tab; "decisions": to its actions that need a decision. */
  onOrganize: (to?: "decisions") => void;
  /** Switches to the Memory tab and shows this record. */
  onVersions: (chainId: string) => void;
  /** Handed-over dropped files that ended in Organize without a plan. */
  dropOutcomes?: Record<string, DropOutcome>;
};

export function FindPage({ active, onOrganize, onVersions, dropOutcomes = NO_OUTCOMES }: Props) {
  const [question, setQuestion] = useState("");
  const [view, setView] = useState<FindView | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<(Failure & { title: string }) | null>(null);
  // The search the page waits for; any other answer is dropped.
  const latest = useRef<string | null>(null);
  // Moves on with each search and each clear: a search that was starting
  // when the box was cleared is cancelled and never shown.
  const searchTurn = useRef(0);

  const [app, setApp] = useState<AppState | null>(null);
  const [appError, setAppError] = useState<Failure | null>(null);
  const [setup, setSetup] = useState<FindSetupView | null>(null);
  const [setupError, setSetupError] = useState<Failure | null>(null);
  const [consentBusy, setConsentBusy] = useState(false);
  const [consentError, setConsentError] = useState<Failure | null>(null);
  // Moves on when an agreement change starts and when it ends: a setup read
  // sent before then may answer with the old setting, and is dropped.
  const setupTurn = useRef(0);

  // The home's views. The recent files are a finished search that the core
  // keeps among its last few, so they are read again each time the home
  // shows and their search is never kept for later.
  const [recent, setRecent] = useState<FindView | null>(null);
  const [recentError, setRecentError] = useState<Failure | null>(null);
  const [drops, setDrops] = useState<DropItemView[]>([]);
  const [records, setRecords] = useState<MemoryRecordView[]>([]);
  // Only the latest reading of the home is shown.
  const homeTurn = useRef(0);

  // The cursor goes to the search box when the page shows and whenever the
  // window comes back (the Dock icon) while it shows.
  const input = useRef<HTMLInputElement>(null);
  const activeNow = useRef(active);
  activeNow.current = active;

  const searchShown = starting || view !== null || error !== null;
  const homeShown = useRef(!searchShown);
  homeShown.current = !searchShown;

  // Folders are added and the model is set up in Settings: read
  // both each time the tab shows and when the window comes back.
  const read = useCallback(() => {
    getAppState().then(
      (state) => {
        setApp(state);
        setAppError(null);
      },
      (reason: unknown) => setAppError(failureOf(reason)),
    );
    const turn = setupTurn.current;
    getFindSetup().then(
      (next) => {
        if (turn !== setupTurn.current) return;
        setSetup(next);
        setSetupError(null);
      },
      (reason: unknown) => {
        if (turn === setupTurn.current) setSetupError(failureOf(reason));
      },
    );
  }, []);
  const readHome = useCallback(() => {
    const turn = ++homeTurn.current;
    const current = () => turn === homeTurn.current;
    getRecentFiles().then(
      (next) => {
        if (!current()) return;
        setRecent(next);
        setRecentError(null);
      },
      (reason: unknown) => {
        if (!current()) return;
        setRecent(null);
        setRecentError(failureOf(reason));
      },
    );
    // Organize and Memory show these with their errors; here an unreadable
    // one only leaves its rows out.
    getDropQueue().then(
      (queue) => {
        if (current()) setDrops(queue.items);
      },
      () => {
        if (current()) setDrops([]);
      },
    );
    familyApi.listRecords().then(
      (list) => {
        if (current()) setRecords(list);
      },
      () => {
        if (current()) setRecords([]);
      },
    );
  }, []);
  useEffect(() => {
    if (!active) return;
    input.current?.focus();
    read();
    if (homeShown.current) readHome();
  }, [active, read, readHome]);
  useEffect(() => {
    const back = () => {
      if (!activeNow.current) return;
      // Without scrolling: the user may come back from Finder to a result
      // further down.
      input.current?.focus({ preventScroll: true });
      read();
      if (homeShown.current) readHome();
    };
    // Also when showing the pet gave the window the focus back (#259).
    window.addEventListener("focus", back);
    window.addEventListener(MAIN_FOCUS_BACK, back);
    return () => {
      window.removeEventListener("focus", back);
      window.removeEventListener(MAIN_FOCUS_BACK, back);
    };
  }, [read, readHome]);
  // ⌘K puts the cursor in the search box while the page shows.
  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      if (!event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || event.key.toLowerCase() !== "k") return;
      event.preventDefault();
      input.current?.focus();
      input.current?.select();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  const show = useCallback((next: FindView) => {
    setView((current) => (accepts(latest.current, current, next) ? next : current));
  }, []);
  // A failed call ends the search it was for, if that is still the latest.
  const failed = useCallback((findId: string | null, title: string, reason: unknown) => {
    if (latest.current !== findId) return;
    latest.current = null;
    setView(null);
    setError({ title, ...failureOf(reason) });
  }, []);

  const pollingId = view?.state === "working" ? view.find_id : null;
  useEffect(() => {
    if (pollingId === null) return;
    const timer = setInterval(() => {
      getFind(pollingId).then(show, (reason: unknown) => failed(pollingId, "The search could not be read", reason));
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [pollingId, show, failed]);

  const activeRoots = app?.roots.filter((root) => root.status === "active") ?? [];
  const storageBlocked = app?.storage.status === "blocked";
  const noFolder = app !== null && !storageBlocked && activeRoots.length === 0;
  const canSearch = app !== null && !storageBlocked && activeRoots.length > 0;
  // The first start (#248): a guide instead of the search box and the home.
  const guide = useGuide(app !== null && !storageBlocked ? activeRoots.length : null);
  // Until Traeva's state is read on what may be the first start, nothing
  // shows, so the search box does not flash before the guide; once it
  // shows, the cursor goes there as on any other start.
  const waiting = guide.flag !== "done" && app === null && !appError;
  useEffect(() => {
    if (!waiting && activeNow.current) input.current?.focus();
  }, [waiting]);

  // A new search cancels the old one (the core does it); from now on only
  // the new one's answers are shown.
  const search = () => {
    if (!canSearch || starting || question.trim() === "") return;
    const turn = ++searchTurn.current;
    // The recent files' search may be dropped by the core after a few more
    // searches: the home reads them again when it comes back.
    homeTurn.current += 1;
    setRecent(null);
    setRecentError(null);
    latest.current = null;
    setView(null);
    setError(null);
    setStarting(true);
    startFind(question)
      .then((started) => {
        if (turn !== searchTurn.current) {
          // Cleared while it started. Nothing is shown for it either way.
          if (started.state === "working") cancelFind(started.find_id).catch(() => undefined);
          return;
        }
        latest.current = started.find_id;
        show(started);
      })
      .catch((reason: unknown) => {
        if (turn === searchTurn.current) failed(null, "The search did not start", reason);
      })
      .finally(() => setStarting(false));
  };
  const cancel = () => {
    if (view === null) return;
    const findId = view.find_id;
    cancelFind(findId).then(show, (reason: unknown) => failed(findId, "The search could not be cancelled", reason));
  };
  // Esc, or emptying the box: the search goes (a running one is cancelled;
  // nothing is shown for it either way) and the home comes back, read again.
  const clear = () => {
    const running = view?.state === "working" ? view.find_id : null;
    searchTurn.current += 1;
    latest.current = null;
    setQuestion("");
    setView(null);
    setError(null);
    if (running !== null) cancelFind(running).catch(() => undefined);
    if (searchShown) readHome();
    input.current?.focus();
  };

  const working = starting || view?.state === "working";
  const use = setup ? modelUse(setup) : null;
  const consent = (agree: boolean) => {
    if (consentBusy) return;
    setConsentBusy(true);
    setConsentError(null);
    setupTurn.current += 1;
    setFindConsent(agree)
      .then((next) => {
        setupTurn.current += 1;
        setSetup(next);
        setSetupError(null);
      })
      .catch((reason: unknown) => setConsentError(failureOf(reason)))
      .finally(() => setConsentBusy(false));
  };

  if (waiting) return <main className="page find-page" aria-busy="true" />;

  if (guide.from !== null && app !== null) {
    return (
      <main className="page find-page">
        <div className="find-body">
          {appError && (
            <section className="card card-error">
              <h2>Could not read Traeva's state</h2>
              <FailureText failure={appError} />
            </section>
          )}
          <OnboardingGuide
            app={app}
            from={guide.from}
            onChanged={read}
            onDone={() => {
              guide.finish();
              read();
              readHome();
            }}
            onFinishShown={guide.reached}
          />
        </div>
      </main>
    );
  }

  return (
    <main
      className="page find-page"
      onKeyDown={(event) => {
        if (event.key !== "Escape" || event.nativeEvent.isComposing) return;
        if (question === "" && !searchShown) return;
        event.preventDefault();
        clear();
      }}
    >
      <div className="find-body">
        <h1 className="sr-only">Find</h1>
        <form
          className="find-search"
          role="search"
          onSubmit={(event) => {
            event.preventDefault();
            search();
          }}
        >
          <label className="find-box">
            <SearchGlyph />
            <input
              ref={input}
              type="text"
              aria-label="What do you remember about the file?"
              aria-keyshortcuts="Meta+K"
              value={question}
              maxLength={MAX_FIND_QUESTION_CHARS}
              placeholder="Find a file or describe what you remember…"
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              autoFocus
              onChange={(event) => {
                if (event.target.value === "" && searchShown) clear();
                else setQuestion(event.target.value);
              }}
            />
            <kbd className="find-kbd" aria-hidden="true">
              ⌘K
            </kbd>
          </label>
        </form>

        {use === "agreed" && (
          <p className="find-line">
            Searching with the model
            {setup?.provider && setup.model && (
              <>
                {" "}
                (<span className="mono">{setup.provider} · {setup.model}</span>)
              </>
            )}
            : only what you type is sent.{" "}
            <button type="button" className="find-quiet" disabled={consentBusy} onClick={() => consent(false)}>
              Turn off
            </button>
          </p>
        )}
        {use === "agreed" && consentError && (
          <section className="card card-error">
            <h2>Could not turn off searching with the model</h2>
            <FailureText failure={consentError} />
          </section>
        )}
        {setupError && (
          <section className="card card-error">
            <h2>Could not read the model setup</h2>
            <FailureText failure={setupError} />
          </section>
        )}
        {appError && (
          <section className="card card-error">
            <h2>Could not read Traeva's state</h2>
            <FailureText failure={appError} />
          </section>
        )}
        {storageBlocked && (
          <section className="card card-error">
            <h2>Storage blocked</h2>
            <p>Traeva will not search until this is fixed. The Organize page shows why.</p>
            <button type="button" className="primary" onClick={() => onOrganize()}>
              Go to Organize
            </button>
          </section>
        )}
        {noFolder && (
          <FirstFolderCard
            onAdded={() => {
              read();
              readHome();
            }}
          />
        )}

        {searchShown && (
          <>
            {working && (
              <p className="find-line row">
                <span>Searching…</span>
                {view?.state === "working" && (
                  <button type="button" onClick={cancel}>
                    Cancel
                  </button>
                )}
              </p>
            )}
            {view?.state === "working" && view.words && (
              <p className="find-line">
                {wordsText(view.words)} {view.words_by && wordsByText(view.words_by)}
              </p>
            )}
            {use === "no_model" && (
              <p className="find-line">
                Search uses your words only. A model can be set up in Settings to also search with related
                words and other languages.
              </p>
            )}
            {use === "ask" && (
              <p className="find-line">
                Search uses your words only. It can also search with the model once you agree to the notice below.
              </p>
            )}
            {error && (
              <section className="card card-error">
                <h2>{error.title}</h2>
                <FailureText failure={error} />
              </section>
            )}
            {view?.state === "cancelled" && <p className="muted">The search was cancelled.</p>}
            {view?.state === "done" && <Results key={view.find_id} view={view} onVersions={onVersions} />}
          </>
        )}

        {!searchShown && app !== null && !storageBlocked && (
          <Home
            pickUp={pickUpRows({
              drops,
              outcomes: dropOutcomes,
              decisions: app.decisions_needed,
              lastAction: app.recent_actions[0] ?? null,
              records,
              now: Date.now(),
            })}
            recent={recent}
            recentError={recentError}
            primary={!noFolder}
            onOrganize={onOrganize}
            onVersions={onVersions}
          />
        )}

        {/* After the results, so it never pushes them down; not before the
            first folder, which comes first. */}
        {use === "ask" && setup && !noFolder && (
          <section className="card">
            <h2>Search with the model?</h2>
            <p className="find-notice">{setup.notice}</p>
            <button type="button" disabled={consentBusy} onClick={() => consent(true)}>
              Agree
            </button>
            {consentError && <FailureText failure={consentError} />}
          </section>
        )}
      </div>

      <footer className="find-footer">
        <span>Return to search · Esc to clear</span>
        <span>Searches only the folders you added</span>
      </footer>
    </main>
  );
}

/** The home: each part only when it has something. `primary`: one button
 * is the screen's filled one: the first thing that waits for the user, else
 * the newest recent file's Open (View history never). */
function Home(props: {
  pickUp: { rows: PickUpRow[]; more: number };
  recent: FindView | null;
  recentError: Failure | null;
  primary: boolean;
  onOrganize: (to?: "decisions") => void;
  onVersions: (chainId: string) => void;
}) {
  const { pickUp, recent, recentError, onOrganize, onVersions } = props;
  const firstAction = firstWaiting(pickUp.rows);
  const files = recent?.results.slice(0, MAX_RECENT_FILES) ?? [];
  // From all the recent files, not only the ones listed.
  const folders = recent ? recentFolders(recent.results).slice(0, MAX_RECENT_FOLDERS) : [];
  const now = Date.now();
  return (
    <>
      {pickUp.rows.length > 0 && (
        <section className="find-section" aria-labelledby="find-pick-up">
          <h2 id="find-pick-up">Pick up where you left off</h2>
          <ul className="find-rows" role="list">
            {pickUp.rows.map((row, index) => (
              <PickUpItem
                key={row.key}
                row={row}
                primary={props.primary && index === firstAction}
                onOrganize={onOrganize}
                onVersions={onVersions}
              />
            ))}
          </ul>
          {pickUp.more > 0 && (
            <p className="find-line">
              {pickUp.more === 1 ? "1 more thing waits" : `${pickUp.more} more things wait`} on the Organize page.
            </p>
          )}
        </section>
      )}

      {recent !== null && files.length > 0 && (
        <section className="find-section" aria-labelledby="find-recent">
          <h2 id="find-recent">Recent files</h2>
          <p className="find-section-note">{RECENT_NOTE}</p>
          <ul className="find-rows" role="list">
            {files.map((result, index) => (
              <ResultRow
                key={`${recent.find_id}:${result.result_id}`}
                findId={recent.find_id}
                result={result}
                meta={result.opened_at_ms === null ? null : `Opened ${whenText(result.opened_at_ms, now)}`}
                primary={props.primary && firstAction < 0 && index === 0}
                onVersions={onVersions}
              />
            ))}
          </ul>
        </section>
      )}
      {recent !== null && files.length === 0 && props.primary && (
        <p className="find-line">Files you open in the folders you added show here, as macOS records them.</p>
      )}
      {recentError && (
        <section className="find-section" aria-labelledby="find-recent">
          <h2 id="find-recent">Recent files</h2>
          <p className="find-row-error">Could not read the recent files: {failureMessage(recentError)}</p>
        </section>
      )}

      {recent !== null && folders.length > 0 && (
        <section className="find-section" aria-labelledby="find-folders">
          <h2 id="find-folders">Recent folders</h2>
          <ul className="find-rows" role="list">
            {folders.map((folder) => (
              <FolderRow key={`${recent.find_id}:${folder.folder}`} findId={recent.find_id} folder={folder} />
            ))}
          </ul>
        </section>
      )}
    </>
  );
}

/** A finished search: its results in the core's order, then what it
 * looked for and what it looked at; or only why it could not search. */
function Results({ view, onVersions }: { view: FindView; onVersions: (chainId: string) => void }) {
  const now = Date.now();
  const noSearch = noSearchText(view);
  if (noSearch !== null) {
    return (
      <section className="find-section" aria-label="Results">
        <div className="find-summary">
          <p>
            <strong>{noSearch}</strong>
          </p>
        </div>
      </section>
    );
  }
  return (
    <section className="find-section" aria-label="Results">
      {view.results.length > 0 && (
        <ShowMore items={view.results} first={10}>
          {(visible) => (
            <ul className="find-rows" role="list">
              {visible.map((result, index) => {
                const kind = kindLabel(result.kind, result.name);
                const time = resultTimeText(result, now);
                return (
                  <ResultRow
                    key={result.result_id}
                    findId={view.find_id}
                    result={result}
                    meta={kind ? `${kind} · ${time}` : time.charAt(0).toUpperCase() + time.slice(1)}
                    primary={index === 0}
                    reveal
                    onVersions={onVersions}
                  >
                    <ul className="find-matches">
                      {matchLines(result).map((line, at) => (
                        <li key={at}>
                          {line.label}: <strong>“{line.wording}”</strong>
                          {line.snippet && <span className="muted"> {line.snippet}</span>}
                          {line.outdated && <span className="muted"> (described before it changed)</span>}
                        </li>
                      ))}
                    </ul>
                  </ResultRow>
                );
              })}
            </ul>
          )}
        </ShowMore>
      )}
      <div className="find-summary">
        <p>
          <strong>{matchedText(view)}</strong>
        </p>
        {view.words && <p>{wordsText(view.words)}</p>}
        {view.words_by && <p className="muted">{wordsByText(view.words_by)}</p>}
        {view.coverage && <p className="muted">{coverageLines(view.coverage, view.words).join(" ")}</p>}
      </div>
    </section>
  );
}

/** One row: an icon, the name, grey lines, buttons on the right. */
function Row(props: {
  icon: ReactNode;
  name: string;
  lines: (string | null)[];
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <li className="find-row">
      {props.icon}
      <div className="find-row-text">
        <div className="find-row-name">{props.name}</div>
        {props.lines.map((line, index) =>
          line === null ? null : (
            <div key={index} className="find-row-line">
              {line}
            </div>
          ),
        )}
        {props.children}
      </div>
      {props.actions && <div className="find-row-actions">{props.actions}</div>}
    </li>
  );
}

/** A file from a search or the recent files. Opening or showing it names
 * the search and the result; a refusal (e.g. file_changed: moved or
 * replaced since) shows on this row. */
function ResultRow(props: {
  findId: string;
  result: FindResult;
  meta: string | null;
  primary: boolean;
  /** Offer Show in Finder. */
  reveal?: boolean;
  onVersions: (chainId: string) => void;
  children?: ReactNode;
}) {
  const { findId, result } = props;
  const [error, setError] = useState<Failure | null>(null);
  const run = (call: Promise<void>) => {
    setError(null);
    call.catch((reason: unknown) => setError(failureOf(reason)));
  };
  const chainId = result.memory_chain_id;
  return (
    <Row
      icon={<FileIconSvg icon={fileIcon(result.kind, result.name)} />}
      name={result.name}
      lines={[props.meta, result.folder]}
      actions={
        <>
          <button
            type="button"
            className={props.primary ? "primary" : undefined}
            aria-label={`Open ${result.name}`}
            onClick={() => run(openFindResult(findId, result.result_id))}
          >
            Open
          </button>
          {props.reveal && (
            <button
              type="button"
              aria-label={`Show ${result.name} in Finder`}
              onClick={() => run(revealFindResult(findId, result.result_id))}
            >
              Show in Finder
            </button>
          )}
          {chainId !== null && (
            <button
              type="button"
              aria-label={`View history of ${result.name}`}
              onClick={() => props.onVersions(chainId)}
            >
              View history
            </button>
          )}
        </>
      }
    >
      {props.children}
      {error && <p className="find-row-error">{failureMessage(error)}</p>}
    </Row>
  );
}

function PickUpItem(props: {
  row: PickUpRow;
  primary: boolean;
  onOrganize: (to?: "decisions") => void;
  onVersions: (chainId: string) => void;
}) {
  const { row } = props;
  const action = row.action;
  const label = action === null ? null : action.kind === "history" ? "View history" : action.label;
  const go = () => {
    if (action === null) return;
    if (action.kind === "history") props.onVersions(action.chainId);
    else props.onOrganize(action.kind === "decision" ? "decisions" : undefined);
  };
  return (
    <Row
      icon={<FileIconSvg icon={fileIcon(null, row.name)} />}
      name={row.name}
      lines={[row.detail]}
      actions={
        action !== null && (
          <button
            type="button"
            className={props.primary ? "primary" : undefined}
            aria-label={`${label}: ${row.name}`}
            onClick={go}
          >
            {label}
          </button>
        )
      }
    />
  );
}

/** A folder of the recent files: Finder opens it with its newest file selected. */
function FolderRow({ findId, folder }: { findId: string; folder: RecentFolder }) {
  const [error, setError] = useState<Failure | null>(null);
  const reveal = () => {
    setError(null);
    revealFindResult(findId, folder.resultId).catch((reason: unknown) => setError(failureOf(reason)));
  };
  return (
    <Row
      icon={<FolderIconSvg />}
      name={folder.name}
      lines={[folder.folder]}
      actions={
        <button type="button" aria-label={`Reveal ${folder.name} in Finder`} onClick={reveal}>
          Reveal in Finder
        </button>
      }
    >
      {error && <p className="find-row-error">{failureMessage(error)}</p>}
    </Row>
  );
}
