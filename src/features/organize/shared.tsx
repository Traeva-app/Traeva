// What the organize page's cards share: where the flow is, and how a file
// and an error are shown.

import {
  isAppError,
  type ActionView,
  type AppError,
  type CandidateView,
  type FileView,
  type OperationStage,
  type OperationView,
  type PlanView,
} from "./types";
import { isImageName } from "../find/view";
import { extractionText } from "./flow";

// Where the organize flow is. Each step shows one card. While a chosen file
// is read and analyzed, the step is "operation" and the core's operation
// view decides the card (flow.ts cardFor).
export type Flow =
  | { step: "start" }
  | { step: "operation"; operation: OperationView }
  | { step: "preview"; plan: PlanView; context: PreviewContext | null; error?: AppError }
  | { step: "applying"; plan: PlanView; action: ActionView | null }
  | { step: "done"; action: ActionView; error?: AppError }
  | { step: "in_place"; file: FileView; operation: OperationView | null }
  | { step: "failed"; title: string; error: AppError | null; detail: string };

/** A preview made within an operation: the way back to it, and the
 * candidate whose reason is shown (null for a folder picked by hand). */
export type PreviewContext = { operation: OperationView; candidate: CandidateView | null };

export type SetFlow = (flow: Flow) => void;

export function failure(title: string, reason: unknown): Flow {
  return {
    step: "failed",
    title,
    error: isAppError(reason) ? reason : null,
    detail: String(reason),
  };
}

export const STAGE_TITLES: Record<OperationStage, string> = {
  observing: "Still reading",
  ready_for_analysis: "Read",
  analyzing: "Asking the model",
  needs_clarification: "A question",
  recommendation_ready: "Recommendation",
  needs_manual_location: "Pick the folder",
  analysis_failed: "The analysis failed",
  invalid_model_output: "The model's answer was not usable",
  waiting_for_file: "The file is still being changed",
  stale: "The file changed",
  blocked: "This file cannot be organized",
  cancelled: "Cancelled",
};

const NEXT_STEPS = {
  restart_app: "Quit Traeva, fix the cause above, then open it again.",
  choose_folder: "Pick the folder by hand, or choose another folder.",
  choose_file: "Choose the file again.",
  try_again: "Wait a moment, then try again.",
  preview_again: "Make a new preview.",
} as const;

export function ErrorDetails({ error }: { error: AppError }) {
  return (
    <dl>
      <dt>Reason</dt>
      <dd className="exact">{error.message}</dd>
      <dt>Code</dt>
      <dd className="mono">{error.code}</dd>
      {error.allowed_next.length > 0 && (
        <>
          <dt>Next step</dt>
          <dd>{error.allowed_next.map((step) => NEXT_STEPS[step]).join(" ")}</dd>
        </>
      )}
    </dl>
  );
}

export function FileSummary({ file }: { file: FileView }) {
  return (
    <dl>
      <dt>File</dt>
      <dd className="mono">{file.display_path}</dd>
      <dt>Size</dt>
      <dd>{file.byte_length.toLocaleString()} bytes</dd>
      <dt>SHA-256</dt>
      <dd className="mono small">{file.sha256}</dd>
      <dt>Text</dt>
      <dd>
        <ExtractionText file={file} />
      </dd>
    </dl>
  );
}

/** What the model may read of the file: all of it, or only its head and
 * tail, with the reader's note (what it left out; for recognized text, that
 * it may contain mistakes, which is not greyed out). Shown wherever a
 * suggestion or a preview is shown (organize-spec.md §6). */
export function ExtractionText({ file }: { file: FileView }) {
  const { note, recognized } = file.extraction;
  return (
    <>
      {extractionText(file.extraction, isImageName(file.relative_path))}
      {note && <span className={recognized ? undefined : "muted"}> — {note}</span>}
    </>
  );
}

/** A move drawn as a route: the file's name, kept; where it is; the full
 * target; every folder that will be made (organize-spec.md §6). */
export function MoveRoute(props: { name: string; from: string; to: string; newFolders: string[] }) {
  return (
    <div className="move-route">
      <div className="move-route-name">
        {props.name} <span className="muted small">(name unchanged)</span>
      </div>
      <div className="move-route-row">
        <span className="muted">From</span>
        <span className="mono">{props.from}</span>
      </div>
      <div className="move-route-arrow" aria-hidden="true">
        ↓
      </div>
      <div className="move-route-row move-route-to">
        <span className="muted">To</span>
        <span className="mono">{props.to}</span>
      </div>
      <div className="move-route-row">
        <span className="muted">New folders</span>
        {props.newFolders.length === 0 ? (
          <span>none</span>
        ) : (
          <ul className="plain">
            {props.newFolders.map((folder) => (
              <li key={folder} className="mono">
                {folder}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
