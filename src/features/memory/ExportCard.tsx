// Exporting an old version to the user's folder (#35). The save panel is
// opened by Rust in an authorized folder; the core shows where the copy
// would go, and writes it only after the user confirms here: a new file,
// never over one (export.rs).

import { ErrorDetails } from "../organize/shared";
import type { AppError } from "../organize/types";
import { versionRef, type LineNumbers } from "./history";
import type { ExportedVersion, ExportPreview } from "./types";

export type ExportStep =
  | { kind: "preview"; preview: ExportPreview }
  | { kind: "done"; done: ExportedVersion }
  | { kind: "refused"; text: string }
  /** The copy was moved, but where it is could not be confirmed. */
  | { kind: "unconfirmed"; error: AppError }
  | { kind: "error"; error: AppError | null };

export function ExportCard({
  step,
  numbers,
  busy,
  onConfirm,
  onClose,
}: {
  step: ExportStep;
  numbers: LineNumbers;
  busy: boolean;
  onConfirm: (preview: ExportPreview) => void;
  onClose: () => void;
}) {
  switch (step.kind) {
    case "preview": {
      const { preview } = step;
      return (
        <section className="card">
          <h2>Export a copy of {versionRef(preview.snapshot, numbers)}</h2>
          <p>
            <strong className="mono">{preview.display_path}</strong>{" "}
            <span className="muted small">{preview.byte_length.toLocaleString()} bytes</span>
          </p>
          <p className="small">
            Traeva writes a new file there with this version's exact text. It never replaces a file: if the name is
            taken by then, nothing is written. Your file and the saved version stay as they are. (A folder made in the
            save panel was made by macOS, not by Traeva.)
          </p>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => onConfirm(preview)}>
              Export
            </button>
            <button type="button" disabled={busy} onClick={onClose}>
              Cancel
            </button>
          </div>
        </section>
      );
    }
    case "done":
      return (
        <section className="card card-ok">
          <h2>Exported {versionRef(step.done.snapshot, numbers)}</h2>
          <p>
            The new file is <span className="mono">{step.done.display_path}</span>. Its bytes were checked against the
            saved version before it was put there.
          </p>
          <button type="button" onClick={onClose}>
            OK
          </button>
        </section>
      );
    case "refused":
      return (
        <section className="card">
          <p className="warn">{step.text}</p>
          <button type="button" onClick={onClose}>
            OK
          </button>
        </section>
      );
    case "unconfirmed":
      return (
        <section className="card">
          <h2>Not sure the copy arrived</h2>
          <p className="warn">{step.error.message}</p>
          <p className="small">
            It may already be in the folder. Look there before exporting again; Traeva never moves it back or writes
            over it.
          </p>
          <button type="button" onClick={onClose}>
            OK
          </button>
        </section>
      );
    case "error":
      return (
        <section className="card card-error">
          <h2>The version could not be exported</h2>
          {step.error && <ErrorDetails error={step.error} />}
          <button type="button" onClick={onClose}>
            OK
          </button>
        </section>
      );
  }
}
