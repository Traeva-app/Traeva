// The automatic mode's settings (#168, organize-spec.md §13.2): on or off,
// and the target range inside one authorized folder. Before the mode is
// turned on (or its folders change) the core's notice is shown in full, and
// the mode starts only when the user agrees to it. Whether the agreement
// still counts, and why not, comes from the core.

import { useCallback, useEffect, useRef, useState } from "react";
import { listFolders } from "../organize/api";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type FolderCatalogView, type RootView } from "../organize/types";
import { getAutoDropSettings, setAutoDropSettings } from "./api";
import { MAX_SCOPES, modeStatus, scopeChoiceProblem, scopeLabel, validRootId } from "./queue";
import type { AutoDropSettingsView } from "./types";
import "./autodrop.css";
import { ShowMore } from "../../shared/ShowMore";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

const STATUS_BADGE = {
  on: { text: "On", tone: "card-ok" },
  not_working: { text: "Not working", tone: "card-error" },
  off: { text: "Off", tone: "" },
} as const;

/** `roots`: the authorized folders that are active now. `pageKey` changes
 * whenever the page is read again: the setting is read again with it. */
export function AutoDropCard({
  roots,
  modelReady,
  pageKey,
}: {
  roots: RootView[];
  modelReady: boolean;
  pageKey: number;
}) {
  const [settings, setSettings] = useState<AutoDropSettingsView | null>(null);
  const [readError, setReadError] = useState<AppError | null | undefined>(undefined);
  const [error, setError] = useState<AppError | null>(null);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);

  // Every read, and every saved change, takes the next number; a read's
  // answer is shown only while it is the latest. A read sent before Turn off
  // (slow while on, as folders are listed) never brings back On.
  const latest = useRef(0);
  const read = useCallback(() => {
    const mine = ++latest.current;
    getAutoDropSettings().then(
      (read) => {
        if (mine !== latest.current) return;
        setSettings(read);
        setReadError(undefined);
      },
      (reason: unknown) => {
        if (mine === latest.current) setReadError(asError(reason));
      },
    );
  }, []);
  const saved = (next: AutoDropSettingsView) => {
    latest.current += 1;
    setSettings(next);
    setEditing(false);
  };
  // A folder added, removed or changed on disk can make the range count or
  // stop counting: read again with the page and when the window comes back.
  useEffect(() => {
    read();
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [read, pageKey]);

  const turnOff = () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setAutoDropSettings({ enable: false })
      .then(saved)
      .catch((reason: unknown) => setError(asError(reason)))
      .finally(() => setBusy(false));
  };

  const title = <h2 className="grow">Organize dropped files automatically</h2>;
  if (settings === null) {
    return (
      <section className="card">
        {title}
        {readError === undefined ? (
          <p className="muted">Reading the setting…</p>
        ) : (
          <>
            <p className="warn">The setting could not be read.</p>
            {readError && <ErrorDetails error={readError} />}
            <button type="button" onClick={read}>
              Read it again
            </button>
          </>
        )}
      </section>
    );
  }

  const status = modeStatus(settings);
  const badge = STATUS_BADGE[status];
  return (
    <section className="card">
      <div className="row">
        {title}
        <span className={`badge ${badge.tone}`}>{badge.text}</span>
      </div>
      {editing ? (
        <RangeEditor
          settings={settings}
          roots={roots}
          turningOn={status === "off"}
          onCancel={() => setEditing(false)}
          onSaved={saved}
        />
      ) : (
        <>
          {status === "off" ? (
            <p>
              Files you drop on Traeva&apos;s icon in the Dock go through the usual steps: a preview,
              then your confirmation.
            </p>
          ) : (
            <>
              {status === "on" ? (
                <p>
                  On. Files you drop on Traeva&apos;s icon in the Dock are moved without a preview when
                  Traeva finds one clear, existing folder for them in:
                </p>
              ) : (
                <p>
                  Not working now, so dropped files go through the usual steps. The folders you agreed
                  to:
                </p>
              )}
              <ShowMore items={settings.scopes} first={5} keep={(scope) => !scope.valid}>
                {(visible) => (
                  <ul className="list small">
                    {visible.map((scope) => (
                      <li key={scope.relative_path} className="row">
                        <span className="mono grow">{scope.display_path}</span>
                        {!scope.valid && <span className="warn small">No longer available</span>}
                      </li>
                    ))}
                  </ul>
                )}
              </ShowMore>
              {settings.agreed_at_ms !== null && (
                <p className="muted small">
                  You agreed on {new Date(settings.agreed_at_ms).toLocaleString()}. Each move is
                  listed on the Activity page, where Undo works as for any other move.
                </p>
              )}
            </>
          )}
          {settings.problem && <p className="warn">{settings.problem}</p>}
          {!modelReady && (
            <p className="muted small">
              A dropped file is sent to the model only with Organize&apos;s permission, given in the Model card.
              Without it, nothing is sent and nothing is moved automatically.
            </p>
          )}
          <div className="row">
            {status === "off" ? (
              <button type="button" onClick={() => setEditing(true)} disabled={busy || roots.length === 0}>
                Turn on…
              </button>
            ) : (
              <>
                <button type="button" onClick={() => setEditing(true)} disabled={busy || roots.length === 0}>
                  Change the folders…
                </button>
                <button type="button" onClick={turnOff} disabled={busy}>
                  Turn off
                </button>
              </>
            )}
          </div>
          {roots.length === 0 && <p className="muted small">Add a folder first.</p>}
        </>
      )}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

// ---------------------------------------------------------------------------
// The target range and the agreement
// ---------------------------------------------------------------------------

function RangeEditor(props: {
  settings: AutoDropSettingsView;
  roots: RootView[];
  turningOn: boolean;
  onCancel: () => void;
  onSaved: (settings: AutoDropSettingsView) => void;
}) {
  const { settings, roots } = props;
  const [chosenRoot, setRootId] = useState(settings.target_root_id ?? "");
  // A folder removed meanwhile falls back to the first one still offered.
  const rootId = validRootId(chosenRoot, roots.map((root) => root.root_id));
  // The range as it is now, when it stays in the same folder.
  const [chosen, setChosen] = useState<string[]>(() =>
    rootId === settings.target_root_id
      ? settings.scopes.filter((scope) => scope.valid).map((scope) => scope.relative_path)
      : [],
  );
  const [catalog, setCatalog] = useState<FolderCatalogView | null>(null);
  const [listError, setListError] = useState<AppError | null | undefined>(undefined);
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    if (rootId === "") return;
    let alive = true;
    setCatalog(null);
    setListError(undefined);
    listFolders(rootId).then(
      (listed) => {
        if (alive) setCatalog(listed);
      },
      (reason: unknown) => {
        if (alive) setListError(asError(reason));
      },
    );
    return () => {
      alive = false;
    };
  }, [rootId, attempt]);

  const root = roots.find((candidate) => candidate.root_id === rootId) ?? null;
  // Only folders in this listing can be sent; the core checks them again.
  const listed = new Set(catalog?.folders.map((folder) => folder.relative_path) ?? []);
  const sendable = chosen.filter((path) => listed.has(path));
  const problem = scopeChoiceProblem(sendable.length);
  const toggle = (path: string) =>
    setChosen((current) =>
      current.includes(path) ? current.filter((one) => one !== path) : [...current, path],
    );

  const agree = () => {
    if (busy || problem !== null || catalog === null) return;
    setBusy(true);
    setError(null);
    setAutoDropSettings({
      enable: true,
      notice_version: settings.notice_version,
      target_root_id: rootId,
      scopes: sendable,
    })
      .then(props.onSaved)
      .catch((reason: unknown) => setError(asError(reason)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="autodrop-editor">
      <label className="field">
        Folder to organize into
        <select
          value={rootId}
          onChange={(event) => {
            setRootId(event.target.value);
            setChosen([]);
          }}
        >
          {roots.map((one) => (
            <option key={one.root_id} value={one.root_id}>
              {one.display_path}
            </option>
          ))}
        </select>
      </label>
      <fieldset className="field">
        <legend>Folders it may put files in (existing folders, with their subfolders)</legend>
        {catalog === null ? (
          listError === undefined ? (
            <p className="muted">Listing folders…</p>
          ) : (
            <>
              <p>The folders could not be listed.</p>
              {listError && <ErrorDetails error={listError} />}
              <button type="button" onClick={() => setAttempt(attempt + 1)}>
                List the folders again
              </button>
            </>
          )
        ) : (
          <ul className="plain autodrop-folders">
            {catalog.folders.map((folder) => (
              <li key={folder.folder_id} style={{ paddingLeft: folder.depth * 16 }}>
                <label className="row">
                  <input
                    type="checkbox"
                    checked={chosen.includes(folder.relative_path)}
                    disabled={folder.protected !== null}
                    onChange={() => toggle(folder.relative_path)}
                  />
                  <span className="mono exact">
                    {scopeLabel(folder.relative_path, root?.display_path ?? "")}
                  </span>
                  {folder.protected !== null && <span className="muted small">— {folder.protected}</span>}
                </label>
              </li>
            ))}
          </ul>
        )}
        {catalog?.truncated && (
          <p className="muted small">Only the first 500 folders, 6 levels deep, are listed.</p>
        )}
        <p className={sendable.length > MAX_SCOPES ? "warn small" : "muted small"}>
          {sendable.length} chosen, at most {MAX_SCOPES}.
        </p>
      </fieldset>
      <h3>Before you turn it on</h3>
      <div className="autodrop-notice exact">{settings.notice}</div>
      <div className="row">
        <button type="button" onClick={agree} disabled={busy || problem !== null || catalog === null}>
          {props.turningOn ? "Agree and turn on" : "Agree and save these folders"}
        </button>
        <button type="button" onClick={props.onCancel} disabled={busy}>
          Cancel
        </button>
      </div>
      {problem !== null && sendable.length > 0 && <p className="warn small">{problem}</p>}
      {error && <ErrorDetails error={error} />}
    </div>
  );
}
