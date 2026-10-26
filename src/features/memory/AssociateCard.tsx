// Adding a file to a work (#31 M2b-2, contract §4.3a): the core shows the
// file, the work, what it knows and the choices it allows; the user picks
// one and confirms with the page's token. Evidence is only shown, never
// taken as a relation.

import { useState } from "react";
import { familyApi } from "../family/api";
import type { AssociateChoice, AssociateChoiceKind, AssociationPreview, WorkDecisionOutcome } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import {
  ASSOCIATE_CHOICE_TEXT,
  associationNotes,
  associationRefusal,
  evidenceText,
  savedVersionText,
  standingText,
} from "./records";

export function AssociateCard({
  preview,
  onDone,
  onCancel,
}: {
  preview: AssociationPreview;
  onDone: (outcome: WorkDecisionOutcome) => void;
  onCancel: () => void;
}) {
  const [kind, setKind] = useState<AssociateChoiceKind | null>(null);
  const [parent, setParent] = useState("");
  const [busy, setBusy] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  const parents =
    kind === "continue"
      ? preview.continue_from.map((option) => option.parent)
      : kind === "branch"
        ? preview.fork_points
        : [];
  const stops = preview.continue_from.find((option) => option.parent.snapshot_id === parent)?.stops_record ?? null;
  const choice: AssociateChoice | null =
    kind === "continue" || kind === "branch"
      ? parent
        ? { kind, parent_snapshot_id: parent }
        : null
      : kind
        ? { kind }
        : null;

  const save = () => {
    if (!choice) return;
    setBusy(true);
    setRefusal(null);
    setError(null);
    familyApi
      .associate({
        family_id: preview.family_id,
        subject: preview.request_subject,
        choice,
        page_token: preview.page_token,
      })
      .then(
        (outcome) => {
          setBusy(false);
          onDone(outcome);
        },
        (reason: unknown) => {
          setBusy(false);
          const failure = isAppError(reason) ? reason : null;
          const text = failure ? associationRefusal(failure.code) : null;
          if (text) setRefusal(text);
          else setError(failure);
        },
      );
  };

  return (
    <div>
      <p>
        <strong className="mono">{preview.subject.place.display_path}</strong>{" "}
        <span className="muted small">
          {preview.subject.versions.length === 1 ? "1 saved version" : `${preview.subject.versions.length} saved versions`}
        </span>
      </p>
      <p className="small">{standingText(preview)}</p>
      <ul className="small">
        {evidenceText(preview).map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <div className="row small">
        {preview.choices.map((option) => (
          <label key={option}>
            <input
              type="radio"
              name="associate-choice"
              checked={kind === option}
              onChange={() => {
                setKind(option);
                setParent("");
              }}
            />{" "}
            {ASSOCIATE_CHOICE_TEXT[option]}
          </label>
        ))}
      </div>
      {parents.length > 0 && (
        <label className="small">
          {kind === "continue" ? "It continues from " : "It branches from "}
          <select value={parent} onChange={(event) => setParent(event.target.value)}>
            <option value="">choose a version…</option>
            {parents.map((version) => (
              <option key={version.snapshot_id} value={version.snapshot_id}>
                {savedVersionText(version)}
              </option>
            ))}
          </select>
        </label>
      )}
      {kind === "continue" && stops && (
        <p className="warn small">
          The recording of {stops.place.display_path} stops: its line goes on as this file.
        </p>
      )}
      {kind && (
        <ul className="small">
          {associationNotes(preview, kind).map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      )}
      <div className="row">
        <button type="button" disabled={busy || choice === null} onClick={save}>
          Save
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </button>
      </div>
      {refusal && <p className="warn small">{refusal}</p>}
      {error && <ErrorDetails error={error} />}
    </div>
  );
}
