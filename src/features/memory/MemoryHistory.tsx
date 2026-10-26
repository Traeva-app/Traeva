// One file's Memory (#33/#35): the agreement to send its changed lines,
// a search by change (MemorySearch.tsx), and its saved versions with what
// changed between them. Every version opens as a new read-only copy, never
// as the user's file. The Versions tab of a file's page in Memory (#248).

import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import {
  chooseMemoryExport,
  exportMemoryVersion,
  getMemoryChanges,
  getMemoryConsent,
  openMemoryVersion,
  setMemoryConsent,
} from "./api";
import { ExportCard, type ExportStep } from "./ExportCard";
import { previewDocxVersion } from "../docx/api";
import { DocxSnapshotPreview } from "../docx/DocxSnapshotPreview";
import { ChangeItem } from "./ChangeItem";
import { VersionTextPreview } from "./VersionTextPreview";
import {
  DECK_PREVIEW_NOTE,
  DOCX_PREVIEW_NOTE,
  WORKBOOK_PREVIEW_NOTE,
  previewKind,
  MODEL_WORDS_NOTE,
  isDocxVersion,
  openActionText,
  openedCopyRef,
  canExport,
  exportName,
  exportRefusal,
  historyKey,
  isSettling,
  lineNumbers,
  lineVersions,
  minorText,
  pairGapText,
  pairStateText,
  pairsAround,
  STOP_UNCLEAR_TEXT,
  stopLabel,
  stopNotes,
  timelineLabel,
  versionLabel,
  versionRef,
  type LineNumbers,
} from "./history";
import { useShowMore } from "../../shared/ShowMore";
import { MemorySearch } from "./MemorySearch";
import { UndoLinkNote } from "./RelationCards";
import type {
  ExportPreview,
  MemoryConsentView,
  OpenedVersion,
  SnapshotRef,
  VersionPairView,
} from "./types";

/** `consentFileId`: the file whose Memory agreement covers this history,
 * the recording's first file (#31: a confirmed continuation keeps it).
 * `only`: the newest saved version, shown on its own while there is no pair
 * yet. `active`: the page shows; hidden, it asks nothing. `onChange`: the
 * user undid a link Traeva made by itself (the recording pauses). */
export function MemoryHistory({
  fileId,
  consentFileId = fileId,
  only = null,
  active = true,
  onChange,
}: {
  fileId: string;
  consentFileId?: string;
  only?: SnapshotRef | null;
  active?: boolean;
  onChange?: () => void;
}) {
  const [consent, setConsent] = useState<MemoryConsentView | null>(null);
  const [pairs, setPairs] = useState<VersionPairView[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [opened, setOpened] = useState<OpenedVersion | null>(null);
  const [openError, setOpenError] = useState<AppError | null>(null);

  // Where the agreement's card sits is decided when it is read, not when
  // the user agrees or stops: the card stays where it was clicked.
  const [consentFirst, setConsentFirst] = useState(true);
  const refresh = useCallback(() => {
    getMemoryConsent(consentFileId).then(
      (read) => {
        setConsent(read);
        setConsentFirst(!read.agreed);
      },
      (reason: unknown) => setError(asError(reason)),
    );
    getMemoryChanges(fileId).then(setPairs, (reason: unknown) => setError(asError(reason)));
  }, [fileId, consentFileId]);

  useEffect(() => {
    if (active) refresh();
  }, [active, refresh]);
  const linkUndone = () => {
    refresh();
    onChange?.();
  };

  // New versions and descriptions arrive in the background: ask again
  // often while a pair is on its way, now and then otherwise.
  const settling = pairs !== null && isSettling(pairs);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => {
      getMemoryChanges(fileId).then(setPairs, () => {});
    }, settling ? 2000 : 10000);
    return () => clearInterval(timer);
  }, [active, fileId, settling]);

  const numbers = useMemo(() => lineNumbers(pairs ?? [], only), [pairs, only]);
  const changed = useMemo(() => historyKey(pairs), [pairs]);
  // The version shown in the card (#248): the newest until the user picks
  // one on the timeline or a search finds one.
  const versions = useMemo(() => lineVersions(pairs ?? [], only), [pairs, only]);
  const [chosenId, setChosenId] = useState<string | null>(null);
  const chosen = versions.find((version) => version.snapshot_id === chosenId) ?? versions[versions.length - 1] ?? null;

  // A Word, PowerPoint or Excel version is never opened in its app (#66,
  // #28): its saved text is shown on the page instead.
  const [previewing, setPreviewing] = useState<SnapshotRef | null>(null);
  const open = useCallback((version: SnapshotRef) => {
    setOpened(null);
    setOpenError(null);
    if (isDocxVersion(version) || previewKind(version) !== null) {
      setPreviewing(version);
      return;
    }
    setPreviewing(null);
    openMemoryVersion(version.snapshot_id).then(setOpened, (reason: unknown) => setOpenError(asError(reason)));
  }, []);

  // Exporting a version: the save panel, then a preview, then the user's
  // confirmation; nothing is written before it.
  const [exporting, setExporting] = useState<ExportStep | null>(null);
  const [allShown, setAllShown] = useState(false);
  // What opening, previewing or exporting gave shows under the version's
  // card; brought into view, since the button may be in the search above.
  // The focus goes there too, so VoiceOver reads it and the keyboard goes
  // on from it (a confirmed export replaces the button that had it). A
  // preview grows when its text arrives: its top is brought into view.
  const results = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const box = results.current;
    if (!box || !(opened || openError || previewing || exporting)) return;
    box.focus({ preventScroll: true });
    box.scrollIntoView({ block: previewing ? "start" : "nearest" });
  }, [opened, openError, previewing, exporting]);
  const [exportBusy, setExportBusy] = useState(false);
  const exportFailed = useCallback((reason: unknown) => {
    setExportBusy(false);
    const failure = asError(reason);
    if (failure?.code === "result_unconfirmed") {
      setExporting({ kind: "unconfirmed", error: failure });
      return;
    }
    if (failure?.code === "invalid_input") {
      setExporting({ kind: "refused", text: failure.message });
      return;
    }
    const text = failure ? exportRefusal(failure.code) : null;
    setExporting(text ? { kind: "refused", text } : { kind: "error", error: failure });
  }, []);
  const startExport = useCallback(
    (version: SnapshotRef) => {
      setExporting(null);
      setExportBusy(true);
      chooseMemoryExport(version.snapshot_id, version.root_id, exportName(version, numbers)).then((preview) => {
        setExportBusy(false);
        setExporting(preview ? { kind: "preview", preview } : null);
      }, exportFailed);
    },
    [numbers, exportFailed],
  );
  const confirmExport = (shown: ExportPreview) => {
    setExportBusy(true);
    exportMemoryVersion({
      snapshot_id: shown.snapshot.snapshot_id,
      root_id: shown.root_id,
      folder: shown.folder,
      name: shown.name,
      page_token: shown.page_token,
    }).then((done) => {
      setExportBusy(false);
      setExporting({ kind: "done", done });
    }, exportFailed);
  };

  return (
    <div>
      <p className="muted small">
        Saved versions of this file, and what changed between them. Opening a version opens a
        read-only copy; your file is never changed.
      </p>
      {error && (
        <section className="card card-error">
          <h2>Memory could not be read</h2>
          <ErrorDetails error={error} />
        </section>
      )}
      {/* Not agreed yet, the agreement comes first: without it nothing is
          described or searched. Agreed, it waits at the bottom (#248). */}
      {consent && consentFirst && <ConsentCard consent={consent} onChange={setConsent} />}
      <MemorySearch
        fileId={fileId}
        numbers={numbers}
        historyKey={changed}
        onOpen={open}
        onFound={(version) => setChosenId(version.snapshot_id)}
      />
      {pairs && versions.length === 0 && (
        <section className="card">
          <p className="muted">No version is saved yet.</p>
        </section>
      )}
      {pairs && chosen && (
        <ChosenVersion
          version={chosen}
          versions={versions}
          pairs={pairs}
          numbers={numbers}
          onChoose={(version) => setChosenId(version.snapshot_id)}
          onOpen={open}
          onExport={exportBusy ? null : startExport}
          onLinkUndone={linkUndone}
        />
      )}
      <div ref={results} tabIndex={-1} className="memory-results">
        <OpenedNotice opened={opened} error={openError} />
        {previewing && (
          <section className="card">
            <p>
              <strong>{versionLabel(previewing, numbers)}</strong>
            </p>
            {isDocxVersion(previewing) ? (
              <>
                <DocxSnapshotPreview snapshot={previewing} previewApi={previewDocxVersion} />
                <p className="muted small">{DOCX_PREVIEW_NOTE}</p>
              </>
            ) : (
              <VersionTextPreview
                snapshot={previewing}
                note={previewKind(previewing) === "deck" ? DECK_PREVIEW_NOTE : WORKBOOK_PREVIEW_NOTE}
              />
            )}
            <button type="button" onClick={() => setPreviewing(null)}>
              Close
            </button>
          </section>
        )}
        {exporting && (
          <ExportCard
            step={exporting}
            numbers={numbers}
            busy={exportBusy}
            onConfirm={confirmExport}
            onClose={() => setExporting(null)}
          />
        )}
      </div>
      {pairs && versions.length > 1 && (
        <details className="memory-all" onToggle={(event) => setAllShown(event.currentTarget.open)}>
          <summary>All versions and what changed ({versions.length})</summary>
          {/* Drawn only while open: each change reads its lines again from
              the saved versions. */}
          {allShown && (
            <VersionsCard
              pairs={pairs}
              only={only}
              numbers={numbers}
              onOpen={open}
              onExport={exportBusy ? null : startExport}
              onLinkUndone={linkUndone}
            />
          )}
        </details>
      )}
      {consent && !consentFirst && <ConsentCard consent={consent} onChange={setConsent} />}
    </div>
  );
}

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** The agreement to send this file's changed lines (#33): separate from
 * Organize's, for this file only, and it can be withdrawn at any time. */
function ConsentCard({
  consent,
  onChange,
}: {
  consent: MemoryConsentView;
  onChange: (consent: MemoryConsentView) => void;
}) {
  const [error, setError] = useState<AppError | null>(null);
  const change = (agree: boolean) => {
    setError(null);
    setMemoryConsent(consent.file_id, agree).then(onChange, (reason: unknown) => setError(asError(reason)));
  };
  const service = consent.provider && consent.model ? `${consent.provider} · ${consent.model}` : null;
  return (
    <section className={consent.agreed ? "card card-ok" : "card"}>
      <h2>Describing changes with the model</h2>
      {service === null ? (
        <p>Set up a model in Settings first. Until then, changes are compared on this Mac only.</p>
      ) : consent.agreed ? (
        <>
          <p>
            Memory may send this file's changed lines to {service}
            {consent.agreed_at_ms !== null && ` (since ${new Date(consent.agreed_at_ms).toLocaleString()})`}.
            This covers later saves too, until you stop it.
          </p>
          <button type="button" onClick={() => change(false)}>
            Stop sending
          </button>
        </>
      ) : (
        <>
          {consent.outdated && (
            <p className="warn">The model settings changed since you agreed. Agree again to go on.</p>
          )}
          <p style={{ whiteSpace: "pre-wrap" }}>{consent.notice}</p>
          <button type="button" onClick={() => change(true)}>
            Agree for this file
          </button>
        </>
      )}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** What opening a version gave: a read-only copy, opened or not. */
function OpenedNotice({ opened, error }: { opened: OpenedVersion | null; error: AppError | null }) {
  if (error) {
    return (
      <section className="card card-error">
        <h2>The version could not be opened</h2>
        <ErrorDetails error={error} />
      </section>
    );
  }
  if (!opened) return null;
  const { copy } = opened;
  return (
    <section className={opened.opened ? "card card-ok" : "card card-error"}>
      <h2>
        {opened.opened
          ? `Opened a copy of ${openedCopyRef(copy)}`
          : `A copy of ${openedCopyRef(copy)} was made but not opened`}
      </h2>
      <p>
        {opened.opened
          ? "This is a read-only copy, not your file. Saving it in the app asks for a new name; your file and the saved version stay as they are."
          : copy.can_open
            ? "macOS could not open the copy (perhaps no app opens this kind of file). The read-only copy is here:"
            : "Traeva opens copies of text files only (.txt, .md, .markdown). The read-only copy of this version is here:"}
      </p>
      <dl>
        <dt>Copy</dt>
        <dd className="mono small">{copy.path}</dd>
        <dt>SHA-256</dt>
        <dd className="mono small">{copy.sha256}</dd>
        <dt>Check</dt>
        <dd>The copy's bytes were read back and match the saved version.</dd>
      </dl>
    </section>
  );
}

/** The version the Versions tab shows (#248, concept v2/v3 "4-memory-found"):
 * a timeline of the line's versions, then the chosen one, what changed to
 * make it, and the screen's one primary button, which opens it. */
function ChosenVersion({
  version,
  versions,
  pairs,
  numbers,
  onChoose,
  onOpen,
  onExport,
  onLinkUndone,
}: {
  version: SnapshotRef;
  versions: SnapshotRef[];
  pairs: VersionPairView[];
  numbers: LineNumbers;
  onChoose: (version: SnapshotRef) => void;
  onOpen: (version: SnapshotRef) => void;
  /** Null while an export is being chosen. */
  onExport: ((version: SnapshotRef) => void) | null;
  onLinkUndone: () => void;
}) {
  const { made } = pairsAround(pairs, version);
  return (
    <section className="card memory-version" aria-labelledby="memory-version-title">
      {versions.length > 1 && (
        <Timeline versions={versions} pairs={pairs} chosen={version} numbers={numbers} onChoose={onChoose} />
      )}
      <h2 id="memory-version-title" className="memory-version-title">
        {versionLabel(version, numbers)}
      </h2>
      {made ? (
        <PairDetails pair={made} numbers={numbers} onLinkUndone={onLinkUndone} />
      ) : (
        <p className="muted small">
          The first saved version.{pairs.length === 0 && " The next save of the file adds another."}
        </p>
      )}
      <div className="row memory-version-actions">
        {canExport(version) && (
          <button
            type="button"
            disabled={onExport === null}
            aria-describedby="memory-version-title"
            onClick={() => onExport?.(version)}
          >
            Export a copy…
          </button>
        )}
        <button type="button" className="primary" onClick={() => onOpen(version)}>
          {openActionText(version, versionRef(version, numbers))}
        </button>
      </div>
    </section>
  );
}

/** The line's versions as stops on a line, oldest on the left; the chosen
 * one is larger. One choice among them, so it is a radio group: arrows move
 * the choice, Tab leaves it. Many versions scroll sideways, and the chosen
 * one is brought into view. */
function Timeline({
  versions,
  pairs,
  chosen,
  numbers,
  onChoose,
}: {
  versions: SnapshotRef[];
  pairs: VersionPairView[];
  chosen: SnapshotRef;
  numbers: LineNumbers;
  onChoose: (version: SnapshotRef) => void;
}) {
  const stops = useRef<Map<string, HTMLButtonElement>>(new Map());
  const scroller = useRef<HTMLDivElement>(null);
  const made = versions.map((version) => pairsAround(pairs, version).made);
  const notes = made.map(stopNotes);
  // A stretch is dashed only when the pair with the gap joins the two
  // stops it is drawn between (a file's branches can sit in between).
  const dashed = versions.map(
    (_, index) => index > 0 && notes[index].gap && made[index]?.before.snapshot_id === versions[index - 1].snapshot_id,
  );
  // From the stop that has the focus, not the chosen one: they differ when
  // a new version arrives or VoiceOver moved the focus. The arrows go round
  // at the ends, as radio groups do.
  const onKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const last = versions.length - 1;
    const back = event.key === "ArrowLeft" || event.key === "ArrowUp";
    const on = event.key === "ArrowRight" || event.key === "ArrowDown";
    let to: number;
    if (event.key === "Home") to = 0;
    else if (event.key === "End") to = last;
    else if (on) to = index === last ? 0 : index + 1;
    else if (back) to = index === 0 ? last : index - 1;
    else return;
    event.preventDefault();
    const next = versions[to];
    onChoose(next);
    stops.current.get(next.snapshot_id)?.focus();
  };
  // Keep the chosen stop in view inside the timeline, without scrolling
  // the page.
  useEffect(() => {
    const box = scroller.current;
    const stop = stops.current.get(chosen.snapshot_id);
    if (!box || !stop || box.scrollWidth <= box.clientWidth) return;
    const outer = box.getBoundingClientRect();
    const inner = stop.getBoundingClientRect();
    box.scrollLeft += inner.left - outer.left - (outer.width - inner.width) / 2;
  }, [chosen.snapshot_id, versions.length]);
  const anyGap = dashed.some(Boolean);
  const anyUnclear = notes.some((note) => note.unclear);
  return (
    <div className="memory-timeline" ref={scroller}>
      <div className="memory-timeline-track">
        <div className="memory-stops" role="radiogroup" aria-label="Saved versions">
          {versions.map((version, index) => {
            const isChosen = version.snapshot_id === chosen.snapshot_id;
            const note = notes[index];
            return (
              <Fragment key={version.snapshot_id}>
                {/* The stretch before a version: dashed where Traeva may have
                    missed saves (the core's gap on the pair). */}
                {index > 0 && (
                  <span className={dashed[index] ? "memory-seg memory-seg-gap" : "memory-seg"} aria-hidden="true" />
                )}
                <button
                  type="button"
                  role="radio"
                  ref={(element) => {
                    if (element) stops.current.set(version.snapshot_id, element);
                    else stops.current.delete(version.snapshot_id);
                  }}
                  className={note.unclear ? "memory-stop memory-stop-unclear" : "memory-stop"}
                  aria-checked={isChosen}
                  aria-label={stopLabel(version, numbers, note)}
                  tabIndex={isChosen ? 0 : -1}
                  onClick={() => onChoose(version)}
                  onKeyDown={(event) => onKey(event, index)}
                />
              </Fragment>
            );
          })}
        </div>
        <div className="memory-timeline-ends small muted" aria-hidden="true">
          <span>{timelineLabel(versions[0], numbers)}</span>
          <span>{timelineLabel(versions[versions.length - 1], numbers)}</span>
        </div>
      </div>
      {(anyGap || anyUnclear) && (
        <p className="small muted memory-timeline-key" aria-hidden="true">
          {anyGap && (
            <span>
              <span className="memory-key-gap" /> Traeva may have missed saves in this stretch
            </span>
          )}
          {anyUnclear && (
            <span>
              <span className="memory-key-unclear" /> {STOP_UNCLEAR_TEXT}
            </span>
          )}
        </p>
      )}
    </div>
  );
}

/** Every saved version of the record's line, newest first, each with the
 * change from the one before it. */
function VersionsCard({
  pairs,
  only,
  numbers,
  onOpen,
  onExport,
  onLinkUndone,
}: {
  pairs: VersionPairView[];
  only: SnapshotRef | null;
  numbers: LineNumbers;
  onOpen: (version: SnapshotRef) => void;
  /** Null while an export is being chosen. */
  onExport: ((version: SnapshotRef) => void) | null;
  onLinkUndone: () => void;
}) {
  // 5 versions at first: each change shown reads its lines from the core.
  const newestFirst = [...pairs].reverse();
  const versions = useShowMore(newestFirst, 5);
  if (pairs.length === 0) {
    // One saved version is still a version to open (R2-MEM-01).
    return (
      <section className="card">
        <h2>Versions</h2>
        {only ? (
          <ul className="list">
            <li>
              <VersionRow version={only} numbers={numbers} onOpen={onOpen} onExport={onExport} />
              <p className="muted small">The first saved version. The next save of the file adds another.</p>
            </li>
          </ul>
        ) : (
          <p className="muted">No version is saved yet.</p>
        )}
      </section>
    );
  }
  const first = pairs[0].before;
  return (
    <section className="card">
      <h2>Versions</h2>
      <ul className="list">
        {versions.visible.map((pair) => (
          <li key={pair.after.snapshot_id}>
            <VersionRow version={pair.after} numbers={numbers} onOpen={onOpen} onExport={onExport} />
            <PairDetails pair={pair} numbers={numbers} onLinkUndone={onLinkUndone} />
          </li>
        ))}
        {/* The first saved version comes with the last batch. */}
        {versions.visible.length === newestFirst.length && (
          <li>
            <VersionRow version={first} numbers={numbers} onOpen={onOpen} onExport={onExport} />
            <p className="muted small">The first saved version.</p>
          </li>
        )}
      </ul>
      {versions.control}
    </section>
  );
}

function VersionRow({
  version,
  numbers,
  onOpen,
  onExport,
}: {
  version: SnapshotRef;
  numbers: LineNumbers;
  onOpen: (version: SnapshotRef) => void;
  onExport: ((version: SnapshotRef) => void) | null;
}) {
  return (
    <div className="row">
      <strong className="grow">{versionLabel(version, numbers)}</strong>
      <button type="button" onClick={() => onOpen(version)}>
        {openActionText(version)}
      </button>
      {canExport(version) && (
        <button type="button" disabled={onExport === null} onClick={() => onExport?.(version)}>
          Export a copy…
        </button>
      )}
    </div>
  );
}

function PairDetails({
  pair,
  numbers,
  onLinkUndone,
}: {
  pair: VersionPairView;
  numbers: LineNumbers;
  onLinkUndone: () => void;
}) {
  const state = pairStateText(
    pair.state,
    pair.before.sha256 === pair.after.sha256,
    pair.complete,
    previewKind(pair.after),
  );
  const minor = pair.state.kind === "analyzed" ? minorText(pair.state.minor_hunks) : null;
  const gap = pairGapText(pair);
  const changes = useShowMore(pair.changes, 3);
  return (
    <div className="small">
      <p className={state.tone === "warn" ? "warn" : state.tone === "wait" ? "muted" : undefined}>
        Since {versionRef(pair.before, numbers)}: {state.label}
        {state.detail && <span className="muted"> — {state.detail}</span>}
      </p>
      {pair.changes.length > 0 && <p className="muted">{MODEL_WORDS_NOTE}</p>}
      {pair.changes.length > 0 && (
        <ul className="plain">
          {changes.visible.map((change) => (
            <ChangeItem key={change.change_id} change={change} />
          ))}
        </ul>
      )}
      {changes.control}
      {minor && <p className="muted">{minor}</p>}
      {gap && <p className="muted">{gap}</p>}
      {pair.linked_automatically && pair.edge_id && <UndoLinkNote edgeId={pair.edge_id} onUndone={onLinkUndone} />}
    </div>
  );
}
