// What Memory's page says about recordings, candidate capture, gaps and the
// decisions on waiting candidates (#31 material §3, §5, §6 and the
// supplement), kept apart from React so it can be tested (records.test.ts).
// Nothing here decides a relation: which choices are allowed, which parent
// a continuation takes and whether a page is still current all come from
// the core's views; this only words them and builds the request the user
// made.

import type {
  AutoMemoryView,
  AssociateChoiceKind,
  AssociationPreview,
  CandidateCapture,
  CandidateCaptureInfo,
  CandidateChoice,
  CandidateDecisionRequest,
  CurrentObjectChoice,
  FamilyMemberView,
  MemoryRecordView,
  PendingCandidate,
  PendingCandidatesView,
  PendingCounts,
  RecordedGapView,
  MergePreview,
  MovePreview,
  RecordHoldView,
  RelationAction,
  RelationPreview,
  RelocationPreview,
  SameContentHint,
  SnapshotRef,
  VersionRelationView,
  WorkDecisionOutcome,
  WorkSummaryView,
} from "../family/types";
import type { ErrorCode } from "../organize/types";
import type { Tone } from "./history";

export function recordStateText(record: MemoryRecordView): { label: string; detail: string; tone: Tone } {
  if (!record.root_active) {
    return {
      label: "The folder is no longer authorized",
      detail: "Nothing new is saved. The versions saved so far are kept.",
      tone: "warn",
    };
  }
  switch (record.state) {
    case "active":
      if (record.waiting_for_relocation) {
        return {
          label: "File not found",
          detail: `The file is not at its place, so nothing new is saved until it is back or you choose where it is now. ${versionsText(record.line_versions)} kept.`,
          tone: "warn",
        };
      }
      return {
        label: "Recording",
        detail: `${versionsText(record.line_versions)} saved. New saves of the file are kept automatically.`,
        tone: "ok",
      };
    case "lost":
      return {
        label: "Lost",
        detail: record.waiting_for_relocation
          ? "Traeva no longer finds this work's file at its place: it is gone, or you said the file there now is unrelated. Its history is kept. If you moved the file, choose it where it is now."
          : "Traeva no longer finds this work's file at its place. Its history is kept.",
        tone: "warn",
      };
    case "stopped":
      if (record.hold) {
        return {
          label: "Paused",
          detail: `${holdCause(record.hold)} ${versionsText(record.line_versions)} kept. Nothing new is saved until you record it again or stop it.`,
          tone: "warn",
        };
      }
      return {
        label: "Stopped",
        detail: `${versionsText(record.line_versions)} kept. Nothing new is saved.`,
        tone: "wait",
      };
  }
}

function versionsText(count: number): string {
  return count === 1 ? "1 version" : `${count} versions`;
}

/** Said when a record asked for from outside the page (Find's Versions
 * button, #198) is not among Memory's records any more. */
export const RECORD_GONE_TEXT = "This record is no longer available.";

/** The record to show for a request from outside the page, from the list
 * read after the request; null when it is not there. */
export function recordToOpen(records: MemoryRecordView[], chainId: string): MemoryRecordView | null {
  return records.find((record) => record.chain_id === chainId) ?? null;
}

/** What the record list shows that the candidate cards also depend on: when
 * it changes (a new version, a waiting file, the switch, a gap, the place),
 * the cards read their own views again. Unchanged, they keep the user's
 * unsaved choices. */
export function recordSignature(record: MemoryRecordView): string {
  return [
    record.state,
    record.root_active,
    record.watch.watch_id,
    record.watch.versions,
    record.line_versions,
    record.candidate_capture,
    record.pending.objects,
    record.pending.snapshots,
    record.waiting_for_relocation,
    record.open_gaps.map((gap) => gap.gap_id).join(","),
    record.hold?.work_decision_id ?? "",
    // An Organize move keeps the watch: the capture notice names the place.
    record.place.root_id,
    record.place.relative_path,
    record.place.display_path,
  ].join("|");
}

export function isAtLimit(pending: PendingCounts): boolean {
  return pending.objects >= pending.limit || pending.snapshots >= pending.limit;
}

/** Choosing the file's new place (#31 M2): offered while the record is not
 * stopped and waits for it, and also while another file stands at its place
 * unread (Traeva could not follow it, and candidate capture is off): the
 * user's file may have moved (material §4 supplement). */
export function canRelocate(record: MemoryRecordView): boolean {
  return (
    record.state !== "stopped" &&
    record.root_active &&
    (record.waiting_for_relocation || placeTakenUnread(record))
  );
}

/** Another file replaced this one that Traeva did not read (it could not
 * follow it by itself, and candidate capture was off), and nothing was
 * decided about it yet. */
export function placeTakenUnread(record: MemoryRecordView): boolean {
  return record.open_gaps.some((gap) => gap.kind === "replaced_uncaptured");
}

/** What going on at the chosen file does, said before it is confirmed (#31
 * material §4 supplement). */
export function relocationPoints(preview: RelocationPreview): string[] {
  if (preview.old_place_holds === "recorded_file") {
    return [
      `The file is back at its place, ${preview.new_place.display_path}. Recording goes on there.`,
      "Traeva did not see the file while it was away, so the history shows a gap there.",
    ];
  }
  const points = [
    preview.old_place_holds === "empty"
      ? `Nothing is at the old place, ${preview.old_place.display_path}, now.`
      : `The file at the old place now, ${preview.old_place.display_path}, does not join the work.`,
  ];
  if (preview.waiting_candidates > 0) {
    const files = preview.waiting_candidates === 1 ? "1 file" : `${preview.waiting_candidates} files`;
    points.push(`${files} waiting for your decision stay waiting; they do not move with it.`);
  }
  points.push(
    preview.candidate_capture === "off"
      ? "From now on Memory looks at the new place only."
      : "Candidate capture goes along: from now on it looks at the new place only.",
  );
  points.push("Traeva did not see the move, so the history shows a gap between the two places.");
  return points;
}

/** The refusals of a relocation that the user can act on, in their words.
 * Nothing was changed in each case. */
export function relocationRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "not_same_object":
      return "This is not the recorded file: it is a copy, a file on another disk, a hard link, another file, a folder or a link. Nothing was changed; the record keeps waiting.";
    case "page_outdated":
      return "Something changed since this was shown. Nothing was changed; choose the file again.";
    case "unresolved_action":
      return "An Organize action in this folder is not settled yet. Nothing was changed; settle it first.";
    default:
      return null;
  }
}

/** How Traeva treats a file an app saved by replacing it (said with the
 * candidate capture switch). */
export const FOLLOWS_TEXT =
  "While this recording is on, if an app saves by replacing the file at this place, Traeva follows the new file by itself. It cannot when, for example, the new file is too large or a hard link, or another file at this place still waits for your decision.";

/** Candidate capture: off by default, on only when the user turned it on.
 * Either way Traeva first follows a file an app saved by replacing it at
 * this place (the student's 10/14 decision). */
export function captureText(capture: CandidateCapture, pending: PendingCounts): string {
  switch (capture) {
    case "off":
      return `Off. ${FOLLOWS_TEXT} When it cannot, it only reports it.`;
    case "on":
      return `On. ${FOLLOWS_TEXT} When it cannot, the new file is saved as a candidate for you to review.`;
    case "paused":
      return isAtLimit(pending)
        ? "Waiting records reached the limit, so candidate capture is paused. Review them first."
        : "Paused. Nothing is read or saved at this place until it resumes.";
  }
}

export function pendingText(pending: PendingCounts): string {
  const files = pending.objects === 1 ? "1 file" : `${pending.objects} files`;
  const copies = pending.snapshots === 1 ? "1 saved copy" : `${pending.snapshots} saved copies`;
  return `${files} and ${copies} wait for your decision (at most ${pending.limit} of each).`;
}

/** Whether a record waits for the user (#248): files waiting for a
 * decision, a record paused by a correction (HoldCard), or a file that
 * replaced the recorded one at its place and was not read because Traeva
 * could not follow it and capture is off (an open `replaced_uncaptured`
 * gap: the page asks whether it continues the work). */
export function needsDecision(record: MemoryRecordView): boolean {
  return (
    record.pending.objects > 0 ||
    record.hold !== null ||
    record.open_gaps.some((gap) => gap.kind === "replaced_uncaptured" && gap.ended_at_ms === null)
  );
}

/** Where a record shows in the library (#37, the student's layout B,
 * 10/14): it waits for the user (files to decide about, a paused record, a
 * file to find again, a folder no longer authorized: everything its line
 * says in amber), Memory records it, or it is stopped. */
export type LibraryGroup = "needs" | "recording" | "stopped";

export function libraryGroup(record: MemoryRecordView): LibraryGroup {
  if (needsDecision(record) || recordStateText(record).tone === "warn") return "needs";
  return record.state === "active" ? "recording" : "stopped";
}

/** How many things wait for the user at the top of a file's page: a
 * paused record, a file to find again, and the files to decide about (or
 * the file at the place), one each. */
export function needsCount(record: MemoryRecordView): number {
  const deciding =
    record.pending.objects > 0 ||
    record.open_gaps.some((gap) => gap.kind === "replaced_uncaptured" && gap.ended_at_ms === null);
  return (record.hold !== null ? 1 : 0) + (canRelocate(record) ? 1 : 0) + (deciding ? 1 : 0);
}

/** Said in the library under a record that waits for the user. */
export function waitsText(record: MemoryRecordView): string {
  return record.pending.objects > 0 ? pendingText(record.pending) : "Waits for your decision: open the file to see it.";
}

/** The line under Memory's title (#248): how many files Memory keeps and
 * how many wait for a decision. (A sum of the versions would count twice
 * a line another record went on as, so it is not said.) */
export function libraryText(records: MemoryRecordView[]): string {
  if (records.length === 0) return "No file is recorded yet.";
  const parts = [records.length === 1 ? "1 file on this Mac" : `${records.length} files on this Mac`];
  const waiting = records.filter(needsDecision).length;
  if (waiting > 0) parts.push(waiting === 1 ? "1 needs your decision" : `${waiting} need your decision`);
  return parts.join(" · ");
}

/** The tabs of a file's page in Memory (#248). What waits for a decision
 * is shown above them, so no tab hides it. */
export type RecordTab = "versions" | "work" | "waiting";

export function recordTabText(tab: RecordTab): string {
  switch (tab) {
    case "versions":
      return "Versions";
    case "work":
      return "This work";
    case "waiting":
      return "Waiting files";
  }
}

/** Whether a tab gets the small dot that says something there is for the
 * user: This work when Memory saw the same content in another work (the
 * work card suggests adding or merging). */
export function tabHasNews(tab: RecordTab, record: MemoryRecordView): boolean {
  return tab === "work" && record.same_content.length > 0;
}

/** What Traeva did not see, never what happened then. */
export function gapText(gap: RecordedGapView): string {
  switch (gap.kind) {
    case "memory_off":
      return "Memory was off for this file. Changes in that time were not recorded.";
    case "paused":
      return "Candidate capture was paused. Replacements in that time were not saved.";
    case "capture_failed":
      return `A version could not be saved${gap.problem ? ` (${gap.problem})` : ""}. Changes in that time may be missing.`;
    case "replaced_uncaptured":
      return "The file was replaced by one Traeva did not read. That version was not saved.";
    case "lost":
      return "The file was not at its place. Nothing was recorded in that time.";
    case "watcher_stopped":
      return "Traeva was not running. Changes in that time may not be recorded.";
    case "not_recorded":
      return "From before Traeva recorded when it was running. Changes in that time may not be recorded.";
    case "skipped":
      return "Files that are not part of this line stand between these versions.";
  }
}

/** The notice shown before candidate capture is turned on (#31 material
 * §3, and D2 when the object at the place now is saved at once). Changing
 * these words means raising the core's CANDIDATE_NOTICE_VERSION. */
export function candidateNotice(info: CandidateCaptureInfo): string[] {
  const points = [
    `Only this exact place: ${info.place.display_path}. Not the rest of its folder, and not an editor's temporary files.`,
    "A new file at this place may be your editor's save, or an unrelated file put there.",
    "The saved copies are kept in Traeva's data folder on this Mac. During CAC they cannot be deleted.",
    "Saving a copy does not mean Traeva has decided it belongs to this work. You decide that afterwards.",
  ];
  if (info.saves_occupant_now) {
    points.push(
      "This includes the file that has already replaced it at this place: it is saved as soon as you turn this on.",
    );
  }
  return points;
}

export const REASON_TEXT: Record<PendingCandidate["reason"], string> = {
  replaced: "Found at this place after the file was replaced. There is no evidence yet that it continues the work.",
  present_when_enabled: "Was at this place when candidate capture was turned on.",
  read_on_decision: "Read when you made a decision about the file at this place.",
};

export type ChoiceKind = CandidateChoice["kind"];

export const CHOICE_TEXT: Record<ChoiceKind, string> = {
  continue: "Continues the work",
  branch: "Another branch of the work",
  unknown: "Same work, order not known",
  independent: "Unrelated file",
  defer: "Not sure yet",
};

/** The user's choice for the candidates they decided on, by candidate ID.
 * A candidate without an entry is left as it is: it is not sent at all.
 * "Not sure yet" is a decision of its own (`defer`): it is recorded, so it
 * is sent only when the user picks it. */
export type DecisionDraft = Record<string, CandidateChoice>;

/** Nothing is decided for the user. */
export function emptyDraft(): DecisionDraft {
  return {};
}

/** The draft without a choice for `candidateId`: left as it is. */
export function leaveAsItIs(draft: DecisionDraft, candidateId: string): DecisionDraft {
  const next = { ...draft };
  delete next[candidateId];
  return next;
}

/** Something to send: a choice for a listed candidate or the file at the
 * place now. */
export function hasDecisions(draft: DecisionDraft, currentObject: CurrentObjectChoice | null): boolean {
  return Object.keys(draft).length > 0 || currentObject !== null;
}

/** "These continue the work, in this order": every candidate that may
 * continue gets "continue" from the parent the core named; the others keep
 * their choice. */
export function continueAllInOrder(view: PendingCandidatesView, draft: DecisionDraft): DecisionDraft {
  const next = { ...draft };
  for (const item of view.items) {
    if (item.continue_from) {
      next[item.candidate_id] = { kind: "continue", parent_snapshot_id: item.continue_from.snapshot_id };
    }
  }
  return next;
}

/** The choice for "continue" on this item, or null when it cannot continue. */
export function continueChoice(item: PendingCandidate): CandidateChoice | null {
  return item.continue_from ? { kind: "continue", parent_snapshot_id: item.continue_from.snapshot_id } : null;
}

/** A branch starts from a version the core lists (`fork_points`) that was
 * saved before the waiting file; the core checks it again by save order. */
export function branchParents(forkPoints: SnapshotRef[], item: PendingCandidate): SnapshotRef[] {
  const first = item.snapshots[0];
  return first ? forkPoints.filter((point) => point.captured_at_ms <= first.captured_at_ms) : [];
}

/** What the user may say about the file at the place that is not a
 * waiting candidate (#31 material §2, §3). Traeva has not read it, so it
 * cannot keep it as a branch or of unknown order: "continue" reads it,
 * "independent" reads nothing. A file decided earlier, or one another
 * recording keeps, can only be called unrelated. */
export function currentObjectKinds(view: PendingCandidatesView): CurrentObjectChoice["kind"][] {
  switch (view.place_holds) {
    case "unread_object":
      return view.unread_occupant ? ["continue", "independent"] : [];
    case "decided_candidate":
    case "other_record":
      return ["independent"];
    default:
      return [];
  }
}

export const CURRENT_OBJECT_TEXT: Record<CurrentObjectChoice["kind"], string> = {
  continue: "Continues the work",
  independent: "Unrelated file",
};

/** What each choice for the file at the place does, said before it is saved. */
export function currentObjectNote(kind: CurrentObjectChoice["kind"]): string {
  return kind === "continue"
    ? "Traeva reads it now and goes on recording from it."
    : "Traeva does not read it. This recording then counts as lost and stops looking at this place; its history is kept. To record that file, turn Memory on for it on its own. A lost recording cannot be linked to a new place until a later version; until then it can only be stopped.";
}

export const BRANCH_HINT =
  "To call it another branch, or the same work in an unknown order, turn on candidate capture first (that saves it now), or turn Memory on for that file on its own.";

/** After "continue" was saved but recording stayed where it was. Not "Memory
 * is off": it also happens when the files continued are older ones that are
 * not at the place now. */
export function savedText(historyOnly: boolean): string {
  return historyOnly
    ? "Saved. The history now shows these relations. Recording stays where it was: no file you continued is at the file's place now, or this recording is stopped, lost or its folder is no longer authorized."
    : "Saved.";
}

/** Waiting files are told apart by their place in this list: "waiting
 * file 2". Several can stand at the same path, and the core's `seq` has
 * holes once some are decided, so it is used only to match them. */
export function waitingLabel(item: PendingCandidate, items: PendingCandidate[]): string {
  const at = items.findIndex((other) => other.candidate_id === item.candidate_id);
  return `waiting file ${at + 1}`;
}

/** A saved copy, told apart by when it was saved: a candidate's copies are
 * numbered within their own file, not within the work, so a version number
 * would mislead. */
export function snapshotPhrase(snapshot: SnapshotRef, items: PendingCandidate[]): string {
  const when = new Date(snapshot.captured_at_ms).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  const owner = items.find((item) => item.file_id === snapshot.file_id);
  return owner ? `the copy of ${waitingLabel(owner, items)} saved ${when}` : `the version saved ${when}`;
}

/** A relation that skips excluded files is new: the page shows it and the
 * user confirms it as such (#31 supplement, O → N1 → N2 → N3). */
export function skipNote(item: { continue_skips: number }, parent: string | null): string | null {
  if (parent === null || item.continue_skips === 0) return null;
  const files = item.continue_skips === 1 ? "1 file" : `${item.continue_skips} files`;
  return `This links it to ${parent}, past ${files} you marked unrelated. That is a new relation; the difference is not one save.`;
}

/** What stops the request from being sent, in the user's words. The core
 * checks everything again; this only avoids sending what it would refuse. */
export function decisionProblems(view: PendingCandidatesView, draft: DecisionDraft): string[] {
  const problems: string[] = [];
  for (const item of view.items) {
    const choice = draft[item.candidate_id];
    if (choice?.kind !== "continue") continue;
    const label = capitalize(waitingLabel(item, view.items));
    if (!item.continue_from) {
      problems.push(`${label} cannot be marked as continuing the work.`);
    } else if (choice.parent_snapshot_id !== item.continue_from.snapshot_id) {
      problems.push(`${label} can only continue from ${snapshotPhrase(item.continue_from, view.items)}.`);
    }
    if (item.continue_needs && draft[item.continue_needs]?.kind !== "continue") {
      const before = view.items.find((other) => other.candidate_id === item.continue_needs);
      problems.push(
        `To continue ${waitingLabel(item, view.items)}, also mark ${before ? waitingLabel(before, view.items) : "the file before it"} as continuing.`,
      );
    }
  }
  return problems;
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function decisionRequest(
  view: PendingCandidatesView,
  draft: DecisionDraft,
  currentObject: CurrentObjectChoice | null,
): CandidateDecisionRequest {
  return {
    chain_id: view.chain_id,
    page_token: view.page_token,
    // Only what the user decided, in the page's order; the rest is left
    // as it is.
    items: view.items.flatMap((item) => {
      const choice = draft[item.candidate_id];
      return choice ? [{ candidate_id: item.candidate_id, choice }] : [];
    }),
    current_object: currentObject,
  };
}

// --- Works: merging two, and the same-content hint (#31 M2b-1) ---

/** A work to choose, by the files it records. */
export function workText(work: WorkSummaryView): string {
  const files = work.display_paths.length === 1 ? "1 file" : `${work.display_paths.length} files`;
  const shown = work.display_paths.slice(0, 3).join(", ");
  const more = work.display_paths.length > 3 ? ", …" : "";
  return `${files}: ${shown}${more}`;
}

/** What a merge does and does not do, said before it is confirmed (#31
 * material §6), and the earlier "unrelated" judgements it ends (the
 * student's decision 3). */
export function mergePoints(preview: MergePreview): string[] {
  const points = [
    "No file's record is merged into another: each file keeps its own history.",
    "Traeva does not work out which side's versions came first.",
    "Each recording goes on as it is: its switch, its limit and its waiting files do not change.",
  ];
  for (const override of preview.overrides) {
    points.push(
      `You said earlier that ${override.display_path} is unrelated. After the merge that judgement no longer counts.`,
    );
  }
  return points;
}

/** The files of a work, for the preview. */
export function workFiles(family: MergePreview["stays"]): string[] {
  return family.members.map((member) => member.display_path);
}

/** A merge refused in a way the user can act on; others are shown as the
 * core words them. */
export function mergeRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "page_outdated":
      return "The works changed since this was shown, or it was already confirmed. Nothing was merged. Preview again.";
    case "decision_conflict":
      return "These are already one work. Nothing was merged.";
    default:
      return null;
  }
}

/** The same-content hint: only "may be related", never a relation. */
export function sameContentText(hint: SameContentHint): string {
  const when = (version: SnapshotRef) =>
    new Date(version.captured_at_ms).toLocaleString(undefined, {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  const other = hint.same_bytes_as.relative_path.split("/").pop() ?? hint.same_bytes_as.relative_path;
  return `This file's first version (saved ${when(hint.own_version)}) has exactly the bytes of a version of ${other} in another work (saved ${when(hint.same_bytes_as)}). They may be related; Traeva links nothing by itself.`;
}

// --- Adding a file to a work (#31 M2b-2, contract §4.3a "关联到作品") ---

/** A version of any file, by its file and when it was saved. */
export function savedVersionText(version: SnapshotRef): string {
  const file = version.relative_path.split("/").pop() ?? version.relative_path;
  const when = new Date(version.captured_at_ms).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  return `${file}, saved ${when}`;
}

/** Where the chosen file stands, which decides the choices. */
export function standingText(preview: AssociationPreview): string {
  switch (preview.standing) {
    case "not_recorded":
      return "Traeva has no saved version of this file. You can say it belongs to this work (its place unknown) or that it is unrelated. To give it a place, turn Memory on for it and add it again.";
    case "saved_no_work":
      return "This file has saved versions and belongs to no work yet.";
    case "in_other_work":
      return "This file belongs to another work. Saying it continues, branches or is the same work merges that whole work into this one; saying it is unrelated only notes that here.";
    case "member_without_parent":
      return "This file is in this work, but where it starts is not known. Choose the version it continues or branches from.";
  }
}

/** What Traeva knows, shown as it is: never a relation by itself. */
export function evidenceText(preview: AssociationPreview): string[] {
  const lines = preview.evidence.same_bytes_as.map(
    (version) => `Its first version has exactly the bytes of ${savedVersionText(version)}.`,
  );
  if (preview.evidence.same_object_as.length > 0) {
    lines.push("It is the very same file on disk as a file of this work.");
  }
  if (lines.length === 0) {
    lines.push("Traeva knows nothing that links them: only your choice does.");
  }
  lines.push("Traeva does not decide anything from this; you do.");
  return lines;
}

export const ASSOCIATE_CHOICE_TEXT: Record<AssociateChoiceKind, string> = {
  continue: "It continues the work",
  branch: "It is another branch of the work",
  unknown: "Same work, place not known",
  independent: "It is unrelated",
};

/** What a choice does, said before it is saved. */
export function associationNotes(preview: AssociationPreview, choice: AssociateChoiceKind): string[] {
  const notes: string[] = [];
  if (preview.standing === "in_other_work" && choice !== "independent" && preview.merge_scope) {
    const files = preview.merge_scope.members.map((member) => member.display_path).join(", ");
    notes.push(`The other work is merged into this one: ${files}. Each file keeps its own history.`);
  }
  if (choice === "continue" || choice === "branch") {
    notes.push("Traeva saw nothing between the two versions, so the history shows a gap there.");
    if (preview.replaces_unknown_placement) {
      notes.push("Its earlier \"place not known\" is replaced by this.");
    }
  }
  if (choice === "continue") {
    notes.push("No recording starts or moves: a file not recorded yet needs Memory turned on for it.");
  }
  if (choice !== "independent") {
    for (const override of preview.overrides) {
      notes.push(`You said earlier that ${override.display_path} is unrelated. That judgement no longer counts.`);
    }
  }
  return notes;
}

/** An association refused in a way the user can act on; others are shown as
 * the core words them. Nothing was saved in each case. */
export function associationRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "candidate_pending":
      return "This file waits for a decision in its recording: decide it there. Nothing was saved.";
    case "page_outdated":
      return "The work or the file changed since this was shown, or it was already saved. Nothing was saved. Look again.";
    default:
      return null;
  }
}

// --- Correcting one relation, and a paused recording (#31 M2b-3) ---

/** Why a recording is paused: the relation it went on by no longer counts. */
export function holdCause(hold: RecordHoldView): string {
  switch (hold.why) {
    case "relation_revoked":
      return "You revoked the relation this recording went on by.";
    case "moved_apart":
      return "You moved files out of the work, so the relation this recording went on by stopped counting.";
  }
}

/** Why a recording is paused, and since when. */
export function holdText(hold: RecordHoldView): string {
  const since = new Date(hold.held_at_ms).toLocaleString();
  return `Paused since ${since}. ${holdCause(hold)} Nothing new is saved until you decide.`;
}

export const HOLD_CHOICE_TEXT = {
  again:
    "Record again: the same recording goes on at the file's place. The time it was paused shows as a gap, and candidate capture stays off.",
  stop: "Stop: the recording ends. Its history is kept.",
};

/** The relations a user can correct here: a confirmed or disputed
 * continuation or branch between two versions. */
export function correctableRelations(relations: VersionRelationView[]): VersionRelationView[] {
  return relations.filter(
    (relation) =>
      relation.parent !== null &&
      relation.kind !== "unplaced" &&
      (relation.status === "confirmed_system" ||
        relation.status === "confirmed_user" ||
        relation.status === "disputed"),
  );
}

/** "notes.txt, saved 9 Oct, 22:03 continues todo.txt, saved 9 Oct, 21:50". */
export function relationText(relation: VersionRelationView): string {
  const child = savedVersionText(relation.child);
  const parent = relation.parent ? savedVersionText(relation.parent) : "an unknown version";
  return relation.kind === "branch" ? `${child} branches from ${parent}` : `${child} continues ${parent}`;
}

/** Said for a relation Traeva made by itself (`linked_automatically`). */
export const LINKED_AUTOMATICALLY_TEXT = "Linked automatically: a new file at the same place, saved by your app.";

/** Said under a version Traeva linked by itself, with "Undo this link". */
export const AUTO_LINK_PAIR_TEXT =
  "Linked automatically: your app saved this by replacing the file at the same place.";

export function relationStatusText(relation: VersionRelationView): string {
  switch (relation.status) {
    case "confirmed_user":
      return "Confirmed by you.";
    case "confirmed_system":
      return relation.linked_automatically
        ? LINKED_AUTOMATICALLY_TEXT
        : "Confirmed by Traeva: it is the very same file on disk.";
    case "disputed":
      return relation.crosses_works
        ? "Crosses two works, waiting to be checked again: a file was moved out. It is not used until you confirm it again (once both files are in one work) or revoke it."
        : "Disputed: it is not used until you confirm it again or revoke it.";
    default:
      return "";
  }
}

/** What revoking or confirming again does, said before it is saved. */
export function relationNotes(preview: RelationPreview): string[] {
  const notes: string[] = [];
  if (preview.relation.status === "confirmed_system" && preview.relation.linked_automatically) {
    notes.push(`${LINKED_AUTOMATICALLY_TEXT} Undoing this link revokes it; the history keeps that Traeva had linked it.`);
  } else if (preview.relation.status === "confirmed_system") {
    notes.push(
      "Traeva confirmed this from the very same file on disk. You can still revoke it; the history keeps that Traeva had confirmed it.",
    );
  }
  if (preview.can_revoke) {
    notes.push(
      `Revoking it: ${savedVersionText(preview.leaves_unplaced)} stays in this work, its place not known. Nothing joins it again by itself; to place it again, add its file to this work.`,
    );
    notes.push("Changes described across this relation, and search answers that rest on it, are then no longer certain.");
    for (const record of preview.records_held) {
      notes.push(
        `Recording ${record.place.display_path} is paused: it went on by this relation. You then record it again or stop it.`,
      );
    }
  }
  if (preview.can_confirm_again) {
    notes.push("Confirming it again: it counts as confirmed by you, even if Traeva had confirmed it before.");
  } else if (preview.relation.crosses_works) {
    notes.push(
      "Its two files are in two works now. To confirm it again, first bring them into one work: merge the works, or move the file back.",
    );
  }
  return notes;
}

/** What the page says after the correction is saved. */
export function correctedText(action: RelationAction, outcome: WorkDecisionOutcome): string {
  if (action === "confirm_again") return "Confirmed again. The relation counts as confirmed by you.";
  const held = outcome.held_records.length;
  return held === 0
    ? "Revoked. The version stays in this work, its place not known."
    : `Revoked. ${held === 1 ? "1 recording is" : `${held} recordings are`} paused: record again or stop.`;
}

export function relationRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "page_outdated":
      return "The relation or a recording changed since this was shown, or it was already saved. Nothing was changed. Look again.";
    case "decision_conflict":
      return "This relation cannot be changed this way now: it is no longer confirmed or disputed, or its files are not in one work. Nothing was changed.";
    case "unknown_relation":
      return "There is no such relation now. Nothing was changed.";
    default:
      return null;
  }
}

// --- Moving files out of a work (#31 M2b-3b) ---

/** The files a user can move: confirmed members. At least one stays. */
export function movableMembers(members: FamilyMemberView[]): FamilyMemberView[] {
  return members.filter((member) => member.status === "confirmed_user" || member.status === "confirmed_system");
}

/** How the move page tells members apart (R2-MEM-09): an editor that saves
 * by writing a new file leaves several members at one path. Said with what
 * Traeva knows of each: whether this recording looks at it now, whether it
 * is still at its place, and when its versions were saved. */
export function memberLabel(member: FamilyMemberView, record: MemoryRecordView): string {
  return `${member.display_path} (${memberNotes(member, record)})`;
}

/** What memberLabel says about a member after its path. */
export function memberNotes(member: FamilyMemberView, record: MemoryRecordView): string {
  const notes: string[] = [];
  if (member.file_id === record.watch.file_id) {
    notes.push(record.state === "active" ? "recorded now" : "the file this recording looked at last");
  }
  if (!member.present) notes.push("no longer at its place");
  const times = member.snapshots.map((snapshot) => snapshot.captured_at_ms).sort((a, b) => a - b);
  const at = (ms: number) =>
    new Date(ms).toLocaleString(undefined, {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  if (times.length === 0) notes.push("no version saved");
  else if (times.length === 1) notes.push(`1 version, saved ${at(times[0])}`);
  else notes.push(`${times.length} versions, saved ${at(times[0])} to ${at(times[times.length - 1])}`);
  return notes.join("; ");
}

/** A place as a list row shows it (the student, 10/13): the file's name
 * first, then the folder it is in, smaller. For a file at the top of an
 * authorized folder, the folder is that authorized folder. A path without a
 * folder has an empty one. */
export function namedPlace(displayPath: string): { name: string; folder: string } {
  const cut = displayPath.lastIndexOf("/");
  const name = displayPath.slice(cut + 1);
  if (cut < 0 || name === "") return { name: displayPath, folder: "" };
  return { name, folder: cut === 0 ? "/" : displayPath.slice(0, cut) };
}

/** What a move does, said before it is confirmed (#31 material §7).
 * `target`: how the page names where the files go; `label`: how it names a
 * member (memberLabel). */
export function moveNotes(
  preview: MovePreview,
  target: string,
  label: (member: FamilyMemberView) => string = (member) => member.display_path,
): string[] {
  const files = preview.moving.map(label).join(", ");
  const notes = [`${files} leave this work and go to ${target}. From then on they belong there by your decision.`];
  for (const record of preview.records_moving) {
    notes.push(
      `Recording ${record.place.display_path} goes along: its switch, its limit, its waiting files and its gaps go with it.`,
    );
  }
  if (preview.relations_kept.length > 0) {
    const count = preview.relations_kept.length;
    notes.push(`${count === 1 ? "1 relation" : `${count} relations`} between the moved files go along and still count.`);
  }
  for (const relation of preview.relations_disputed) {
    notes.push(
      `${relationText(relation)}: it then joins two works and stops counting until you confirm it again in one work or revoke it. It is not revoked by itself.`,
    );
  }
  for (const record of preview.records_held) {
    notes.push(
      `Recording ${record.place.display_path} is paused: the relation it went on by then joins two works. You then record it again or stop it.`,
    );
  }
  for (const override of preview.overrides) {
    notes.push(`You said earlier that ${override.display_path} is unrelated to that work. That judgement no longer counts.`);
  }
  notes.push("No saved version is changed or deleted.");
  return notes;
}

/** What the page says after the move is saved. */
export function movedText(outcome: WorkDecisionOutcome, files: number, target: string): string {
  const moved = `Moved. ${files === 1 ? "1 file went" : `${files} files went`} to ${target}.`;
  const held = outcome.held_records.length;
  return held === 0
    ? moved
    : `${moved} ${held === 1 ? "1 recording was" : `${held} recordings were`} paused: record again or stop.`;
}

export function moveRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "page_outdated":
      return "The work or a recording changed since this was shown, or it was already saved. Nothing was moved. Preview again.";
    case "decision_conflict":
      return "These files cannot be moved this way: each must be a confirmed member, the target must be another work, and at least one file must stay. Nothing was moved.";
    case "unknown_family":
      return "That work is no longer there. Nothing was moved.";
    case "candidate_pending":
      return "A recording of these files waits for a decision: decide it there first. Nothing was moved.";
    default:
      return null;
  }
}

// --- Keeping versions of files Traeva organizes (#201) ---

/** The switch's name, as the student chose it (#201). */
export const AUTO_MEMORY_TITLE = "Keep versions of files Traeva organizes";

/** The label on a record the switch started. */
export const STARTED_BY_TRAEVA = "Started by Traeva";

/** How the switch stands: on; no model chosen yet; an agreement given for
 * another model or notice (outdated); or off. */
export type AutoMemoryState = "on" | "no_model" | "outdated" | "off";

export function autoMemoryState(view: AutoMemoryView): AutoMemoryState {
  if (view.enabled) return "on";
  if (view.provider === null || view.model === null) return "no_model";
  return view.outdated ? "outdated" : "off";
}

/** What the card says about the switch. `since`: when the user agreed, as
 * the page shows dates. */
export function autoMemoryText(view: AutoMemoryView, since: string | null): string {
  switch (autoMemoryState(view)) {
    case "on":
      return `On since ${since ?? "earlier"}. Files Traeva organizes start keeping their versions, and their changes are described by ${view.model}.`;
    case "no_model":
      return "Choose a model in Settings first. Then you can turn this on here.";
    case "outdated":
      return "Off: the model or this notice changed since you agreed. Read it again and turn it on again if you still want it.";
    case "off":
      return "Off. Read what it does before you turn it on:";
  }
}

/** What turning the switch off does (#201). */
export const AUTO_MEMORY_OFF_TEXT =
  "Turning it off starts no new records and withdraws the agreements it gave, so nothing more is sent for those files. The versions already kept stay; you can stop each record below.";

/** Said while the switch is on and as many records are on as it allows. */
export function autoMemoryLimitText(view: AutoMemoryView): string | null {
  return view.enabled && view.recording >= view.limit
    ? `Traeva starts no new records while ${view.limit} files are being recorded.`
    : null;
}

/** Whether the switch started this record. */
export function startedByTraeva(view: AutoMemoryView | null, chainId: string): boolean {
  return view?.started_chain_ids.includes(chainId) ?? false;
}

/** The refusals of turning the switch on that the user can act on. */
export function autoMemoryRefusal(code: ErrorCode): string | null {
  switch (code) {
    case "page_outdated":
      return "The notice changed. Read it again before turning this on.";
    case "model_not_configured":
      return "Choose a model in Settings first.";
    default:
      return null;
  }
}
