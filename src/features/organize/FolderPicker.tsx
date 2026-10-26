// Choosing a file's folder by hand (#37, the student's request 10/14): a few
// likely folders first, then one level at a time. The rules for what shows
// are in folderTree.ts; this draws them. A folder with subfolders opens
// (its subfolders and "Put it in …"); one without is chosen at once. The
// path above a level, Back and Esc go up. Choosing only marks the folder:
// the card's Preview this folder sends it, as before.

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  childrenOf,
  folderName,
  holdsChosen,
  likelyParents,
  opensInto,
  otherParents,
  pathInside,
  rootFolder,
  topLevel,
  trail,
} from "./folderTree";
import { ShowMore } from "../../shared/ShowMore";
import type { FolderCatalogView, FolderView } from "./types";

type Props = {
  catalog: FolderCatalogView;
  /** The authorized folder's name ("" when unknown). */
  rootName: string;
  /** The places the model named, in the order they count (folderHints). */
  hints: string[];
  /** The folder the file is in now, inside this authorized folder, or null. */
  here: string | null;
  /** The chosen folder's ID, "" for none. */
  chosenId: string;
  onChoose: (folderId: string) => void;
  /** The pet's popup: the path above a level keeps its last two folders. */
  compact?: boolean;
};

export function FolderPicker({ catalog, rootName, hints, here, chosenId, onChoose, compact }: Props) {
  // The folder whose subfolders show; null: the top of the authorized folder.
  const [at, setAt] = useState<string | null>(null);
  const [showOther, setShowOther] = useState(false);
  // After going in or up, the new level's title takes the focus.
  const heading = useRef<HTMLHeadingElement>(null);
  const moved = useRef(false);
  useEffect(() => {
    if (!moved.current) return;
    moved.current = false;
    heading.current?.focus();
  }, [at]);
  const go = (folderId: string | null) => {
    moved.current = true;
    setAt(folderId);
  };

  const root = rootFolder(catalog);
  const current = at === null ? null : (catalog.folders.find((folder) => folder.folder_id === at) ?? null);
  const path = current === null ? [] : trail(catalog, current.folder_id);
  const up = () => go(path.length > 1 ? path[path.length - 2].folder_id : null);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || event.nativeEvent.isComposing || current === null) return;
    event.preventDefault();
    event.stopPropagation();
    up();
  };

  const row = (folder: FolderView) => (
    <FolderRow
      key={folder.folder_id}
      folder={folder}
      name={folderName(folder, rootName)}
      // The authorized folder itself is chosen, never opened: its folders are this list.
      opens={folder.parent_id !== null && opensInto(catalog, folder)}
      chosen={folder.folder_id === chosenId}
      chosenInside={holdsChosen(catalog, folder.folder_id, chosenId)}
      isHere={folder.relative_path === here}
      fileInside={pathInside(here, folder.relative_path)}
      onOpen={() => go(folder.folder_id)}
      onChoose={() => onChoose(folder.folder_id)}
    />
  );
  const rootRow = root === null ? null : row(root);
  // Long lists show 12 folders at first; the rows that say where the choice
  // or the file is always show.
  const marked = (folder: FolderView) =>
    folder.folder_id === chosenId ||
    holdsChosen(catalog, folder.folder_id, chosenId) ||
    folder.relative_path === here ||
    pathInside(here, folder.relative_path);

  if (current !== null) {
    const name = folderName(current, rootName);
    // In the popup a deep path shows "…" for the folders above the last two;
    // Back and the authorized folder's button still go up.
    const shown = compact && path.length > 2 ? path.slice(-2) : path;
    return (
      <div className="folder-picker" onKeyDown={onKeyDown}>
        <nav className="folder-trail" aria-label="Folder path">
          <button type="button" onClick={() => go(null)}>
            {rootName || "Top"}
          </button>
          {shown.length < path.length && (
            <span className="folder-trail-step" aria-hidden="true">
              <span>›</span>
              <span className="muted">…</span>
            </span>
          )}
          {shown.map((folder) => (
            <span key={folder.folder_id} className="folder-trail-step">
              <span aria-hidden="true">›</span>
              {folder.folder_id === current.folder_id ? (
                <span aria-current="location">{folderName(folder, rootName)}</span>
              ) : (
                <button type="button" onClick={() => go(folder.folder_id)}>
                  {folderName(folder, rootName)}
                </button>
              )}
            </span>
          ))}
        </nav>
        <div className="row">
          <h3 className="folder-picker-title grow focus-target" ref={heading} tabIndex={-1}>
            {name}
          </h3>
          <button type="button" onClick={up}>
            Back
          </button>
        </div>
        <ShowMore key={current.folder_id} items={childrenOf(catalog, current.folder_id)} first={12} keep={marked}>
          {(visible) => (
            <ul className="folder-options" role="list">
              <li>
                <button
                  type="button"
                  className={current.folder_id === chosenId ? "folder-option folder-option-chosen" : "folder-option"}
                  aria-pressed={current.folder_id === chosenId}
                  onClick={() => onChoose(current.folder_id)}
                >
                  <span className="folder-option-name">Put it in {name}</span>
                  {current.relative_path === here && <span className="muted small">where the file is now</span>}
                  <span className="folder-option-mark" aria-hidden="true">
                    {current.folder_id === chosenId ? "✓" : ""}
                  </span>
                </button>
              </li>
              {visible.map(row)}
            </ul>
          )}
        </ShowMore>
      </div>
    );
  }

  const likely = likelyParents(catalog, hints);
  const others = otherParents(catalog, likely);
  // The choice is behind Other…: the button says so while the list is closed.
  const chosenInOther =
    chosenId !== "" &&
    (chosenId === root?.folder_id ||
      others.some((folder) => folder.folder_id === chosenId || holdsChosen(catalog, folder.folder_id, chosenId)));
  return (
    <div className="folder-picker" onKeyDown={onKeyDown}>
      <h3 className="folder-picker-title focus-target" ref={heading} tabIndex={-1}>
        Choose a folder
      </h3>
      {likely.length > 0 ? (
        <>
          <p className="muted small">Likely, from the places the model named:</p>
          <ul className="folder-options" role="list">
            {likely.map(row)}
          </ul>
          <button type="button" aria-expanded={showOther} onClick={() => setShowOther(!showOther)}>
            {showOther ? "Hide the other folders" : "Other…"}
            {!showOther && chosenInOther && <span className="muted small"> (your choice is here)</span>}
          </button>
          {showOther && (
            <ShowMore items={others} first={12} keep={marked}>
              {(visible) => (
                <ul className="folder-options folder-options-other" role="list">
                  {visible.map(row)}
                  {rootRow}
                </ul>
              )}
            </ShowMore>
          )}
        </>
      ) : (
        <ShowMore items={topLevel(catalog)} first={12} keep={marked}>
          {(visible) => (
            <ul className="folder-options" role="list">
              {visible.map(row)}
              {rootRow}
            </ul>
          )}
        </ShowMore>
      )}
    </div>
  );
}

/** One folder: opens when it has subfolders, else is chosen. A protected
 * one cannot be used but can still be reached with Tab, so its reason is
 * read out. Once the user has gone back up, a folder says when the choice
 * or the file is inside it. */
function FolderRow(props: {
  folder: FolderView;
  name: string;
  opens: boolean;
  chosen: boolean;
  chosenInside: boolean;
  isHere: boolean;
  fileInside: boolean;
  onOpen: () => void;
  onChoose: () => void;
}) {
  const { folder, opens, chosen } = props;
  const blocked = folder.protected !== null;
  return (
    <li>
      <button
        type="button"
        className={chosen ? "folder-option folder-option-chosen" : "folder-option"}
        aria-disabled={blocked ? true : undefined}
        aria-pressed={opens || blocked ? undefined : chosen}
        onClick={blocked ? undefined : opens ? props.onOpen : props.onChoose}
      >
        <span className="folder-option-name">
          {props.name}
          {opens && <span className="sr-only">, shows its folders</span>}
          {opens && chosen && <span className="sr-only">, chosen</span>}
        </span>
        {props.chosenInside && <span className="muted small">your choice is inside</span>}
        {props.isHere && <span className="muted small">where the file is now</span>}
        {props.fileInside && <span className="muted small">the file is inside</span>}
        {blocked && <span className="muted small">{folder.protected}</span>}
        <span className="folder-option-mark" aria-hidden="true">
          {chosen ? "✓" : ""}
          {opens ? " ›" : ""}
        </span>
      </button>
    </li>
  );
}
