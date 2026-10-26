// The Settings page in groups (#37, the student's choice B, 10/14): what
// needs the user first, then what is still getting ready, then what is set
// up. Worked out from what the cards themselves show; the cards and their
// rules do not change.

import { modeStatus } from "../autodrop/queue.ts";
import type { AutoDropSettingsView } from "../autodrop/types.ts";
import type { ModelStatus, OcrState, RootView } from "../organize/types.ts";

/** The page's parts, in the page's order. */
export type SettingsPart = "folders" | "model" | "recognition" | "automatic";

export type SettingsGroups = {
  needs: SettingsPart[];
  progress: SettingsPart[];
  ok: SettingsPart[];
};

/** What the page reads to group its parts. Null: could not be read now (the
 * card says so itself). */
export type SettingsFacts = {
  roots: RootView[];
  model: ModelStatus;
  recognition: OcrState | null;
  automatic: AutoDropSettingsView | null;
};

const ORDER: SettingsPart[] = ["folders", "model", "recognition", "automatic"];

/** Where each part goes. Text recognition that is off shows nothing, as before. */
export function settingsGroups(facts: SettingsFacts): SettingsGroups {
  const groups: SettingsGroups = { needs: [], progress: [], ok: [] };
  for (const part of ORDER) {
    const group = groupOf(part, facts);
    if (group !== null) groups[group].push(part);
  }
  return groups;
}

function groupOf(part: SettingsPart, facts: SettingsFacts): keyof SettingsGroups | null {
  switch (part) {
    case "folders": {
      // No folder yet, or one that cannot be used now: the user adds or fixes it.
      const active = facts.roots.filter((root) => root.status === "active");
      return active.length === 0 || active.some((root) => root.problem !== null) ? "needs" : "ok";
    }
    case "model":
      // Not set up, or waiting for the user's consent to send.
      return facts.model === "configured" ? "ok" : "needs";
    case "recognition":
      switch (facts.recognition) {
        case null:
        case "off":
          return null;
        case "failed":
        case "timed_out":
          // Off until Traeva starts again.
          return "needs";
        case "not_prepared":
        case "preparing":
          return "progress";
        case "ready":
        case "cancelled":
          return "ok";
      }
      return null;
    case "automatic": {
      // An agreement that no longer counts (its folders are gone, or the
      // notice changed) waits for the user; off is a choice, not a problem.
      const settings = facts.automatic;
      if (settings === null) return "ok";
      return modeStatus(settings) === "not_working" || settings.problem !== null ? "needs" : "ok";
    }
  }
}

/**
 * The groups shown, after a card's change or a new read: a part that now
 * needs the user goes to Needs you at once, so a new problem is never left
 * in a closed All set (R3 P2-1, 10/14). A part that got better stays where
 * it is until the page opens again, so a card the user is fixing does not
 * jump away. A part shown nowhere yet joins its group.
 */
export function worsened(shown: SettingsGroups, now: SettingsGroups): SettingsGroups {
  const placed = (part: SettingsPart) =>
    shown.needs.includes(part) || shown.progress.includes(part) || shown.ok.includes(part);
  const toNeeds = now.needs.filter((part) => !shown.needs.includes(part));
  const keep = (list: SettingsPart[]) => list.filter((part) => !toNeeds.includes(part));
  const inOrder = (list: SettingsPart[]) => ORDER.filter((part) => list.includes(part));
  return {
    needs: inOrder([...shown.needs, ...toNeeds]),
    progress: inOrder([...keep(shown.progress), ...now.progress.filter((part) => !placed(part))]),
    ok: inOrder([...keep(shown.ok), ...now.ok.filter((part) => !placed(part))]),
  };
}

/** The part's name, for the All set group's line while it is closed. */
export function partName(part: SettingsPart): string {
  switch (part) {
    case "folders":
      return "Folders";
    case "model":
      return "Model";
    case "recognition":
      return "Text recognition";
    case "automatic":
      return "Dropped files";
  }
}

/** The All set group's line while it is closed. */
export function allSetSummary(groups: SettingsGroups): string {
  if (groups.needs.length === 0 && groups.progress.length === 0) return "Everything is set up";
  return groups.ok.map(partName).join(" · ");
}
