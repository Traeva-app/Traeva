import type { Tone } from "../../shared/status.ts";
import type { SourceRecordView, SourceState, SourceStatus } from "./types.ts";

export function statusText(status: SourceStatus): string {
  return {
    unknown: "Unknown",
    likely: "Candidate · needs your confirmation",
    user_confirmed: "User Confirmed",
  }[status];
}

export function timeText(time: number | null, missing: string): string {
  if (time === null) return missing;
  const date = new Date(time);
  return Number.isNaN(date.getTime()) ? missing : date.toLocaleString();
}

export function isWebLink(value: string | null): boolean {
  if (!value) return false;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}

export function recordName(record: SourceRecordView): string {
  return record.filename.split(/[\\/]/).filter(Boolean).at(-1) || "Download name unknown";
}

export function sourcesForFile(records: SourceRecordView[], fileId: string): SourceRecordView[] {
  return records.filter((record) => record.association?.file_id === fileId ||
    record.candidates.some((candidate) => candidate.file_id === fileId));
}

export function replaceRecord(state: SourceState | null, record: SourceRecordView): SourceState | null {
  if (!state) return state;
  const current = state.records.find((item) => item.source_id === record.source_id);
  return {
    ...state,
    records: current
      ? state.records.map((item) => item.source_id === record.source_id ? record : item)
      : [record, ...state.records],
  };
}

export function sourceError(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") {
    return error.message;
  }
  return typeof error === "string" ? error : "The request did not finish. Try again.";
}

/** The Sources page's groups (#37, the student's choice B, 10/14). */
export type SourceGroup = "needs" | "progress" | "finished";

/** Where a download shows: a candidate waits for the user's confirmation;
 * a download still running, or one Traeva has not matched to a file yet
 * (it keeps looking, and the user may locate the file), is in progress; a
 * confirmed source, or an interrupted download, is finished. */
export function sourceGroup(record: SourceRecordView): SourceGroup {
  if (record.status === "user_confirmed") return "finished";
  if (record.status === "likely") return "needs";
  if (record.download_state === "interrupted") return "finished";
  return "progress";
}

/** A download's badge in the list: amber only while it waits for the user. */
export function sourceBadge(record: SourceRecordView): { label: string; tone: Tone } {
  if (record.status === "user_confirmed") return { label: "User Confirmed", tone: "ok" };
  if (record.status === "likely") return { label: "Needs your confirmation", tone: "warn" };
  if (record.download_state === "in_progress") return { label: "Downloading", tone: "neutral" };
  if (record.download_state === "interrupted") return { label: "Interrupted", tone: "neutral" };
  return { label: "No file matched yet", tone: "neutral" };
}
