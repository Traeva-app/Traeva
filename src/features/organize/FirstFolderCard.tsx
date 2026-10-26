// Before any folder is added: what Traeva does with a folder, and a way to
// add one right where the user is (Find on the first start, Organize). The
// core shows its own folder picker and keeps the folder; the page then
// reads its state again.

import { useId, useState } from "react";
import petUrl from "../../assets/traeva-pet.png";
import { chooseRoot } from "./api";
import { ErrorDetails } from "./shared";
import { isAppError, type AppError } from "./types";

export function FirstFolderCard({ onAdded }: { onAdded: () => void }) {
  const [error, setError] = useState<AppError | null | undefined>(undefined);
  const [detail, setDetail] = useState("");
  // Find and Organize both stay mounted: each card needs its own id.
  const titleId = useId();
  const add = () => {
    setError(undefined);
    chooseRoot()
      .then((root) => {
        if (root !== null) onAdded();
      })
      .catch((reason: unknown) => {
        setError(isAppError(reason) ? reason : null);
        setDetail(String(reason));
      });
  };
  return (
    <section className="card first-folder" aria-labelledby={titleId}>
      <img src={petUrl} alt="" width={56} height={56} />
      <div>
        <h2 id={titleId}>Add a folder to start</h2>
        <p>
          Traeva looks only in the folders you add. It finds your files there and, with a model,
          suggests where new ones go. It never moves a file without your OK.
        </p>
        <button type="button" className="primary" onClick={add}>
          Add a folder…
        </button>
        <p className="muted small">You can add more folders, or remove one, in Settings.</p>
        {error !== undefined && (error ? <ErrorDetails error={error} /> : <p>{detail}</p>)}
      </div>
    </section>
  );
}
