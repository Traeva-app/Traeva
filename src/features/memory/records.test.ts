// Tests for records.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  libraryGroup,
  needsCount,
  AUTO_LINK_PAIR_TEXT,
  AUTO_MEMORY_TITLE,
  REASON_TEXT,
  STARTED_BY_TRAEVA,
  autoMemoryLimitText,
  autoMemoryRefusal,
  autoMemoryState,
  autoMemoryText,
  startedByTraeva,
  RECORD_GONE_TEXT,
  recordToOpen,
  associationNotes,
  associationRefusal,
  evidenceText,
  savedVersionText,
  standingText,
  mergePoints,
  mergeRefusal,
  sameContentText,
  workText,
  branchParents,
  canRelocate,
  candidateNotice,
  captureText,
  continueAllInOrder,
  currentObjectKinds,
  currentObjectNote,
  decisionProblems,
  decisionRequest,
  emptyDraft,
  gapText,
  recordSignature,
  recordStateText,
  relocationPoints,
  relocationRefusal,
  hasDecisions,
  leaveAsItIs,
  savedText,
  skipNote,
  snapshotPhrase,
  waitingLabel,
  correctableRelations,
  correctedText,
  memberLabel,
  memberNotes,
  movableMembers,
  moveNotes,
  moveRefusal,
  movedText,
  namedPlace,
  libraryText,
  needsDecision,
  recordTabText,
  tabHasNews,
  waitsText,
  holdText,
  relationNotes,
  relationRefusal,
  relationStatusText,
  relationText,
} from "./records.ts";
import type {
  AutoMemoryView,
  AssociationPreview,
  MemoryFamilyView,
  MergePreview,
  CandidateCaptureInfo,
  FamilyMemberView,
  GapKind,
  MovePreview,
  MemoryRecordView,
  RelocationPreview,
  PendingCandidate,
  PendingCandidatesView,
  RecordedGapView,
  RelationPreview,
  SnapshotRef,
  VersionRelationView,
  WorkDecisionOutcome,
} from "../family/types";

function version(sequence: number, id = `s${sequence}`): SnapshotRef {
  return {
    snapshot_id: id,
    file_id: "f",
    sequence,
    file_revision: 1,
    root_id: "r",
    relative_path: "essay.md",
    sha256: "0".repeat(64),
    byte_length: 1,
    captured_at_ms: 0,
  };
}

const place = { root_id: "r", relative_path: "essay.md", display_path: "Drafts/essay.md" };

function record(fields: Partial<MemoryRecordView>): MemoryRecordView {
  return {
    chain_id: "c1",
    family_id: "w1",
    origin_file_id: "f",
    permission_file_id: "f",
    state: "active",
    watch: {
      watch_id: "w",
      file_id: "f",
      status: "active",
      root_id: "r",
      relative_path: "essay.md",
      started_at_ms: 0,
      stopped_at_ms: null,
      versions: 1,
      latest: null,
      checked_at_ms: null,
      problem: null,
    },
    line_versions: 3,
    place,
    root_active: true,
    waiting_for_relocation: false,
    candidate_capture: "off",
    pending: { objects: 0, snapshots: 0, limit: 20 },
    open_gaps: [],
    same_content: [],
    hold: null,
    ...fields,
  };
}

function candidate(id: string, fields: Partial<PendingCandidate>): PendingCandidate {
  return {
    candidate_id: id,
    file_id: `file-${id}`,
    seq: 1,
    reason: "replaced",
    display_path: `Drafts/${id}.md`,
    present: true,
    at_place: false,
    found_at_ms: 0,
    snapshots: [],
    found_after: null,
    continue_from: null,
    continue_skips: 0,
    continue_needs: null,
    deferred: false,
    ...fields,
  };
}

function page(items: PendingCandidate[]): PendingCandidatesView {
  return {
    chain_id: "c1",
    family_id: "w1",
    page_token: "t1",
    state: "active",
    root_active: true,
    candidate_capture: "on",
    pending: { objects: items.length, snapshots: items.length, limit: 20 },
    place,
    place_holds: "pending_candidate",
    tail: null,
    fork_points: [],
    items,
    unread_occupant: null,
    place_problem: null,
  };
}

test("a revoked folder is said first; a lost file keeps its history", () => {
  assert.match(recordStateText(record({ root_active: false })).label, /no longer authorized/);
  const lost = recordStateText(record({ state: "lost", waiting_for_relocation: true }));
  assert.equal(lost.tone, "warn");
  assert.match(lost.detail, /history is kept/);
  assert.match(recordStateText(record({})).detail, /3 versions saved/);
  const missing = recordStateText(record({ waiting_for_relocation: true }));
  assert.equal(missing.label, "File not found");
  assert.match(missing.detail, /nothing new is saved.*3 versions kept/);
  assert.doesNotMatch(missing.detail, /kept automatically/);
});

test("candidate capture at the limit says so, in the material's words", () => {
  const full = { objects: 20, snapshots: 4, limit: 20 };
  assert.match(captureText("paused", full), /reached the limit.*Review them first/);
  assert.doesNotMatch(captureText("paused", { objects: 2, snapshots: 2, limit: 20 }), /limit/);
  assert.match(captureText("off", full), /only reports it/);
});

test("with capture off or on, Traeva first follows a file an app saved at the same place", () => {
  const room = { objects: 0, snapshots: 0, limit: 20 };
  for (const capture of ["off", "on"] as const) {
    const text = captureText(capture, room);
    // Also shown for a stopped or paused record: it follows only while on.
    assert.match(text, /While this recording is on, .*Traeva follows the new file by itself/);
    assert.match(text, /too large or a hard link, or another file at this place still waits for your decision/);
  }
  assert.match(captureText("off", room), /When it cannot, it only reports it\.$/);
  assert.match(captureText("on", room), /When it cannot, the new file is saved as a candidate/);
  // Candidates saved before Traeva followed such files show it too: it
  // says nothing about why Traeva did not follow the file.
  assert.equal(
    REASON_TEXT.replaced,
    "Found at this place after the file was replaced. There is no evidence yet that it continues the work.",
  );
  const gap: RecordedGapView = {
    gap_id: "g",
    chain_id: "c1",
    kind: "replaced_uncaptured",
    after: null,
    problem: null,
    started_at_ms: 0,
    ended_at_ms: null,
    end_reason: null,
  };
  assert.equal(gapText(gap), "The file was replaced by one Traeva did not read. That version was not saved.");
});

test("gaps say what Traeva did not see, never that something changed", () => {
  for (const kind of [
    "memory_off",
    "paused",
    "capture_failed",
    "replaced_uncaptured",
    "lost",
    "watcher_stopped",
    "not_recorded",
    "skipped",
  ] as const) {
    const gap: RecordedGapView = {
      gap_id: "g",
      chain_id: "c1",
      kind,
      after: null,
      problem: kind === "capture_failed" ? "file_changing" : null,
      started_at_ms: 0,
      ended_at_ms: null,
      end_reason: null,
    };
    const text = gapText(gap);
    assert.ok(!/was changed|you changed/i.test(text), text);
  }
});

test("the notice has the four points, and the fifth only when the file there now is saved at once", () => {
  const info: CandidateCaptureInfo = {
    chain_id: "c1",
    state: "active",
    candidate_capture: "off",
    notice_version: 1,
    place,
    pending: { objects: 0, snapshots: 0, limit: 20 },
    place_holds: "recorded_file",
    unread_occupant: null,
    saves_occupant_now: false,
    place_problem: null,
    root_active: true,
  };
  const four = candidateNotice(info);
  assert.equal(four.length, 4);
  assert.match(four[0], /Drafts\/essay\.md/);
  assert.match(four[1], /unrelated file/);
  assert.match(four[2], /cannot be deleted/);
  assert.match(four[3], /does not mean/);
  const five = candidateNotice({ ...info, saves_occupant_now: true });
  assert.equal(five.length, 5);
  assert.match(five[4], /saved as soon as you turn this on/);
});

test("nothing is decided for the user, and 'continue all' uses only the core's parents", () => {
  const view = page([
    candidate("n1", { continue_from: version(4, "o4") }),
    candidate("n2", {}),
    candidate("n3", { continue_from: version(5, "n1-5"), continue_needs: "n1" }),
  ]);
  const empty = emptyDraft();
  assert.deepEqual(empty, {});
  assert.ok(!hasDecisions(empty, null), "nothing to send yet");
  const all = continueAllInOrder(view, empty);
  assert.deepEqual(all.n1, { kind: "continue", parent_snapshot_id: "o4" });
  assert.equal(all.n2, undefined, "it cannot continue, so it is left as it is");
  assert.deepEqual(all.n3, { kind: "continue", parent_snapshot_id: "n1-5" });
  assert.deepEqual(decisionProblems(view, all), []);
});

test("a continuation needs the one before it, and only the named parent", () => {
  const view = page([
    candidate("n1", { seq: 1, continue_from: version(4, "o4") }),
    candidate("n2", { seq: 2 }),
    candidate("n3", { seq: 3, continue_from: version(5, "n1-5"), continue_needs: "n1" }),
  ]);
  const draft = {
    n3: { kind: "continue" as const, parent_snapshot_id: "n1-5" },
    n2: { kind: "continue" as const, parent_snapshot_id: "o4" },
  };
  const problems = decisionProblems(view, draft);
  assert.equal(problems.length, 2, problems.join(" | "));
  assert.match(problems[0], /Waiting file 2 cannot be marked as continuing/);
  assert.match(problems[1], /To continue waiting file 3, also mark waiting file 1/);
});

test("a relation past excluded files is shown as new", () => {
  assert.equal(skipNote({ continue_skips: 0 }, "the version saved 9 Oct"), null);
  assert.match(skipNote({ continue_skips: 1 }, "the version saved 9 Oct") ?? "", /new relation.*not one save/);
});

test("waiting files are numbered by their place in the list, not by seq", () => {
  const items = [candidate("n2", { seq: 2 }), candidate("n5", { seq: 5 }), candidate("n6", { seq: 6 })];
  assert.deepEqual(
    items.map((item) => waitingLabel(item, items)),
    ["waiting file 1", "waiting file 2", "waiting file 3"],
  );
});

test("copies are named by their waiting file and time, never by a work version number", () => {
  const items = [candidate("n0", { seq: 4 }), candidate("n1", { seq: 7, file_id: "fn1" })];
  assert.match(snapshotPhrase({ ...version(1), file_id: "fn1" }, items), /^the copy of waiting file 2 saved /);
  assert.match(snapshotPhrase({ ...version(5), file_id: "f" }, items), /^the version saved /);
  assert.doesNotMatch(snapshotPhrase({ ...version(5), file_id: "f" }, items), /version 5/);
});

test("the request sends only what the user decided; 'not sure yet' only when picked", () => {
  const view = page([candidate("n1", {}), candidate("n2", {}), candidate("n3", {})]);
  let draft = { n3: { kind: "defer" as const }, n1: { kind: "independent" as const } };
  const request = decisionRequest(view, draft, { kind: "independent" });
  assert.equal(request.page_token, "t1");
  assert.deepEqual(request.items, [
    { candidate_id: "n1", choice: { kind: "independent" } },
    { candidate_id: "n3", choice: { kind: "defer" } },
  ]);
  assert.deepEqual(request.current_object, { kind: "independent" });
  draft = leaveAsItIs(draft, "n1") as typeof draft;
  assert.deepEqual(decisionRequest(view, draft, null).items, [{ candidate_id: "n3", choice: { kind: "defer" } }]);
});

test("the file at the place: continue or unrelated when unread, only unrelated when decided or kept elsewhere", () => {
  const occupant = {
    occupant: { occupant_token: "o", byte_length: 10, modified_at_ms: 0 },
    detected_at_ms: null,
    continue_from: version(3),
    continue_skips: 0,
    continue_needs: null,
  };
  assert.deepEqual(currentObjectKinds({ ...page([]), place_holds: "unread_object", unread_occupant: occupant }), [
    "continue",
    "independent",
  ]);
  assert.deepEqual(currentObjectKinds({ ...page([]), place_holds: "unread_object" }), []);
  for (const holds of ["decided_candidate", "other_record"] as const) {
    assert.deepEqual(currentObjectKinds({ ...page([]), place_holds: holds }), ["independent"], holds);
  }
  for (const holds of ["recorded_file", "empty", "pending_candidate", "unavailable", "not_looked"] as const) {
    assert.deepEqual(currentObjectKinds({ ...page([]), place_holds: holds }), [], holds);
  }
  assert.match(currentObjectNote("independent"), /does not read it.*counts as lost.*can only be stopped/);
  assert.match(currentObjectNote("continue"), /reads it now/);
});

test("a branch is offered only from versions saved before the waiting file", () => {
  const at = (ms: number, id: string) => ({ ...version(1, id), captured_at_ms: ms });
  const item = candidate("n1", { snapshots: [at(200, "n1-1"), at(300, "n1-2")] });
  const points = [at(100, "o1"), at(200, "o2"), at(250, "o3")];
  assert.deepEqual(
    branchParents(points, item).map((point) => point.snapshot_id),
    ["o1", "o2"],
  );
  assert.deepEqual(branchParents(points, candidate("n2", {})), []);
});

test("'history only' never says Memory is off", () => {
  assert.equal(savedText(false), "Saved.");
  const text = savedText(true);
  assert.match(text, /no file you continued is at the file's place now/);
  assert.doesNotMatch(text, /Memory is (off|stopped)/);
});

test("the cards read again when the recording changes, not on every refresh", () => {
  const base = record({});
  assert.equal(recordSignature(base), recordSignature(record({})), "same record, no reload");
  for (const changed of [
    record({ pending: { objects: 1, snapshots: 1, limit: 20 } }),
    record({ candidate_capture: "on" }),
    record({ state: "lost" }),
    record({ waiting_for_relocation: true }),
    record({ watch: { ...base.watch, versions: 4 } }),
    record({ watch: { ...base.watch, watch_id: "w2" } }),
    // Moved by Organize, same watch: the capture notice must name the new place.
    record({ place: { ...place, relative_path: "Organized/essay.md", display_path: "Organized/essay.md" } }),
    record({ place: { ...place, root_id: "r2" } }),
    record({
      open_gaps: [
        {
          gap_id: "g1",
          chain_id: "c1",
          kind: "replaced_uncaptured",
          after: null,
          problem: null,
          started_at_ms: 0,
          ended_at_ms: null,
          end_reason: null,
        },
      ],
    }),
  ]) {
    assert.notEqual(recordSignature(changed), recordSignature(base));
  }
});

test("the record counts the versions of its whole line, and Lost covers an unrelated file at the place", () => {
  assert.match(recordStateText(record({ state: "stopped" })).detail, /^3 versions kept/);
  const lost = recordStateText(record({ state: "lost", waiting_for_relocation: true })).detail;
  assert.match(lost, /it is gone, or you said the file there now is unrelated/);
  assert.match(lost, /choose it where it is now/);
});

test("the new place is offered only to a record that waits for it", () => {
  assert.ok(canRelocate(record({ state: "lost", waiting_for_relocation: true })));
  assert.ok(!canRelocate(record({ state: "lost", waiting_for_relocation: false })));
  assert.ok(!canRelocate(record({ state: "stopped", waiting_for_relocation: true })));
  assert.ok(!canRelocate(record({ state: "lost", waiting_for_relocation: true, root_active: false })));
  const replaced = {
    gap_id: "g",
    chain_id: "c1",
    kind: "replaced_uncaptured" as const,
    after: null,
    problem: null,
    started_at_ms: 0,
    ended_at_ms: null,
    end_reason: null,
  };
  assert.ok(canRelocate(record({ open_gaps: [replaced] })), "another file at the place, unread");
  assert.ok(!canRelocate(record({ state: "stopped", open_gaps: [replaced] })));
  assert.ok(!canRelocate(record({ open_gaps: [{ ...replaced, kind: "watcher_stopped" }] })));
});

test("the relocation notice says what happens to the old place, its candidates and the switch", () => {
  const preview: RelocationPreview = {
    chain_id: "c1",
    old_place: place,
    old_place_holds: "unread_object",
    new_place: { root_id: "r", relative_path: "Moved/essay.md", display_path: "Drafts/Moved/essay.md" },
    byte_length: 10,
    modified_at_ms: 0,
    waiting_candidates: 2,
    candidate_capture: "on",
    page_token: "t",
  };
  const points = relocationPoints(preview);
  assert.match(points[0], /old place now, Drafts\/essay\.md, does not join the work/);
  assert.match(points[1], /2 files waiting for your decision stay waiting/);
  assert.match(points[2], /Candidate capture goes along.*new place only/);
  assert.match(points[3], /did not see the move.*gap/);
  const quiet = relocationPoints({ ...preview, old_place_holds: "empty", waiting_candidates: 0, candidate_capture: "off" });
  assert.equal(quiet.length, 3);
  assert.match(quiet[0], /Nothing is at the old place/);
  assert.match(quiet[1], /Memory looks at the new place only/);
  const back = relocationPoints({ ...preview, old_place_holds: "recorded_file", new_place: place });
  assert.match(back[0], /back at its place, Drafts\/essay\.md/);
  assert.equal(back.length, 2);
});

test("a refused relocation says nothing changed; other errors are shown as they are", () => {
  for (const code of ["not_same_object", "page_outdated", "unresolved_action"] as const) {
    assert.match(relocationRefusal(code) ?? "", /Nothing was changed/, code);
  }
  assert.match(relocationRefusal("not_same_object") ?? "", /copy.*another disk.*hard link/);
  assert.equal(relocationRefusal("decision_conflict"), null);
});

test("a merge says what it does not do, and which earlier judgements stop counting", () => {
  const family = (id: string): MemoryFamilyView => ({
    family_id: id,
    created_at_ms: 0,
    merged_family_ids: [],
    members: [],
    records: [],
    relations: [],
    gaps: [],
    boundaries: [],
  });
  const preview: MergePreview = {
    family_id: "w1",
    other_family_id: "w2",
    stays: family("w1"),
    joins: family("w2"),
    overrides: [],
    page_token: "t",
  };
  const plain = mergePoints(preview);
  assert.equal(plain.length, 3);
  assert.match(plain[0], /No file's record is merged/);
  assert.match(plain[1], /does not work out which side's versions came first/);
  assert.match(plain[2], /switch, its limit and its waiting files do not change/);
  const overridden = mergePoints({
    ...preview,
    overrides: [{ member_id: "m", file_id: "f", display_path: "Drafts/old.md", rejected_in_family_id: "w2" }],
  });
  assert.equal(overridden.length, 4);
  assert.match(overridden[3], /Drafts\/old\.md is unrelated.*no longer counts/);
  assert.match(mergeRefusal("page_outdated") ?? "", /Nothing was merged/);
  assert.match(mergeRefusal("decision_conflict") ?? "", /already one work/);
  assert.equal(mergeRefusal("unknown_family"), null);
});

test("a work is told by its files; the same-content hint never claims a relation", () => {
  assert.equal(
    workText({ family_id: "w", created_at_ms: 0, merged_family_ids: [], display_paths: ["a.md", "b.md"], records: 2 }),
    "2 files: a.md, b.md",
  );
  assert.match(
    workText({ family_id: "w", created_at_ms: 0, merged_family_ids: [], display_paths: ["a", "b", "c", "d"], records: 1 }),
    /4 files: a, b, c, …$/,
  );
  const text = sameContentText({
    family_id: "w2",
    own_version: { ...version(1), file_id: "new" },
    same_bytes_as: { ...version(3), relative_path: "Drafts/essay.md" },
  });
  assert.match(text, /exactly the bytes of a version of essay\.md in another work/);
  assert.match(text, /may be related; Traeva links nothing by itself/);
});

function association(fields: Partial<AssociationPreview>): AssociationPreview {
  return {
    family_id: "w1",
    subject: { file_id: "f9", place, sha256: null, versions: [] },
    request_subject: { kind: "file", file_id: "f9" },
    standing: "saved_no_work",
    other_family_id: null,
    merge_scope: null,
    needs_recording: false,
    evidence: { same_bytes_as: [], same_object_as: [] },
    choices: ["continue", "branch", "unknown", "independent"],
    continue_from: [],
    fork_points: [],
    replaces_unknown_placement: null,
    overrides: [],
    page_token: "t",
    ...fields,
  };
}

test("adding a file says where it stands and that evidence is never a relation", () => {
  assert.match(standingText(association({ standing: "not_recorded" })), /no saved version.*turn Memory on for it/);
  assert.match(standingText(association({ standing: "in_other_work" })), /merges that whole work into this one/);
  assert.match(standingText(association({ standing: "member_without_parent" })), /where it starts is not known/);
  const none = evidenceText(association({}));
  assert.match(none[0], /knows nothing that links them/);
  assert.match(none[none.length - 1], /does not decide anything from this/);
  const same = evidenceText(
    association({ evidence: { same_bytes_as: [{ ...version(2), relative_path: "Drafts/essay.md" }], same_object_as: ["f1"] } }),
  );
  assert.match(same[0], /exactly the bytes of essay\.md, saved /);
  assert.match(same[1], /very same file on disk/);
  assert.match(savedVersionText({ ...version(1), relative_path: "a/b.md" }), /^b\.md, saved /);
});

test("each choice says what it does before it is saved", () => {
  const scope: MemoryFamilyView = {
    family_id: "w2",
    created_at_ms: 0,
    merged_family_ids: [],
    members: [
      {
        file_id: "x",
        display_path: "Other/x.md",
        present: true,
        status: "confirmed_user",
        basis: "user_decision",
        placed: true,
        decided_at_ms: null,
        snapshots: [],
        merged_from: null,
      },
    ],
    records: [],
    relations: [],
    gaps: [],
    boundaries: [],
  };
  const other = association({
    standing: "in_other_work",
    merge_scope: scope,
    replaces_unknown_placement: "e1",
    overrides: [{ member_id: "m", file_id: "f", display_path: "Drafts/old.md", rejected_in_family_id: "w1" }],
  });
  const cont = associationNotes(other, "continue");
  assert.ok(cont.some((n) => /merged into this one: Other\/x\.md/.test(n)));
  assert.ok(cont.some((n) => /saw nothing between the two versions/.test(n)));
  assert.ok(cont.some((n) => /place not known" is replaced/.test(n)));
  assert.ok(cont.some((n) => /No recording starts or moves/.test(n)));
  assert.ok(cont.some((n) => /old\.md is unrelated.*no longer counts/.test(n)));
  const apart = associationNotes(other, "independent");
  assert.deepEqual(apart, [], "unrelated merges nothing and overrides nothing");
  assert.ok(!associationNotes(other, "unknown").some((n) => /gap/.test(n)), "no gap without a parent");
  assert.match(associationRefusal("candidate_pending") ?? "", /decide it there. Nothing was saved/);
  assert.match(associationRefusal("page_outdated") ?? "", /Nothing was saved/);
  assert.equal(associationRefusal("decision_conflict"), null, "the core says which conflict");
});

// --- Correcting one relation, and a paused recording (#31 M2b-3a) ---

const hold = { work_decision_id: "d1", edge_id: "e1", held_at_ms: 0, why: "relation_revoked" as const };

test("a recording paused by a correction says so, never only 'stopped'", () => {
  const paused = record({ state: "stopped", hold });
  const state = recordStateText(paused);
  assert.equal(state.label, "Paused");
  assert.match(state.detail, /revoked the relation this recording went on by/);
  assert.match(state.detail, /record it again or stop it/);
  assert.equal(recordStateText(record({ state: "stopped" })).label, "Stopped");
  assert.notEqual(recordSignature(paused), recordSignature(record({ state: "stopped" })), "the cards read again");
  assert.match(holdText(hold), /^Paused since .*\. You revoked the relation this recording went on by\. Nothing new/);
  const apart = { ...hold, why: "moved_apart" as const };
  assert.match(holdText(apart), /You moved files out of the work, so the relation this recording went on by stopped counting\./);
  assert.match(recordStateText(record({ state: "stopped", hold: apart })).detail, /^You moved files out of the work/);
});

function relation(fields: Partial<VersionRelationView>): VersionRelationView {
  return {
    edge_id: "e1",
    kind: "continuation",
    parent: { ...version(2), relative_path: "Drafts/todo.txt" },
    child: { ...version(1, "n1"), file_id: "n", relative_path: "Drafts/notes.txt" },
    status: "confirmed_user",
    skipped_items: 0,
    decided_at_ms: 0,
    crosses_works: false,
    linked_automatically: false,
    ...fields,
  };
}

test("only confirmed or disputed relations between two versions can be corrected", () => {
  const kept = [
    relation({ edge_id: "user" }),
    relation({ edge_id: "system", status: "confirmed_system" }),
    relation({ edge_id: "disputed", status: "disputed", kind: "branch" }),
  ];
  const left = [
    relation({ edge_id: "unknown", status: "unknown", kind: "unplaced", parent: null }),
    relation({ edge_id: "superseded", status: "superseded" }),
    relation({ edge_id: "rejected", status: "rejected" }),
    relation({ edge_id: "candidate", status: "candidate" }),
  ];
  assert.deepEqual(
    correctableRelations([...left, ...kept]).map((r) => r.edge_id),
    ["user", "system", "disputed"],
  );
  assert.match(relationText(relation({})), /^notes\.txt, saved .* continues todo\.txt, saved /);
  assert.match(relationText(relation({ kind: "branch" })), / branches from todo\.txt/);
  assert.equal(relationStatusText(relation({})), "Confirmed by you.");
  assert.match(relationStatusText(relation({ status: "confirmed_system" })), /very same file on disk/);
  assert.match(relationStatusText(relation({ status: "disputed" })), /not used until you confirm it again or revoke it/);
});

test("a relation Traeva linked by itself says so, never 'the very same file'", () => {
  const auto = relation({ status: "confirmed_system", linked_automatically: true });
  assert.equal(relationStatusText(auto), "Linked automatically: a new file at the same place, saved by your app.");
  assert.ok(correctableRelations([auto]).length === 1, "it can be undone");
  const notes = relationNotes(relationPreview({ relation: auto, records_held: [record({})] }));
  assert.equal(
    notes[0],
    "Linked automatically: a new file at the same place, saved by your app. Undoing this link revokes it; the history keeps that Traeva had linked it.",
  );
  assert.ok(!notes.some((note) => /very same file/.test(note)));
  assert.ok(notes.some((note) => /^Revoking it: notes\.txt, saved .* stays in this work, its place not known/.test(note)));
  assert.ok(notes.some((note) => /is paused: it went on by this relation/.test(note)));
  assert.match(AUTO_LINK_PAIR_TEXT, /^Linked automatically: your app saved this by replacing the file at the same place\.$/);
  // Moved apart later, it is disputed like any other relation.
  const crossing = relation({ status: "disputed", crosses_works: true, linked_automatically: true });
  assert.match(relationStatusText(crossing), /^Crosses two works/);
  // Confirmed again by the user, it is theirs.
  const again = relation({ status: "confirmed_user", linked_automatically: true });
  assert.equal(relationStatusText(again), "Confirmed by you.");
  assert.ok(!relationNotes(relationPreview({ relation: again })).some((note) => /Linked automatically/.test(note)));
});

function relationPreview(fields: Partial<RelationPreview>): RelationPreview {
  return {
    family_id: "w1",
    relation: relation({}),
    can_revoke: true,
    can_confirm_again: false,
    leaves_unplaced: { ...version(1, "n1"), file_id: "n", relative_path: "Drafts/notes.txt" },
    records_held: [],
    page_token: "t",
    ...fields,
  };
}

test("the relation page says what revoking does before it is saved", () => {
  const plain = relationNotes(relationPreview({}));
  assert.match(plain[0], /^Revoking it: notes\.txt, saved .* stays in this work, its place not known\. Nothing joins it again by itself/);
  assert.ok(plain.some((note) => /no longer certain/.test(note)));
  assert.ok(!plain.some((note) => /paused/.test(note)));

  const pausing = relationNotes(relationPreview({ records_held: [record({})] }));
  assert.ok(pausing.some((note) => note === "Recording Drafts/essay.md is paused: it went on by this relation. You then record it again or stop it."));

  const system = relationNotes(relationPreview({ relation: relation({ status: "confirmed_system" }) }));
  assert.match(system[0], /^Traeva confirmed this from the very same file on disk\. You can still revoke it/);

  const disputed = relationNotes(
    relationPreview({ relation: relation({ status: "disputed" }), can_revoke: true, can_confirm_again: true }),
  );
  assert.ok(disputed.some((note) => /^Confirming it again: it counts as confirmed by you/.test(note)));
  const onlyAgain = relationNotes(relationPreview({ can_revoke: false, can_confirm_again: true }));
  assert.ok(!onlyAgain.some((note) => /^Revoking it/.test(note)));
});

test("what the page says after a correction, and when it is refused", () => {
  const outcome = (held: string[]) => ({ held_records: held }) as unknown as WorkDecisionOutcome;
  assert.equal(correctedText("revoke", outcome([])), "Revoked. The version stays in this work, its place not known.");
  assert.equal(correctedText("revoke", outcome(["c1"])), "Revoked. 1 recording is paused: record again or stop.");
  assert.equal(correctedText("revoke", outcome(["c1", "c2"])), "Revoked. 2 recordings are paused: record again or stop.");
  assert.match(correctedText("confirm_again", outcome([])), /^Confirmed again\./);
  for (const code of ["page_outdated", "decision_conflict", "unknown_relation"] as const) {
    assert.match(relationRefusal(code) ?? "", /Nothing was changed/);
  }
  assert.equal(relationRefusal("invalid_input"), null);
});

// --- Moving files out of a work (#31 M2b-3b) ---

test("a relation a move left across two works waits to be checked again", () => {
  const crossing = relation({ status: "disputed", crosses_works: true });
  assert.ok(correctableRelations([crossing]).length === 1, "it can be revoked or confirmed again");
  assert.match(relationStatusText(crossing), /^Crosses two works, waiting to be checked again/);
  const notes = relationNotes(relationPreview({ relation: crossing, can_revoke: true, can_confirm_again: false }));
  assert.ok(notes.some((note) => /first bring them into one work/.test(note)));
  const together = relationNotes(relationPreview({ relation: crossing, can_confirm_again: true }));
  assert.ok(!together.some((note) => /first bring them into one work/.test(note)));
});

function member(fileId: string, fields: Partial<FamilyMemberView>): FamilyMemberView {
  return {
    file_id: fileId,
    display_path: `Drafts/${fileId}.txt`,
    present: true,
    status: "confirmed_user",
    basis: "memory_on",
    placed: true,
    decided_at_ms: 0,
    snapshots: [],
    merged_from: null,
    ...fields,
  } as FamilyMemberView;
}

test("only confirmed members can be moved", () => {
  const members = [
    member("a", {}),
    member("b", { status: "confirmed_system" }),
    member("c", { status: "candidate" }),
    member("d", { status: "superseded" }),
  ];
  assert.deepEqual(
    movableMembers(members).map((m) => m.file_id),
    ["a", "b"],
  );
});

test("the move page says what moves, what then crosses two works and what pauses", () => {
  const preview: MovePreview = {
    family_id: "w1",
    target_family_id: null,
    moving: [member("notes", {})],
    records_moving: [record({ place: { ...place, display_path: "Drafts/notes.txt" } })],
    relations_kept: [relation({}), relation({ edge_id: "e2" })],
    relations_disputed: [relation({ edge_id: "e3", status: "disputed", crosses_works: true })],
    records_held: [record({})],
    overrides: [{ member_id: "m", file_id: "notes", display_path: "Drafts/notes.txt", rejected_in_family_id: "w2" }],
    page_token: "t",
  };
  const notes = moveNotes(preview, "a new work");
  assert.equal(notes[0], "Drafts/notes.txt leave this work and go to a new work. From then on they belong there by your decision.");
  assert.ok(notes.includes("Recording Drafts/notes.txt goes along: its switch, its limit, its waiting files and its gaps go with it."));
  assert.ok(notes.includes("2 relations between the moved files go along and still count."));
  assert.ok(notes.some((note) => /: it then joins two works and stops counting .* It is not revoked by itself\.$/.test(note)));
  assert.ok(notes.includes("Recording Drafts/essay.md is paused: the relation it went on by then joins two works. You then record it again or stop it."));
  assert.ok(notes.includes("You said earlier that Drafts/notes.txt is unrelated to that work. That judgement no longer counts."));
  assert.equal(notes[notes.length - 1], "No saved version is changed or deleted.");

  const plain = moveNotes({ ...preview, records_moving: [], relations_kept: [], relations_disputed: [], records_held: [], overrides: [] }, "x");
  assert.equal(plain.length, 2);
});

test("what the page says after a move, and when it is refused", () => {
  const outcome = (held: string[]) => ({ held_records: held }) as unknown as WorkDecisionOutcome;
  assert.equal(movedText(outcome([]), 1, "a new work"), "Moved. 1 file went to a new work.");
  assert.equal(
    movedText(outcome(["c1"]), 2, "a new work"),
    "Moved. 2 files went to a new work. 1 recording was paused: record again or stop.",
  );
  for (const code of ["page_outdated", "decision_conflict", "unknown_family", "candidate_pending"] as const) {
    assert.match(moveRefusal(code) ?? "", /Nothing was moved|decide it there first/);
  }
  assert.equal(moveRefusal("invalid_input"), null);
});

test("members an editor left at one path can be told apart (R2-MEM-09)", () => {
  const saved = (ms: number) => ({ ...version(1), captured_at_ms: ms });
  const old = member("old", {
    display_path: "Drafts/survey.md",
    present: false,
    snapshots: [saved(Date.UTC(2026, 9, 10, 9, 0, 1)), saved(Date.UTC(2026, 9, 10, 9, 0, 9))],
  });
  const now = member("f", { display_path: "Drafts/survey.md", snapshots: [saved(Date.UTC(2026, 9, 10, 9, 1, 0))] });
  const none = member("new", { display_path: "Drafts/survey.md", snapshots: [] });
  const recording = record({});
  const labels = [old, now, none].map((m) => memberLabel(m, recording));
  assert.equal(new Set(labels).size, 3, labels.join(" / "));
  assert.match(labels[0], /^Drafts\/survey\.md \(no longer at its place; 2 versions, saved .+ to .+\)$/);
  assert.match(labels[1], /^Drafts\/survey\.md \(recorded now; 1 version, saved .+\)$/);
  assert.equal(labels[2], "Drafts/survey.md (no version saved)");
  assert.match(memberLabel(now, record({ state: "stopped" })), /the file this recording looked at last/);
  // The move list shows the name, the folder and these notes apart.
  assert.equal(memberNotes(none, recording), "no version saved");
  assert.equal(memberLabel(old, recording), `Drafts/survey.md (${memberNotes(old, recording)})`);

  const preview = {
    family_id: "w1",
    target_family_id: null,
    moving: [old],
    records_moving: [],
    relations_kept: [],
    relations_disputed: [],
    records_held: [],
    overrides: [],
    page_token: "t",
  } as MovePreview;
  assert.match(moveNotes(preview, "a new work", (m) => memberLabel(m, recording))[0], /^Drafts\/survey\.md \(no longer at its place;/);
});

test("a record asked for from outside the page is shown only while Memory still has it (#198)", () => {
  const records = [record({ chain_id: "c1" }), record({ chain_id: "c2" })];
  assert.equal(recordToOpen(records, "c2")?.chain_id, "c2");
  assert.equal(recordToOpen(records, "c9"), null);
  assert.equal(recordToOpen([], "c1"), null);
  assert.equal(RECORD_GONE_TEXT, "This record is no longer available.");
});

function autoView(fields: Partial<AutoMemoryView>): AutoMemoryView {
  return {
    enabled: false,
    outdated: false,
    agreed_at_ms: null,
    provider: "openai",
    model: "gpt-6-luna",
    notice: "When this is on, ...",
    notice_version: 1,
    limit: 50,
    recording: 0,
    started_chain_ids: [],
    ...fields,
  };
}

test("the switch that keeps versions of organized files says how it stands (#201)", () => {
  assert.equal(AUTO_MEMORY_TITLE, "Keep versions of files Traeva organizes");
  const on = autoView({ enabled: true, agreed_at_ms: 1 });
  assert.equal(autoMemoryState(on), "on");
  assert.match(autoMemoryText(on, "10/13"), /^On since 10\/13\. .*described by gpt-6-luna\.$/);
  assert.equal(autoMemoryState(autoView({})), "off");
  assert.match(autoMemoryText(autoView({}), null), /^Off\. Read what it does/);
  // An agreement for another model or notice no longer counts.
  const outdated = autoView({ outdated: true });
  assert.equal(autoMemoryState(outdated), "outdated");
  assert.match(autoMemoryText(outdated, null), /changed since you agreed/);
  // No model yet: Settings first, even with an old agreement.
  assert.equal(autoMemoryState(autoView({ provider: null, model: null, outdated: true })), "no_model");
  assert.match(autoMemoryText(autoView({ model: null }), null), /in Settings first/);
});

test("at the limit the switch says it starts no new records; its records are labelled", () => {
  assert.equal(autoMemoryLimitText(autoView({ enabled: true, recording: 49 })), null);
  assert.equal(
    autoMemoryLimitText(autoView({ enabled: true, recording: 50 })),
    "Traeva starts no new records while 50 files are being recorded.",
  );
  assert.equal(autoMemoryLimitText(autoView({ enabled: false, recording: 60 })), null);
  const view = autoView({ started_chain_ids: ["c2"] });
  assert.equal(startedByTraeva(view, "c2"), true);
  assert.equal(startedByTraeva(view, "c1"), false);
  assert.equal(startedByTraeva(null, "c2"), false);
  assert.equal(STARTED_BY_TRAEVA, "Started by Traeva");
  assert.match(autoMemoryRefusal("page_outdated") ?? "", /notice changed/);
  assert.match(autoMemoryRefusal("model_not_configured") ?? "", /in Settings first/);
  assert.equal(autoMemoryRefusal("unknown_file"), null);
});

test("a list row shows a file's name first and its folder after it", () => {
  assert.deepEqual(namedPlace("/Users/student/Files/School/English/later-start-essay.txt"), {
    name: "later-start-essay.txt",
    folder: "/Users/student/Files/School/English",
  });
  // A file at the top of an authorized folder: the folder is that folder.
  assert.deepEqual(namedPlace("/Users/student/Files/notes.md"), { name: "notes.md", folder: "/Users/student/Files" });
  // Spaces and non-Latin names are kept as they are.
  assert.deepEqual(namedPlace("/Users/student/My Files/读书 笔记/第一章 草稿.docx"), {
    name: "第一章 草稿.docx",
    folder: "/Users/student/My Files/读书 笔记",
  });
  // No folder in the path (a record whose folder is not known).
  assert.deepEqual(namedPlace("survey.md"), { name: "survey.md", folder: "" });
  assert.deepEqual(namedPlace("/survey.md"), { name: "survey.md", folder: "/" });
});

test("the library's line counts files and the ones that wait for a decision", () => {
  assert.equal(libraryText([]), "No file is recorded yet.");
  assert.equal(libraryText([record({})]), "1 file on this Mac");
  const waiting = record({ chain_id: "c2", pending: { objects: 1, snapshots: 1, limit: 20 } });
  assert.equal(libraryText([record({}), waiting]), "2 files on this Mac · 1 needs your decision");
  assert.match(libraryText([waiting, { ...waiting, chain_id: "c3" }]), / · 2 need your decision$/);
});

test("a record waits for the user: files waiting, a paused record, or an unread file at its place", () => {
  assert.equal(needsDecision(record({})), false);
  assert.equal(needsDecision(record({ pending: { objects: 1, snapshots: 1, limit: 20 } })), true);
  const hold = { work_decision_id: "d", edge_id: null, held_at_ms: 1, why: "relation_revoked" as const };
  assert.equal(needsDecision(record({ hold })), true);
  const gap = (kind: GapKind, ended: number | null) => ({
    gap_id: "g", chain_id: "c1", kind, after: null, problem: null, started_at_ms: 1, ended_at_ms: ended, end_reason: null,
  });
  // Capture off and the file replaced: nothing waits in the list, but the
  // page asks about the file at the place.
  assert.equal(needsDecision(record({ open_gaps: [gap("replaced_uncaptured", null)] })), true);
  assert.equal(needsDecision(record({ open_gaps: [gap("memory_off", null)] })), false);
  assert.match(waitsText(record({ hold })), /open the file to see it/);
  assert.match(waitsText(record({ pending: { objects: 2, snapshots: 2, limit: 20 } })), /^2 files and 2 saved copies wait/);
});

test("a file's tabs, and the dot on This work", () => {
  assert.equal(recordTabText("versions"), "Versions");
  assert.equal(recordTabText("work"), "This work");
  assert.equal(recordTabText("waiting"), "Waiting files");
  assert.equal(tabHasNews("work", record({})), false);
  const hint = { family_id: "w2", own_version: version(1), same_bytes_as: version(4, "other") };
  assert.equal(tabHasNews("work", record({ same_content: [hint] })), true);
  assert.equal(tabHasNews("waiting", record({ same_content: [hint] })), false);
});

test("the library groups records: what waits for the user, what is recorded, what is stopped", () => {
  const recording = record({});
  assert.equal(libraryGroup(recording), "recording");
  const stopped = record({ state: "stopped" });
  assert.equal(libraryGroup(stopped), "stopped");
  // Everything the line says in amber waits for the user.
  assert.equal(libraryGroup(record({ root_active: false })), "needs");
  assert.equal(libraryGroup(record({ state: "lost" })), "needs");
  assert.equal(libraryGroup(record({ waiting_for_relocation: true })), "needs");
  assert.equal(libraryGroup(record({ pending: { ...recording.pending, objects: 1 } })), "needs");
  assert.equal(libraryGroup(record({ state: "stopped", hold: { work_decision_id: "d", edge_id: "e", held_at_ms: 1, why: "relation_revoked" } })), "needs");
});

test("a file's page counts what waits for the user once each", () => {
  assert.equal(needsCount(record({})), 0);
  assert.equal(needsCount(record({ waiting_for_relocation: true })), 1);
  assert.equal(
    needsCount(
      record({
        state: "stopped",
        hold: { work_decision_id: "d", edge_id: "e", held_at_ms: 1, why: "relation_revoked" },
        pending: { objects: 2, snapshots: 2, limit: 20 },
      }),
    ),
    2,
  );
});
