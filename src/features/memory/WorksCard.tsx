// The work a recording belongs to, merging it with another work, and the
// same-content hint (#31 M2b-1, material §6). The core lists the works,
// shows what a merge does and checks the page again when it is confirmed;
// this card only shows that and sends the user's choice. A work's ID can
// change with a merge, so nothing here keeps one between refreshes.

import { useState } from "react";
import { familyApi } from "../family/api";
import type { AssociationPreview, MemoryRecordView, MergePreview, WorkSummaryView } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type RootView } from "../organize/types";
import { AssociateCard } from "./AssociateCard";
import { associationRefusal, mergePoints, mergeRefusal, sameContentText, workFiles, workText } from "./records";

export function WorksCard({
  record,
  roots,
  onChange,
}: {
  record: MemoryRecordView;
  roots: RootView[];
  onChange: () => void;
}) {
  const [association, setAssociation] = useState<AssociationPreview | null>(null);
  const [rootId, setRootId] = useState(record.place.root_id);
  const [works, setWorks] = useState<WorkSummaryView[] | null>(null);
  const [other, setOther] = useState("");
  const [preview, setPreview] = useState<MergePreview | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const fail = (reason: unknown) => {
    setBusy(false);
    const failure = isAppError(reason) ? reason : null;
    const text = failure ? mergeRefusal(failure.code) : null;
    if (text) {
      setNotice(text);
      setPreview(null);
    } else {
      setError(failure);
    }
  };
  const chooseOther = () => {
    setNotice(null);
    setError(null);
    familyApi.listWorks().then((all) => setWorks(all.filter((work) => work.family_id !== record.family_id)), fail);
  };
  const showMerge = (otherId: string) => {
    setNotice(null);
    setError(null);
    setBusy(true);
    familyApi.previewMerge(record.family_id, otherId).then((shown) => {
      setBusy(false);
      setPreview(shown);
    }, fail);
  };
  const associationFailed = (reason: unknown) => {
    setBusy(false);
    const failure = isAppError(reason) ? reason : null;
    const text = failure ? associationRefusal(failure.code) : null;
    if (text) setNotice(text);
    else setError(failure);
  };
  // A file chosen in the native picker, to add to this work.
  const addFile = () => {
    setNotice(null);
    setError(null);
    setBusy(true);
    familyApi.previewAssociation(record.family_id, rootId).then((shown) => {
      setBusy(false);
      setAssociation(shown);
    }, associationFailed);
  };
  // This recording's file, added to the work the hint names.
  const addThisFile = (familyId: string) => {
    setNotice(null);
    setError(null);
    setBusy(true);
    familyApi.previewAssociationForFile(familyId, record.origin_file_id).then((shown) => {
      setBusy(false);
      setAssociation(shown);
    }, associationFailed);
  };

  const confirm = (shown: MergePreview) => {
    setBusy(true);
    familyApi
      .mergeWorks({ family_id: shown.family_id, other_family_id: shown.other_family_id, page_token: shown.page_token })
      .then(() => {
        setBusy(false);
        setPreview(null);
        setWorks(null);
        setNotice("Merged. The two works are one now; each file keeps its own history.");
        onChange();
      }, fail);
  };

  return (
    <section className="card">
      <h2>This work</h2>
      {record.same_content.map((hint) => (
        <div key={`${hint.family_id}-${hint.same_bytes_as.snapshot_id}`}>
          <p className="small">{sameContentText(hint)}</p>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => addThisFile(hint.family_id)}>
              Add this file to that work…
            </button>
            <button type="button" disabled={busy} onClick={() => showMerge(hint.family_id)}>
              Merge with that work…
            </button>
          </div>
        </div>
      ))}
      {association ? (
        <AssociateCard
          preview={association}
          onDone={(outcome) => {
            setAssociation(null);
            const stopped = outcome.stopped_records.length;
            setNotice(
              stopped === 0
                ? "Saved. The file's relation to the work is recorded."
                : `Saved. ${stopped === 1 ? "1 recording" : `${stopped} recordings`} stopped: the line goes on as the added file.`,
            );
            onChange();
          }}
          onCancel={() => setAssociation(null)}
        />
      ) : (
        !preview && (
          <div className="row">
            <select value={rootId} onChange={(event) => setRootId(event.target.value)}>
              {roots.map((root) => (
                <option key={root.root_id} value={root.root_id}>
                  {root.display_path}
                </option>
              ))}
            </select>
            <button type="button" disabled={busy || rootId === ""} onClick={addFile}>
              Add a file to this work…
            </button>
          </div>
        )
      )}
      {!preview && works === null && (
        <button type="button" disabled={busy} onClick={chooseOther}>
          Merge with another work…
        </button>
      )}
      {!preview && works !== null && (
        works.length === 0 ? (
          <p className="muted small">There is no other work to merge with.</p>
        ) : (
          <div className="row">
            <select value={other} onChange={(event) => setOther(event.target.value)}>
              <option value="">Choose a work…</option>
              {works.map((work) => (
                <option key={work.family_id} value={work.family_id}>
                  {workText(work)}
                </option>
              ))}
            </select>
            <button type="button" disabled={busy || other === ""} onClick={() => showMerge(other)}>
              Preview the merge
            </button>
            <button type="button" onClick={() => setWorks(null)}>
              Cancel
            </button>
          </div>
        )
      )}
      {preview && (
        <div>
          <p>
            <strong>This work</strong> ({workFiles(preview.stays).join(", ")}) and <strong>the other work</strong> (
            {workFiles(preview.joins).join(", ")}) become one work.
          </p>
          <ul>
            {mergePoints(preview).map((point) => (
              <li key={point}>{point}</li>
            ))}
          </ul>
          <div className="row">
            <button type="button" disabled={busy} onClick={() => confirm(preview)}>
              Merge these works
            </button>
            <button type="button" disabled={busy} onClick={() => setPreview(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {notice && <p className="small">{notice}</p>}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}
