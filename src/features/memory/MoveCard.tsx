// Moving files out of a work (#31 M2b-3b, contract §4.3a, material §7):
// the user picks confirmed members of this work and where they go (a new
// work or another one); the core shows what moves, which relations then
// cross two works and which recordings pause, and checks the page again
// when it is confirmed. This card only shows that and sends the choice.

import { useCallback, useEffect, useRef, useState } from "react";
import { familyApi } from "../family/api";
import type { FamilyMemberView, MemoryRecordView, MovePreview, MoveTarget, WorkSummaryView } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import { memberLabel, memberNotes, movableMembers, moveNotes, moveRefusal, movedText, namedPlace, workText } from "./records";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** `changed` is the record's signature: when it changes, the members are
 * read again. */
export function MoveCard({
  record,
  changed,
  onChange,
}: {
  record: MemoryRecordView;
  changed: string;
  onChange: () => void;
}) {
  const [members, setMembers] = useState<FamilyMemberView[]>([]);
  const [works, setWorks] = useState<WorkSummaryView[] | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const [target, setTarget] = useState("new");
  const [preview, setPreview] = useState<MovePreview | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    familyApi.family(record.family_id).then(
      (family) => setMembers(movableMembers(family.members)),
      (reason: unknown) => setError(asError(reason)),
    );
  }, [record.family_id]);
  useEffect(load, [load, changed]);
  // A notice that a recording is paused is out of date once it records
  // again or stops (R2-MEM-07).
  const held = record.hold !== null;
  const wasHeld = useRef(held);
  useEffect(() => {
    if (wasHeld.current && !held) setNotice(null);
    wasHeld.current = held;
  }, [held]);

  const targetOf = (): MoveTarget => (target === "new" ? { kind: "new_work" } : { kind: "work", family_id: target });
  const targetName = (): string => {
    const work = works?.find((w) => w.family_id === target);
    return work ? `the work of ${workText(work)}` : "a new work";
  };
  const fail = (reason: unknown) => {
    setBusy(false);
    const failure = asError(reason);
    const text = failure ? moveRefusal(failure.code) : null;
    if (text) {
      setNotice(text);
      setPreview(null);
      load();
    } else {
      setError(failure);
    }
  };
  const open = () => {
    setNotice(null);
    setError(null);
    setPicked([]);
    setTarget("new");
    familyApi.listWorks().then((all) => setWorks(all.filter((work) => work.family_id !== record.family_id)), fail);
  };
  const toggle = (fileId: string) =>
    setPicked((now) => (now.includes(fileId) ? now.filter((id) => id !== fileId) : [...now, fileId]));
  const showPreview = () => {
    setBusy(true);
    setError(null);
    familyApi.previewMove({ family_id: record.family_id, file_ids: picked, target: targetOf() }).then((shown) => {
      setBusy(false);
      setPreview(shown);
    }, fail);
  };
  const move = () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    familyApi
      .moveMembers({ family_id: record.family_id, file_ids: picked, target: targetOf(), page_token: preview.page_token })
      .then((outcome) => {
        setBusy(false);
        setNotice(movedText(outcome, preview.moving.length, targetName()));
        setPreview(null);
        setWorks(null);
        onChange();
      }, fail);
  };
  const close = () => {
    setWorks(null);
    setPreview(null);
  };

  // At least one file stays: with one member there is nothing to move.
  if (members.length < 2 && !notice && !error) return null;
  return (
    <section className="card">
      {works === null ? (
        members.length >= 2 && (
          <button type="button" disabled={busy} onClick={open}>
            Move files out of this work…
          </button>
        )
      ) : preview ? (
        <div>
          <h2>Move files out of this work</h2>
          <ul className="plain small">
            {moveNotes(preview, targetName(), (member) => memberLabel(member, record)).map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
          <div className="row">
            <button type="button" disabled={busy} onClick={move}>
              Move these files
            </button>
            <button type="button" disabled={busy} onClick={close}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div>
          <h2>Move files out of this work</h2>
          <p className="muted small">Choose the files that do not belong here. At least one file stays.</p>
          <ul className="plain">
            {members.map((member) => {
              const place = namedPlace(member.display_path);
              return (
                <li key={member.file_id}>
                  <label>
                    <input
                      type="checkbox"
                      checked={picked.includes(member.file_id)}
                      onChange={() => toggle(member.file_id)}
                    />{" "}
                    <strong>{place.name}</strong> <span className="small">({memberNotes(member, record)})</span>
                    {place.folder && (
                      <>
                        <br />
                        <span className="mono small muted">{place.folder}</span>
                      </>
                    )}
                  </label>
                </li>
              );
            })}
          </ul>
          <div className="row">
            <select value={target} onChange={(event) => setTarget(event.target.value)}>
              <option value="new">To a new work</option>
              {works.map((work) => (
                <option key={work.family_id} value={work.family_id}>
                  To the work of {workText(work)}
                </option>
              ))}
            </select>
            <button
              type="button"
              disabled={busy || picked.length === 0 || picked.length >= members.length}
              onClick={showPreview}
            >
              Preview the move
            </button>
            <button type="button" disabled={busy} onClick={close}>
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
