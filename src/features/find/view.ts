// What the Find page shows (#198, contract §4.10): labels and sentences
// for the core's views, and the rows of its home. Text only: which files
// match and in what order is the core's; the page shows its results as
// returned, and the home lists only what other views already hold.

import { needsYou, type DropOutcome } from "../autodrop/queue.ts";
import type { DropItemView } from "../autodrop/types.ts";
import type { MemoryRecordView } from "../family/types.ts";
import type { ActionView } from "../organize/types.ts";
import type {
  FileKind,
  FindCoverage,
  FindMatch,
  FindResult,
  FindSetupView,
  FindView,
  FindWords,
  MatchPlace,
  TimeAgo,
  WordsBy,
} from "./types.ts";

const KIND_LABELS: Record<FileKind, string> = {
  text: "Text",
  markdown: "Markdown",
  word: "Word",
  powerpoint: "PowerPoint",
  excel: "Excel",
  pdf: "PDF",
};

/** A name's extension in lower case, or null. */
function extension(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return null;
  return name.slice(dot + 1).toLowerCase();
}

// Images Organize reads by on-device text recognition (#37, 10/14). Find
// and Memory do not read them; their kind comes from the extension only.
const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png"];

/** Whether a file is an image Organize reads by text recognition, by its
 * name's extension. */
export function isImageName(name: string): boolean {
  const ext = extension(name);
  return ext !== null && IMAGE_EXTENSIONS.includes(ext);
}

/** The file's kind; without one Traeva reads, its extension, or null. An
 * image says so ("JPG image"). */
export function kindLabel(kind: FileKind | null, name: string): string | null {
  if (kind !== null) return KIND_LABELS[kind];
  const ext = extension(name)?.toUpperCase() ?? null;
  return ext !== null && isImageName(name) ? `${ext} image` : ext;
}

/** A file icon: its colour by kind, and the short label on its band ("" for
 * none). An image's icon also shows a small picture. */
export type FileIcon = { kind: FileKind | "image" | "other"; label: string };

const ICON_LABELS: Record<FileKind, string> = {
  text: "TXT",
  markdown: "MD",
  word: "W",
  powerpoint: "PPT",
  excel: "XLS",
  pdf: "PDF",
};

// For a name without a kind from the core (dropped files, actions, records).
const KINDS_BY_EXTENSION = new Map<string, FileKind>([
  ["txt", "text"],
  ["md", "markdown"],
  ["markdown", "markdown"],
  ["docx", "word"],
  ["doc", "word"],
  ["pptx", "powerpoint"],
  ["ppt", "powerpoint"],
  ["xlsx", "excel"],
  ["xls", "excel"],
  ["pdf", "pdf"],
]);

/** The icon for a file: its kind, else one guessed from the extension for
 * the colour only; any other extension of up to 4 letters as the label. */
export function fileIcon(kind: FileKind | null, name: string): FileIcon {
  const ext = extension(name);
  const known = kind ?? (ext === null ? undefined : KINDS_BY_EXTENSION.get(ext));
  if (known !== undefined) return { kind: known, label: ICON_LABELS[known] };
  if (ext !== null && isImageName(name)) return { kind: "image", label: ext.toUpperCase() };
  return { kind: "other", label: ext !== null && ext.length <= 4 ? ext.toUpperCase() : "" };
}

function sameDay(a: Date, b: Date): boolean {
  return a.toDateString() === b.toDateString();
}

/** When something happened: the time today or yesterday, else the date
 * (with the year when it is not this year's). */
export function whenText(ms: number, now: number): string {
  const at = new Date(ms);
  if (Number.isNaN(at.getTime())) return "unknown";
  const today = new Date(now);
  const time = at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (sameDay(at, today)) return `today, ${time}`;
  if (sameDay(at, new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1))) {
    return `yesterday, ${time}`;
  }
  return at.toLocaleDateString(
    [],
    at.getFullYear() === today.getFullYear()
      ? { month: "short", day: "numeric" }
      : { month: "short", day: "numeric", year: "numeric" },
  );
}

const PLACE_LABELS: Record<MatchPlace, string> = {
  name: "In its name",
  folder: "In its folder's name",
  description: "From Traeva's description",
  text: "From its text",
};

/** Related words can be in different places of one file: one label. */
const RELATED_LABEL = "Related words it uses";

/** Why a result matched: one line per match, in the core's order. */
export type MatchLine = {
  label: string;
  /** The wording that was found. */
  wording: string;
  /** A few words around it (description and text only). */
  snippet: string | null;
  /** Traeva described the file before it last changed. */
  outdated: boolean;
};

export function matchLines(result: FindResult): MatchLine[] {
  return result.matches.map((match: FindMatch) => ({
    label: match.related_words.length > 0 ? RELATED_LABEL : PLACE_LABELS[match.place],
    wording: match.related_words.length > 0 ? match.related_words.join(", ") : match.wording,
    snippet: match.place === "description" || match.place === "text" ? match.snippet : null,
    outdated: match.place === "description" && result.description_outdated,
  }));
}

/** "a", "a and b", "a, b and c". */
function listText(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

function count(n: number, one: string, many: string): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** "changed, added or opened 1 to 3 days ago", "… in the last 15 minutes". */
export function timeText(when: TimeAgo): string {
  const unit = (n: number) => (n === 1 ? when.unit : `${when.unit}s`);
  const range =
    when.newest === 0
      ? `in the last ${when.oldest === 1 ? when.unit : `${when.oldest} ${unit(when.oldest)}`}`
      : `${when.newest} to ${when.oldest} ${unit(when.oldest)} ago`;
  return `changed, added or opened ${range}`;
}

/** The time a result shows: the one that fell in the range asked for,
 * otherwise when it was modified. */
export function resultTimeText(result: FindResult, now: number): string {
  if (result.time_found === "added" && result.added_at_ms !== null) {
    return `added to its folder ${whenText(result.added_at_ms, now)}`;
  }
  if (result.time_found === "opened" && result.opened_at_ms !== null) {
    return `opened ${whenText(result.opened_at_ms, now)}`;
  }
  return `modified ${whenText(result.modified_at_ms, now)}`;
}

/** “chemistry” or “化学” (or 2 of reaction, mole, acid). */
function thingText(wordings: string[], related: string[]): string {
  const found = wordings.map((wording) => `“${wording}”`).join(" or ");
  return related.length > 0 ? `${found} (or 2 of ${related.join(", ")})` : found;
}

/** The words the search used, with the kinds and the time range. */
export function wordsText(words: FindWords): string {
  const parts = [
    words.things.length === 0
      ? "Looked for any file"
      : `Looked for ${words.things.map((thing, at) => thingText(thing, words.related[at] ?? [])).join("; ")}`,
  ];
  if (words.kinds.length > 0) {
    parts.push(`only ${listText(words.kinds.map((kind) => KIND_LABELS[kind]))} files`);
  }
  if (words.when !== null) parts.push(timeText(words.when));
  return `${parts.join(" · ")}.`;
}

export function wordsByText(by: WordsBy): string {
  return by.kind === "model" ? "Searched with the model." : `Searched with your words: ${by.reason}`;
}

/** Why a finished search shows no files before any count: the core could
 * not search (its problem), or the question left nothing to look for (only
 * common words, no kind, no time range: the core lists no file then,
 * contract §4.10). Null: the usual count and coverage. */
export function noSearchText(view: FindView): string | null {
  if (view.problem !== null) return view.problem;
  const words = view.words;
  if (words !== null && words.things.length === 0 && words.kinds.length === 0 && words.when === null) {
    return "Type a word about the file.";
  }
  return null;
}

export function matchedText(view: FindView): string {
  if (view.matched === 0) return "No files matched.";
  const matched = `${count(view.matched, "file", "files")} matched`;
  return view.matched > view.results.length
    ? `${matched}, showing the first ${view.results.length}.`
    : `${matched}.`;
}

/** What the search looked at and what it left out; only what happened.
 * `folders` counts the folders the user added, not their subfolders.
 * Without words (any file), no description was compared. */
export function coverageLines(coverage: FindCoverage, words: FindWords | null): string[] {
  const lines: string[] = [];
  const added = coverage.folders === 1 ? "the folder" : `the ${count(coverage.folders, "folder", "folders")}`;
  lines.push(
    coverage.folders === 0
      ? "No folder could be searched."
      : `Searched ${count(coverage.files, "file", "files")} in ${added} you added.`,
  );
  const compared = words !== null && words.things.length > 0;
  if (compared && coverage.described > 0) {
    lines.push(`Searched Traeva's descriptions of ${count(coverage.described, "file", "files")}.`);
  }
  if (coverage.texts_searched > 0) {
    lines.push(`Searched the text of ${count(coverage.texts_searched, "file", "files")}.`);
  }
  const skipped = (
    [
      [coverage.texts_too_large, "over 20 MB, searched by name only"],
      [coverage.texts_in_cloud, "only in iCloud, not downloaded"],
      [coverage.texts_unreadable, "unreadable"],
      [coverage.texts_not_reached, "not reached within 10 seconds"],
    ] as const
  )
    .filter(([n]) => n > 0)
    .map(([n, why]) => `${n.toLocaleString()} ${why}`);
  if (skipped.length > 0) lines.push(`Text not searched: ${skipped.join("; ")}.`);
  if (coverage.folders_unavailable > 0) {
    lines.push(
      `${count(coverage.folders_unavailable, "folder you added was", "folders you added were")} unavailable and not searched.`,
    );
  }
  if (coverage.folders_left_out) {
    lines.push("Some subfolders were left out: the search stops at 5,000 folders or 12 levels deep.");
  }
  if (coverage.file_limit_reached) lines.push("The search stopped at 20,000 files.");
  return lines;
}

/** How the model takes part: none is set up; set up but not agreed to;
 * agreed to. */
export type ModelUse = "no_model" | "ask" | "agreed";

export function modelUse(setup: FindSetupView): ModelUse {
  if (!setup.model_ready) return "no_model";
  return setup.consent_agreed_at_ms === null ? "ask" : "agreed";
}

// How far a search is: working, working with its words, finished.
function progress(view: FindView): number {
  if (view.state !== "working") return 2;
  return view.words === null ? 0 : 1;
}

/** Whether a view from the core may replace the one shown: only the latest
 * search (`latest`), and never one that is behind it (a poll answered late
 * after a newer answer or a cancel). */
export function accepts(latest: string | null, shown: FindView | null, next: FindView): boolean {
  if (latest === null || next.find_id !== latest) return false;
  if (shown === null || shown.find_id !== next.find_id) return true;
  return progress(next) >= progress(shown);
}

// ---------------------------------------------------------------------------
// The home: shown while no search is
// ---------------------------------------------------------------------------

/** The last part of a path ("" for none). */
export function lastSegment(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? "";
}

/** The folder a path is in ("" for a bare name). */
export function parentPath(path: string): string {
  const slash = path.lastIndexOf("/");
  if (slash < 0) return "";
  return slash === 0 ? "/" : path.slice(0, slash);
}

/** A folder of the recent files, with its newest one. */
export type RecentFolder = {
  /** The full path. */
  folder: string;
  name: string;
  /** The newest file in it: Reveal in Finder opens the folder and selects it. */
  resultId: string;
};

/** The folders of the recent files, each once, the one with the newest
 * opened file first (the core's order when the times are equal). */
export function recentFolders(results: FindResult[]): RecentFolder[] {
  const newest = new Map<string, FindResult>();
  for (const result of results) {
    const known = newest.get(result.folder);
    if (known === undefined || (result.opened_at_ms ?? -1) > (known.opened_at_ms ?? -1)) {
      newest.set(result.folder, result);
    }
  }
  return [...newest.values()]
    .sort((a, b) => (b.opened_at_ms ?? -1) - (a.opened_at_ms ?? -1))
    .map((result) => ({
      folder: result.folder,
      name: lastSegment(result.folder) || result.folder,
      resultId: result.result_id,
    }));
}

/** A row's one action: go to Organize (its dropped files, or its actions
 * that need a decision), or show the record in Memory. */
export type PickUpAction =
  | { kind: "organize"; label: "Answer" | "Review" }
  | { kind: "decision"; label: "Review" }
  | { kind: "history"; chainId: string };

/** One row of "Pick up where you left off". */
export type PickUpRow = {
  key: string;
  name: string;
  detail: string;
  /** Null: there is nothing to do, only to know. */
  action: PickUpAction | null;
};

/** The row whose button is the home's one filled button: the first thing
 * that waits for the user (-1: none; the newest recent file's Open is then). */
export function firstWaiting(rows: PickUpRow[]): number {
  return rows.findIndex((row) => row.action !== null && row.action.kind !== "history");
}

/** At most this many rows. */
export const MAX_PICK_UP = 3;
/** A version or a move older than this is not "where you left off". */
export const LEFT_OFF_MS = 7 * 24 * 60 * 60 * 1000;

const DROP_ROWS: Partial<Record<DropItemView["state"], { detail: string; label: "Answer" | "Review" }>> = {
  needs_answer: { detail: "Traeva has a question about it", label: "Answer" },
  needs_source_authorization: { detail: "Needs your OK for its folder", label: "Review" },
  handed_over: { detail: "Needs you to finish organizing", label: "Review" },
};

function versionsText(n: number): string {
  return count(n, "version", "versions");
}

/** The recorded file whose newest version was saved last, in the past week. */
function newestVersionRow(records: MemoryRecordView[], now: number): PickUpRow | null {
  let best: { record: MemoryRecordView; at: number } | null = null;
  for (const record of records) {
    const latest = record.watch.latest;
    if (record.state !== "active" || !record.root_active || record.waiting_for_relocation || latest === null) continue;
    const at = latest.captured_at_ms;
    if (now - at > LEFT_OFF_MS) continue;
    if (best === null || at > best.at) best = { record, at };
  }
  if (best === null) return null;
  const { record, at } = best;
  return {
    key: `record:${record.chain_id}`,
    name: lastSegment(record.place.relative_path) || record.place.display_path,
    detail: `Last version saved ${whenText(at, now)} · ${versionsText(record.line_versions)}`,
    action: { kind: "history", chainId: record.chain_id },
  };
}

/** The newest action, when it moved a file in the past week. */
function lastMoveRow(action: ActionView | null, now: number): PickUpRow | null {
  if (action === null || action.status !== "committed") return null;
  const at = action.committed_at_ms ?? action.created_at_ms;
  if (now - at > LEFT_OFF_MS) return null;
  // An undo's target is where the file was before it was organized.
  const verb = action.kind === "undo" ? "Put back in" : "Moved to";
  const folder = lastSegment(parentPath(action.target.display_path)) || "the folder you added";
  return {
    key: `action:${action.action_id}`,
    name: lastSegment(action.target.relative_path),
    detail: `${verb} ${folder} · ${whenText(at, now)}`,
    action: null,
  };
}

/** "Pick up where you left off": what waits for the user first (dropped
 * files, then actions that need a decision), then the file with the newest
 * version, then the last move; at most MAX_PICK_UP rows. `more`: the
 * waiting things left out. Only what these views hold; nothing is guessed. */
export function pickUpRows(from: {
  drops: DropItemView[];
  /** Handed-over files that ended in Organize without a plan (this window only). */
  outcomes: Record<string, DropOutcome>;
  decisions: ActionView[];
  /** The newest action (recent_actions[0]), if any. */
  lastAction: ActionView | null;
  records: MemoryRecordView[];
  now: number;
}): { rows: PickUpRow[]; more: number } {
  const waiting: PickUpRow[] = [];
  for (const item of from.drops) {
    const row = DROP_ROWS[item.state];
    if (row === undefined || !needsYou(item, from.outcomes)) continue;
    waiting.push({
      key: `drop:${item.drop_id}`,
      name: item.file_name,
      detail: row.detail,
      action: { kind: "organize", label: row.label },
    });
  }
  for (const action of from.decisions) {
    waiting.push({
      key: `decision:${action.action_id}`,
      name: lastSegment(action.source.relative_path),
      detail: "Needs your decision",
      action: { kind: "decision", label: "Review" },
    });
  }
  const rows = [...waiting];
  const version = newestVersionRow(from.records, from.now);
  if (version !== null) rows.push(version);
  const move = lastMoveRow(from.lastAction, from.now);
  if (move !== null) rows.push(move);
  return { rows: rows.slice(0, MAX_PICK_UP), more: Math.max(0, waiting.length - MAX_PICK_UP) };
}
