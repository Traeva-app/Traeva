// The quick search window (#208, the student's idea, 10/13): a click on
// the desktop pet, when nothing waits for the user, opens this small window
// next to it. At first only the search box shows; the window grows with
// the results (the core keeps its height between 64 and 480 px). Return
// searches, and once the results are for what is typed it opens the chosen
// one; the arrow keys choose. Esc, or a click anywhere else, hides it (the
// core also hides it when it loses the focus; the page's own blur is a
// backstop).
//
// The search is Find's (contract §4.10): the core searches and orders; this
// window shows the first results in that order and names a result by its
// search and id, never by a path. Agreeing to search with the model happens
// in Traeva's main window only.

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { hideQuickSearch, resizeQuickSearch, showMainWindow } from "../features/pet/api";
import {
  getFind,
  getFindSetup,
  openFindResult,
  revealFindResult,
  startFind,
} from "../features/find/api";
import { FileIconSvg, SearchGlyph } from "../features/find/icons";
import type { FindResult, FindSetupView, FindView } from "../features/find/types";
import { MAX_FIND_QUESTION_CHARS } from "../features/find/types";
import { accepts, fileIcon, modelUse, wordsByText } from "../features/find/view";
import { isAppError } from "../features/organize/types";
import { enterAction, moveChoice, quickWindowHeight, shownResults, statusText } from "./quick";
import "../features/find/find.css";
import "./quick.css";

const POLL_MS = 300;

function messageOf(reason: unknown): string {
  return isAppError(reason) ? reason.message : String(reason);
}

function hide() {
  hideQuickSearch().catch(() => undefined);
}

export function QuickSearch() {
  const [text, setText] = useState("");
  const [view, setView] = useState<FindView | null>(null);
  // The text the shown search was asked with: Return opens a result only
  // while the box still says that.
  const [asked, setAsked] = useState<string | null>(null);
  const [setup, setSetup] = useState<FindSetupView | null>(null);
  const [chosen, setChosen] = useState(0);
  const [failure, setFailure] = useState<string | null>(null);
  const [rowError, setRowError] = useState<{ resultId: string; message: string } | null>(null);
  const latest = useRef<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const root = useRef<HTMLDivElement>(null);

  // The window is as tall as what it shows: asked for whenever that
  // changes, and again each time the window is shown, since the core opens
  // it at the search box's height while the last results are still here
  // (#242).
  const fit = useCallback(() => {
    const box = root.current;
    if (box === null) return;
    resizeQuickSearch(quickWindowHeight(box.getBoundingClientRect().height)).catch(() => undefined);
  }, []);

  // Shown again (the pet was clicked): the cursor goes to the box with the
  // last words selected, the window takes the height of what it shows, and
  // whether the model may help is read again. Another app or window in
  // front: hidden, as Spotlight does.
  useEffect(() => {
    const shown = () => {
      input.current?.focus();
      input.current?.select();
      fit();
      getFindSetup()
        .then(setSetup)
        .catch(() => setSetup(null));
    };
    shown();
    window.addEventListener("focus", shown);
    window.addEventListener("blur", hide);
    return () => {
      window.removeEventListener("focus", shown);
      window.removeEventListener("blur", hide);
    };
  }, [fit]);

  useEffect(() => {
    const box = root.current;
    if (box === null) return;
    const observer = new ResizeObserver(fit);
    observer.observe(box);
    return () => observer.disconnect();
  }, [fit]);

  // A search under way is read again until it is done or cancelled.
  useEffect(() => {
    if (view === null || view.state !== "working") return;
    const timer = window.setTimeout(() => {
      getFind(view.find_id)
        .then((next) => setView((shown) => (accepts(latest.current, shown, next) ? next : shown)))
        .catch((reason: unknown) => {
          if (latest.current === view.find_id) setFailure(messageOf(reason));
        });
    }, POLL_MS);
    return () => window.clearTimeout(timer);
  }, [view]);

  const results = shownResults(view);
  const current = view !== null && asked === text;

  const search = () => {
    const question = text.trim();
    if (question === "") return;
    setFailure(null);
    setRowError(null);
    setChosen(0);
    startFind(question)
      .then((next) => {
        latest.current = next.find_id;
        setAsked(text);
        setView(next);
      })
      .catch((reason: unknown) => setFailure(messageOf(reason)));
  };

  const open = (result: FindResult, how: "open" | "reveal") => {
    if (view === null) return;
    setRowError(null);
    const call = how === "open" ? openFindResult : revealFindResult;
    call(view.find_id, result.result_id)
      .then(hide)
      .catch((reason: unknown) => setRowError({ resultId: result.result_id, message: messageOf(reason) }));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      hide();
    } else if (event.key === "Enter") {
      // Return that ends typing in an input method (Chinese, Japanese…)
      // belongs to it.
      if (event.nativeEvent.isComposing) return;
      event.preventDefault();
      const action = enterAction(view, current);
      if (action === "open") open(results[chosen] ?? results[0], "open");
      else if (action === "search") search();
    } else if ((event.key === "ArrowDown" || event.key === "ArrowUp") && current && results.length > 0) {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setChosen((index) => moveChoice(index, results.length, step));
    }
  };

  const status = statusText(view, failure);
  const askModel = setup !== null && modelUse(setup) === "ask";

  return (
    <div className="quick" ref={root}>
      <label className="quick-box">
        <SearchGlyph />
        <input
          ref={input}
          type="text"
          value={text}
          maxLength={MAX_FIND_QUESTION_CHARS}
          placeholder="Find a file or describe what you remember…"
          aria-label="What do you remember about the file?"
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={onKeyDown}
        />
      </label>

      {view !== null && (
        <div className="quick-body">
          {results.length > 0 && (
            <ul className="quick-results" aria-label="Results">
              {results.map((result, index) => (
                <li
                  key={result.result_id}
                  className={index === chosen && current ? "quick-row quick-row-chosen" : "quick-row"}
                  onMouseEnter={() => setChosen(index)}
                >
                  <FileIconSvg icon={fileIcon(result.kind, result.name)} />
                  <div className="quick-row-text">
                    <div className="quick-row-name">{result.name}</div>
                    <div className="quick-row-line">{result.folder}</div>
                    {rowError?.resultId === result.result_id && (
                      <div className="quick-row-error">{rowError.message}</div>
                    )}
                  </div>
                  <div className="quick-row-actions">
                    <button type="button" onClick={() => open(result, "open")}>
                      Open
                    </button>
                    <button type="button" onClick={() => open(result, "reveal")}>
                      Show in Finder
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
          {status && <p className="quick-status">{status}</p>}
          {view.state === "done" && view.words_by !== null && (
            <p className="quick-note">
              {wordsByText(view.words_by)}
              {askModel && (
                <>
                  {" "}
                  <button type="button" className="quick-link" onClick={() => showMainWindow("find").catch(() => undefined)}>
                    Turn on searching with the model in Traeva
                  </button>
                </>
              )}
            </p>
          )}
          <div className="quick-footer">
            <span className="quick-hint">
              {current && results.length > 0 ? "Return to open · ↑↓ to choose · Esc to close" : "Return to search · Esc to close"}
            </span>
            <button type="button" className="quick-link" onClick={() => showMainWindow("find").catch(() => undefined)}>
              Open Traeva
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
