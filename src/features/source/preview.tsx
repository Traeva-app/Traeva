import { StrictMode, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { SourcePage } from "./SourcePage";
import { FileSourceSection } from "./FileSourceSection";
import type { SourceApi, SourceRecordView, SourceState } from "./types";
import "../../styles.css";

const initial: SourceRecordView = {
  source_id: "synthetic-source-1", profile_id: "synthetic-chrome-profile", download_id: "101",
  download_state: "complete", status: "likely", filename: "/Synthetic Downloads/field-notes.txt",
  initial_url: "https://example.test/redirect/field-notes", final_url: "https://example.test/files/field-notes.txt",
  referrer: "https://example.test/downloads", title: null,
  started_at_ms: 1791450000000, completed_at_ms: 1791450005000,
  imported_at_ms: 1791536400000, updated_at_ms: 1791536400000,
  reason: "The recorded path has a file candidate. That does not prove it is the downloaded object.",
  candidates: [{ file_id: "synthetic-file-1", display_path: "/Synthetic Downloads/field-notes.txt", reason: "A current file was observed at the recorded download path.", can_confirm: true }],
  association: null,
};

function Preview() {
  const scenario = new URLSearchParams(location.search).get("scenario");
  const [failWrites, setFailWrites] = useState(false);
  const [failReads, setFailReads] = useState(false);
  const [cancelPicker, setCancelPicker] = useState(false);
  const [fileView, setFileView] = useState("");
  const [reverseOrder, setReverseOrder] = useState(false);
  const fail = useRef(false);
  const unreadable = useRef(false);
  const cancelled = useRef(false);
  const reversed = useRef(false);
  fail.current = failWrites;
  unreadable.current = failReads;
  cancelled.current = cancelPicker;
  reversed.current = reverseOrder;
  const [event, setEvent] = useState("No actions yet.");
  const api = useMemo<SourceApi>(() => {
    let state: SourceState = { enabled: false, records: [structuredClone(initial), {
      ...structuredClone(initial), source_id: "synthetic-source-2", download_id: "102",
      filename: "/Synthetic Downloads/archive.txt", status: "unknown", candidates: [],
      initial_url: "file:///synthetic/unsupported", final_url: null, referrer: null,
      started_at_ms: null, completed_at_ms: null,
      reason: "File observation failed. This does not mean the file is absent.",
    }] };
    if (scenario === "empty") state.records = [];
    if (scenario === "long") {
      const longName = `${"synthetic-long-name-".repeat(12)}.txt`;
      state.records[0].filename = `/Synthetic Downloads/${longName}`;
      state.records[0].initial_url = `https://example.test/${longName}`;
      state.records[0].candidates[0].display_path = state.records[0].filename;
      state.records[0].candidates[0].can_confirm = false;
    }
    const source = (id: string) => state.records.find((item) => item.source_id === id)!;
    const write = async (id: string, update: (record: SourceRecordView) => void) => {
      if (scenario === "slow") await new Promise((resolve) => setTimeout(resolve, 700));
      if (fail.current) throw { message: "Synthetic storage failure: the change was not saved." };
      const copy = structuredClone(source(id));
      update(copy);
      copy.updated_at_ms = Date.now();
      state.records = state.records.map((item) => item.source_id === id ? copy : item);
      return Promise.resolve(structuredClone(copy));
    };
    return {
      state: async () => {
        if (unreadable.current) throw { message: "Synthetic storage read failure." };
        const view = structuredClone(state);
        if (reversed.current) view.records.reverse();
        return view;
      },
      setEnabled: async (enabled) => {
        if (fail.current) throw { message: "Synthetic storage failure: collection setting was not saved." };
        state.enabled = enabled;
        setEvent(enabled ? "Collection enabled in preview." : "Collection disabled in preview.");
        return structuredClone(state);
      },
      refresh: async (id) => { setEvent("Synthetic observation requested."); return structuredClone(source(id)); },
      chooseFile: async (id) => {
        if (cancelled.current) {
          setEvent("Synthetic picker cancelled. Source state unchanged.");
          return null;
        }
        return write(id, (record) => {
          setEvent("Synthetic file selected. No native picker or file I/O occurred.");
          record.candidates = [{ file_id: "synthetic-file-selected", display_path: "/Synthetic Downloads/selected.txt", reason: "You selected this synthetic file for review.", can_confirm: true }];
          if (!record.association) record.status = "likely";
        });
      },
      confirm: (id, fileId, relation, note) => write(id, (record) => {
        const candidate = record.candidates.find((item) => item.file_id === fileId)!;
        record.association = { file_id: fileId, display_path: candidate.display_path, relation, basis: "user_confirmed", note, confirmed_at_ms: Date.now(), current_location_confirmed: true };
        record.status = "user_confirmed";
        record.candidates = [];
        record.reason = "Saved with your explicit confirmation.";
        setEvent("Synthetic relationship saved as User Confirmed.");
      }),
      reject: (id, fileId) => write(id, (record) => {
        record.candidates = record.candidates.filter((item) => item.file_id !== fileId);
        if (!record.association && record.candidates.length === 0) record.status = "unknown";
        setEvent("Synthetic candidate rejected.");
      }),
      unlink: (id) => write(id, (record) => {
        record.association = null;
        record.status = "unknown";
        record.reason = "The association was removed. Download evidence is retained.";
        setEvent("Synthetic association removed; source retained.");
      }),
      openLink: async (_id, kind) => { setEvent(`Synthetic ${kind} link action requested. No website was opened.`); },
      connection: async () => ({ enabled: state.enabled, active: state.enabled, error: null, endpoint: "http://127.0.0.1:12345", pairing_token: "synthetic-preview-token-not-a-secret" }),
    };
  }, [scenario]);
  return <>
    <aside className="page" aria-label="Synthetic preview controls">
      <strong>Synthetic UI preview</strong>
      <p>No real files, Chrome connection, or storage are used.</p>
      <div className="field">
        <label><input type="checkbox" checked={failWrites} onChange={(change) => setFailWrites(change.target.checked)} /> Simulate storage failures</label>
        <label><input type="checkbox" checked={failReads} onChange={(change) => setFailReads(change.target.checked)} /> Simulate read failures</label>
        <label><input type="checkbox" checked={cancelPicker} onChange={(change) => setCancelPicker(change.target.checked)} /> Simulate cancelled picker</label>
        <label><input type="checkbox" checked={reverseOrder} onChange={(change) => setReverseOrder(change.target.checked)} /> Reverse list on refresh</label>
      </div>
      <label>Preview view <select value={fileView} onChange={(change) => setFileView(change.target.value)}>
        <option value="">Whole source list</option>
        <option value="synthetic-file-1">File details: field-notes</option>
        <option value="synthetic-file-selected">File details: selected file</option>
        <option value="synthetic-unrelated-file">File details: unrelated file</option>
      </select></label>
      <p role="status">{event}</p>
    </aside>
    {fileView ? <div className="page"><FileSourceSection fileId={fileView} api={api} /></div> : <SourcePage api={api} />}
  </>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><Preview /></StrictMode>);
