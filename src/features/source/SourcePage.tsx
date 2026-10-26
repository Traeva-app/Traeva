import { useCallback, useEffect, useId, useRef, useState } from "react";
import { sourceApi } from "./api";
import type {
  SourceApi,
  SourceConnection,
  SourceLinkKind,
  SourceRecordView,
  SourceRelation,
  SourceState,
} from "./types";
import {
  isWebLink,
  recordName,
  replaceRecord,
  sourceBadge,
  sourceError,
  sourceGroup,
  sourcesForFile,
  statusText,
  timeText,
} from "./view";
import "./source.css";
import { FileIconSvg } from "../find/icons";
import { fileIcon } from "../find/view";
import { Group, Row, Rows, StatusLine } from "../../shared/Group";
import { ShowMore } from "../../shared/ShowMore";

export function SourcePage({ api = sourceApi, pollIntervalMs = 2000, fileId, active = true }: {
  api?: SourceApi;
  pollIntervalMs?: number;
  fileId?: string;
  active?: boolean;
}) {
  const sectionId = useId();
  const embedded = fileId !== undefined;
  const [state, setState] = useState<SourceState | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [connection, setConnection] = useState<SourceConnection | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);
  const working = useRef(false);
  const reading = useRef(false);
  const generation = useRef(0);
  const alive = useRef(true);

  const refresh = useCallback(async () => {
    if (working.current || reading.current) return;
    reading.current = true;
    const started = generation.current;
    try {
      const next = await api.state();
      const details = !embedded && setupOpen ? await api.connection() : null;
      if (alive.current && started === generation.current) {
        setState(next);
        if (details) setConnection(details);
        setReadError(null);
      }
    } catch (error) {
      if (alive.current && started === generation.current) setReadError(sourceError(error));
    } finally {
      reading.current = false;
      if (alive.current) setLoading(false);
    }
  }, [api, embedded, setupOpen]);

  useEffect(() => {
    alive.current = true;
    const initial = setTimeout(() => { void refresh(); }, 0);
    return () => { clearTimeout(initial); alive.current = false; generation.current++; };
  }, [refresh]);

  useEffect(() => {
    if (!active) return;
    const activeRefresh = setTimeout(() => { void refresh(); }, 0);
    const timer = (state?.enabled || embedded) && pollIntervalMs > 0
      ? setInterval(() => { void refresh(); }, pollIntervalMs) : undefined;
    return () => { clearTimeout(activeRefresh); clearInterval(timer); };
  }, [state?.enabled, embedded, active, pollIntervalMs, refresh]);

  const run = async (action: () => Promise<void>): Promise<boolean> => {
    if (working.current) return false;
    working.current = true;
    generation.current++;
    setBusy(true);
    setActionError(null);
    try {
      await action();
      return true;
    } catch (error) {
      if (alive.current) setActionError(sourceError(error));
      return false;
    } finally {
      working.current = false;
      if (alive.current) setBusy(false);
    }
  };

  const changeRecord = (action: () => Promise<SourceRecordView | null>) => run(async () => {
    const saved = await action();
    if (saved && alive.current) setState((current) => replaceRecord(current, saved));
  });

  const records = fileId === undefined ? state?.records ?? [] : sourcesForFile(state?.records ?? [], fileId);
  const isConfirmed = (record: SourceRecordView) => fileId === undefined
    ? record.status === "user_confirmed" : record.association?.file_id === fileId;
  const pending = records.filter((record) => !isConfirmed(record));
  const confirmed = records.filter(isConfirmed);
  const selectedSource = records.find((record) => record.source_id === selectedSourceId);
  const Container = embedded ? "section" : "main";
  const Title = embedded ? "h2" : "h1";
  const GroupTitle = embedded ? "h3" : "h2";
  const renderRecord = (record: SourceRecordView) => (
    <SourceRecord
      key={record.source_id}
      record={record}
      contextFileId={fileId}
      expanded={!embedded}
      busy={busy}
      onRefresh={() => changeRecord(() => api.refresh(record.source_id))}
      onChoose={() => changeRecord(() => api.chooseFile(record.source_id))}
      onConfirm={(fileId, relation, note) => changeRecord(() => api.confirm(record.source_id, fileId, relation, note))}
      onReject={(fileId) => changeRecord(() => api.reject(record.source_id, fileId))}
      onUnlink={(note) => changeRecord(() => api.unlink(record.source_id, note))}
      onOpen={(kind) => run(() => api.openLink(record.source_id, kind))}
    />
  );
  // The Sources page's groups (#37, the student's choice B, 10/14): what
  // waits for the user's confirmation first and always open; downloads
  // Traeva still works on or looks for; then confirmed or interrupted ones.
  const needs = records.filter((record) => sourceGroup(record) === "needs");
  const progress = records.filter((record) => sourceGroup(record) === "progress");
  const finished = records.filter((record) => sourceGroup(record) === "finished");
  const isSelected = (record: SourceRecordView) => record.source_id === selectedSourceId;
  // The first Review is the page's one amber button.
  const sourceRow = (record: SourceRecordView, primary: boolean) => {
    const badge = sourceBadge(record);
    const name = recordName(record);
    return (
      <Row
        key={record.source_id}
        icon={<FileIconSvg icon={fileIcon(null, name)} />}
        name={name}
        lines={<div className="find-row-line">Chrome download {record.download_id} · imported {timeText(record.imported_at_ms, "at an unknown time")}</div>}
        status={<StatusLine badge={badge} />}
        attention={badge.tone === "warn"}
        actions={<button type="button" className={primary ? "primary" : undefined} disabled={busy}
          aria-pressed={isSelected(record)}
          aria-controls={`${sectionId}-detail`}
          onClick={() => setSelectedSourceId(record.source_id)}>{badge.tone === "warn" ? "Review" : "Details"}</button>}
      />
    );
  };
  // 10 at first; the selected source always stays in the list.
  const groupRows = (items: SourceRecordView[]) => (
    <ShowMore items={items} first={10} keep={isSelected}>
      {(visible) => <Rows>{visible.map((record) => sourceRow(record, false))}</Rows>}
    </ShowMore>
  );
  const confirmedCount = finished.filter((record) => record.status === "user_confirmed").length;
  const finishedSummary = [
    confirmedCount > 0 && `${confirmedCount} confirmed`,
    finished.length > confirmedCount && `${finished.length - confirmedCount} interrupted`,
  ].filter(Boolean).join(" · ");

  // In a flow step with no candidate and no confirmed source, the section
  // is one line; it opens to the full section. With a candidate it shows
  // in full, so it can be confirmed (UI, the A1 review, 10/14).
  const quiet = embedded && state !== null && records.length === 0 && readError === null && actionError === null;
  const body = (
    <>
      <header className="page-header">
        <Title>{embedded ? "Source" : "Source Memory"}</Title>
        <button type="button" disabled={busy} onClick={() => { void refresh(); }}>Refresh list</button>
      </header>
      <p className="muted">{embedded
        ? "Sources associated with this file, and download records that suggest it as a candidate."
        : "Keep Chrome download evidence with your files. Review suggested matches when you are ready."}</p>

      {loading && <p role="status">Reading saved sources…</p>}
      {readError && <div className="source-error" role="alert">
        <h2>Could not read sources</h2>
        <p>{readError}</p>
        {state && <p className="small">The records below are the last successfully read state.</p>}
      </div>}
      {actionError && <div className="source-error" role="alert">
        <h2>The request did not finish</h2>
        <p>{actionError}</p>
        <p className="small">No success has been recorded here. Refresh the list to check the saved state before retrying.</p>
      </div>}

      {state && <>
        {!embedded && <details className="card source-collection" open={state.records.length === 0 ? true : undefined}>
          <summary>
            <span className="source-record-name">Chrome collection in Traeva is {state.enabled ? "on" : "off"}</span>
            <span className="small muted">Connection and setup</span>
          </summary>
          <p>Only downloads in folders you have authorized in Traeva are saved. Incognito downloads are excluded. URLs stay in local app storage.</p>
          <p className="small muted">Authorize folders in Settings. Collection does not authorize access to another folder.</p>
          <details className="source-setup" open={!state.enabled && state.records.length === 0 ? true : undefined}>
            <summary>Set up Chrome Source Memory</summary>
            <ol>
              <li>In Settings, authorize the folder where Chrome saves your downloads.</li>
              <li>In Chrome, open <code>chrome://extensions</code>, enable Developer mode, and choose <strong>Load unpacked</strong>. Select the provided Traeva Chrome extension folder (<code>extensions/chrome</code> in the source package). Leave Incognito access off.</li>
              <li>Enable Chrome collection here and open <strong>Connect Chrome extension</strong>. Copy this app's address and pairing token into the extension's <strong>Options</strong>.</li>
              <li>In Options, select <strong>Collect new downloads</strong>, save settings, and check the connection. Verify that your download folder is listed.</li>
              <li>Download a small TXT file into that folder. Its source should appear under <strong>Needs you</strong>. Review the candidate before confirming its file association.</li>
            </ol>
            <p className="small muted">If the extension is already installed, open its Options from Chrome's extension menu. Each Chrome profile pairs separately. No browser debugger permission is needed.</p>
          </details>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => { void run(async () => {
              const saved = await api.setEnabled(!state.enabled);
              if (!alive.current) return;
              setState(saved);
              const details = saved.enabled ? await api.connection() : null;
              if (alive.current) { setConnection(details); setSetupOpen(saved.enabled); }
            }); }}>{state.enabled ? "Turn collection off" : "Enable Chrome collection"}</button>
            {state.enabled && <button type="button" disabled={busy} aria-expanded={setupOpen} onClick={() => {
              if (setupOpen) { setSetupOpen(false); setConnection(null); return; }
              void run(async () => {
                const details = await api.connection();
                if (alive.current) { setConnection(details); setSetupOpen(true); }
              });
            }}>{setupOpen ? "Hide connection details" : "Connect Chrome extension"}</button>}
          </div>
          {!state.enabled && state.records.length > 0 && <p className="small">Saved sources remain available while collection is off.</p>}
          {state.enabled && setupOpen && connection && <ConnectionDetails connection={connection} />}
          {state.enabled && setupOpen && connection && !connection.active && <button type="button" disabled={busy} onClick={() => { void run(async () => {
            const saved = await api.setEnabled(true);
            const details = await api.connection();
            if (alive.current) { setState(saved); setConnection(details); }
          }); }}>Retry local connection</button>}
          <p className="small">After an App or Chrome restart, saved sources stay here. The extension keeps its waiting list across Chrome restarts. Open Extension Options to check the connection, retry waiting downloads, or explicitly import missed downloads from the last 24 hours.</p>
          <p className="small muted">Turning collection off here stops Traeva's local receiver. Check Extension Options for waiting or skipped downloads. Downloads started while collection was off in Traeva require an explicit manual import after re-enabling it.</p>
          <p className="small muted">Turning collection off in Extension Options, or changing that Chrome profile's pairing, clears its waiting queue. Review the warning before confirming a change that clears waiting deliveries. Sources already saved in Traeva remain available. If a download is missing, also check the pairing information and its folder's availability in Settings.</p>
        </details>}

        <div className={embedded ? undefined : "source-workspace"}>
          {embedded ? <div>
            <section className="source-queue" aria-labelledby={`${sectionId}-pending`}>
              <GroupTitle id={`${sectionId}-pending`}>Pending confirmation ({pending.length})</GroupTitle>
              <p className="small muted">A candidate is a suggestion, not a confirmed source for this file.</p>
              {pending.length === 0 ? <p>No source candidates are recorded for this file.</p> : pending.map(renderRecord)}
            </section>

            <section className="source-queue" aria-labelledby={`${sectionId}-confirmed`}>
              <GroupTitle id={`${sectionId}-confirmed`}>Confirmed sources ({confirmed.length})</GroupTitle>
              {confirmed.length === 0 ? <p className="muted">Source unknown: no association is recorded for this file. Review unlinked downloads in Source Memory.</p> : confirmed.map(renderRecord)}
            </section>
          </div> : <div className="source-list-pane">
            {records.length === 0 && <p>No downloads collected yet. Enable collection, connect the Chrome extension, and download into an authorized folder.</p>}
            {needs.length > 0 && <Group id={`${sectionId}-needs`} title="Needs you" count={needs.length} attention
              note="A candidate is a suggestion: it has no confirmed file association until you confirm it.">
              <Rows>{needs.map((record, index) => sourceRow(record, index === 0))}</Rows>
            </Group>}
            {progress.length > 0 && <Group id={`${sectionId}-progress`} title="In progress" count={progress.length}
              summary={progress.length === 1 ? "1 download" : `${progress.length} downloads`}
              note="New downloads appear here without a popup. Traeva keeps looking for each file; you can also locate it yourself.">
              {groupRows(progress)}
            </Group>}
            {finished.length > 0 && <Group id={`${sectionId}-finished`} title="Finished" count={finished.length}
              defaultOpen={false} forceOpen={finished.some(isSelected)} summary={finishedSummary}>
              {groupRows(finished)}
            </Group>}
          </div>}
          {!embedded && <section className="source-detail-pane" id={`${sectionId}-detail`} aria-label="Selected source details" aria-busy={busy}>
            {selectedSource ? renderRecord(selectedSource) : <div className="source-empty-detail">
              <h2>{selectedSourceId ? "This source is no longer in the list" : "Select a source"}</h2>
              <p className="muted">{records.length > 0
                ? "Choose a download in the list to see its evidence and review its file association."
                : "Collected downloads and their file candidates will appear here."}</p>
              <p className="small muted">Viewing a source does not confirm a match.</p>
            </div>}
          </section>}
        </div>
        <p className="small muted">Automatic Verified association is not available in this version. User Confirmed associations keep that basis after a rename or move.</p>
      </>}
    </>
  );

  return (
    <Container className={embedded ? "card source-page source-file-section" : "page source-page"} aria-label={embedded ? "Source for this file" : undefined}>
      {quiet ? <details className="source-quiet"><summary>Source: unknown</summary>{body}</details> : body}
    </Container>
  );
}

function ConnectionDetails({ connection }: { connection: SourceConnection }) {
  const [showToken, setShowToken] = useState(false);
  if (!connection.enabled || !connection.active || !connection.endpoint || !connection.pairing_token) {
    return <div className="source-error" role="status"><p>{connection.enabled ? "The local receiver could not start. Chrome cannot send sources yet." : "Chrome collection is off in Traeva."}</p>
      <p>{connection.error ?? "Check that collection is enabled, then try again."}</p></div>;
  }
  return <div className="source-connection">
    <p role="status">Traeva's local receiver is ready. Check the connection in Chrome Extension Options to verify pairing; this app cannot tell whether the extension is installed or connected.</p>
    <p>In the Traeva Chrome extension options, enter this address and pairing token. Keep the token private.</p>
    <label>Local connection address<input type="text" readOnly value={connection.endpoint} onFocus={(event) => event.currentTarget.select()} /></label>
    <label>Pairing token<input type={showToken ? "text" : "password"} readOnly value={connection.pairing_token} autoComplete="off" onFocus={(event) => event.currentTarget.select()} /></label>
    <button type="button" onClick={() => setShowToken(!showToken)}>{showToken ? "Hide token" : "Show token"}</button>
  </div>;
}

interface RecordProps {
  record: SourceRecordView;
  contextFileId?: string;
  expanded?: boolean;
  busy: boolean;
  onRefresh(): Promise<boolean>;
  onChoose(): Promise<boolean>;
  onConfirm(fileId: string, relation: SourceRelation, note: string): Promise<boolean>;
  onReject(fileId: string): Promise<boolean>;
  onUnlink(note: string): Promise<boolean>;
  onOpen(kind: SourceLinkKind): Promise<boolean>;
}

export function SourceRecord({ record, contextFileId, expanded = false, busy, onRefresh, onChoose, onConfirm, onReject, onUnlink, onOpen }: RecordProps) {
  const [selected, setSelected] = useState<string | null>(null);
  const [unlinking, setUnlinking] = useState(false);
  const [relation, setRelation] = useState<SourceRelation>("download");
  const [note, setNote] = useState("");
  const candidate = record.candidates.find((item) => item.file_id === selected);
  const association = record.association;
  const associatedElsewhere = contextFileId !== undefined && association !== null && association.file_id !== contextFileId;
  const candidates = contextFileId === undefined ? record.candidates : record.candidates.filter((item) => item.file_id === contextFileId);

  const cancel = () => { setSelected(null); setUnlinking(false); setNote(""); };
  const RecordContainer = expanded ? "article" : "details";
  const RecordHeader = expanded ? "header" : "summary";
  return <RecordContainer className={expanded ? "source-record source-record-expanded" : "source-record"}>
    <RecordHeader>
      {expanded ? <h2>{recordName(record)}</h2> : <span className="source-record-name">{recordName(record)}</span>}
      <span className="source-record-status">{associatedElsewhere ? "Candidate for this file; confirmed for another file" : statusText(record.status)}</span>
    </RecordHeader>
    <div className="source-record-body">
      {record.reason && <p>{record.reason}</p>}
      {association && <>
        {associatedElsewhere && <p className="warn">The existing confirmation below belongs to another file. Confirming this candidate will correct that association.</p>}
        <h3>{association.relation === "copy" ? "User Confirmed copy relationship" : "User Confirmed association"}</h3>
        <p className="mono">{association.display_path}</p>
        <p>{association.relation === "copy"
          ? "You confirmed this file is a copy related to the download. It is not recorded as the original downloaded object."
          : "You confirmed the file associated with this download. This is not system verification."}</p>
        <p className="small">{association.current_location_confirmed
          ? "The file location was confirmed at the last successful check."
          : "The current file location is not confirmed. The historical source record is retained."}</p>
        <p className="small">Confirmed: {timeText(association.confirmed_at_ms, "Confirmation time unknown")}</p>
        {association.note && <p className="small">Your note: {association.note}</p>}
      </>}
      <dl className="source-fields">
        <dt>Initial download URL</dt>
        <dd><SourceLink value={record.initial_url} kind="initial" missing="Download address unknown: Chrome supplied no initial URL." busy={busy} onOpen={onOpen} /></dd>
        <dt>Final download URL</dt>
        <dd><SourceLink value={record.final_url} kind="final" missing="Final download address unknown: Chrome supplied no final URL." busy={busy} onOpen={onOpen} /></dd>
        <dt>Browser referrer</dt>
        <dd><SourceLink value={record.referrer} kind="referrer" missing="Referrer unknown: Chrome supplied no referrer." busy={busy} onOpen={onOpen} />
          <span className="small muted">A browser referrer is not proof of the original source page.</span></dd>
        <dt>Source page</dt><dd>Unknown: this channel has no independent page evidence.</dd>
        <dt>Title</dt><dd>{record.title ?? "Title unknown: Chrome download records do not provide a page title."}</dd>
        <dt>Started download</dt><dd>{timeText(record.started_at_ms, "Start time unknown: not provided by Chrome.")}</dd>
        <dt>Download completed</dt><dd>{timeText(record.completed_at_ms, "Completion time unknown: no successful completion time was recorded.")}</dd>
        <dt>Imported into Traeva</dt><dd>{timeText(record.imported_at_ms, "Import time unknown")}</dd>
        <dt>Download state</dt><dd>{{ complete: "Completed", interrupted: "Interrupted", in_progress: "In progress" }[record.download_state]}</dd>
        <dt>Evidence channel</dt><dd>Chrome download record</dd>
        <dt>Download record</dt><dd className="mono">{record.download_id}</dd>
        <dt>Browser environment</dt><dd className="mono">{record.profile_id}</dd>
        <dt>Recorded download path</dt><dd className="mono">{record.filename || "Path unknown"}</dd>
      </dl>

      <div className="row">
        <button type="button" disabled={busy} onClick={() => { void onRefresh(); }}>Check file again</button>
        <button type="button" disabled={busy} onClick={() => { void onChoose(); }}>{association ? "Choose another file" : "Locate file"}</button>
        {association && <button type="button" disabled={busy} onClick={() => { cancel(); setUnlinking(true); }}>Remove association</button>}
      </div>
      <p className="small muted">Checks use authorized folders. Choosing or confirming a file does not rename, move, or edit it.</p>

      {candidates.length > 0 && <section aria-label="Suggested files">
        <h3>{association ? "Other candidates" : "Suggested files"}</h3>
        {candidates.map((item) => <div className="source-candidate" key={item.file_id}>
          <p className="mono">{item.display_path}</p>
          <p>{item.reason}</p>
          <div className="row">
            <button type="button" disabled={busy || !item.can_confirm} onClick={() => {
              setSelected(item.file_id); setUnlinking(false); setRelation("download"); setNote("");
            }}>Review this match</button>
            <button type="button" disabled={busy} onClick={() => { void onReject(item.file_id).then((ok) => {
              if (ok && selected === item.file_id) cancel();
            }); }}>Reject candidate</button>
          </div>
          {!item.can_confirm && <p className="small muted">This candidate cannot be confirmed from the current observation.</p>}
        </div>)}
      </section>}
      {!association && candidates.length === 0 && <p>No reliable candidate is available. Locate a file to review it; Traeva will not guess.</p>}

      {selected && <form className="source-confirm" onSubmit={(event) => {
        event.preventDefault();
        if (!candidate?.can_confirm || busy || (association && !note.trim())) return;
        void onConfirm(selected, relation, note.trim()).then((ok) => { if (ok) cancel(); });
      }}>
        <h3>{association ? "Correct this association" : "Confirm the relationship"}</h3>
        <p className="mono">{candidate?.display_path ?? "The selected candidate is no longer available."}</p>
        <label>How is this file related?
          <select value={relation} disabled={busy} onChange={(event) => setRelation(event.target.value as SourceRelation)}>
            <option value="download">I confirm this is the downloaded file</option>
            <option value="copy">I confirm this is a copy of the downloaded file</option>
          </select>
        </label>
        <label>{association ? "Reason for correction (required)" : "Your note (optional)"}
          <textarea value={note} maxLength={1000} required={Boolean(association)} disabled={busy} onChange={(event) => setNote(event.target.value)} rows={2} />
        </label>
        <p className="small">Your confirmation will be saved as User Confirmed. It will not establish system verification.</p>
        <div className="row">
          <button type="submit" disabled={busy || !candidate?.can_confirm || Boolean(association && !note.trim())}>Confirm relationship</button>
          <button type="button" disabled={busy} onClick={cancel}>Cancel</button>
        </div>
      </form>}
      {unlinking && <form className="source-confirm" onSubmit={(event) => {
        event.preventDefault();
        if (!note.trim() || busy) return;
        void onUnlink(note.trim()).then((ok) => { if (ok) cancel(); });
      }}>
        <h3>Remove this association</h3>
        <p>The download evidence stays saved. The file itself is unchanged.</p>
        <label>Reason (required)<textarea value={note} maxLength={1000} required disabled={busy} onChange={(event) => setNote(event.target.value)} rows={2} /></label>
        <div className="row">
          <button type="submit" disabled={busy || !note.trim()}>Remove association</button>
          <button type="button" disabled={busy} onClick={cancel}>Cancel</button>
        </div>
      </form>}
    </div>
  </RecordContainer>;
}

function SourceLink({ value, kind, missing, busy, onOpen }: {
  value: string | null;
  kind: SourceLinkKind;
  missing: string;
  busy: boolean;
  onOpen(kind: SourceLinkKind): Promise<boolean>;
}) {
  if (!value) return <>{missing}</>;
  return <>
    <span className="source-url">{value}</span>
    {isWebLink(value) ? <>
      <button type="button" className="source-link-action" disabled={busy} onClick={() => { void onOpen(kind); }}>
        {kind === "referrer" ? "Open browser referrer" : "Open download link"}
      </button>
      {kind !== "referrer" && <span className="small muted">Opening this link may download the file again.</span>}
    </> : <span className="small muted">This address cannot be opened here. Only HTTP and HTTPS links are supported.</span>}
  </>;
}
