import type { SnapshotRef } from "../family/types";

// Component input only. A owns the eventual IPC DTO and its adapter.
export interface DocxPreview {
  snapshot: SnapshotRef;
  extraction: {
    summary: {
      reader: string;
      status: "complete" | "partial" | "unsupported" | "failed";
      chars: number;
      covered: string[];
      left_out: string[];
      note: string | null;
    };
    text: string;
  };
}

export type DocxPreviewApi = (snapshotId: string, signal: AbortSignal) => Promise<DocxPreview>;

export const CACHE_WARNING = "Citation field display text is a saved cache and may be missing or out of date. It has not been evaluated or refreshed.";
export const FLATTENED_WARNING = "Unmarked, flattened citation-manager data cannot always be identified or excluded.";

export function snapshotKey(snapshot: SnapshotRef): string {
  return JSON.stringify([
    snapshot.snapshot_id, snapshot.file_id, snapshot.sequence, snapshot.file_revision,
    snapshot.root_id, snapshot.relative_path, snapshot.sha256, snapshot.byte_length,
    snapshot.captured_at_ms,
  ]);
}

// Reject a misbound or malformed adapter result before rendering any body.
// The text ceiling matches the existing Reader, in Unicode scalar values.
export function checkedPreview(snapshot: SnapshotRef, value: DocxPreview): DocxPreview {
  if (!value || !value.snapshot || snapshotKey(value.snapshot) !== snapshotKey(snapshot)) {
    throw new Error("Preview does not match the selected snapshot");
  }
  const extraction = value.extraction;
  const summary = extraction?.summary;
  const stringList = (list: unknown): list is string[] =>
    Array.isArray(list) && list.length <= 256 && list.every((item) => typeof item === "string" && item.length <= 16_384);
  if (!summary || typeof extraction.text !== "string" || extraction.text.length > 2_000_000 ||
      typeof summary.reader !== "string" || summary.reader.length > 256 ||
      !Number.isInteger(summary.chars) || summary.chars < 0 || summary.chars > 1_000_000 ||
      !stringList(summary.covered) || !stringList(summary.left_out) ||
      !(summary.note === null || (typeof summary.note === "string" && summary.note.length <= 16_384))) {
    throw new Error("Invalid preview result");
  }
  switch (summary.status) {
    case "failed":
    case "unsupported":
      if (extraction.text !== "" || summary.chars !== 0 || summary.covered.length || summary.left_out.length || !summary.note) {
        throw new Error("Invalid result without text");
      }
      break;
    case "complete":
    case "partial": {
      if ((summary.status === "complete" && summary.left_out.length !== 0) ||
          (summary.status === "partial" && summary.left_out.length === 0)) {
        throw new Error("Invalid extraction coverage");
      }
      let chars = 0;
      for (const char of extraction.text) {
        const point = char.codePointAt(0)!;
        if (point >= 0xd800 && point <= 0xdfff) throw new Error("Invalid Unicode text");
        chars += 1;
      }
      if (chars !== summary.chars) throw new Error("Invalid text character count");
      break;
    }
    default:
      throw new Error("Unknown extraction status");
  }
  return value;
}
