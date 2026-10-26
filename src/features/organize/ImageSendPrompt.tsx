// The question before Suggest a folder, when the setting asks each time
// (organize-spec §14.10): send a smaller copy of the image to the model, or
// use the recognized text only. "Don't ask again" keeps the choice made
// here as the setting (Settings → Model changes it back). An image with no
// text to send offers picking the folder instead, since the text alone
// would be refused.

import { useId, useState } from "react";

export function ImageSendPrompt(props: {
  /** The file has text the model may read without the image. */
  textUsable: boolean;
  busy: boolean;
  /** `send`: send the image; `remember`: keep this choice from now on. */
  onChoose: (send: boolean, remember: boolean) => void;
  onManual: () => void;
}) {
  const [remember, setRemember] = useState(false);
  const titleId = useId();
  return (
    <div className="image-ask" role="group" aria-labelledby={titleId}>
      <p id={titleId}>
        Send a smaller copy of this image to OpenAI so the model can look at it? It may show faces, documents or
        places.
        {!props.textUsable && " Traeva found no text in it to send instead."}
      </p>
      <label className="row small">
        <input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />
        Don't ask again
      </label>
      <div className="row">
        <button type="button" className="primary" disabled={props.busy} onClick={() => props.onChoose(true, remember)}>
          Send image
        </button>
        {props.textUsable ? (
          <button type="button" disabled={props.busy} onClick={() => props.onChoose(false, remember)}>
            Use recognized text only
          </button>
        ) : (
          <button type="button" disabled={props.busy} onClick={props.onManual}>
            Pick the folder myself
          </button>
        )}
      </div>
    </div>
  );
}
