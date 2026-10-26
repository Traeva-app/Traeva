// Small decisions the page makes while it shows an operation, kept apart
// from React so they can be tested on their own (flow.test.ts). None of them
// decides anything the core owns: whether to recommend or ask, whether a
// plan can be confirmed, and whether a failure can be retried all come from
// the core's views and errors.

import type {
  ActionView,
  AppError,
  CandidateView,
  ClarificationAnswer,
  ExtractionSummary,
  FileView,
  OcrState,
  OperationStage,
  OperationView,
  PlanEnd,
  QuestionView,
  RootView,
  SuggestionView,
  UndoStatus,
  UserChoice,
} from "./types";
import type { Flow } from "./shared";

/** No file is on its way, so Undo or Check again may start their own flow:
 * one flow at a time, and a preview on screen is never replaced. */
export function isIdle(step: Flow["step"]): boolean {
  return step === "start" || step === "done" || step === "in_place" || step === "failed";
}

/** The flow on screen is still the one a request (Undo, Check again) started
 * from, so the request's answer may replace it. A result read again in the
 * background (the same action) still counts: a dropped file can make the
 * page read its result again while the request runs (#168). */
export function isStill(started: Flow, current: Flow): boolean {
  if (current === started) return true;
  return (
    started.step === "done" &&
    current.step === "done" &&
    current.action.action_id === started.action.action_id
  );
}

/** The core is still working: the window keeps asking about every 300 ms. */
export function isBusy(stage: OperationStage): boolean {
  return stage === "observing" || stage === "analyzing";
}

/** A polled view may replace the shown one only if it is the same operation
 * and not older (a late answer never replaces a newer state). */
export function isNewer(next: OperationView, shown: OperationView): boolean {
  return next.operation_id === shown.operation_id && next.generation >= shown.generation;
}

/** Which card the operation needs. */
export type OperationCardKind =
  | "reading"
  | "ready"
  | "analyzing"
  | "question"
  | "recommendation"
  | "manual"
  | "analysis_failed"
  | "ended";

export function cardFor(stage: OperationStage): OperationCardKind {
  switch (stage) {
    case "observing":
      return "reading";
    case "ready_for_analysis":
      return "ready";
    case "analyzing":
      return "analyzing";
    case "needs_clarification":
      return "question";
    case "recommendation_ready":
      return "recommendation";
    case "needs_manual_location":
      return "manual";
    case "analysis_failed":
    case "invalid_model_output":
      return "analysis_failed";
    case "waiting_for_file":
    case "stale":
    case "blocked":
    case "cancelled":
      return "ended";
  }
}

/** The organize flow's four steps, shown above its card. */
export const FLOW_STEPS = ["File", "Folder", "Preview", "Result"] as const;

/** Which of FLOW_STEPS the flow is at, read from the flow's step and the
 * operation's stage only; null where the bar does not fit: an Undo, a
 * failure, an operation that ended. */
export function flowStepIndex(flow: Flow): number | null {
  switch (flow.step) {
    case "start":
      return 0;
    case "operation": {
      const card = cardFor(flow.operation.stage);
      if (card === "ended") return null;
      return card === "reading" ? 0 : 1;
    }
    case "preview":
      return flow.plan.kind === "undo" ? null : 2;
    case "applying":
      return flow.plan.kind === "undo" ? null : 3;
    case "done":
      return flow.action.kind === "undo" ? null : 3;
    case "in_place":
      return 3;
    case "failed":
      return null;
  }
}

/** The core's error offers "try again" (never decided by the window). */
export function canRetry(error: AppError | null): boolean {
  return error !== null && error.allowed_next.includes("try_again");
}

/** The file's own name, which a plan never changes. */
export function basename(file: FileView): string {
  const parts = file.relative_path.split("/");
  return parts[parts.length - 1];
}

/** Where a candidate would put the file, as a path inside the target folder. */
export function candidatePath(candidate: CandidateView, name: string): string {
  return [candidate.folder_path, ...candidate.new_segments, name]
    .filter((part) => part !== "")
    .join("/");
}

/** The candidate Traeva recommends: set only in recommendation_ready. */
export function recommendedCandidate(suggestion: SuggestionView | null): CandidateView | null {
  if (suggestion === null || suggestion.recommended_id === null) return null;
  return (
    suggestion.candidates.find(
      (candidate) => candidate.candidate_id === suggestion.recommended_id,
    ) ?? null
  );
}

/** The candidates offered for a manual choice: none is preselected or
 * marked as recommended. */
export function offeredCandidates(suggestion: SuggestionView | null): CandidateView[] {
  if (suggestion === null || suggestion.recommended_id !== null) return [];
  return suggestion.candidates;
}

/** The one place the model named when it recommended none (the student,
 * 10/14): the card to pick a folder shows it as the likely one, with why it
 * was not recommended, instead of listing it twice. Null when the model
 * named none or several (those keep the full card). */
export function singleCandidate(suggestion: SuggestionView | null): CandidateView | null {
  const offered = offeredCandidates(suggestion);
  return offered.length === 1 ? offered[0] : null;
}

/** The place to show first when picking by hand: the one the model named,
 * unless the user turned to picking by hand while its question was still
 * open ("None of these"): the core keeps that round's places on offer, but
 * the user just passed them by, so the full card shows (A's review of #279).
 * A question counts as open while it is not among the answered ones (up to
 * 3 per file, so an earlier answer does not close a later question). */
export function likelyCandidate(operation: OperationView): CandidateView | null {
  const question = operation.question;
  if (question !== null && !operation.answered.some((answered) => answered.question_id === question.question_id)) {
    return null;
  }
  return singleCandidate(operation.suggestion);
}

/** The name of the folder a candidate points to: its last folder, a new
 * one included; `rootName` for the target folder itself. */
export function candidateFolderName(candidate: CandidateView, rootName: string): string {
  const parts = [...candidate.folder_path.split("/"), ...candidate.new_segments].filter((part) => part !== "");
  return parts[parts.length - 1] ?? rootName;
}

/** The answer to send: the chosen option, or else the user's own words.
 * Null when there is nothing to send yet. The core checks it again. */
export function answerFor(
  question: QuestionView,
  optionId: string | null,
  text: string,
): ClarificationAnswer | null {
  if (optionId !== null) return { question_id: question.question_id, option_id: optionId };
  if (text.trim() === "") return null;
  return { question_id: question.question_id, text };
}

/** A confidence as a percentage, or a dash when the model gave none. It is
 * a score, not an accuracy. */
export function scoreText(confidence: number | null): string {
  return confidence === null ? "—" : `${Math.round(confidence * 100)}%`;
}

/** The folders the next file comes from and goes into: the user's choice
 * while both are still authorized, else the newest folder for both (the one
 * just added is the one most likely meant). Null without any folder. */
export function pickRoots(
  roots: RootView[],
  choice: { source: string; target: string } | null,
): { source: string; target: string } | null {
  const ids = roots.map((root) => root.root_id);
  if (choice !== null && ids.includes(choice.source) && ids.includes(choice.target)) {
    return choice;
  }
  if (roots.length === 0) return null;
  const newest = roots.reduce((a, b) => (b.granted_at_ms > a.granted_at_ms ? b : a));
  return { source: newest.root_id, target: newest.root_id };
}

/** A path inside an authorized folder, written out in full. */
export function inRoot(rootPath: string, relative: string): string {
  return relative === "" ? rootPath : `${rootPath}/${relative}`;
}

/** The core found text it can send (it decides; the window only shows it). */
export function hasText(file: FileView): boolean {
  return file.extraction.usable_text;
}

/** What was read of the file, for Text and Text read (organize-spec.md
 * §6). Text recognized from a scanned PDF's pages or from an image
 * (`fromImage`: the file is a JPG or PNG) says so wherever it shows; the
 * core's note after it says it may contain mistakes (§14). */
export function extractionText(extraction: ExtractionSummary, fromImage = false): string {
  const chars = extraction.total_chars.toLocaleString();
  const excerpt = `only the first 6,000 and last 2,000 of ${chars} characters; the middle was not read`;
  if (extraction.recognized) {
    const read = extraction.truncated ? excerpt : `${chars} characters`;
    return `text recognized from ${fromImage ? "the image" : "scanned pages"}: ${read}`;
  }
  if (extraction.status === "ok" && !extraction.blank) return `whole text (${chars} characters)`;
  if (extraction.status === "partial" && extraction.truncated) return excerpt;
  // The reader left part of the file out (a PDF's images, say); the note
  // says what (#196).
  if (extraction.status === "partial") return `part of the file: ${chars} characters of text`;
  if (extraction.blank) return "blank: no text to read";
  return `${extraction.status}: no text the model may read`;
}

/** Whether the recognition state can still change in this launch: off,
 * failed, timed out and stopped stay so until Traeva starts again; ready
 * can still fall to failed (a recognition past its time or memory limit). */
export function ocrStateMayChange(state: OcrState): boolean {
  return state === "not_prepared" || state === "preparing" || state === "ready";
}

// The one on-device recognizer reads scanned PDFs and images (JPG, PNG).
const OCR = "Text recognition for scanned PDFs and images (JPG, PNG)";

/** Where text recognition for scanned PDFs and images stands, for the
 * settings; null when it is off. */
export function ocrStateText(state: OcrState): string | null {
  switch (state) {
    case "off":
      return null;
    case "not_prepared":
      return `${OCR} has not started getting ready.`;
    case "preparing":
      return `${OCR} is getting ready (the first time can take about a minute).`;
    case "ready":
      return `${OCR} is ready.`;
    case "failed":
      // The preparation failed, or a recognition passed its time or memory
      // limit: off for the rest of this launch either way.
      return `${OCR} stopped working; it is off until Traeva starts again.`;
    case "timed_out":
      return `${OCR} could not get ready; it is off until Traeva starts again.`;
    case "cancelled":
      return `${OCR} was stopped.`;
  }
}

/** Why a manual choice has no recommendation, from the core's view. Null
 * when the user chose to pick the folder themselves. */
export function manualReason(operation: OperationView): string | null {
  if (operation.error !== null) return `No suggestion: ${operation.error.message}`;
  const suggestion = operation.suggestion;
  if (suggestion !== null) {
    return suggestion.candidates.length === 0
      ? "The model did not suggest a folder for this file."
      : "Traeva did not find one folder the text clearly supports, so none is recommended.";
  }
  if (operation.file !== null && !hasText(operation.file)) {
    return "This file has no text the model may read, so the model was not asked.";
  }
  return null;
}

/** How much of the read text a candidate's reason rests on. The pieces
 * themselves are not kept, so only their number is shown. */
export function evidenceText(evidenceIds: string[]): string {
  const count = evidenceIds.length;
  if (count === 0) return "no part of the text";
  return count === 1 ? "1 part of the text" : `${count} parts of the text`;
}

/** A hint for an operation that ended, when the error has a simple cause
 * the window can point to. */
export function endedHint(operation: OperationView): string | null {
  if (operation.error?.code === "outside_root") {
    return (
      "Choose a file inside the folder selected under “File from”. To organize a file " +
      "from another folder, add that folder in Settings first, then select it under “File from”."
    );
  }
  return null;
}

/** The candidate is the folder the file is in now (same root, same path).
 * Only a hint for the card: the core decides with build_plan, which then
 * answers "already there". */
export function isCurrentPlace(
  candidate: CandidateView,
  file: FileView,
  targetRootId: string,
): boolean {
  return (
    file.root_id === targetRootId &&
    candidatePath(candidate, basename(file)) === file.relative_path
  );
}

/** Whether Undo… is offered for an action: only when the core's undo_status
 * says it is available. Organize, its drop queue and Activity all use this;
 * the core checks again when the preview is asked for. */
export function undoOffered(status: UndoStatus): boolean {
  return status === "available";
}

/** Why an action shows no usable Undo button, from the core's undo_status:
 * a short phrase shown next to it and the full reason for its tooltip. Null
 * when Undo is offered or there is nothing to say (#79). */
export function undoNote(status: UndoStatus): { short: string; full: string } | null {
  if (status === "root_revoked") {
    return {
      short: "Folder removed",
      full:
        "Can't be undone: a folder it used was removed from Traeva. Adding the folder again " +
        "does not bring Undo back. The file itself was not changed.",
    };
  }
  return null;
}

/** The authorized folder a plan end lies in: its full path without the path
 * inside it. */
export function rootPathOf(end: PlanEnd): string {
  const suffix = `/${end.relative_path}`;
  return end.display_path.endsWith(suffix)
    ? end.display_path.slice(0, -suffix.length)
    : end.display_path;
}

/** The authorized folders an action belongs to: one, or two when the file
 * went into another folder. `removed` when that folder is no longer among
 * the authorized ones (#80). */
export function actionRoots(
  action: ActionView,
  roots: RootView[],
): { path: string; removed: boolean }[] {
  const active = new Set(roots.map((root) => root.root_id));
  const ends = action.source.root_id === action.target.root_id ? [action.source] : [action.source, action.target];
  return ends.map((end) => ({ path: rootPathOf(end), removed: !active.has(end.root_id) }));
}

/** What the preview says about the user's answers (up to 3, oldest first):
 * only when the plan accepts the model's recommendation made after them.
 * A folder picked by hand ("correct" or "manual"), or a flow without an
 * answer, says nothing (#80). */
export function answerNote(answers: string[], choice: UserChoice | null): string | null {
  if (answers.length === 0 || choice !== "accept") return null;
  const quoted = answers.map((answer) => `“${answer}”`).join(", ");
  return answers.length === 1
    ? `The recommended folder took your answer into account: ${quoted}.`
    : `The recommended folder took your answers into account: ${quoted}.`;
}

/** The folder the file is in now, as a path inside the target folder, or
 * null when the file is in another authorized folder. */
export function currentFolderIn(file: FileView, targetRootId: string): string | null {
  if (file.root_id !== targetRootId) return null;
  const parts = file.relative_path.split("/");
  return parts.slice(0, -1).join("/");
}

/** The new folders exactly as typed, outermost first. Only the empty boxes
 * after the last filled one are left out: a name is never trimmed and a gap
 * is never closed, so the core refuses a bad name instead of getting another
 * one (organize-spec.md §4.2, #93). */
export function typedNewFolders(boxes: string[]): string[] {
  let end = boxes.length;
  while (end > 0 && boxes[end - 1] === "") end -= 1;
  return boxes.slice(0, end);
}

/** Where a folder picked by hand would put the file, written out in full:
 * the chosen folder, the new folders as they are sent, and the file's own
 * name. */
export function chosenTargetPath(
  rootPath: string,
  folderPath: string,
  newFolders: string[],
  name: string,
): string {
  const folder = folderPath === "" ? [] : [folderPath];
  return inRoot(rootPath, [...folder, ...newFolders, name].join("/"));
}
