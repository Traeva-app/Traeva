import { SourcePage } from "./SourcePage";
import type { SourceApi } from "./types";

export function FileSourceSection({ fileId, api, pollIntervalMs }: {
  fileId: string | null;
  api?: SourceApi;
  pollIntervalMs?: number;
}) {
  if (!fileId) return null;
  return <SourcePage key={fileId} fileId={fileId} api={api} pollIntervalMs={pollIntervalMs} />;
}
