// One chosen file on its way to a plan: reading, the analysis, up to three
// questions (one at a time), the recommendation, the manual choice and
// failures (#16, #17; organize-spec §15).
// The card shows the core's operation view and sends the user's choices;
// the core decides every next stage. Whatever is picked here becomes a plan
// only through build_plan, and then still needs the preview and the
// confirmation: the model never moves anything.

import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  answerClarification,
  buildPlan,
  cancelOperation,
  chooseManualLocation,
  getOperation,
  listFolders,
  setImageSending,
  startAnalysis,
} from "./api";
import {
  answerFor,
  basename,
  canRetry,
  candidateFolderName,
  candidatePath,
  cardFor,
  chosenTargetPath,
  currentFolderIn,
  endedHint,
  evidenceText,
  inRoot,
  isCurrentPlace,
  likelyCandidate,
  manualReason,
  offeredCandidates,
  recommendedCandidate,
  scoreText,
  typedNewFolders,
} from "./flow";
import { folderHints } from "./folderTree";
import { FolderPicker } from "./FolderPicker";
import {
  ErrorDetails,
  ExtractionText,
  FileSummary,
  MoveRoute,
  STAGE_TITLES,
  type SetFlow,
} from "./shared";
import {
  isAppError,
  type AnsweredView,
  type AppError,
  type CandidateView,
  type FileView,
  type FolderCatalogView,
  type ImageSending,
  type OperationView,
  type PlanTarget,
  type QuestionView,
  type SuggestionView,
} from "./types";
import { hasImage, imageStep, rememberedMode } from "./imageSend";
import { ImageSendPrompt } from "./ImageSendPrompt";
import { folderTail, textTail } from "./compact";
import { Details } from "./Details";
import "./compact.css";

type Props = {
  operation: OperationView;
  /** The model is set up and the user agreed to send. */
  modelReady: boolean;
  setFlow: SetFlow;
  /** Shows a newer view of this operation (never an older one). */
  showOperation: (operation: OperationView) => void;
  /** The target folder's full path, to show where a file would go; null if
   * it is no longer authorized (the core then refuses any plan). */
  targetRootPath: string | null;
  /** Reads the setting for sending images (organize-spec §14.10). Only the
   * main window passes it: without it nothing but the text is sent and
   * nothing is asked (the pet's popup). */
  imageChoice?: () => Promise<ImageSending>;
  /** The pet's small popup (#37): each card shows its main lines and
   * buttons, paths cut to their last folders, and folds the rest into
   * Details. The main window leaves this out and shows the full cards. */
  compact?: boolean;
  /** With a recommendation, show the folder list instead (the popup goes
   * from its Move card to "Choose another folder" this way); `onBack`
   * returns to the recommendation. */
  chooseFolder?: boolean;
  onBack?: () => void;
};

/** The folder a candidate points to, inside the target folder (new folders
 * included); "" for the target folder itself. */
function candidateFolder(candidate: CandidateView): string {
  return [candidate.folder_path, ...candidate.new_segments].filter((part) => part !== "").join("/");
}

/** A folder's short place, with its full path in the tooltip (compact
 * cards); "new" when Traeva would make folders for it. */
export function ShortPlace(props: { label: string; short: string; full: string; made?: number }) {
  const made = props.made ?? 0;
  return (
    <p className="compact-place" title={props.full}>
      <span className="muted">{props.label}</span> <span className="mono exact">{props.short}</span>
      {made > 0 && (
        <span className="compact-new" title={made === 1 ? "Traeva makes this folder" : `Traeva makes ${made} folders`}>
          new
        </span>
      )}
    </p>
  );
}

export function OperationCard(props: Props) {
  const { operation, modelReady, setFlow, showOperation, targetRootPath, compact = false } = props;
  // A path inside the target folder, written out in full when it is known.
  const inTarget = (relative: string) =>
    targetRootPath === null ? relative : inRoot(targetRootPath, relative);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  // Bumped to list the folders again after a refused folder choice.
  const [listing, setListing] = useState(0);
  // The one place the model named is shown first; the folder list opens
  // when the user wants another folder.
  const [otherFolder, setOtherFolder] = useState(false);
  useEffect(() => setOtherFolder(false), [operation.operation_id]);
  const id = operation.operation_id;
  const file = operation.file;

  // One call at a time: a click while a call runs is ignored, so a double
  // click never makes a second plan that replaces the one about to be shown.
  const once = (call: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    call()
      .catch((reason: unknown) => setError(isAppError(reason) ? reason : null))
      .finally(() => setBusy(false));
  };
  // Runs a call that returns the operation's new view.
  const act = (call: () => Promise<OperationView>) => once(() => call().then(showOperation));
  const cancel = () => once(() => cancelOperation(id).then(() => setFlow({ step: "start" })));
  const manual = () => act(() => chooseManualLocation(id));

  // Suggest a folder and Try again. For a file the core read as an image,
  // the setting decides first: ask, send a smaller copy, or send the text
  // only (organize-spec §14.10). The core sends a copy only if it allows it.
  const [askImage, setAskImage] = useState(false);
  // Whether this analysis was asked with the image, for the Sending line.
  const [withImage, setWithImage] = useState(false);
  const analyze = () => {
    const choice = props.imageChoice;
    if (choice === undefined || file === null || !hasImage(file)) {
      setWithImage(false);
      act(() => startAnalysis(id));
      return;
    }
    once(() =>
      choice().then((mode) => {
        const step = imageStep(mode, file);
        if (step === "ask") {
          setAskImage(true);
          return;
        }
        setWithImage(step === "send");
        return startAnalysis(id, step === "send" ? true : undefined).then(showOperation);
      }),
    );
  };
  const answerImage = (send: boolean, remember: boolean) =>
    once(async () => {
      if (remember) await setImageSending(rememberedMode(send));
      setAskImage(false);
      setWithImage(send);
      showOperation(await startAnalysis(id, send));
    });
  const imagePrompt = askImage && file !== null && (
    <ImageSendPrompt
      textUsable={file.extraction.usable_text}
      busy={busy}
      onChoose={answerImage}
      onManual={() => {
        setAskImage(false);
        manual();
      }}
    />
  );

  // A target becomes a plan; then the preview. A refused plan leaves the
  // card where it is, and the operation is read again (a folder picked
  // during an analysis already made it a manual choice).
  const plan = (target: PlanTarget, candidate: CandidateView | null) => {
    if (file === null) return;
    once(() =>
      buildPlan({
        file_id: file.file_id,
        file_revision: file.file_revision,
        target_root_id: operation.target_root_id,
        operation_id: id,
        target,
      })
        .then((outcome) => {
          if (outcome.outcome === "planned") {
            setFlow({ step: "preview", plan: outcome.plan, context: { operation, candidate } });
          } else {
            setFlow({ step: "in_place", file, operation });
          }
        })
        .catch((reason: unknown) => {
          // An outdated list, or a name taken by a folder made since the
          // list, is listed again; other refusals keep the user's choice on
          // screen to correct it.
          const code = isAppError(reason) ? reason.code : null;
          if (code === "folder_list_outdated" || code === "target_conflict") {
            setListing((count) => count + 1);
          }
          getOperation(id).then(showOperation).catch(() => {});
          throw reason;
        }),
    );
  };
  const chooseCandidate = (candidate: CandidateView) =>
    plan({ kind: "candidate", candidate_id: candidate.candidate_id }, candidate);

  // The pet's popup goes from a suggestion straight to its Move card
  // (organize-spec §6, v0.13): the suggested place's plan is built at once,
  // once for each suggestion. A refused plan leaves the suggestion's card,
  // with the core's reason and Try again.
  const autoCandidate =
    compact && !props.chooseFolder && cardFor(operation.stage) === "recommendation"
      ? recommendedCandidate(operation.suggestion)
      : null;
  const autoKey = autoCandidate === null ? null : `${id}/${autoCandidate.candidate_id}`;
  const autoTried = useRef<string | null>(null);
  useEffect(() => {
    if (autoKey === null || autoCandidate === null || autoTried.current === autoKey) return;
    autoTried.current = autoKey;
    chooseCandidate(autoCandidate);
  }, [autoKey]);
  const preparing = autoKey !== null && (autoTried.current !== autoKey || busy);

  // In the popup, Cancel is the window's own small link, under the card.
  const cancelButton = compact ? null : (
    <button type="button" onClick={cancel}>
      Cancel
    </button>
  );
  // Each step has one amber button: its usual next step.
  const manualButton = (primary: boolean, label = "Pick the folder myself") => (
    <button type="button" className={primary ? "primary" : undefined} onClick={manual}>
      {label}
    </button>
  );
  const errorBox = error && <ErrorDetails error={error} />;

  switch (cardFor(operation.stage)) {
    case "reading":
      if (compact) {
        return (
          <section className="card">
            <h2>Reading the file…</h2>
            {cancelButton}
            {errorBox}
          </section>
        );
      }
      return (
        <section className="card">
          <h2>Reading the file…</h2>
          <p className="muted">
            Traeva reads it twice with a short pause to be sure it is not still being saved. Nothing
            is written in your folders.
          </p>
          {cancelButton}
          {errorBox}
        </section>
      );

    case "ready":
      if (compact) {
        return (
          <section className="card">
            <h2>Where should it go?</h2>
            {imagePrompt || (
              <div className="row">
                <button
                  type="button"
                  className={modelReady ? "primary" : undefined}
                  onClick={analyze}
                  disabled={!modelReady}
                >
                  Suggest a folder
                </button>
                {manualButton(!modelReady)}
              </div>
            )}
            {!modelReady && (
              <p className="muted small">To get a suggestion, set up the model in Traeva's Settings.</p>
            )}
            <Details>
              {file && <FileSummary file={file} />}
            </Details>
            {errorBox}
          </section>
        );
      }
      return (
        <section className="card">
          <h2>Where should it go?</h2>
          {file && <FileSummary file={file} />}
          {imagePrompt || (
            <div className="row">
              <button
                type="button"
                className={modelReady ? "primary" : undefined}
                onClick={analyze}
                disabled={!modelReady}
              >
                Suggest a folder
              </button>
              {manualButton(!modelReady)}
              {cancelButton}
            </div>
          )}
          {imagePrompt && <div className="row">{cancelButton}</div>}
          {!modelReady && (
            <p className="muted small">
              To get a suggestion, set up the model in Settings and allow sending.
            </p>
          )}
          {errorBox}
        </section>
      );

    case "analyzing":
      if (compact) {
        return (
          <section className="card">
            <h2>Asking the model…</h2>
            <div className="row">{manualButton(false)}</div>
            <Details>
              <AnsweredList answered={operation.answered} />
              <p className="muted small">
                Sending: the file name, the path of its folder inside the folder you chose it from, the
                text excerpt, the folder paths inside the target folder
                {operation.answered.length === 1 ? " and your answer" : ""}
                {operation.answered.length > 1 ? " and your answers" : ""}. Nothing is moved by this.
              </p>
            </Details>
            {errorBox}
          </section>
        );
      }
      return (
        <section className="card">
          <h2>Asking the model…</h2>
          {file && <p className="mono">{file.display_path}</p>}
          <AnsweredList answered={operation.answered} />
          <p className="muted small">
            Sending: the file name, the path of its folder inside the folder you chose it from, the
            text excerpt, the folder paths inside the target folder
            {withImage ? ", a smaller copy of the image when one can be made" : ""}
            {operation.answered.length === 1 ? " and your answer" : ""}
            {operation.answered.length > 1 ? " and your answers" : ""}. Nothing is moved by this.
          </p>
          <div className="row">
            {manualButton(false)}
            {cancelButton}
          </div>
          {errorBox}
        </section>
      );

    case "question": {
      const question = operation.question;
      if (question === null) return null;
      return (
        <QuestionCard
          question={question}
          answered={operation.answered}
          file={file}
          compact={compact}
          onAnswer={(optionId, text) => {
            const answer = answerFor(question, optionId, text);
            if (answer) act(() => answerClarification(id, answer));
          }}
          footer={
            <>
              <div className="row">
                {manualButton(
                  false,
                  question.options.length > 0
                    ? "None of these: pick the folder myself"
                    : "Pick the folder myself",
                )}
                {cancelButton}
              </div>
              {errorBox}
            </>
          }
        />
      );
    }

    case "recommendation":
      return (
        <RecommendationCard
          operation={operation}
          listing={listing}
          targetRootPath={targetRootPath}
          inTarget={inTarget}
          onAccept={chooseCandidate}
          onFolder={(target) => plan(target, null)}
          cancel={cancelButton}
          footer={errorBox}
          compact={compact}
          chooseFolder={props.chooseFolder ?? false}
          onBack={props.onBack}
          preparing={preparing}
        />
      );

    case "manual": {
      // The model named one place but did not recommend it: that place
      // first, with why not, and the folder list only on request.
      const likely = file === null ? null : likelyCandidate(operation);
      if (likely !== null && file !== null && compact) {
        const rootName = targetRootPath?.split("/").pop() ?? "";
        const name = candidateFolderName(likely, rootName);
        const folder = candidateFolder(likely);
        return (
          <section className="card">
            <h2 className="exact">Probably: {name}</h2>
            <ShortPlace
              label="To"
              short={folderTail(targetRootPath, folder)}
              full={inTarget(folder)}
              made={likely.new_segments.length}
            />
            <div className="row">
              <button
                type="button"
                className={otherFolder ? undefined : "primary"}
                onClick={() => chooseCandidate(likely)}
              >
                Use {name}
              </button>
              {!otherFolder && (
                <button type="button" onClick={() => setOtherFolder(true)}>
                  Choose another folder
                </button>
              )}
            </div>
            {otherFolder && (
              <FolderChooser
                key={listing}
                targetRootId={operation.target_root_id}
                targetRootPath={targetRootPath}
                file={file}
                hints={folderHints(operation.suggestion)}
                onChosen={(target) => plan(target, null)}
                primary
                compact
              />
            )}
            <Details>
              <p className="small">Not sure enough to recommend it: {likely.reason}</p>
              <FileSummary file={file} />
              {operation.suggestion && (
                <p className="muted small">About the file: {operation.suggestion.summary}</p>
              )}
            </Details>
            {errorBox}
          </section>
        );
      }
      if (likely !== null && file !== null) {
        const rootName = targetRootPath?.split("/").pop() ?? "";
        const name = candidateFolderName(likely, rootName);
        return (
          <section className="card">
            <h2 className="exact">Probably: {name}</h2>
            <p className="mono exact">{inTarget(candidatePath(likely, basename(file)))}</p>
            {likely.new_segments.length > 0 && (
              <p className="muted small">New folder: {likely.new_segments.join(" / ")}</p>
            )}
            <p>Not sure enough to recommend it: {likely.reason}</p>
            <div className="row">
              <button
                type="button"
                className={otherFolder ? undefined : "primary"}
                onClick={() => chooseCandidate(likely)}
              >
                Use {name}
              </button>
              {!otherFolder && (
                <button type="button" onClick={() => setOtherFolder(true)}>
                  Choose another folder
                </button>
              )}
              {!otherFolder && cancelButton}
            </div>
            {otherFolder && (
              <FolderChooser
                key={listing}
                targetRootId={operation.target_root_id}
                targetRootPath={targetRootPath}
                file={file}
                hints={folderHints(operation.suggestion)}
                onChosen={(target) => plan(target, null)}
                extra={cancelButton}
                primary
              />
            )}
            <FileSummary file={file} />
            {operation.suggestion && (
              <p className="muted small">About the file: {operation.suggestion.summary}</p>
            )}
            {errorBox}
          </section>
        );
      }
      if (compact) {
        const offered = file === null ? [] : offeredCandidates(operation.suggestion);
        return (
          <section className="card">
            <h2>Pick its folder</h2>
            {offered.length > 0 && (
              <ul className="list compact-places">
                {offered.map((candidate) => {
                  const folder = candidateFolder(candidate);
                  return (
                    <li key={candidate.candidate_id} className="row">
                      <span className="grow mono exact" title={inTarget(folder)}>
                        {folderTail(targetRootPath, folder)}
                      </span>
                      <button type="button" onClick={() => chooseCandidate(candidate)}>
                        Use this
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            <FolderChooser
              key={listing}
              targetRootId={operation.target_root_id}
              targetRootPath={targetRootPath}
              file={file}
              hints={folderHints(operation.suggestion)}
              onChosen={(target) => plan(target, null)}
              primary
              compact
            />
            <Details>
              {manualReason(operation) && <p className="muted small">{manualReason(operation)}</p>}
              {offered.length > 0 && (
                <>
                  <p className="small">The model mentioned these places. None is recommended; use one only if it fits.</p>
                  <ul className="list small">
                    {offered.map((candidate) => (
                      <li key={candidate.candidate_id}>
                        <span className="mono exact">{folderTail(targetRootPath, candidateFolder(candidate))}</span>:{" "}
                        {candidate.reason}
                      </li>
                    ))}
                  </ul>
                </>
              )}
              {file && <FileSummary file={file} />}
              {operation.suggestion && (
                <p className="muted small">About the file: {operation.suggestion.summary}</p>
              )}
            </Details>
            {errorBox}
          </section>
        );
      }
      return (
        <section className="card">
          <h2>Pick its folder</h2>
          {file && <FileSummary file={file} />}
          {manualReason(operation) && <p className="muted small">{manualReason(operation)}</p>}
          {operation.suggestion && (
            <p className="muted small">About the file: {operation.suggestion.summary}</p>
          )}
          {file && offeredCandidates(operation.suggestion).length > 0 && (
            <>
              <p>
                The model mentioned these places. None is recommended; use one only if it fits.
              </p>
              <ul className="list">
                {offeredCandidates(operation.suggestion).map((candidate) => (
                  <li key={candidate.candidate_id} className="row">
                    <span className="grow">
                      <span className="mono">
                        {inTarget(candidatePath(candidate, basename(file)))}
                      </span>
                      <br />
                      <span className="small muted">{candidate.reason}</span>
                    </span>
                    <button type="button" onClick={() => chooseCandidate(candidate)}>
                      Use this
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
          <FolderChooser
            key={listing}
            targetRootId={operation.target_root_id}
            targetRootPath={targetRootPath}
            file={file}
            hints={folderHints(operation.suggestion)}
            onChosen={(target) => plan(target, null)}
            extra={cancelButton}
            primary
          />
          {errorBox}
        </section>
      );
    }

    case "analysis_failed":
      return (
        <section className="card card-error">
          <h2>{STAGE_TITLES[operation.stage]}</h2>
          {operation.error && <ErrorDetails error={operation.error} />}
          <p className="muted small">Nothing was moved.</p>
          {imagePrompt || (
            <div className="row">
              {canRetry(operation.error) && (
                <button type="button" className="primary" onClick={analyze}>
                  Try again
                </button>
              )}
              {manualButton(!canRetry(operation.error))}
              {cancelButton}
            </div>
          )}
          {imagePrompt && <div className="row">{cancelButton}</div>}
          {errorBox}
        </section>
      );

    case "ended":
      return (
        <section className={`card ${operation.stage === "cancelled" ? "" : "card-error"}`}>
          <h2>{STAGE_TITLES[operation.stage]}</h2>
          {operation.error && <ErrorDetails error={operation.error} />}
          {endedHint(operation) && <p>{endedHint(operation)}</p>}
          <button type="button" className="primary" onClick={() => setFlow({ step: "start" })}>
            Start over
          </button>
        </section>
      );
  }
}

// ---------------------------------------------------------------------------
// The questions: up to `max_questions`, one at a time
// ---------------------------------------------------------------------------

/** The questions answered so far, oldest first. */
function AnsweredList(props: { answered: AnsweredView[] }) {
  if (props.answered.length === 0) return null;
  return (
    <ol className="list small">
      {props.answered.map((answered) => (
        <li key={answered.question_id}>
          <span className="muted">{answered.question}</span>
          <br />
          Your answer: <strong>{answered.answer}</strong>
        </li>
      ))}
    </ol>
  );
}

type QuestionCardProps = {
  question: QuestionView;
  /** The questions answered before this one, oldest first. */
  answered?: AnsweredView[];
  file: FileView | null;
  onAnswer: (optionId: string | null, text: string) => void;
  footer: ReactNode;
  /** The answer is on its way: the card stays, with what was chosen or typed. */
  sending?: boolean;
  /** Send answer is the screen's amber button (not while another one is). */
  primary?: boolean;
  /** The pet's small popup: the question and its choices first, paths cut
   * to their last folders, the rest in Details. */
  compact?: boolean;
};

/** The current question. A new question starts with nothing chosen or
 * typed, so an option picked for the last one is never sent for this one. */
export function QuestionCard(props: QuestionCardProps) {
  return <QuestionForm key={props.question.question_id} {...props} />;
}

function QuestionForm(props: QuestionCardProps) {
  const { question } = props;
  const [optionId, setOptionId] = useState<string | null>(null);
  const [text, setText] = useState("");
  const ready = optionId !== null || text.trim() !== "";
  const earlier = (props.answered ?? []).filter(
    (answered) => answered.question_id !== question.question_id,
  );
  const last = question.number >= question.max_questions;
  const rule = last
    ? "Your answer is final once sent. This is the last question: then Traeva suggests a folder, or you pick it."
    : `Your answer is final once sent. Traeva asks the model again, with all your answers; it asks at most ${question.max_questions} questions in all, one at a time.`;

  if (props.compact) {
    const ownWords = (
      <label className="field">
        {question.options.length > 0 ? "Or in your own words" : "Your answer"}
        <input
          type="text"
          maxLength={500}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setOptionId(null);
          }}
        />
      </label>
    );
    return (
      <section className="card">
        <h2>
          Question {question.number} of up to {question.max_questions}
        </h2>
        <p>
          <strong>{question.question}</strong>
        </p>
        {question.options.length > 0 && (
          <fieldset className="field">
            {question.options.map((option) => (
              <label key={option.option_id} className="row" title={option.text}>
                <input
                  type="radio"
                  name={`answer-${question.question_id}`}
                  checked={optionId === option.option_id}
                  onChange={() => setOptionId(option.option_id)}
                />
                {textTail(option.text)}
              </label>
            ))}
          </fieldset>
        )}
        {question.options.length === 0 && ownWords}
        <div className="row">
          <button
            type="button"
            className={props.primary === false ? undefined : "primary"}
            disabled={!ready || props.sending}
            onClick={() => props.onAnswer(optionId, text)}
          >
            {props.sending ? "Sending…" : "Send answer"}
          </button>
          {/* Its other buttons (Cancel this file) on the same line. */}
          {props.footer}
        </div>
        <Details>
          {question.options.length > 0 && ownWords}
          <AnsweredList answered={earlier} />
          <p className="muted small">About: {FIELD_TEXT[question.uncertain_field]}</p>
          <p className="muted small">{rule}</p>
        </Details>
      </section>
    );
  }

  return (
    <section className="card">
      <h2>
        Question {question.number} of up to {question.max_questions}
      </h2>
      {props.file && <p className="mono">{props.file.display_path}</p>}
      <AnsweredList answered={earlier} />
      <p>
        <strong>{question.question}</strong>
      </p>
      <p className="muted small">About: {FIELD_TEXT[question.uncertain_field]}</p>
      {question.options.length > 0 && (
        <fieldset className="field">
          {question.options.map((option) => (
            <label key={option.option_id} className="row">
              <input
                type="radio"
                name={`answer-${question.question_id}`}
                checked={optionId === option.option_id}
                onChange={() => setOptionId(option.option_id)}
              />
              {option.text}
            </label>
          ))}
        </fieldset>
      )}
      <label className="field">
        {question.options.length > 0 ? "Or in your own words" : "Your answer"}
        <input
          type="text"
          maxLength={500}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setOptionId(null);
          }}
        />
      </label>
      <p className="muted small">{rule}</p>
      <button
        type="button"
        className={props.primary === false ? undefined : "primary"}
        disabled={!ready || props.sending}
        onClick={() => props.onAnswer(optionId, text)}
      >
        {props.sending ? "Sending…" : "Send answer"}
      </button>
      {props.footer}
    </section>
  );
}

const FIELD_TEXT = {
  topic: "what the file is about",
  purpose: "what the file is for",
  destination: "where you keep such files",
} as const;

// ---------------------------------------------------------------------------
// The recommendation
// ---------------------------------------------------------------------------

function RecommendationCard(props: {
  operation: OperationView;
  listing: number;
  targetRootPath: string | null;
  inTarget: (relative: string) => string;
  onAccept: (candidate: CandidateView) => void;
  onFolder: (target: PlanTarget) => void;
  cancel: ReactNode;
  footer: ReactNode;
  compact: boolean;
  chooseFolder: boolean;
  onBack?: () => void;
  /** The popup builds the suggested place's plan at once: while it does. */
  preparing: boolean;
}) {
  const { operation } = props;
  const [choosing, setChoosing] = useState(false);
  const suggestion = operation.suggestion;
  const candidate = recommendedCandidate(suggestion);
  const file = operation.file;
  if (suggestion === null || candidate === null || file === null) return null;

  if (props.compact && props.chooseFolder) {
    return (
      <section className="card">
        <FolderChooser
          key={props.listing}
          targetRootId={operation.target_root_id}
          targetRootPath={props.targetRootPath}
          file={file}
          hints={folderHints(suggestion)}
          onChosen={props.onFolder}
          primary
          compact
        />
        {props.onBack && (
          <div className="row">
            <button type="button" onClick={props.onBack}>
              Back to the suggestion
            </button>
          </div>
        )}
        {props.footer}
      </section>
    );
  }
  if (props.compact && props.preparing) {
    return (
      <section className="card">
        <h2>Getting the move ready…</h2>
        <p className="muted small">Nothing has changed in your folders.</p>
      </section>
    );
  }
  if (props.compact) {
    // Only after the suggested place's plan was refused (the reason is in
    // the footer): Try again builds a new one.
    const folder = candidateFolder(candidate);
    return (
      <section className="card">
        <h2>Suggested folder</h2>
        <ShortPlace
          label="To"
          short={folderTail(props.targetRootPath, folder)}
          full={props.inTarget(folder)}
          made={candidate.new_segments.length}
        />
        {isCurrentPlace(candidate, file, operation.target_root_id) && (
          <p className="warn">This is the folder the file is in now: there is nothing to move.</p>
        )}
        <div className="row">
          <button type="button" className={choosing ? undefined : "primary"} onClick={() => props.onAccept(candidate)}>
            Try again
          </button>
          <button type="button" onClick={() => setChoosing(!choosing)}>
            {choosing ? "Hide the folder list" : "Choose another folder"}
          </button>
        </div>
        {props.footer}
        {choosing && (
          <FolderChooser
            key={props.listing}
            targetRootId={operation.target_root_id}
            targetRootPath={props.targetRootPath}
            file={file}
            hints={folderHints(suggestion)}
            onChosen={props.onFolder}
            primary
            compact
          />
        )}
        <Details>
          <RecommendationFacts candidate={candidate} suggestion={suggestion} file={file} />
          <p className="muted small">Nothing has changed in your folders.</p>
        </Details>
      </section>
    );
  }

  return (
    <section className="card">
      <h2>Suggested folder</h2>
      <MoveRoute
        name={basename(file)}
        from={file.display_path}
        to={props.inTarget(candidatePath(candidate, basename(file)))}
        newFolders={candidate.new_segments.length > 0 ? [candidate.new_segments.join(" / ")] : []}
      />
      <RecommendationFacts candidate={candidate} suggestion={suggestion} file={file} />
      {isCurrentPlace(candidate, file, operation.target_root_id) && (
        <p className="warn">This is the folder the file is in now: there is nothing to move.</p>
      )}
      <p className="muted small">Nothing has changed in your folders. The next step is a preview.</p>
      <div className="row">
        <button type="button" className="primary" onClick={() => props.onAccept(candidate)}>
          Preview this move
        </button>
        <button type="button" onClick={() => setChoosing(!choosing)}>
          {choosing ? "Hide the folder list" : "Choose another folder"}
        </button>
        {props.cancel}
      </div>
      {choosing && (
        <FolderChooser
          key={props.listing}
          targetRootId={operation.target_root_id}
          targetRootPath={props.targetRootPath}
          file={file}
          hints={folderHints(suggestion)}
          onChosen={props.onFolder}
        />
      )}
      {props.footer}
    </section>
  );
}

/** Why the model suggests a place, and what it read: the recommendation's
 * facts (organize-spec §6). The popup's Move card shows them in Details. */
export function RecommendationFacts(props: { candidate: CandidateView; suggestion: SuggestionView; file: FileView }) {
  const { candidate, suggestion, file } = props;
  return (
    <dl>
      <dt>Why</dt>
      <dd>{candidate.reason}</dd>
      <dt>Based on</dt>
      <dd className="small">{evidenceText(candidate.evidence_ids)}</dd>
      <dt>About the file</dt>
      <dd className="small">{suggestion.summary}</dd>
      <dt>Score</dt>
      <dd className="small">
        {scoreText(suggestion.confidence)} <span className="muted">(a score, not an accuracy)</span>
      </dd>
      <dt>Text read</dt>
      <dd className="small">
        <ExtractionText file={file} />
      </dd>
    </dl>
  );
}

// ---------------------------------------------------------------------------
// Picking a folder by hand
// ---------------------------------------------------------------------------

/** The folders of the target root, a few likely ones first and then one
 * level at a time (FolderPicker), and up to 3 new folders inside the chosen
 * one. The plan itself is made by the caller; a new `key` lists the folders
 * again. `extra` buttons sit next to Preview.
 *
 * No folder is chosen at first (#80): the list is new for every file and
 * every listing, so nothing from another file or an earlier list carries
 * over, and Preview waits until the user picks one. The folder the file is
 * in now is marked, and the full target is shown as it would be. */
function FolderChooser(props: {
  targetRootId: string;
  targetRootPath: string | null;
  /** The file being organized: its name and current folder are shown. */
  file: FileView | null;
  /** The places the model named, in the order they count (folderHints). */
  hints: string[];
  onChosen: (target: PlanTarget) => void;
  extra?: ReactNode;
  /** Preview this folder is the step's amber button (picking by hand). */
  primary?: boolean;
  /** The pet's popup: the folder list and Preview first; new folders and
   * List again in Details; the target as its last folders. */
  compact?: boolean;
}) {
  const { targetRootId, onChosen } = props;
  const rootName = props.targetRootPath?.split("/").pop() ?? "";
  const [catalog, setCatalog] = useState<FolderCatalogView | null>(null);
  const [failed, setFailed] = useState<{ error: AppError | null; detail: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [folderId, setFolderId] = useState("");
  const [newFolders, setNewFolders] = useState(["", "", ""]);

  useEffect(() => {
    setFailed(null);
    listFolders(targetRootId)
      .then((listed) => {
        setCatalog(listed);
        setFolderId("");
      })
      .catch((reason: unknown) =>
        setFailed({ error: isAppError(reason) ? reason : null, detail: String(reason) }),
      );
  }, [targetRootId, attempt]);

  if (failed !== null) {
    return (
      <>
        <p>The folders could not be listed.</p>
        {failed.error ? <ErrorDetails error={failed.error} /> : <p>{failed.detail}</p>}
        <div className="row">
          <button type="button" onClick={() => setAttempt(attempt + 1)}>
            List the folders again
          </button>
          {props.extra}
        </div>
      </>
    );
  }
  if (catalog === null) {
    return (
      <>
        <p className="muted">Listing folders…</p>
        {props.extra}
      </>
    );
  }
  const listAgain = (
    <button type="button" onClick={() => setAttempt(attempt + 1)}>
      List again
    </button>
  );
  const here = props.file ? currentFolderIn(props.file, targetRootId) : null;
  const chosen = catalog.folders.find((folder) => folder.folder_id === folderId) ?? null;
  // Shown and sent alike; the core refuses a bad name (#93).
  const typed = typedNewFolders(newFolders);
  const newFolderFields = (
    <fieldset className="field">
      <legend>New folders inside it (optional, outermost first)</legend>
      {newFolders.map((name, index) => (
        <input
          key={index}
          type="text"
          value={name}
          placeholder={`Level ${index + 1}`}
          onChange={(event) =>
            setNewFolders(newFolders.map((old, i) => (i === index ? event.target.value : old)))
          }
        />
      ))}
    </fieldset>
  );
  const previewButton = (
    <button
      type="button"
      className={props.primary ? "primary" : undefined}
      disabled={folderId === ""}
      onClick={() =>
        onChosen({
          kind: "folder",
          catalog_id: catalog.catalog_id,
          folder_id: folderId,
          new_folders: typed,
        })
      }
    >
      Preview this folder
    </button>
  );
  if (props.compact) {
    const folder = chosen === null ? null : [chosen.relative_path, ...typed].filter((part) => part !== "").join("/");
    return (
      <>
        <FolderPicker
          key={catalog.catalog_id}
          catalog={catalog}
          rootName={rootName}
          hints={props.hints}
          here={here}
          chosenId={folderId}
          onChoose={setFolderId}
          compact
        />
        {folder !== null && props.targetRootPath && (
          <ShortPlace
            label="To"
            short={folderTail(props.targetRootPath, folder)}
            full={inRoot(props.targetRootPath, folder)}
          />
        )}
        <div className="row">
          {previewButton}
          {props.extra}
        </div>
        <Details label="New folder or list again">
          {newFolderFields}
          {catalog.truncated && <p className="muted small">Only the first 500 folders, 6 levels deep, are listed.</p>}
          <div className="row">{listAgain}</div>
        </Details>
      </>
    );
  }
  return (
    <>
      <FolderPicker
        key={catalog.catalog_id}
        catalog={catalog}
        rootName={rootName}
        hints={props.hints}
        here={here}
        chosenId={folderId}
        onChoose={setFolderId}
      />
      {catalog.truncated && (
        <p className="muted small">Only the first 500 folders, 6 levels deep, are listed.</p>
      )}
      {newFolderFields}
      {chosen && props.file && props.targetRootPath && (
        <p className="small">
          Would go to{" "}
          <span className="mono exact">
            {chosenTargetPath(
              props.targetRootPath,
              chosen.relative_path,
              typed,
              basename(props.file),
            )}
          </span>
        </p>
      )}
      <div className="row">
        {previewButton}
        {listAgain}
        {props.extra}
      </div>
    </>
  );
}
