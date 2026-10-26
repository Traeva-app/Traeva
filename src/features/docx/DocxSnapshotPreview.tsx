import { useEffect, useId, useState } from "react";
import type { SnapshotRef } from "../family/types";
import { CACHE_WARNING, FLATTENED_WARNING, checkedPreview, snapshotKey } from "./preview";
import type { DocxPreview, DocxPreviewApi } from "./preview";
import "./DocxSnapshotPreview.css";

export interface DocxSnapshotPreviewProps {
  snapshot: SnapshotRef | null;
  previewApi: DocxPreviewApi;
}

export function DocxSnapshotPreview({ snapshot, previewApi }: DocxSnapshotPreviewProps) {
  const [apiBinding, setApiBinding] = useState({ api: previewApi, generation: 0 });
  // Returning to a previous adapter must not revive its earlier result.
  if (apiBinding.api !== previewApi) {
    setApiBinding({ api: previewApi, generation: apiBinding.generation + 1 });
  }
  return snapshot === null ? (
    <section className="docx-preview" aria-label="Historical DOCX text preview">
      <h2>Saved DOCX text</h2>
      <p role="status">Select a saved version to preview its text.</p>
    </section>
  ) : <PreviewRequest key={`${snapshotKey(snapshot)}:${apiBinding.generation}`} snapshot={snapshot} previewApi={previewApi} />;
}

function PreviewRequest({ snapshot, previewApi }: { snapshot: SnapshotRef; previewApi: DocxPreviewApi }) {
  const heading = useId();
  const requestKey = snapshotKey(snapshot);
  const [attempt, setAttempt] = useState(0);
  const [result, setResult] = useState<{
    api: DocxPreviewApi;
    attempt: number;
    preview: DocxPreview | null;
  } | null>(null);
  const current = result?.api === previewApi && result.attempt === attempt ? result : null;

  useEffect(() => {
    const controller = new AbortController();
    async function load() {
      try {
        const value = await previewApi(snapshot.snapshot_id, controller.signal);
        if (controller.signal.aborted) return;
        const preview = checkedPreview(snapshot, value);
        setResult({ api: previewApi, attempt, preview });
      } catch {
        // Transport errors can contain document text or paths. Do not echo them.
        if (!controller.signal.aborted) setResult({ api: previewApi, attempt, preview: null });
      }
    }
    void load();
    return () => controller.abort();
  }, [requestKey, previewApi, attempt]);

  return (
    <section className="docx-preview" aria-labelledby={heading} aria-busy={current === null}>
      <h2 id={heading}>Saved DOCX text</h2>
      <p className="docx-preview-context">Plain text from the selected saved version.</p>
      <SnapshotInfo snapshot={snapshot} />
      {current === null ? <p role="status">Loading saved text…</p> : current.preview === null ? (
        <div>
          <p role="alert">Could not load this saved version. No preview text is shown.</p>
          <button type="button" onClick={() => setAttempt((previous) => previous + 1)}>Retry local preview</button>
        </div>
      ) : <PreviewText preview={current.preview} />}
    </section>
  );
}

function SnapshotInfo({ snapshot }: { snapshot: SnapshotRef }) {
  const date = new Date(snapshot.captured_at_ms);
  return (
    <details className="docx-preview-info">
      <summary>Snapshot information</summary>
      <dl>
        <dt>Saved path</dt><dd>{snapshot.relative_path}</dd>
        <dt>Captured (UTC)</dt><dd>{Number.isNaN(date.getTime()) ? "Unknown" : date.toISOString()}</dd>
        <dt>File sequence</dt><dd>{snapshot.sequence}</dd>
        <dt>File revision</dt><dd>{snapshot.file_revision}</dd>
        <dt>Size</dt><dd>{snapshot.byte_length} bytes</dd>
        <dt>Snapshot ID</dt><dd>{snapshot.snapshot_id}</dd>
        <dt>File ID</dt><dd>{snapshot.file_id}</dd>
        <dt>Root ID</dt><dd>{snapshot.root_id}</dd>
        <dt>SHA-256</dt><dd className="docx-preview-hash">{snapshot.sha256}</dd>
      </dl>
    </details>
  );
}

function PreviewText({ preview }: { preview: DocxPreview }) {
  const { summary, text } = preview.extraction;
  const status = { complete: "Complete", partial: "Partial", unsupported: "Unsupported", failed: "Failed" }[summary.status];
  const readable = summary.status === "complete" || summary.status === "partial";
  return (
    <div>
      <p role="status"><strong>{status}</strong>{summary.status === "partial" ? ": some content or guarantees are unavailable." : ""}</p>
      {summary.note && <p>{summary.note}</p>}
      {readable ? (
        <>
          <div className="docx-preview-warning" role="note" aria-label="Citation cache limitations">
            <p>{CACHE_WARNING}</p>
            <p>{FLATTENED_WARNING}</p>
          </div>
          {summary.covered.length > 0 && <Coverage title="Included" items={summary.covered} />}
          {summary.left_out.length > 0 && <Coverage title="Not covered" items={summary.left_out} />}
          {text.length === 0 ? <p>No body text was extracted.</p> : (
            <div className="docx-preview-body" role="region" aria-label="Extracted body text" tabIndex={0}>{text}</div>
          )}
        </>
      ) : <p>{summary.status === "unsupported" ? "This saved version cannot be previewed by the DOCX reader." : "Text extraction failed for this saved version."}</p>}
      <p className="docx-preview-context">Reader: {summary.reader}. Extracted characters: {summary.chars}.</p>
    </div>
  );
}

function Coverage({ title, items }: { title: string; items: string[] }) {
  return <div className="docx-preview-coverage"><strong>{title}</strong><ul>{items.map((item, index) => <li key={index}>{item}</li>)}</ul></div>;
}
