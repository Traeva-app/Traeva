// Tests for folderTree.ts. Run with `npm test`.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  childrenOf,
  folderHints,
  folderName,
  holdsChosen,
  likelyParents,
  opensInto,
  otherParents,
  pathInside,
  rootFolder,
  topLevel,
  trail,
} from "./folderTree.ts";
import type { CandidateView, FolderCatalogView, FolderView, SuggestionView } from "./types.ts";

function folder(id: string, path: string, parent: string | null, guarded: string | null = null): FolderView {
  return {
    folder_id: id,
    parent_id: parent,
    relative_path: path,
    depth: path === "" ? 0 : path.split("/").length,
    protected: guarded,
  };
}

// 课件 › Chem › AP, as in the request; a project folder (the only kind of
// protected folder the core lists; hidden folders and packages are left
// out of the listing); folders without subfolders.
const CATALOG: FolderCatalogView = {
  root_id: "r1",
  catalog_id: "cat-1",
  truncated: false,
  folders: [
    folder("d0", "", null),
    folder("d1", "课件", "d0"),
    folder("d2", "课件/Chem", "d1"),
    folder("d3", "课件/Chem/AP", "d2"),
    folder("d4", "课件/History", "d1"),
    folder("d5", "Book Club", "d0"),
    folder("d6", "Personal", "d0"),
    folder("d7", "Personal/Money", "d6"),
    folder("d8", "Proj", "d0", "project folder (contains “.git”)"),
    folder("d9", "Robotics", "d0"),
  ],
};

function candidate(id: string, path: string, newSegments: string[] = []): CandidateView {
  return { candidate_id: id, folder_path: path, new_segments: newSegments, reason: "r", evidence_ids: [] };
}

function suggestion(paths: string[], recommended: number | null): SuggestionView {
  return {
    request_id: "q",
    summary: "s",
    confidence: null,
    recommended_id: recommended === null ? null : `c${recommended}`,
    candidates: paths.map((path, index) => candidate(`c${index}`, path)),
  };
}

const ids = (folders: FolderView[]) => folders.map((f) => f.folder_id);

test("the listing's folder itself, its top folders and each folder's subfolders", () => {
  assert.equal(rootFolder(CATALOG)?.folder_id, "d0");
  assert.deepEqual(ids(topLevel(CATALOG)), ["d1", "d5", "d6", "d8", "d9"]);
  assert.deepEqual(ids(childrenOf(CATALOG, "d1")), ["d2", "d4"]);
  assert.deepEqual(ids(childrenOf(CATALOG, "d3")), []);
  assert.equal(rootFolder({ ...CATALOG, folders: [] }), null);
  assert.deepEqual(topLevel({ ...CATALOG, folders: [] }), []);
});

test("the places count in order: the recommended one first, then the model's order", () => {
  assert.deepEqual(folderHints(null), []);
  assert.deepEqual(folderHints(suggestion(["Personal", "课件/Chem/AP", "Book Club"], null)), [
    "Personal",
    "课件/Chem/AP",
    "Book Club",
  ]);
  assert.deepEqual(folderHints(suggestion(["Personal", "课件/Chem/AP", "Book Club"], 1)), [
    "课件/Chem/AP",
    "Personal",
    "Book Club",
  ]);
  // A recommendation that matches no candidate changes nothing.
  assert.deepEqual(folderHints({ ...suggestion(["Personal", "Book Club"], null), recommended_id: "zz" }), [
    "Personal",
    "Book Club",
  ]);
});

test("a place with new folders counts only for the folder that exists", () => {
  // 课件 + a new Bio inside: 课件. A new folder at the top: nothing (it does not exist yet).
  const places: SuggestionView = {
    request_id: "q",
    summary: "s",
    confidence: 0.9,
    recommended_id: "c0",
    candidates: [candidate("c0", "", ["Chem"]), candidate("c1", "课件", ["Bio"])],
  };
  assert.deepEqual(folderHints(places), ["", "课件"]);
  assert.deepEqual(ids(likelyParents(CATALOG, folderHints(places))), ["d1"]);
});

test("likely folders: each place's top folder, once, at most three", () => {
  // A deep place counts for its top folder.
  assert.deepEqual(ids(likelyParents(CATALOG, ["课件/Chem/AP"])), ["d1"]);
  // In order, and the same top folder only once.
  assert.deepEqual(ids(likelyParents(CATALOG, ["Personal/Money", "课件/History", "Personal", "课件/Chem"])), [
    "d6",
    "d1",
  ]);
  // Cut to three.
  assert.deepEqual(ids(likelyParents(CATALOG, ["Robotics", "Book Club", "Personal", "课件"])), ["d9", "d5", "d6"]);
  // The folder itself, a folder the listing does not have and a protected one are left out.
  assert.deepEqual(ids(likelyParents(CATALOG, ["", "Labs/2026", "Proj/src", "Book Club"])), ["d5"]);
  // No places: none, so the whole list shows instead.
  assert.deepEqual(likelyParents(CATALOG, []), []);
});

test("the other top folders keep the listing's order and leave out the likely ones", () => {
  const likely = likelyParents(CATALOG, ["Personal", "课件"]);
  assert.deepEqual(ids(otherParents(CATALOG, likely)), ["d5", "d8", "d9"]);
  assert.deepEqual(ids(otherParents(CATALOG, [])), ["d1", "d5", "d6", "d8", "d9"]);
});

test("the path above a level runs from the top folder down, without the folder itself", () => {
  assert.deepEqual(ids(trail(CATALOG, "d3")), ["d1", "d2", "d3"]);
  assert.deepEqual(ids(trail(CATALOG, "d1")), ["d1"]);
  assert.deepEqual(ids(trail(CATALOG, "d0")), []);
  assert.deepEqual(ids(trail(CATALOG, "missing")), []);
  // A loop in a broken listing ends instead of running forever.
  const loop: FolderCatalogView = { ...CATALOG, folders: [folder("a", "A", "b"), folder("b", "A/B", "a")] };
  assert.deepEqual(ids(trail(loop, "b")), ["a", "b"]);
});

test("names: a folder's last part; the folder itself says so", () => {
  assert.equal(folderName(folder("d3", "课件/Chem/AP", "d2"), "School"), "AP");
  assert.equal(folderName(folder("d0", "", null), "School"), "School (the folder itself)");
  assert.equal(folderName(folder("d0", "", null), ""), "(the folder itself)");
});

test("a folder opens when it has subfolders and is not protected; otherwise it is chosen", () => {
  assert.equal(opensInto(CATALOG, folder("d1", "课件", "d0")), true);
  assert.equal(opensInto(CATALOG, folder("d3", "课件/Chem/AP", "d2")), false);
  assert.equal(opensInto(CATALOG, folder("d5", "Book Club", "d0")), false);
  const guarded: FolderCatalogView = {
    ...CATALOG,
    folders: [...CATALOG.folders, folder("d10", "Proj/src", "d8", "project folder (contains “.git”)")],
  };
  assert.equal(opensInto(guarded, guarded.folders.find((f) => f.folder_id === "d8")!), false);
});

test("a folder says when the choice or the file is deeper inside it", () => {
  // AP chosen: 课件 and Chem hold it; AP itself and other folders do not.
  assert.equal(holdsChosen(CATALOG, "d1", "d3"), true);
  assert.equal(holdsChosen(CATALOG, "d2", "d3"), true);
  assert.equal(holdsChosen(CATALOG, "d3", "d3"), false);
  assert.equal(holdsChosen(CATALOG, "d6", "d3"), false);
  assert.equal(holdsChosen(CATALOG, "d1", ""), false);
  // Never the authorized folder itself, which holds everything.
  assert.equal(holdsChosen(CATALOG, "d0", "d3"), false);
  // The file in 课件/Chem: 课件 holds it; Chem is where it is (said separately).
  assert.equal(pathInside("课件/Chem", "课件"), true);
  assert.equal(pathInside("课件/Chem", "课件/Chem"), false);
  assert.equal(pathInside("课件/Chemistry", "课件/Chem"), false);
  assert.equal(pathInside("课件/Chem", ""), false);
  assert.equal(pathInside(null, "课件"), false);
});
