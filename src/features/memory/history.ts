// What Memory's page says about versions, changes and searches, kept apart
// from React so it can be tested on its own (history.test.ts). None of it
// decides anything the core owns: what changed, what matches, whether a
// version can be opened and whether Traeva tries again all come from the
// core's views.

import type {
  AskedBy,
  ChangeAction,
  Coverage,
  HunkPlace,
  PairRefusal,
  PairState,
  ReachedBy,
  StructureChange,
  SearchState,
  SnapshotRef,
  VersionPairView,
} from "./types";
import type { ErrorCode } from "../organize/types";
import type { StructureLine } from "../preview/structure";

/** Each version's number on the record's line, 1 for the oldest. The
 * history lists the line in save order, and one record can go on across
 * several files (#31), so a version is numbered where it first appears: a
 * file's own `sequence` starts again at 1 on every file. */
export type LineNumbers = ReadonlyMap<string, number>;

export function lineNumbers(pairs: VersionPairView[], only: SnapshotRef | null = null): LineNumbers {
  const numbers = new Map<string, number>();
  // One saved version and no pair yet: it is version 1 (R2-MEM-01).
  if (pairs.length === 0 && only) numbers.set(only.snapshot_id, 1);
  for (const pair of pairs) {
    for (const version of [pair.before, pair.after]) {
      if (!numbers.has(version.snapshot_id)) numbers.set(version.snapshot_id, numbers.size + 1);
    }
  }
  return numbers;
}

/** The saved versions of the record's line, oldest first, in the order
 * lineNumbers numbers them: the Versions tab's timeline (#248). */
export function lineVersions(pairs: VersionPairView[], only: SnapshotRef | null = null): SnapshotRef[] {
  if (pairs.length === 0) return only ? [only] : [];
  const seen = new Set<string>();
  const versions: SnapshotRef[] = [];
  for (const pair of pairs) {
    for (const version of [pair.before, pair.after]) {
      if (seen.has(version.snapshot_id)) continue;
      seen.add(version.snapshot_id);
      versions.push(version);
    }
  }
  return versions;
}

/** The pair that made a version (from the one before it) and the first
 * pair that goes on from it, if any. */
export function pairsAround(
  pairs: VersionPairView[],
  version: SnapshotRef,
): { made: VersionPairView | null; next: VersionPairView | null } {
  return {
    made: pairs.find((pair) => pair.after.snapshot_id === version.snapshot_id) ?? null,
    next: pairs.find((pair) => pair.before.snapshot_id === version.snapshot_id) ?? null,
  };
}

/** What the timeline marks on a version, from the pair that made it (A's
 * review of #251): Traeva may have missed saves before it (the core's
 * `gap`), and Memory could not fully say what changed (the pair's state
 * needs a look, as pairStateText says: cannot be compared, described in
 * part, not described and not tried again; or only part of each version
 * was read and that part did not change, while the bytes did). A try that
 * comes again by itself, waiting, and not permitted are not marks. Only the
 * core's facts; the page judges nothing. */
export function stopNotes(made: VersionPairView | null): { gap: boolean; unclear: boolean } {
  if (made === null) return { gap: false, unclear: false };
  const sameBytes = made.before.sha256 === made.after.sha256;
  const state = pairStateText(made.state, sameBytes, made.complete, previewKind(made.after));
  const retrying = (made.state.kind === "failed" || made.state.kind === "invalid_output") && made.state.will_retry;
  const unreadChange =
    (made.state.kind === "no_text_change" || made.state.kind === "structure_only") && !made.complete && !sameBytes;
  return { gap: made.gap, unclear: (state.tone === "warn" && !retrying) || unreadChange };
}

/** What the timeline says of a mark. */
export const STOP_GAP_TEXT = "Traeva may have missed saves before this version";
export const STOP_UNCLEAR_TEXT = "Memory could not fully say what changed";

/** How the timeline names a stop to VoiceOver: the version, then its
 * marks in words. */
export function stopLabel(version: SnapshotRef, numbers: LineNumbers, notes: { gap: boolean; unclear: boolean }): string {
  const parts = [versionLabel(version, numbers)];
  if (notes.gap) parts.push(STOP_GAP_TEXT);
  if (notes.unclear) parts.push(STOP_UNCLEAR_TEXT);
  return parts.join(". ");
}

/** How the timeline's ends name a version: "v1 · 28 Sep, 21:40". */
export function timelineLabel(version: SnapshotRef, numbers: LineNumbers): string {
  const n = numbers.get(version.snapshot_id);
  return n === undefined ? savedAt(version) : `v${n} · ${savedAt(version)}`;
}

function savedAt(version: SnapshotRef): string {
  return new Date(version.captured_at_ms).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A Word version (#66, the student's limited Go of 2026-10-11): it is
 * never opened in Word; its text is shown on the page instead. Told by the
 * name it was saved under, as the core's readers are. */
export function isDocxVersion(version: SnapshotRef): boolean {
  return /\.docx$/i.test(version.relative_path);
}

/** A slide deck or workbook version (#28, contract v23): like a Word one,
 * never opened in its app; its text is shown on the page. */
export function previewKind(version: SnapshotRef): "deck" | "workbook" | null {
  if (/\.pptx$/i.test(version.relative_path)) return "deck";
  if (/\.xlsx$/i.test(version.relative_path)) return "workbook";
  return null;
}

/** The button that opens a version: a copy for a text file, the text on
 * the page for a Word, PowerPoint or Excel file. `ref` is how the page
 * names the version. */
export function openActionText(version: SnapshotRef, ref: string | null = null): string {
  if (isDocxVersion(version) || previewKind(version) !== null) {
    return ref ? `Preview the text of ${ref}` : "Preview text";
  }
  return ref ? `Open a copy of ${ref}` : "Open a copy";
}

/** Said under a slide deck's or workbook's text: what the preview shows
 * that Memory leaves out or does not check (contract v23 §4.4). */
export const DECK_PREVIEW_NOTE =
  "This is the saved PowerPoint version's text, read for this preview only. Hidden slides are shown here and marked; Memory leaves them out and never sends them. Pictures, charts and animations are not shown. PowerPoint is not opened, and showing it sends nothing.";
export const WORKBOOK_PREVIEW_NOTE =
  "This is the saved Excel version's cells, read for this preview only. A formula's value is the one the file saved; nobody checked it is current. Hidden sheets are shown here and marked; Memory leaves them out and never sends them. Rows or columns that moved show as many cells changing. Excel is not opened, and showing it sends nothing.";

/** A structure line of a deck or workbook preview, as the page labels it. */
export function structureLabel(line: StructureLine): string {
  switch (line.kind) {
    case "slide":
      return `Slide ${line.number}${line.hidden ? " (hidden)" : ""}`;
    case "role":
      return line.role === "notes" ? "Speaker notes" : line.role === "title" ? "Title" : "Text";
    case "sheet":
      return `Sheet “${line.name}”${line.visibility === "hidden" ? " (hidden)" : line.visibility === "very_hidden" ? " (very hidden)" : ""}`;
    case "cell":
      if (line.field === "formula") return `${line.address} · formula`;
      if (line.field === "literal") return `${line.address} · value`;
      if (line.valueType === null) return `${line.address} · no saved value`;
      if (line.valueType === "blank") return `${line.address} · saved value: empty (not verified)`;
      return `${line.address} · saved value (not verified)`;
    case "unrecognized":
      return "(a line Traeva could not read)";
  }
}

/** What a preview's summary says before its text: nothing for a complete
 * one; what was left out of a partial one; why there is none otherwise. */
export function previewStatusText(summary: {
  status: "complete" | "partial" | "unsupported" | "failed";
  left_out: string[];
  note: string | null;
}): string | null {
  switch (summary.status) {
    case "complete":
      return null;
    case "partial":
      return `Part of the text: left out are ${summary.left_out.join("; ")}.${summary.note ? ` ${summary.note}` : ""}`;
    case "unsupported":
      return `This version's text cannot be shown${summary.note ? `: ${summary.note}` : "."}`;
    case "failed":
      return `This version could not be read${summary.note ? `: ${summary.note}` : "."}`;
  }
}

/** Said under a Word version's text. The preview's reader also shows the
 * saved text of citation fields, which the reader Memory compares and
 * sends leaves out (#124): the note says so (A's review). */
export const DOCX_PREVIEW_NOTE =
  "This is the saved Word version's text, read for this preview only. It can include the saved text of citation fields, which Memory does not compare or send; citations that are plain text are part of the body. Word is not opened, and showing it sends nothing.";

/** "Version 3 · 9 Oct, 22:03"; only the time when the line does not list it. */
export function versionLabel(version: SnapshotRef, numbers: LineNumbers): string {
  const n = numbers.get(version.snapshot_id);
  return n === undefined ? `Saved ${savedAt(version)}` : `Version ${n} · ${savedAt(version)}`;
}

/** The name an export of `version` starts with: "essay (version 3).md".
 * The user may change it in the save panel; the core checks it. */
export function exportName(version: SnapshotRef, numbers: LineNumbers): string {
  const file = version.relative_path.split("/").pop() ?? "";
  const dot = file.lastIndexOf(".");
  const stem = dot > 0 ? file.slice(0, dot) : file;
  const extension = dot > 0 ? file.slice(dot) : "";
  const n = numbers.get(version.snapshot_id);
  return n === undefined ? `${stem} (saved copy)${extension}` : `${stem} (version ${n})${extension}`;
}

/** Only the text files Memory reads are exported (export.rs), as for
 * opening a copy. */
export function canExport(version: SnapshotRef): boolean {
  return /\.(txt|md|markdown)$/i.test(version.relative_path);
}

/** The refusals of an export the user can act on, in their words. Nothing
 * was written in each case. Others are shown as the core words them: a
 * name that cannot be used says why, and `result_unconfirmed` is not a
 * failure (the copy may be there). */
export function exportRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "target_conflict":
      return "A file with this name is already there. Traeva never replaces a file: export again and choose another name. Nothing was written.";
    case "outside_root":
      return "That place is not inside the folder this version is from. Export again and stay inside it. Nothing was written.";
    case "unsupported_location":
      return "Traeva cannot write there (a link, another disk, or not a plain folder). Choose another folder. Nothing was written.";
    case "protected_location":
      return "That folder is hidden, a package or a project folder, where Traeva never writes. Nothing was written.";
    case "unresolved_action":
      return "An Organize action in this folder is not settled yet. Settle it first. Nothing was written.";
    case "page_outdated":
      return "The folder changed since it was shown. Nothing was written. Export again.";
    default:
      return null;
  }
}

/** Changes whenever the history the core reports changes (a new version,
 * a new description, a new state), so a finished search can be read again
 * for its `updates_available` (R2-MEM-02). */
export function historyKey(pairs: VersionPairView[] | null): string {
  return JSON.stringify(pairs ?? []);
}

/** "version 3", inside a sentence. */
export function versionRef(version: SnapshotRef, numbers: LineNumbers): string {
  const n = numbers.get(version.snapshot_id);
  return n === undefined ? `the version saved ${savedAt(version)}` : `version ${n}`;
}

export type Tone = "ok" | "wait" | "warn";

/** How a pair of versions stands, in a few words and a tone. `sameBytes`:
 * the two saved versions have the same SHA-256, so only that is said, not
 * that the bytes differ (R2-MEM-04: the same content saved again as a new
 * file). `kind`: what the file is (`previewKind`), for what a change of
 * labels means. */
export function pairStateText(
  state: PairState,
  sameBytes = false,
  complete = true,
  kind: "deck" | "workbook" | null = null,
): { label: string; detail: string | null; tone: Tone } {
  switch (state.kind) {
    case "not_compared":
      return { label: "Not compared yet", detail: null, tone: "wait" };
    case "no_text_change":
      // A reader that leaves parts out (hidden slides, tables, fields...)
      // can only speak for what it read.
      if (!complete) {
        return {
          label: "No change in the text Memory reads",
          detail: PARTIAL_TEXT,
          tone: "ok",
        };
      }
      return {
        label: "The text did not change",
        detail: sameBytes
          ? "This version holds exactly the bytes of the one before it."
          : "The file's bytes differ (line endings, for example), but its text is the same.",
        tone: "ok",
      };
    case "structure_only":
      // A workbook's labels are its cells' addresses and value types: the
      // writer moved values or stored them another way (contract v23 §4.4).
      if (state.change === "labels" && kind === "workbook") {
        return { label: CELL_LABELS_TEXT, detail: CELL_LABELS_DETAIL, tone: "ok" };
      }
      return { label: STRUCTURE_TEXT[state.change], detail: STRUCTURE_DETAIL, tone: "ok" };
    case "unreadable":
      return { label: "Cannot be compared", detail: state.note, tone: "warn" };
    case "not_permitted":
      return {
        label: "Changed, not described",
        detail:
          "Memory may not send this file's changed lines to the model. Agree under Describing changes with the model to have them described.",
        tone: "wait",
      };
    case "queued":
      return { label: "Changed, waiting to be described", detail: null, tone: "wait" };
    case "analyzing":
      return { label: "Being described…", detail: null, tone: "wait" };
    case "analyzed":
      if (state.complete) return { label: "Described", detail: null, tone: "ok" };
      return {
        label: "Described in part",
        detail:
          state.left_out_hunks > 0
            ? `${state.left_out_hunks} of ${state.hunks} changed places were too large to send; nothing is said about them.`
            : PARTIAL_TEXT,
        tone: "warn",
      };
    case "failed":
      return {
        label: "Could not be described",
        detail: `${state.error_code}. ${retryText(state.will_retry)}`,
        tone: "warn",
      };
    case "invalid_output":
      return {
        label: "The model's description was not usable",
        detail: `It broke a rule, so it was dropped (${
          state.reason ?? "which rule was not recorded: an earlier analysis"
        }). ${retryText(state.will_retry)}`,
        tone: "warn",
      };
  }
}

function retryText(willRetry: boolean): string {
  return willRetry
    ? "Traeva tries again by itself."
    : "Traeva does not try again by itself; check the model settings.";
}

/** Said of a pair whose texts were not read whole. */
const PARTIAL_TEXT =
  "Memory does not read every part of these versions (hidden slides or sheets, formatting, tables, fields or text beyond its limit, for example), so nothing is said about those parts.";

/** What changed when only a slide deck's or a workbook's structure did. */
const STRUCTURE_TEXT: Record<StructureChange, string> = {
  slide_order: "Only the order of the slides changed",
  sheet_order: "Only the order of the sheets changed",
  sheet_renamed: "A sheet was renamed; no cell changed",
  labels: "Only the lines Traeva adds changed (such as slide numbers)",
};
const STRUCTURE_DETAIL = "The text on the slides or in the cells is the same, so nothing was sent to be described.";
const CELL_LABELS_TEXT = "Only cell addresses or value types changed";
const CELL_LABELS_DETAIL =
  "The values are the same, but some are in other cells or stored as another type (a number stored as text, for example). Memory compares cells by their address, so nothing was sent to be described.";

/** A pair Memory may not have seen every save of: what it shows is the
 * difference between the two versions, never one save. */
export function pairGapText(pair: VersionPairView): string | null {
  if (!pair.gap) return null;
  return "Traeva may not have seen every save between these two versions, so this is the difference between them, not one save.";
}

/** Some pair is still on its way: the page asks again in a moment. */
export function isSettling(pairs: VersionPairView[]): boolean {
  return pairs.some(
    (pair) =>
      pair.state.kind === "not_compared" || pair.state.kind === "queued" || pair.state.kind === "analyzing",
  );
}

export const ACTION_TEXT: Record<ChangeAction, string> = {
  added: "Added",
  removed: "Removed",
  modified: "Changed",
};

/** Edits the model called minor: spacing, line breaks, punctuation, case. */
export function minorText(minorHunks: string[]): string | null {
  if (minorHunks.length === 0) return null;
  const places = minorHunks.length === 1 ? "1 place" : `${minorHunks.length} places`;
  return `Small edits in ${places} (spacing, line breaks, punctuation or case).`;
}

/** What a search could look at. Complete only when every pair is described
 * in full and no recorded gap lies between two versions. */
export function coverageText(coverage: Coverage): string {
  if (coverage.pairs === 0) return "This file has only one saved version: no changes to search.";
  if (coverage.complete) {
    return `Searched every change in all ${coverage.pairs} pairs of versions.`;
  }
  const parts: string[] = [];
  const add = (count: number, text: string) => {
    if (count > 0) parts.push(`${count} ${text}`);
  };
  add(coverage.described_in_part, "described in part");
  add(coverage.waiting, "not described yet");
  add(coverage.not_permitted, "not permitted to be described");
  add(coverage.failed, "could not be described");
  add(coverage.unreadable, "unreadable");
  add(coverage.gaps, "where Traeva may have missed saves in between");
  const left = coverage.changes_left_out > 0 ? ` The oldest ${coverage.changes_left_out} changes were left out.` : "";
  return (
    `Searched ${coverage.described} of ${coverage.pairs} pairs of versions in full` +
    (parts.length > 0 ? `; ${parts.join(", ")}.` : ".") +
    left +
    (coverage.gap_after_latest
      ? " Traeva may have missed saves after the newest version (it was not running, or Memory was off)."
      : "") +
    " A match may also be in the part not searched."
  );
}

/** A search entry for a pair whose structure only changed (core:
 * `structure_edit`): its description is Traeva's fixed line. */
export function isStructureEdit(change: { change_id: string; position: number }): boolean {
  return change.position === 0 && change.change_id.startsWith("structure-");
}

/** Where a change is in a slide deck or workbook, from the reader's
 * structure lines: "Slide 2 · notes", "Sheet “Data” · C2 · value the file
 * saved (not verified)". Null when there is nothing to say. */
export function placeText(place: HunkPlace | null): string | null {
  if (!place) return null;
  const parts: string[] = [];
  if (place.slide !== null) parts.push(`Slide ${place.slide}`);
  if (place.role !== null) parts.push(place.role === "notes" ? "speaker notes" : place.role);
  if (place.sheet !== null) parts.push(`Sheet “${place.sheet}”`);
  if (place.cell !== null) parts.push(place.cell);
  if (place.field !== null) parts.push(FIELD_TEXT[place.field]);
  return parts.length === 0 ? null : parts.join(" · ");
}

const FIELD_TEXT: Record<NonNullable<HunkPlace["field"]>, string> = {
  literal: "value",
  formula: "formula",
  cache: "value the file saved (not verified)",
};

/** The search is waiting for the model: the page asks again in a moment. */
export function isWorking(state: SearchState): boolean {
  return state.kind === "working";
}

/** Said above described changes (R2-MEM-08): Traeva checks that each
 * description rests on lines that really changed, not that its words are
 * right. */
export const MODEL_WORDS_NOTE =
  "Each description is the model's wording. The lines under it are read again from your saved versions: check that they match.";

/** How a receipt names an opened copy: by the number in its name, fixed
 * when it was made, so a later change of the line never renames it on the
 * page (R2-MEM-11). */
export function openedCopyRef(copy: { number: number | null }): string {
  return copy.number === null ? "an older version" : `version ${copy.number}`;
}

export function reachedByText(reachedBy: ReachedBy): string {
  switch (reachedBy) {
    case "search":
      return "Memory found this as the only match for your description.";
    case "answer":
      return "Chosen by your answer to the question.";
    case "pick":
      return "You picked this one. Memory did not check that it is the only match.";
  }
}

/** A candidate whose pair the core no longer lists: a relation was
 * corrected after the search found it, so it is no longer certain
 * (#31 material §8, last paragraph). */
export function noLongerCertainText(reason: PairRefusal): string {
  const why: Record<PairRefusal, string> = {
    relation_superseded: "the relation between these two versions was revoked or replaced",
    relation_disputed: "the relation between these two versions is disputed",
    no_confirmed_relation: "these two versions are no longer joined by a confirmed relation",
    not_a_member: "a version's file is no longer part of this work",
    candidate_version: "a version waits for your decision",
    not_adjacent: "these two versions are no longer next to each other",
  };
  return `No longer certain: ${why[reason]} since the search. Search again to use the history as it is now.`;
}

/** Why a search stopped without an answer. `page_outdated`: a relation
 * it read was corrected meanwhile, so nothing more was sent. */
export function searchFailedText(code: ErrorCode): string {
  return code === "page_outdated"
    ? "The history changed while searching (a relation was corrected), so nothing more was sent. Search again."
    : `The model could not be asked (${code}).`;
}

export function unsupportedText(askedBy: AskedBy): string {
  switch (askedBy) {
    case "state":
      return "Finding a version by what the text looked like is not supported yet. Describe a change you made instead, for example “before I added the conclusion”.";
    case "time":
      return "Finding a version by its date or time is not supported yet. Describe a change you made instead, or look through the versions below.";
    case "name":
      return "Finding a version by a file name is not supported yet. Describe a change you made instead.";
  }
}

/** The title of a search's state. */
export function searchTitle(state: SearchState): string {
  switch (state.kind) {
    case "working":
      return "Searching…";
    case "found":
      return state.candidate.no_longer_usable ? "No longer certain" : "Found";
    case "candidates":
      if (state.question) return "One question";
      // One candidate from a history with gaps is not an answer, but it is
      // not several either.
      return state.candidates.length === 1 ? "One possible match" : "Several matches";
    case "not_found":
      return "Not found";
    case "cannot_tell":
      return "Cannot tell yet";
    case "unsupported":
      return "Not supported yet";
    case "unclear":
      return "Not clear";
    case "none_of_these":
      return "None of these";
    case "not_permitted":
      return "Not permitted";
    case "failed":
      return "The search failed";
    case "cancelled":
      return "Cancelled";
  }
}
