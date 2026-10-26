import type { Flow } from "../organize/shared.tsx";

export function sourceFileId(flow: Flow): string | null {
  switch (flow.step) {
    case "operation": return flow.operation.file?.file_id ?? null;
    case "preview":
    case "applying": return flow.plan.file_id;
    case "done": return flow.action.file_id;
    case "in_place": return flow.file.file_id;
    case "start":
    case "failed": return null;
  }
}
