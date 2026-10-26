// The first-start guide on the Find page (#248): add a folder, then the
// model and what may be sent to it, then the automatic mode for dropped
// files. Each step is the card Settings shows, with the same commands; every
// step can be skipped. Whether the guide shows is kept in this window's
// localStorage: "started" when it first shows, "done" when it is finished or
// skipped, or at once for someone who already had folders before it existed.
// Storage that cannot be read counts as not started.

import type { ModeStatus } from "../autodrop/queue.ts";
import type { ModelStatus } from "../organize/types.ts";

const KEY = "traeva.onboarding";

export type GuideFlag = "started" | "done" | null;
export type GuideStep = 1 | 2 | 3 | "finish";
export const STEPS = 3;

/** A saved flag as read from storage; anything else is not started. */
export function parseFlag(saved: string | null): GuideFlag {
  return saved === "started" || saved === "done" ? saved : null;
}

/** The step the guide starts on, or null when it does not show. It never
 * shows again once finished or skipped. It shows while there is no folder,
 * and after a reload while it was under way; a folder already added starts
 * it at step 2. Someone with folders who never saw it does not see it. */
export function guideStart(flag: GuideFlag, folderCount: number): 1 | 2 | null {
  if (flag === "done") return null;
  if (folderCount === 0) return 1;
  return flag === "started" ? 2 : null;
}

/** The flag to save once Traeva's state is read, or null for none. With no
 * flag yet: no folder is a first start ("started"); folders already there
 * are an existing user ("done"), so removing every folder later does not
 * bring the guide up. */
export function flagToSave(flag: GuideFlag, folderCount: number): "started" | "done" | null {
  if (flag !== null) return null;
  return folderCount === 0 ? "started" : "done";
}

/** The step after Traeva's state is read again: a folder added anywhere
 * (Organize, Settings, a drop) while step 1 shows moves on to step 2. */
export function stepAfterRead(step: GuideStep, folderCount: number): GuideStep {
  return step === 1 && folderCount > 0 ? 2 : step;
}

export function nextStep(step: GuideStep): GuideStep {
  if (step === 1) return 2;
  if (step === 2) return 3;
  return "finish";
}

export function stepLine(step: 1 | 2 | 3): string {
  return `Step ${step} of ${STEPS}`;
}

/** Under Continue on steps 2 and 3: it only goes on; the card above keeps
 * what it saved or turned on itself. True whatever the card shows now. */
export function continueNote(step: 2 | 3): string {
  return step === 2
    ? "Continue only goes on. A change in the card above is kept once you use its Save."
    : "Continue only goes on. The card above turns the automatic mode on or off itself.";
}

// The finish card's lines, from the core's state only.

export function foldersText(count: number): string {
  if (count === 0) return "None yet";
  return count === 1 ? "1 folder" : `${count} folders`;
}

/** As the core reports it: no key saved is not set up; a saved key still
 * waits for the user's consent to send. That consent is Organize's, for
 * folder suggestions only: Find with the model asks for its own. */
export function modelText(status: ModelStatus): string {
  if (status === "configured") return "Set up, sending for folder suggestions allowed";
  if (status === "consent_required") return "Saved, sending for folder suggestions not allowed yet";
  return "Not set up";
}

export function autoModeText(status: ModeStatus): string {
  if (status === "on") return "On";
  if (status === "not_working") return "Not working now";
  return "Off";
}

export function loadFlag(): GuideFlag {
  try {
    return parseFlag(window.localStorage.getItem(KEY));
  } catch {
    return null;
  }
}

export function saveFlag(flag: "started" | "done"): void {
  try {
    window.localStorage.setItem(KEY, flag);
  } catch {
    // Not kept: after a reload the guide decides again from the folders.
  }
}
