// The folders Traeva may organize (organize-spec §2): add one with the
// core's own picker, or stop organizing one. Shown on the Settings page.

import { useState } from "react";
import { chooseRoot, revokeRoot } from "./api";
import { ErrorDetails } from "./shared";
import { isAppError, type AppError, type RootView } from "./types";
import { ShowMore } from "../../shared/ShowMore";

export function RootsCard({ roots, onChanged }: { roots: RootView[]; onChanged: () => void }) {
  const [error, setError] = useState<AppError | null>(null);

  const add = () => {
    setError(null);
    chooseRoot()
      .then(onChanged)
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null));
  };
  const remove = (root: RootView) => {
    if (!window.confirm(`Stop organizing ${root.display_path}? Its open previews end too.`)) return;
    revokeRoot(root.root_id)
      .then(onChanged)
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null));
  };

  return (
    <section className="card">
      <h2>Folders Traeva may organize</h2>
      {roots.length === 0 && <p className="muted">None yet. Add a folder to start.</p>}
      <ShowMore items={roots} first={8} keep={(root) => root.problem !== null}>
        {(visible) => (
          <ul className="list">
            {visible.map((root) => (
              <li key={root.root_id} className="row">
                <span className="mono grow">{root.display_path}</span>
                {root.problem && <span className="warn small">{root.problem.message}</span>}
                <button type="button" onClick={() => remove(root)}>
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </ShowMore>
      <button type="button" onClick={add}>
        Add folder…
      </button>
      {error && <ErrorDetails error={error} />}
    </section>
  );
}
