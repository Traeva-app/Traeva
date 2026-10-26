// The first-start guide (#248), shown on the Find page instead of the
// search box and the home: add a folder, then the model and what may be
// sent to it, then the automatic mode for dropped files, then what is set
// now. Each step is the card the Settings page shows, unchanged, with its
// own commands: the guide adds no consent of its own and decides nothing.
// Every step can be skipped, Skip setup ends the guide, and everything can
// be changed later in Settings.

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { getAutoDropSettings } from "../autodrop/api";
import { AutoDropCard } from "../autodrop/AutoDropCard";
import { modeStatus, type ModeStatus } from "../autodrop/queue";
import { FirstFolderCard } from "../organize/FirstFolderCard";
import { ModelSettingsCard } from "../organize/ModelSettingsCard";
import type { AppState } from "../organize/types";
import {
  autoModeText,
  continueNote,
  flagToSave,
  foldersText,
  guideStart,
  loadFlag,
  modelText,
  nextStep,
  saveFlag,
  stepAfterRead,
  stepLine,
  type GuideFlag,
  type GuideStep,
} from "./onboarding";
import "./onboarding.css";

/** Whether the guide shows, and the step it starts on. `folderCount`: the
 * active folders, or null until Traeva's state is read (and while storage
 * is blocked); the guide waits for it. */
export function useGuide(folderCount: number | null) {
  const [flag, setFlag] = useState<GuideFlag>(loadFlag);
  const from = folderCount === null ? null : guideStart(flag, folderCount);
  // Kept in the page even when storage fails, so the guide stays once the
  // first folder is added.
  const toSave = folderCount === null ? null : flagToSave(flag, folderCount);
  useEffect(() => {
    if (toSave === null) return;
    saveFlag(toSave);
    setFlag(toSave);
  }, [toSave]);
  const finish = useCallback(() => {
    saveFlag("done");
    setFlag("done");
  }, []);
  // The finish card is reached: done is kept at once, so closing the window
  // there does not start the guide again; the card stays until it is left.
  const reached = useCallback(() => saveFlag("done"), []);
  return { flag, from, finish, reached };
}

const TITLES: Record<GuideStep, string> = {
  1: "Add a folder",
  2: "Suggestions from a model (optional)",
  3: "Organize files you drop on Traeva (optional)",
  finish: "You're set",
};

type Props = {
  app: AppState;
  from: 1 | 2;
  /** Reads Traeva's state again, after a step changed it. */
  onChanged: () => void;
  /** Finished or skipped: the page shows Find. */
  onDone: () => void;
  /** The finish card shows: the guide is done, kept at once. */
  onFinishShown: () => void;
};

export function OnboardingGuide({ app, from, onChanged, onDone, onFinishShown }: Props) {
  const [step, setStep] = useState<GuideStep>(from);
  const roots = app.roots.filter((root) => root.status === "active");
  const titleId = useId();

  // Counts the reads of Traeva's state, so the automatic mode's card reads
  // its setting again with them, as on the Settings page.
  const [pageKey, setPageKey] = useState(0);
  useEffect(() => setPageKey((key) => key + 1), [app]);

  // A new step puts the focus on its heading; not when the guide first shows.
  const title = useRef<HTMLHeadingElement>(null);
  const moved = useRef(false);
  useEffect(() => {
    if (moved.current) title.current?.focus();
    if (step === "finish") onFinishShown();
  }, [step, onFinishShown]);
  const next = () => {
    moved.current = true;
    setStep(nextStep(step));
  };
  // A folder added somewhere else while step 1 shows: on to step 2.
  const after = stepAfterRead(step, roots.length);
  useEffect(() => {
    if (after === step) return;
    moved.current = true;
    setStep(after);
  }, [after, step]);

  return (
    <div className="onboarding">
      <h1 className="sr-only">Set up Traeva</h1>
      {step !== "finish" && (
        <div className="onboarding-top">
          <p className="onboarding-progress">{stepLine(step)}</p>
          <button type="button" className="onboarding-skip" onClick={onDone}>
            Skip setup
          </button>
        </div>
      )}
      <section key={step} className="onboarding-step" aria-labelledby={titleId}>
        <h2 id={titleId} ref={title} tabIndex={-1} className="onboarding-title">
          {TITLES[step]}
        </h2>

        {step === 1 && (
          <FirstFolderCard
            onAdded={() => {
              onChanged();
              next();
            }}
          />
        )}

        {step === 2 && (
          <>
            <p className="onboarding-lead">
              Traeva works without a model: you pick folders yourself, and Find searches with your own
              words. With a model, Traeva can suggest folders, after you agree to what is sent.
            </p>
            <ModelSettingsCard onChanged={onChanged} />
            <StepButtons step={2} onNext={next} />
          </>
        )}

        {step === 3 && (
          <>
            <p className="onboarding-lead">
              If you turn this on, Traeva may move files you drop on it (its Dock icon or the desktop
              pet) or open with it, without a preview, only into folders that already exist in the
              range you choose.
            </p>
            <AutoDropCard roots={roots} modelReady={app.model === "configured"} pageKey={pageKey} />
            <StepButtons step={3} onNext={next} />
          </>
        )}

        {step === "finish" && <Finish app={app} folders={roots.length} onStart={onDone} />}
      </section>
    </div>
  );
}

/** Continue and Skip both go on: the card saves its own changes, and the
 * note under them says so. */
function StepButtons({ step, onNext }: { step: 2 | 3; onNext: () => void }) {
  const noteId = useId();
  return (
    <>
      <div className="onboarding-actions">
        <button type="button" className="primary" onClick={onNext} aria-describedby={noteId}>
          Continue
        </button>
        <button type="button" onClick={onNext}>
          Skip
        </button>
      </div>
      <p id={noteId} className="muted small onboarding-note">
        {continueNote(step)}
      </p>
    </>
  );
}

/** What is set now, as the core reports it. The automatic mode's line is
 * left out when its setting cannot be read. */
function Finish({ app, folders, onStart }: { app: AppState; folders: number; onStart: () => void }) {
  const [auto, setAuto] = useState<ModeStatus | null>(null);
  useEffect(() => {
    let alive = true;
    getAutoDropSettings().then(
      (settings) => {
        if (alive) setAuto(modeStatus(settings));
      },
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, []);
  return (
    <div className="card">
      <dl className="onboarding-summary">
        <dt>Folders</dt>
        <dd>{foldersText(folders)}</dd>
        <dt>Model</dt>
        <dd>{modelText(app.model)}</dd>
        {auto !== null && (
          <>
            <dt>Organize dropped files automatically</dt>
            <dd>{autoModeText(auto)}</dd>
          </>
        )}
      </dl>
      <p className="muted small">You can change these any time in Settings.</p>
      <button type="button" className="primary" onClick={onStart}>
        Start using Traeva
      </button>
    </div>
  );
}
