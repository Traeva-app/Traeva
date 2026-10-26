// The pet's popup shows Organize's cards in a smaller form (#37, the
// student's request 10/14): paths cut to their last folders, with the full
// path in the tooltip. The popup's Move card is shortened too (organize-spec
// §6, v0.13): its full route (From, the full To, new folders) is in its
// Details. The main window's preview keeps every path in full.

/** How many folders a short path keeps. */
export const TAIL_LEVELS = 3;

/** A folder's place for a small window: its last folders, starting at the
 * target folder's name, with "…/" when more come before. `rootPath`: the
 * target folder's full path (null: not known); `relativeFolder`: the folder
 * inside it ("" for the target folder itself). */
export function folderTail(rootPath: string | null, relativeFolder: string, levels = TAIL_LEVELS): string {
  const rootName = rootPath?.split("/").filter((part) => part !== "").pop() ?? "";
  const parts = [rootName, ...relativeFolder.split("/")].filter((part) => part !== "");
  if (parts.length <= levels) return parts.join("/");
  return `…/${parts.slice(-levels).join("/")}`;
}

/** A text that may be a folder path (a question's option), cut the same
 * way when it has more parts than that; any other text as it is. */
export function textTail(text: string, levels = TAIL_LEVELS): string {
  const parts = text.split("/").filter((part) => part.trim() !== "");
  if (parts.length <= levels) return text;
  return `…/${parts.slice(-levels).join("/")}`;
}
