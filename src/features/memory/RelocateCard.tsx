// Choosing the file's new place after it moved (#31 M2, material §4
// supplement). The native picker is opened by Rust inside an authorized
// folder; the core checks the chosen file is the recorded one (same object,
// one link) and shows what going on there does. Nothing is read or written
// before the user confirms.

import { useState } from "react";
import { familyApi } from "../family/api";
import type { MemoryRecordView, RelocationPreview } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type RootView } from "../organize/types";
import { placeTakenUnread, relocationPoints, relocationRefusal } from "./records";

export function RelocateCard({
  record,
  roots,
  onChange,
}: {
  record: MemoryRecordView;
  roots: RootView[];
  onChange: () => void;
}) {
  const [rootId, setRootId] = useState(record.place.root_id);
  const [preview, setPreview] = useState<RelocationPreview | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fail = (reason: unknown) => {
    setBusy(false);
    const failure = isAppError(reason) ? reason : null;
    const text = failure ? relocationRefusal(failure.code) : null;
    if (text) {
      setRefusal(text);
      setPreview(null);
    } else {
      setError(failure);
    }
  };
  const choose = () => {
    setError(null);
    setRefusal(null);
    setPreview(null);
    setBusy(true);
    familyApi.previewRelocation(record.chain_id, rootId).then((next) => {
      setBusy(false);
      setPreview(next);
    }, fail);
  };
  const confirm = (shown: RelocationPreview) => {
    setError(null);
    setRefusal(null);
    setBusy(true);
    familyApi
      .relocate({
        chain_id: shown.chain_id,
        root_id: shown.new_place.root_id,
        relative_path: shown.new_place.relative_path,
        page_token: shown.page_token,
      })
      .then(() => {
        setBusy(false);
        setPreview(null);
        onChange();
      }, fail);
  };

  return (
    <section className="card">
      <h2>Find the file at its new place</h2>
      <p>
        {placeTakenUnread(record) && !record.waiting_for_relocation
          ? "Another file is at this place now. If you moved your file and that one is different, you can choose your file where it is now instead of deciding about the one at the place."
          : "If you moved or renamed the file, choose it where it is now."}{" "}
        Traeva checks that it is the same file, not a copy, before it goes on recording.
      </p>
      <div className="row">
        <select value={rootId} onChange={(event) => setRootId(event.target.value)}>
          {roots.map((root) => (
            <option key={root.root_id} value={root.root_id}>
              {root.display_path}
            </option>
          ))}
        </select>
        <button type="button" disabled={busy || rootId === ""} onClick={choose}>
          Choose the file…
        </button>
      </div>
      {preview && (
        <div>
          <p>
            <strong className="mono">{preview.new_place.display_path}</strong>{" "}
            <span className="muted small">
              {preview.byte_length.toLocaleString()} bytes, changed {new Date(preview.modified_at_ms).toLocaleString()}
            </span>
          </p>
          <ul>
            {relocationPoints(preview).map((point) => (
              <li key={point}>{point}</li>
            ))}
          </ul>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => confirm(preview)}>
              Go on recording it here
            </button>
            <button type="button" disabled={busy} onClick={() => setPreview(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {refusal && <p className="warn">{refusal}</p>}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}
