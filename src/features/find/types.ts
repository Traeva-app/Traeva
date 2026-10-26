// Hand-written mirror of Find's Rust types (#198): find.rs in
// crates/traeva-core/src. The Rust types are the source of truth; change
// both together.

/** Whether searches can use the model, for the search page. */
/** The longest question Find takes, in characters (find.rs). */
export const MAX_FIND_QUESTION_CHARS = 200;

export interface FindSetupView {
  /** Settings are saved and the Keychain holds a key. */
  model_ready: boolean;
  provider: string | null;
  model: string | null;
  /** When the user agreed that Find sends the question, if it still counts. */
  consent_agreed_at_ms: number | null;
  /** What will be sent; shown before the user agrees. */
  notice: string;
}

export type FindState = "working" | "done" | "cancelled";

export type WordsBy =
  | { kind: "model" }
  /** The question's own words; `reason` says why the model was not used. */
  | { kind: "question"; reason: string };

export type FileKind = "text" | "markdown" | "word" | "powerpoint" | "excel" | "pdf";

/** A month is 30 days, a year 365. */
export type TimeUnit = "minute" | "hour" | "day" | "week" | "month" | "year";

/** A time range counted back from the search: from `newest` to `oldest`
 * units ago. A file is in it when it was changed, put in its folder or
 * opened then. */
export interface TimeAgo {
  newest: number;
  oldest: number;
  unit: TimeUnit;
}

/** Which of a file's times fell in the range asked for. */
export type FileTime = "modified" | "added" | "opened";

export interface FindWords {
  /** Each thing to look for, in its wordings. Empty: every file of the kinds and time range. */
  things: string[][];
  /** For each thing (same order), words a file on it would use without
   * naming it; 2 of them in a file match the thing, ranked after a wording. */
  related: string[][];
  /** Only files of these kinds; empty: any file. */
  kinds: FileKind[];
  /** Null: any time. */
  when: TimeAgo | null;
}

/** Where a thing was found, best first. */
export type MatchPlace = "name" | "description" | "folder" | "text";

export interface FindMatch {
  /** Index into `words.things`. */
  thing: number;
  /** The wording that was found; for related words, the first of them. */
  wording: string;
  place: MatchPlace;
  /** A few words around it, for the description and the text. */
  snippet: string | null;
  /** The related words found (2 or more) when no wording is in the file. */
  related_words: string[];
}

export interface FindResult {
  /** "R1", "R2", ... within this search. */
  result_id: string;
  root_id: string;
  /** The folder it is in, as a full path. */
  folder: string;
  relative_path: string;
  name: string;
  /** Null for a kind Traeva has no reader for (it matched by name or folder). */
  kind: FileKind | null;
  size: number;
  modified_at_ms: number;
  /** One per thing it matched, in the order of `words.things`. */
  matches: FindMatch[];
  /** Organize's model described it when the file was analysed. */
  described: boolean;
  /** The file changed after that description. */
  description_outdated: boolean;
  /** Memory's record of this file, if it has one. */
  memory_chain_id: string | null;
  /** When the file was last opened, as macOS recorded it; null when it has
   * no record. Always set on the recent files. */
  opened_at_ms: number | null;
  /** When it was put in its folder (created there, or moved or renamed into it). */
  added_at_ms: number | null;
  /** Which of its times fell in the range asked for; null without a time. */
  time_found: FileTime | null;
}

/** What one search looked at. */
export interface FindCoverage {
  folders: number;
  folders_unavailable: number;
  /** More than 5,000 folders, or deeper than 12 levels. */
  folders_left_out: boolean;
  files: number;
  /** More than 20,000 files. */
  file_limit_reached: boolean;
  described: number;
  texts_searched: number;
  texts_too_large: number;
  /** Only in iCloud; Find never downloads them. */
  texts_in_cloud: number;
  texts_unreadable: number;
  /** Not reached within the 10 seconds. */
  texts_not_reached: number;
}

/** One search. It never holds the question text. */
export interface FindView {
  find_id: string;
  state: FindState;
  words: FindWords | null;
  words_by: WordsBy | null;
  /** At most 30, best first. */
  results: FindResult[];
  /** How many files matched in all. */
  matched: number;
  coverage: FindCoverage | null;
  /** Why the search ended without searching the files, in Traeva's words. */
  problem: string | null;
}
