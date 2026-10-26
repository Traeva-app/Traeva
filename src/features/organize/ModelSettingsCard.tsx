// The model setup: provider and model, the API key, and the user's consent
// to send (organize-spec.md §5, §9). The key is typed here once and handed
// to the core, which keeps it in the Keychain; the page forgets it right
// after saving and never shows a saved key. Consent to send never allows a
// move: every move still needs its own preview and confirmation. Whether
// the model may also look at a smaller copy of an image is a choice of its
// own (organize-spec §14.10, the student's decision 10/14), off at first.

import { useEffect, useState } from "react";
import { getModelSettings, saveModelSettings, setAnalysisConsent, setImageSending } from "./api";
import { ErrorDetails } from "./shared";
import { isAppError, type AppError, type ImageSending, type ModelSettingsView } from "./types";

/** The one provider Traeva supports in this release. */
const PROVIDER = "openai";

export function ModelSettingsCard({ onChanged }: { onChanged: () => void }) {
  const [settings, setSettings] = useState<ModelSettingsView | null>(null);
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () => {
    setError(null);
    getModelSettings()
      .then((loaded) => {
        setSettings(loaded);
        setModel(loaded.model ?? "");
      })
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null));
  };
  useEffect(load, []);

  // After a save the field shows the saved name; a consent change leaves
  // what the user typed alone.
  const run = (call: Promise<ModelSettingsView>, showSavedModel: boolean) => {
    setError(null);
    setBusy(true);
    call
      .then((saved) => {
        setSettings(saved);
        if (showSavedModel) setModel(saved.model ?? "");
        onChanged();
      })
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null))
      .finally(() => setBusy(false));
  };

  const save = () => {
    const key = apiKey.trim();
    // Forget the key in the page at once, whatever the result.
    setApiKey("");
    run(
      saveModelSettings({
        provider: PROVIDER,
        model: model.trim(),
        ...(key === "" ? {} : { api_key: key }),
      }),
      true,
    );
  };

  // Consent is for the saved model; an edited name is saved first.
  const edited = settings !== null && model.trim() !== (settings.model ?? "");

  return (
    <section className="card">
      <h2>Model</h2>
      {settings === null ? (
        // Unreadable settings can still be saved again below.
        error ? (
          <button type="button" onClick={load}>
            Read the settings again
          </button>
        ) : (
          <p className="muted">Reading the settings…</p>
        )
      ) : (
        <p>
          <span className={`badge ${settings.status === "configured" ? "card-ok" : ""}`}>
            {STATUS_TEXT[settings.status]}
          </span>{" "}
          {settings.provider && settings.model && (
            <span className="mono small">
              {settings.provider} · {settings.model}
            </span>
          )}
        </p>
      )}

      <label className="field">
        Model
        <input
          type="text"
          value={model}
          placeholder="type the model name, for example gpt-6-luna"
          onChange={(event) => setModel(event.target.value)}
        />
      </label>
      <label className="field">
        OpenAI API key
        <input
          type="password"
          value={apiKey}
          autoComplete="off"
          placeholder={settings?.key_saved ? "saved in the Keychain; leave empty to keep it" : ""}
          onChange={(event) => setApiKey(event.target.value)}
        />
      </label>
      {settings && (
        <p className="muted small">
          The key is kept in the Keychain as <span className="mono">{settings.key_account}</span>.
        </p>
      )}
      <button type="button" onClick={save} disabled={busy || model.trim() === ""}>
        Save
      </button>
      {model.trim() === "" && <p className="muted small">Type the model name to save.</p>}

      {settings?.key_saved && (
        <>
          <p className="small">{settings.notice}</p>
          {settings.status === "configured" ? (
            <button type="button" onClick={() => run(setAnalysisConsent(false), false)} disabled={busy}>
              Stop sending to the model
            </button>
          ) : (
            <button
              type="button"
              onClick={() => run(setAnalysisConsent(true), false)}
              disabled={busy || edited}
            >
              Allow sending to the model
            </button>
          )}
          {edited && <p className="warn small">Save the model name first.</p>}
          <p className="muted small">
            Sending lets Traeva ask the model for suggestions. The model never moves anything. Every
            move is shown first and waits for your confirmation, except files you drop on Traeva
            while Organize dropped files automatically is on, which has its own card and agreement.
          </p>
        </>
      )}
      {settings && (
        <ImageSendingChoice
          mode={settings.image_sending}
          notice={settings.image_notice}
          // The core keeps it off until sending to the model is allowed.
          allowed={settings.status === "configured"}
          busy={busy}
          onChoose={(mode) => run(setImageSending(mode), false)}
        />
      )}
      {error && <ErrorDetails error={error} />}
    </section>
  );
}

const IMAGE_MODES: { mode: ImageSending; label: string }[] = [
  { mode: "off", label: "Off" },
  { mode: "ask_each_time", label: "Ask each time" },
  { mode: "without_asking", label: "Don't ask" },
];

/** Whether the model may look at a smaller copy of an image: off, asked
 * each time in Organize, or without asking. What it means (the core's
 * notice) shows under the choice. */
function ImageSendingChoice(props: {
  mode: ImageSending;
  notice: string;
  allowed: boolean;
  busy: boolean;
  onChoose: (mode: ImageSending) => void;
}) {
  return (
    <fieldset className="field" disabled={props.busy || !props.allowed}>
      <legend>Let the model look at images</legend>
      <div className="row">
        {IMAGE_MODES.map(({ mode, label }) => (
          <label key={mode} className="row">
            <input
              type="radio"
              name="image-sending"
              checked={props.mode === mode}
              onChange={() => props.onChoose(mode)}
            />
            {label}
          </label>
        ))}
      </div>
      <p className="small">{props.notice}</p>
      {!props.allowed && (
        <p className="muted small">Allow sending to the model first. Until then no image is sent.</p>
      )}
    </fieldset>
  );
}

const STATUS_TEXT = {
  not_configured: "Not set up",
  consent_required: "Waiting for your consent",
  configured: "Ready to suggest folders",
} as const;
