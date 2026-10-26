// Tests for onboarding.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  autoModeText,
  continueNote,
  flagToSave,
  foldersText,
  guideStart,
  loadFlag,
  modelText,
  nextStep,
  parseFlag,
  saveFlag,
  stepAfterRead,
  stepLine,
} from "./onboarding.ts";

test("parseFlag keeps only the two saved values", () => {
  assert.equal(parseFlag("started"), "started");
  assert.equal(parseFlag("done"), "done");
  assert.equal(parseFlag(null), null);
  assert.equal(parseFlag(""), null);
  assert.equal(parseFlag("Done"), null);
  assert.equal(parseFlag("true"), null);
});

test("first start: no flag and no folder shows the guide at step 1", () => {
  assert.equal(guideStart(null, 0), 1);
});

test("an existing user with folders and no flag never sees it", () => {
  assert.equal(guideStart(null, 1), null);
  assert.equal(guideStart(null, 4), null);
});

test("a reload while under way resumes: step 2 once a folder was added", () => {
  assert.equal(guideStart("started", 1), 2);
  assert.equal(guideStart("started", 3), 2);
  assert.equal(guideStart("started", 0), 1);
});

test("finished or skipped: never again, with or without folders", () => {
  assert.equal(guideStart("done", 0), null);
  assert.equal(guideStart("done", 2), null);
});

test("once the state is read: a first start saves started, an existing user done", () => {
  assert.equal(flagToSave(null, 0), "started");
  assert.equal(flagToSave(null, 1), "done");
  assert.equal(flagToSave(null, 3), "done");
  // A saved flag is never changed by a read.
  assert.equal(flagToSave("started", 0), null);
  assert.equal(flagToSave("started", 2), null);
  assert.equal(flagToSave("done", 0), null);
});

test("an existing user who later removes every folder does not get the guide", () => {
  const saved = flagToSave(null, 2);
  assert.equal(saved, "done");
  assert.equal(guideStart(saved, 0), null);
});

test("a folder added elsewhere while step 1 shows moves on to step 2", () => {
  assert.equal(stepAfterRead(1, 0), 1);
  assert.equal(stepAfterRead(1, 1), 2);
  assert.equal(stepAfterRead(1, 3), 2);
  // Later steps stay where they are, with or without folders.
  assert.equal(stepAfterRead(2, 0), 2);
  assert.equal(stepAfterRead(3, 1), 3);
  assert.equal(stepAfterRead("finish", 0), "finish");
});

test("the steps go 1, 2, 3, then the finish", () => {
  assert.equal(nextStep(1), 2);
  assert.equal(nextStep(2), 3);
  assert.equal(nextStep(3), "finish");
  assert.equal(nextStep("finish"), "finish");
  assert.equal(stepLine(1), "Step 1 of 3");
  assert.equal(stepLine(3), "Step 3 of 3");
  assert.match(continueNote(2), /^Continue only goes on\. .*Save/);
  assert.match(continueNote(3), /^Continue only goes on\. .*on or off/);
});

test("the finish card's lines follow the core's state", () => {
  assert.equal(foldersText(0), "None yet");
  assert.equal(foldersText(1), "1 folder");
  assert.equal(foldersText(2), "2 folders");
  assert.equal(modelText("not_configured"), "Not set up");
  // Organize's consent only: the text names what it allows.
  assert.equal(modelText("consent_required"), "Saved, sending for folder suggestions not allowed yet");
  assert.equal(modelText("configured"), "Set up, sending for folder suggestions allowed");
  assert.equal(autoModeText("off"), "Off");
  assert.equal(autoModeText("on"), "On");
  assert.equal(autoModeText("not_working"), "Not working now");
});

// Node has no window: storage is missing, as in a blocked private window.
const global = globalThis as { window?: unknown };
afterEach(() => {
  delete global.window;
});

test("without storage the flag reads as not started and saving does not throw", () => {
  assert.equal(loadFlag(), null);
  assert.doesNotThrow(() => saveFlag("started"));
});

test("storage that throws reads as not started and saving does not throw", () => {
  global.window = {
    get localStorage(): Storage {
      throw new Error("SecurityError");
    },
  };
  assert.equal(loadFlag(), null);
  assert.doesNotThrow(() => saveFlag("done"));
});

test("the flag is saved under traeva.onboarding and read back", () => {
  const kept = new Map<string, string>();
  global.window = {
    localStorage: {
      getItem: (key: string) => kept.get(key) ?? null,
      setItem: (key: string, value: string) => void kept.set(key, value),
    },
  };
  assert.equal(loadFlag(), null);
  saveFlag("started");
  assert.equal(kept.get("traeva.onboarding"), "started");
  assert.equal(loadFlag(), "started");
  saveFlag("done");
  assert.equal(loadFlag(), "done");
});
