// Memory's page (#35): the files Memory records, turning it on or off for
// a file, what happens when an app replaces a file (#31), the files waiting
// for a decision, and one file's history and search (MemoryHistory). Since
// #248 it is a library of the recorded files; choosing one shows that file's
// page with three tabs: Versions, This work and Waiting files. The
// top-level "Memory" tab shows it (Claude-A's entry). The recordings and
// their rules are the core's; this page shows its views.

import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { getAppState } from "../organize/api";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type RootView } from "../organize/types";
import { CandidateCaptureCard, PendingCard } from "./CandidateCards";
import { familyApi } from "../family/api";
import type { AutoMemoryView, MemoryRecordView } from "../family/types";
import { AutoMemoryCard } from "./AutoMemoryCard";
import { MemoryHistory } from "./MemoryHistory";
import { MoveCard } from "./MoveCard";
import { HoldCard, RelationsCard } from "./RelationCards";
import { RelocateCard } from "./RelocateCard";
import { WorksCard } from "./WorksCard";
import "./memory.css";
import {
  RECORD_GONE_TEXT,
  STARTED_BY_TRAEVA,
  canRelocate,
  gapText,
  libraryGroup,
  autoMemoryLimitText,
  autoMemoryState,
  libraryText,
  namedPlace,
  needsCount,
  needsDecision,
  recordSignature,
  recordStateText,
  recordTabText,
  recordToOpen,
  startedByTraeva,
  tabHasNews,
  waitsText,
  type RecordTab,
} from "./records";
import { ShowMore } from "../../shared/ShowMore";
import { Group } from "../../shared/Group";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** `active`: the Memory tab is showing. Hidden, the page asks nothing;
 * shown again, it asks at once. `openChainId`: a record to show, asked for
 * from outside the page (Find's Versions button, #198), the first time the
 * tab loads or while it is open. The page reads the list again, shows the
 * record (or says it is no longer available) and calls `onChainOpened`:
 * the caller then clears `openChainId`, so the same record can be asked
 * for again. */
export function MemoryPage({
  active = true,
  openChainId = null,
  onChainOpened,
}: {
  active?: boolean;
  openChainId?: string | null;
  onChainOpened?: () => void;
}) {
  const [records, setRecords] = useState<MemoryRecordView[] | null>(null);
  const [roots, setRoots] = useState<RootView[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [auto, setAuto] = useState<AutoMemoryView | null>(null);
  const [gone, setGone] = useState(false);
  const [reveal, setReveal] = useState(0);
  const [starting, setStarting] = useState(false);
  const top = useRef<HTMLElement>(null);
  const opened = useRef(onChainOpened);
  useEffect(() => {
    opened.current = onChainOpened;
  });

  const refresh = useCallback(() => {
    familyApi.listRecords().then(setRecords, (reason: unknown) => setError(asError(reason)));
    // The switch starts records in the background: its labels and count
    // are read with the list. Unreadable, the card is not shown.
    familyApi.autoMemory().then(setAuto, () => {});
  }, []);
  // Folders are added in Settings: read them each time the tab
  // shows, not only the first time.
  useEffect(() => {
    if (!active) return;
    getAppState().then((state) => setRoots(state.roots.filter((root) => root.status === "active")), () => {});
  }, [active]);
  // Recordings change in the background (new versions, a replaced or lost
  // file): ask again now and then while the tab shows.
  useEffect(() => {
    if (!active) return;
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, [active, refresh]);

  // A record asked for from outside: the list read after the request says
  // whether it is still there.
  useEffect(() => {
    if (openChainId === null) return;
    let live = true;
    familyApi.listRecords().then(
      (list) => {
        if (!live) return;
        setRecords(list);
        const found = recordToOpen(list, openChainId);
        setGone(found === null);
        setChosen(found ? found.chain_id : null);
        setReveal((n) => n + 1);
        opened.current?.();
      },
      (reason: unknown) => {
        if (!live) return;
        setError(asError(reason));
        opened.current?.();
      },
    );
    return () => {
      live = false;
    };
  }, [openChainId]);
  // Going from the library to a file's page and back starts at the top
  // (not when the page first loads).
  const moved = useRef(false);
  useEffect(() => {
    if (!moved.current) {
      moved.current = true;
      return;
    }
    top.current?.scrollIntoView({ block: "start" });
  }, [chosen, reveal]);
  // Back in the library, the focus returns to the row of the file that was
  // shown, so a keyboard or VoiceOver user goes on where they were.
  const left = useRef<string | null>(null);
  useEffect(() => {
    if (chosen !== null || left.current === null) return;
    const id = left.current;
    left.current = null;
    document.querySelector<HTMLButtonElement>(`.memory-file[data-chain="${CSS.escape(id)}"]`)?.focus();
  }, [chosen, records]);
  const choose = (chainId: string | null) => {
    setGone(false);
    if (chainId === null) left.current = chosen;
    setChosen(chainId);
  };

  const record = records?.find((r) => r.chain_id === chosen) ?? null;
  const readError = error && (
    <section className="card card-error">
      <h2>Memory could not be read</h2>
      <ErrorDetails error={error} />
    </section>
  );
  // With no folder or no recorded file yet, the card to record one is
  // always open; otherwise "Record a file…" opens it.
  const startShown = roots.length === 0 || records?.length === 0;
  // The switch's card waits under the list, unless it needs the user: its
  // agreement no longer counts, or it reached its limit.
  const autoFirst = auto !== null && (autoMemoryState(auto) === "outdated" || autoMemoryLimitText(auto) !== null);
  return (
    <main className="page memory-page" ref={top}>
      {record ? (
        <RecordPage
          key={record.chain_id}
          record={record}
          roots={roots}
          auto={auto}
          active={active}
          reveal={reveal}
          readError={readError}
          onBack={() => choose(null)}
          onChange={refresh}
        />
      ) : (
        <>
          <header className="page-header memory-head">
            <div className="grow">
              <h1>Memory</h1>
              {records && <p className="muted memory-summary">{libraryText(records)}</p>}
            </div>
            {!startShown && (
              <button
                type="button"
                aria-expanded={starting}
                aria-controls="memory-start"
                onClick={() => setStarting(!starting)}
              >
                Record a file…
              </button>
            )}
          </header>
          {readError}
          <p className="muted small">
            Memory keeps each saved version of the files you choose, on this Mac. It reads only those files and never
            changes them.
          </p>
          {gone && (
            <section className="card">
              <p className="warn">{RECORD_GONE_TEXT}</p>
            </section>
          )}
          {(starting || startShown) && (
            <StartCard
              roots={roots}
              onStarted={(r) => {
                setStarting(false);
                refresh();
                choose(r.chain_id);
              }}
            />
          )}
          {auto && autoFirst && <AutoMemoryCard view={auto} onChange={setAuto} onReload={refresh} />}
          {records && records.length > 0 && <FileLibrary records={records} auto={auto} onChoose={choose} />}
          {auto && !autoFirst && <AutoMemoryCard view={auto} onChange={setAuto} onReload={refresh} />}
        </>
      )}
    </main>
  );
}

/** Choose a file to record: pick the folder here, then the file in the
 * native picker (opened by Rust, so a path never comes from the page). */
function StartCard({ roots, onStarted }: { roots: RootView[]; onStarted: (record: MemoryRecordView) => void }) {
  const [rootId, setRootId] = useState("");
  const [error, setError] = useState<AppError | null>(null);
  const start = () => {
    setError(null);
    familyApi.startRecord(rootId).then(
      (record) => {
        if (record) onStarted(record);
      },
      (reason: unknown) => setError(asError(reason)),
    );
  };
  return (
    <section className="card" id="memory-start">
      <h2>Record a file</h2>
      {roots.length === 0 ? (
        <p className="muted">Add a folder in Settings first. Memory records files in those folders only.</p>
      ) : (
        <div className="row">
          <select value={rootId} onChange={(event) => setRootId(event.target.value)}>
            <option value="">Choose a folder…</option>
            {roots.map((root) => (
              <option key={root.root_id} value={root.root_id}>
                {root.display_path}
              </option>
            ))}
          </select>
          <button type="button" disabled={rootId === ""} onClick={start}>
            Choose a file to record
          </button>
        </div>
      )}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** The recorded files (#248) in groups (#37, the student's layout B,
 * 10/14): what waits for the user first, always open; then what Memory
 * records; then what is stopped, closed at first. Each row names the file
 * first and its folder under it (#246), how its recording stands, and
 * whether files wait for a decision; choosing a row shows the file's page. */
function FileLibrary({
  records,
  auto,
  onChoose,
}: {
  records: MemoryRecordView[];
  /** The switch (#201): which records it started. */
  auto: AutoMemoryView | null;
  onChoose: (chainId: string) => void;
}) {
  const needs = records.filter((record) => libraryGroup(record) === "needs");
  const recording = records.filter((record) => libraryGroup(record) === "recording");
  const stopped = records.filter((record) => libraryGroup(record) === "stopped");
  const list = (items: MemoryRecordView[]) => (
    <ul className="memory-files">
      {items.map((record) => (
        <FileRow key={record.chain_id} record={record} auto={auto} onChoose={onChoose} />
      ))}
    </ul>
  );
  const files = (count: number) => (count === 1 ? "1 file" : `${count} files`);
  return (
    <section className="memory-library" aria-label="Recorded files">
      {needs.length > 0 && (
        <Group id="memory-needs" title="Needs you" count={needs.length} attention>
          {list(needs)}
        </Group>
      )}
      {recording.length > 0 && (
        <Group id="memory-recording" title="Recording" count={recording.length} summary={files(recording.length)}>
          <ShowMore items={recording} first={10}>
            {list}
          </ShowMore>
        </Group>
      )}
      {stopped.length > 0 && (
        <Group
          id="memory-stopped"
          title="Stopped"
          count={stopped.length}
          defaultOpen={false}
          summary={`${files(stopped.length)}, versions kept`}
        >
          <ShowMore items={stopped} first={10}>
            {list}
          </ShowMore>
        </Group>
      )}
    </section>
  );
}

/** One recorded file in the library. */
function FileRow({
  record,
  auto,
  onChoose,
}: {
  record: MemoryRecordView;
  auto: AutoMemoryView | null;
  onChoose: (chainId: string) => void;
}) {
  const state = recordStateText(record);
  const place = namedPlace(record.place.display_path);
  const waiting = needsDecision(record);
  // The row's name is short (the file, how it stands); the rest is its
  // description, so VoiceOver does not read every sentence.
  const id = `memory-file-${record.chain_id}`;
  return (
    <li>
      <button
        type="button"
        className="memory-file"
        data-chain={record.chain_id}
        aria-labelledby={`${id}-name ${id}-state`}
        aria-describedby={[place.folder && `${id}-folder`, `${id}-more`, waiting && `${id}-waits`].filter(Boolean).join(" ")}
        onClick={() => onChoose(record.chain_id)}
      >
        <span className="grow">
          <span className="memory-file-name" id={`${id}-name`}>
            <strong>{place.name}</strong>
            {startedByTraeva(auto, record.chain_id) && <span className="badge">{STARTED_BY_TRAEVA}</span>}
          </span>
          {place.folder && (
            <span className="mono small muted memory-line" id={`${id}-folder`}>
              {place.folder}
            </span>
          )}
          <span className={`small memory-line ${state.tone === "warn" ? "warn" : "muted"}`}>
            <span id={`${id}-state`}>{state.label}.</span> <span id={`${id}-more`}>{state.detail}</span>
          </span>
          {waiting && (
            <span className="small memory-line memory-waits" id={`${id}-waits`}>
              <span className="memory-dot" aria-hidden="true" />
              {waitsText(record)}
            </span>
          )}
        </span>
        <span className="memory-chevron" aria-hidden="true">
          ›
        </span>
      </button>
    </li>
  );
}

const TABS: RecordTab[] = ["versions", "work", "waiting"];

/** One file's page (#248): its name and folder, how its recording stands,
 * what needs attention (times Traeva did not see, a paused record, a file
 * to find again), then three tabs. Every tab stays mounted, so a search or
 * a choice in progress survives switching; the history asks for news only
 * while its tab shows. */
function RecordPage({
  record,
  roots,
  auto,
  active,
  reveal,
  readError,
  onBack,
  onChange,
}: {
  record: MemoryRecordView;
  roots: RootView[];
  auto: AutoMemoryView | null;
  active: boolean;
  /** Bumped when Find asks for this record again: show its versions. */
  reveal: number;
  /** The library could not be read (shown under the title). */
  readError: ReactNode;
  onBack: () => void;
  onChange: () => void;
}) {
  const place = namedPlace(record.place.display_path);
  const state = recordStateText(record);
  const [tab, setTab] = useState<RecordTab>("versions");
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    if (reveal === 0) return;
    setTab("versions");
    title.current?.focus();
  }, [reveal]);
  // Arriving on the page (from the library or Find), the focus goes to the
  // file's name, so VoiceOver reads where the user is.
  const title = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    title.current?.focus();
  }, []);
  const tabs = useRef<Record<RecordTab, HTMLButtonElement | null>>({ versions: null, work: null, waiting: null });
  // A merge, an association, a correction or a move can change the work's
  // members and relations without changing this record: the work's cards
  // read again after each one.
  const [workDecisions, setWorkDecisions] = useState(0);
  const workChanged = () => {
    setWorkDecisions((n) => n + 1);
    onChange();
  };
  const workSignature = `${recordSignature(record)}|${record.family_id}|${workDecisions}`;
  const stop = () => {
    setError(null);
    familyApi.stopRecord(record.chain_id).then(
      () => {
        // The button goes away with the recording: keep the focus on the page.
        title.current?.focus();
        onChange();
      },
      (reason: unknown) => setError(asError(reason)),
    );
  };
  // Left and right arrows move between the tabs (WAI-ARIA tabs pattern).
  const onTabKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0) return;
    event.preventDefault();
    const next = TABS[(TABS.indexOf(tab) + step + TABS.length) % TABS.length];
    setTab(next);
    tabs.current[next]?.focus();
  };
  return (
    <div>
      <button type="button" className="memory-back" onClick={onBack}>
        <span aria-hidden="true">‹ </span>All files
      </button>
      <header className="memory-file-head">
        <div className="grow">
          <h1 className="memory-title" ref={title} tabIndex={-1}>
            {place.name}
            {startedByTraeva(auto, record.chain_id) && <span className="badge">{STARTED_BY_TRAEVA}</span>}
          </h1>
          {place.folder && <p className="mono small muted">{place.folder}</p>}
          <p className={`small ${state.tone === "warn" ? "warn" : "muted"}`}>
            {state.label}. {state.detail}
          </p>
        </div>
        {record.state !== "stopped" && (
          <button type="button" onClick={stop}>
            Stop recording
          </button>
        )}
      </header>
      {readError}
      {error && <ErrorDetails error={error} />}
      {/* What waits for the user (#37, layout B): a paused record, a file to
          find again, files to decide about or the file at the place. Above
          the tabs, so no tab hides it; always open. Nothing waits: no group,
          and the decision card still decides by itself whether it shows. */}
      {record.hold !== null || canRelocate(record) || needsDecision(record) ? (
        <Group id="memory-file-needs" title="Needs you" count={needsCount(record)} attention>
          <HoldCard record={record} onChange={onChange} />
          {canRelocate(record) && <RelocateCard record={record} roots={roots} onChange={onChange} />}
          <PendingCard chainId={record.chain_id} changed={recordSignature(record)} onDecided={onChange} />
        </Group>
      ) : (
        <PendingCard chainId={record.chain_id} changed={recordSignature(record)} onDecided={onChange} />
      )}
      {record.open_gaps.length > 0 && (
        <section className="card">
          <h2>Times Traeva did not see</h2>
          <ShowMore items={record.open_gaps} first={3}>
            {(visible) => (
              <ul className="plain small">
                {visible.map((gap) => (
                  <li key={gap.gap_id}>
                    Since {new Date(gap.started_at_ms).toLocaleString()}: {gapText(gap)}
                  </li>
                ))}
              </ul>
            )}
          </ShowMore>
        </section>
      )}
      <div className="memory-tabs" role="tablist" aria-label={place.name}>
        {TABS.map((each) => (
          <button
            key={each}
            ref={(element) => {
              tabs.current[each] = element;
            }}
            type="button"
            role="tab"
            id={`memory-tab-${each}`}
            aria-controls={`memory-panel-${each}`}
            aria-selected={tab === each}
            tabIndex={tab === each ? 0 : -1}
            className="memory-tab"
            onClick={() => setTab(each)}
            onKeyDown={onTabKey}
          >
            {recordTabText(each)}
            {tabHasNews(each, record) && <span className="memory-dot" aria-hidden="true" />}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id="memory-panel-versions"
        aria-labelledby="memory-tab-versions"
        tabIndex={0}
        hidden={tab !== "versions"}
      >
        <MemoryHistory
          fileId={record.watch.file_id}
          consentFileId={record.permission_file_id}
          only={record.watch.latest}
          active={active && tab === "versions"}
          onChange={workChanged}
        />
      </div>
      <div
        role="tabpanel"
        id="memory-panel-work"
        aria-labelledby="memory-tab-work"
        tabIndex={0}
        hidden={tab !== "work"}
      >
        <WorksCard record={record} roots={roots} onChange={workChanged} />
        <RelationsCard record={record} changed={workSignature} onChange={workChanged} />
        <MoveCard record={record} changed={workSignature} onChange={workChanged} />
      </div>
      <div
        role="tabpanel"
        id="memory-panel-waiting"
        aria-labelledby="memory-tab-waiting"
        tabIndex={0}
        hidden={tab !== "waiting"}
      >
        <CandidateCaptureCard chainId={record.chain_id} changed={recordSignature(record)} onChange={onChange} />
      </div>
    </div>
  );
}
