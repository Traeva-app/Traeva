// The Settings page's groups: what needs the user, what is still getting
// ready, and what is set up.

import assert from "node:assert/strict";
import { test } from "node:test";
import { allSetSummary, settingsGroups, worsened, type SettingsFacts } from "./groups.ts";
import type { AutoDropSettingsView } from "../autodrop/types.ts";
import type { RootView } from "../organize/types.ts";

const root: RootView = {
  root_id: "r1",
  display_path: "/Synthetic/Notes",
  status: "active",
  granted_at_ms: 1,
  revoked_at_ms: null,
  problem: null,
};

const automatic: AutoDropSettingsView = {
  enabled: true,
  notice: "Synthetic notice.",
  notice_version: 1,
  consent_id: "c1",
  agreed_at_ms: 1,
  target_root_id: "r1",
  scopes: [],
  problem: null,
};

const allSet: SettingsFacts = { roots: [root], model: "configured", recognition: "ready", automatic };

test("everything set up: one All set group, closed to one line", () => {
  const groups = settingsGroups(allSet);
  assert.deepEqual(groups, { needs: [], progress: [], ok: ["folders", "model", "recognition", "automatic"] });
  assert.equal(allSetSummary(groups), "Everything is set up");
});

test("what the user must fix goes first", () => {
  const problem = { code: "root_unavailable", message: "Gone." } as unknown as RootView["problem"];
  const groups = settingsGroups({
    roots: [root, { ...root, root_id: "r2", problem }],
    model: "consent_required",
    recognition: "failed",
    automatic: { ...automatic, enabled: false, problem: "The folders you chose are gone." },
  });
  assert.deepEqual(groups.needs, ["folders", "model", "recognition", "automatic"]);
  assert.deepEqual(groups.ok, []);
  // No folder at all, and a model never set up, need the user too.
  assert.deepEqual(settingsGroups({ ...allSet, roots: [], model: "not_configured" }).needs, ["folders", "model"]);
  // A removed folder alone is not a problem.
  assert.deepEqual(settingsGroups({ ...allSet, roots: [root, { ...root, status: "revoked" }] }).needs, []);
});

test("recognition getting ready is in progress; off shows nowhere", () => {
  const preparing = settingsGroups({ ...allSet, recognition: "preparing" });
  assert.deepEqual(preparing.progress, ["recognition"]);
  assert.equal(allSetSummary(preparing), "Folders · Model · Dropped files");
  assert.deepEqual(settingsGroups({ ...allSet, recognition: "not_prepared" }).progress, ["recognition"]);
  const off = settingsGroups({ ...allSet, recognition: "off" });
  assert.ok(![...off.needs, ...off.progress, ...off.ok].includes("recognition"));
  assert.deepEqual(settingsGroups({ ...allSet, recognition: "timed_out" }).needs, ["recognition"]);
});

test("the automatic mode off is a choice; an agreement that no longer counts needs the user", () => {
  const off = { ...automatic, enabled: false, consent_id: null, agreed_at_ms: null };
  assert.deepEqual(settingsGroups({ ...allSet, automatic: off }).needs, []);
  const notWorking = { ...automatic, enabled: false };
  assert.deepEqual(settingsGroups({ ...allSet, automatic: notWorking }).needs, ["automatic"]);
  // Unreadable now: its card says so; it is not counted as a problem here.
  assert.deepEqual(settingsGroups({ ...allSet, automatic: null }).needs, []);
});

test("a new problem goes to Needs you at once; a fixed one waits for the next visit", () => {
  // All set when the page opened; then the automatic mode's folder goes
  // away (R3 P2-1): the card moves to Needs you, and All set is no longer
  // "Everything is set up".
  const opened = settingsGroups(allSet);
  const broken = settingsGroups({ ...allSet, automatic: { ...automatic, enabled: false } });
  const shown = worsened(opened, broken);
  assert.deepEqual(shown, { needs: ["automatic"], progress: [], ok: ["folders", "model", "recognition"] });
  assert.equal(allSetSummary(shown), "Folders · Model · Text recognition");

  // The last folder removed, and recognition failing, follow the same way,
  // in the page's order.
  const worse = worsened(shown, settingsGroups({ ...allSet, roots: [], recognition: "failed" }));
  assert.deepEqual(worse.needs, ["folders", "recognition", "automatic"]);
  assert.deepEqual(worse.ok, ["model"]);

  // Fixed while the page is open: it stays in Needs you until the page opens again.
  assert.deepEqual(worsened(shown, opened), shown);
  assert.deepEqual(settingsGroups(allSet), opened, "the next visit groups afresh");
});

test("getting ready that fails moves to Needs you; a part shown nowhere joins its group", () => {
  const preparing = settingsGroups({ ...allSet, recognition: "preparing" });
  assert.deepEqual(worsened(preparing, settingsGroups({ ...allSet, recognition: "timed_out" })).needs, ["recognition"]);
  // Ready again while the page is open: still shown under In progress.
  assert.deepEqual(worsened(preparing, settingsGroups(allSet)).progress, ["recognition"]);
  // Recognition unread when the page opened, read now: it joins All set.
  const unread = settingsGroups({ ...allSet, recognition: null });
  assert.deepEqual(worsened(unread, settingsGroups(allSet)).ok, ["folders", "model", "recognition", "automatic"]);
});
