// The folder picker for choosing a file's folder by hand (#37, the
// student's request 10/14): first a few likely folders at the top of the
// target folder, then one level at a time inside the one opened. Only the
// listing the core returned (list_folders) and the places the model named
// are used; the choice is sent as before, with the listing's folder ID, and
// the core checks it again. Nothing here decides a move or makes up a folder.

import { recommendedCandidate } from "./flow.ts";
import type { FolderCatalogView, FolderView, SuggestionView } from "./types.ts";

/** At most this many likely folders are offered first. */
export const LIKELY_MAX = 3;

/** The folder the listing is of (the authorized folder itself), or null. */
export function rootFolder(catalog: FolderCatalogView): FolderView | null {
  return catalog.folders.find((folder) => folder.parent_id === null && folder.relative_path === "") ?? null;
}

/** A folder's subfolders, in the listing's order. */
export function childrenOf(catalog: FolderCatalogView, folderId: string): FolderView[] {
  return catalog.folders.filter((folder) => folder.parent_id === folderId);
}

/** The folders at the top of the authorized folder. */
export function topLevel(catalog: FolderCatalogView): FolderView[] {
  const root = rootFolder(catalog);
  return root === null ? [] : childrenOf(catalog, root.folder_id);
}

/** The places the model named for the file, in the order they count: the
 * recommended one first, then the others in the model's order. Paths are
 * inside the target folder ("" is the folder itself). */
export function folderHints(suggestion: SuggestionView | null): string[] {
  if (suggestion === null) return [];
  const recommended = recommendedCandidate(suggestion);
  const ordered =
    recommended === null
      ? suggestion.candidates
      : [recommended, ...suggestion.candidates.filter((candidate) => candidate !== recommended)];
  return ordered.map((candidate) => candidate.folder_path);
}

/** Up to LIKELY_MAX top folders the places point to: each place's top
 * folder, in the places' order, once each. A place at the folder itself, a
 * top folder the listing does not have, and a protected one are left out.
 * None without places. */
export function likelyParents(catalog: FolderCatalogView, hints: string[]): FolderView[] {
  const byName = new Map(topLevel(catalog).map((folder) => [folder.relative_path, folder]));
  const likely: FolderView[] = [];
  for (const hint of hints) {
    const top = hint.split("/").find((part) => part !== "");
    const folder = top === undefined ? undefined : byName.get(top);
    if (folder === undefined || folder.protected !== null || likely.includes(folder)) continue;
    likely.push(folder);
    if (likely.length === LIKELY_MAX) break;
  }
  return likely;
}

/** The top folders that are not among the likely ones, in the listing's order. */
export function otherParents(catalog: FolderCatalogView, likely: FolderView[]): FolderView[] {
  return topLevel(catalog).filter((folder) => !likely.includes(folder));
}

/** The folders from the top down to this one, for the path shown above a
 * level: the authorized folder itself is not included. */
export function trail(catalog: FolderCatalogView, folderId: string): FolderView[] {
  const byId = new Map(catalog.folders.map((folder) => [folder.folder_id, folder]));
  const path: FolderView[] = [];
  let current = byId.get(folderId);
  while (current !== undefined && current.parent_id !== null && !path.includes(current)) {
    path.unshift(current);
    current = byId.get(current.parent_id);
  }
  return path;
}

/** A folder's own name: the last part of its path; for the authorized
 * folder itself, its name with "(the folder itself)". */
export function folderName(folder: FolderView, rootName: string): string {
  if (folder.relative_path === "") return `${rootName} (the folder itself)`.trim();
  return folder.relative_path.split("/").pop() ?? folder.relative_path;
}

/** Whether the chosen folder is inside this one (deeper down, not this
 * one itself): the row says so once the user has gone back up. Never for
 * the authorized folder itself, which holds every folder. */
export function holdsChosen(catalog: FolderCatalogView, folderId: string, chosenId: string): boolean {
  if (chosenId === "" || chosenId === folderId) return false;
  return trail(catalog, chosenId).some((folder) => folder.folder_id === folderId);
}

/** Whether a path inside the authorized folder lies inside this folder
 * (deeper down, not the folder itself): for "the file is inside". Never
 * for the authorized folder itself (""), where every file is. */
export function pathInside(path: string | null, folderPath: string): boolean {
  if (path === null || folderPath === "" || path === folderPath) return false;
  return path.startsWith(`${folderPath}/`);
}

/** Whether a folder opens to show its subfolders, rather than being chosen
 * at once: it has some, and it is not protected (then it cannot be used at
 * all, and neither can anything inside it). */
export function opensInto(catalog: FolderCatalogView, folder: FolderView): boolean {
  return folder.protected === null && childrenOf(catalog, folder.folder_id).length > 0;
}
