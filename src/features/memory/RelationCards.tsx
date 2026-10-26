// Correcting one relation, and a recording paused by it (#31 M2b-3a,
// contract §4.3a, material §7). The core lists the work's relations, says
// what revoking or confirming again does, which recordings it pauses and
// checks the page again when it is confirmed; these cards only show that
// and send the user's choice.

import { useCallback, useEffect, useRef, useState } from "react";
import { familyApi } from "../family/api";
import type { MemoryRecordView, RelationAction, RelationPreview, VersionRelationView } from "../family/types";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError } from "../organize/types";
import {
  AUTO_LINK_PAIR_TEXT,
  HOLD_CHOICE_TEXT,
  correctableRelations,
  correctedText,
  holdText,
  relationNotes,
  relationRefusal,
  relationStatusText,
  relationText,
} from "./records";
import { ShowMore } from "../../shared/ShowMore";

function asError(reason: unknown): AppError | null {
  return isAppError(reason) ? reason : null;
}

/** A recording paused because the user revoked the relation it went on by:
 * record it again (the same recording goes on) or stop it. */
export function HoldCard({ record, onChange }: { record: MemoryRecordView; onChange: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  if (!record.hold) return null;
  const act = (call: Promise<unknown>) => {
    setBusy(true);
    setError(null);
    call.then(
      () => {
        setBusy(false);
        onChange();
      },
      (reason: unknown) => {
        setBusy(false);
        setError(asError(reason));
      },
    );
  };
  return (
    <section className="card">
      <h2>Recording paused</h2>
      <p>{holdText(record.hold)}</p>
      <ul className="plain small">
        <li>{HOLD_CHOICE_TEXT.again}</li>
        <li>{HOLD_CHOICE_TEXT.stop}</li>
      </ul>
      <div className="row">
        <button type="button" disabled={busy} onClick={() => act(familyApi.startForFile(record.watch.file_id))}>
          Record again
        </button>
        <button type="button" disabled={busy} onClick={() => act(familyApi.stopRecord(record.chain_id))}>
          Stop recording
        </button>
      </div>
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** The work's relations between versions; each can be revoked, and a
 * disputed one confirmed again. `changed` is the record's signature: when
 * it changes, the list is read again. */
export function RelationsCard({
  record,
  changed,
  onChange,
}: {
  record: MemoryRecordView;
  changed: string;
  onChange: () => void;
}) {
  const [relations, setRelations] = useState<VersionRelationView[] | null>(null);
  const [preview, setPreview] = useState<RelationPreview | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    familyApi.family(record.family_id).then(
      (family) => setRelations(correctableRelations(family.relations)),
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

  const fail = (reason: unknown) => {
    setBusy(false);
    const failure = asError(reason);
    const text = failure ? relationRefusal(failure.code) : null;
    if (text) {
      setNotice(text);
      setPreview(null);
      load();
    } else {
      setError(failure);
    }
  };
  const show = (edgeId: string) => {
    setNotice(null);
    setError(null);
    setBusy(true);
    familyApi.previewRelation(edgeId).then((shown) => {
      setBusy(false);
      setPreview(shown);
    }, fail);
  };
  const correct = (action: RelationAction) => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    familyApi
      .correctRelation({ edge_id: preview.relation.edge_id, action, page_token: preview.page_token })
      .then((outcome) => {
        setBusy(false);
        setPreview(null);
        setNotice(correctedText(action, outcome));
        load();
        onChange();
      }, fail);
  };

  if (!error && (relations === null || (relations.length === 0 && !notice))) return null;
  return (
    <section className="card">
      <details>
        <summary>
          <strong>Relations in this work</strong> ({relations?.length ?? 0})
        </summary>
        {preview ? (
          <div>
            <p>
              <strong>{relationText(preview.relation)}</strong>
              <br />
              <span className="muted small">{relationStatusText(preview.relation)}</span>
            </p>
            <ul className="plain small">
              {relationNotes(preview).map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
            <div className="row">
              {preview.can_revoke && (
                <button type="button" disabled={busy} onClick={() => correct("revoke")}>
                  Revoke this relation
                </button>
              )}
              {preview.can_confirm_again && (
                <button type="button" disabled={busy} onClick={() => correct("confirm_again")}>
                  Confirm it again
                </button>
              )}
              <button type="button" disabled={busy} onClick={() => setPreview(null)}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <ShowMore items={(relations ?? [])} first={5}>
            {(visible) => (
              <ul className="list">
                {visible.map((relation) => (
                  <li key={relation.edge_id}>
                    <div className="row">
                      <span className="grow small">
                        {relationText(relation)}
                        <br />
                        <span className={relation.status === "disputed" ? "warn" : "muted"}>
                          {relationStatusText(relation)}
                        </span>
                      </span>
                      <button type="button" disabled={busy} onClick={() => show(relation.edge_id)}>
                        Change…
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </ShowMore>
        )}
      </details>
      {notice && <p className="small">{notice}</p>}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

/** Under a version Traeva linked by itself (an app saved the file by
 * replacing it at the same place, the student's 10/14 decision): says so,
 * and lets the user undo the link. Undoing is the core's revoke of that
 * relation: the preview says what it does (the version stays in the work,
 * its place not known; the recording pauses) before it is confirmed. */
export function UndoLinkNote({ edgeId, onUndone }: { edgeId: string; onUndone: () => void }) {
  const [preview, setPreview] = useState<RelationPreview | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const fail = (reason: unknown) => {
    setBusy(false);
    const failure = asError(reason);
    const text = failure ? relationRefusal(failure.code) : null;
    if (text) {
      setNotice(text);
      setPreview(null);
    } else {
      setError(failure);
    }
  };
  const show = () => {
    setNotice(null);
    setError(null);
    setBusy(true);
    familyApi.previewRelation(edgeId).then((shown) => {
      setBusy(false);
      setPreview(shown);
    }, fail);
  };
  const undo = () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    familyApi
      .correctRelation({ edge_id: preview.relation.edge_id, action: "revoke", page_token: preview.page_token })
      .then((outcome) => {
        setBusy(false);
        setPreview(null);
        setNotice(correctedText("revoke", outcome));
        onUndone();
      }, fail);
  };
  return (
    <div>
      <p className="muted">{AUTO_LINK_PAIR_TEXT}</p>
      {preview ? (
        <>
          <ul className="plain">
            {relationNotes(preview).map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
          <div className="row">
            {preview.can_revoke && (
              <button type="button" disabled={busy} onClick={undo}>
                Undo this link
              </button>
            )}
            <button type="button" disabled={busy} onClick={() => setPreview(null)}>
              Cancel
            </button>
          </div>
        </>
      ) : (
        <button type="button" disabled={busy} onClick={show}>
          Undo this link…
        </button>
      )}
      {notice && <p>{notice}</p>}
      {error && <ErrorDetails error={error} />}
    </div>
  );
}
