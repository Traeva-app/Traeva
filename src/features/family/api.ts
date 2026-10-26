import { invoke } from "@tauri-apps/api/core";
import type { FamilyApi } from "./types";

// One function per command in src-tauri/src/family_commands.rs.
export const familyApi: FamilyApi = {
  listRecords: () => invoke("list_memory_records"),
  startRecord: (rootId) => invoke("start_memory_record", { root_id: rootId }),
  startForFile: (fileId) => invoke("start_memory_for_file", { file_id: fileId }),
  stopRecord: (chainId) => invoke("stop_memory_record", { chain_id: chainId }),
  family: (familyId) => invoke("get_memory_family", { family_id: familyId }),
  candidateCapture: (chainId) => invoke("get_candidate_capture", { chain_id: chainId }),
  setCandidateCapture: (request) => invoke("set_candidate_capture", { request }),
  pendingCandidates: (chainId) => invoke("get_pending_candidates", { chain_id: chainId }),
  decide: (request) => invoke("decide_pending_candidates", { request }),
  previewRelocation: (chainId, rootId) =>
    invoke("preview_memory_relocation", { chain_id: chainId, root_id: rootId }),
  relocate: (request) => invoke("relocate_memory_record", { request }),
  listWorks: () => invoke("list_memory_works"),
  previewMerge: (familyId, otherFamilyId) =>
    invoke("preview_memory_merge", { family_id: familyId, other_family_id: otherFamilyId }),
  mergeWorks: (request) => invoke("merge_memory_works", { request }),
  previewAssociation: (familyId, rootId) =>
    invoke("preview_memory_association", { family_id: familyId, root_id: rootId }),
  previewAssociationForFile: (familyId, fileId) =>
    invoke("preview_memory_association_for_file", { family_id: familyId, file_id: fileId }),
  associate: (request) => invoke("associate_memory_file", { request }),
  previewRelation: (edgeId) => invoke("preview_memory_relation", { edge_id: edgeId }),
  correctRelation: (request) => invoke("correct_memory_relation", { request }),
  previewMove: (query) => invoke("preview_memory_move", { query }),
  moveMembers: (request) => invoke("move_memory_members", { request }),
  autoMemory: () => invoke("get_auto_memory"),
  setAutoMemory: (request) => invoke("set_auto_memory", { request }),
};
