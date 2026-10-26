// Mirrors of the automatic mode's DTOs (#168, organize-spec.md §13;
// crates/traeva-core/src/auto_drop.rs). The Rust structs are the source of
// truth. The core decides everything: the window only shows these and
// collects the user's choices.

import type { ActionStatus } from "../organize/types";

export interface ScopeView {
  /** Inside the target root; "" is the root itself. */
  relative_path: string;
  display_path: string;
  /** Still an existing folder of an authorized folder. */
  valid: boolean;
}

export interface AutoDropSettingsView {
  /** On, and the agreement counts. */
  enabled: boolean;
  /** Show this before turning the mode on; the user agrees to this text. */
  notice: string;
  notice_version: number;
  consent_id: string | null;
  agreed_at_ms: number | null;
  target_root_id: string | null;
  scopes: ScopeView[];
  /** Why an open agreement does not count now. */
  problem: string | null;
}

/** Turns the mode on (with the notice version the user saw) or off. */
export interface AutoDropRequest {
  enable: boolean;
  notice_version?: number | null;
  target_root_id?: string | null;
  /** Existing folders of the target root ("" is the root), 1 to 20. */
  scopes?: string[];
}

export type DropState =
  | "waiting"
  | "needs_source_authorization"
  | "working"
  | "needs_answer"
  | "handed_over"
  | "moved"
  | "already_in_place"
  | "stopped"
  | "not_processed"
  | "cancelled";

export interface DropItemView {
  drop_id: string;
  file_name: string;
  /** The drop it came in. A stop or a cancel only reaches the files after it in the same drop. */
  batch_id: string;
  /** When it was dropped (ms since 1970), to tell apart drops of the same file. */
  received_at_ms: number;
  state: DropState;
  /** Handled by the automatic mode (false: the normal flow). */
  automatic: boolean;
  /** In needs_source_authorization: the folder to authorize, shown as is. */
  folder_to_authorize: string | null;
  /** The file's operation: answer its question, or go on in the normal flow. */
  operation_id: string | null;
  /** The plan run (or, handed over, the one the normal flow confirmed). Never apply it. */
  plan_id: string | null;
  /**
   * The action: once moved, for its Undo; once the normal flow confirmed a
   * handed-over file; or when it stopped after the move started (the file
   * may have moved). Read its current state with get_action.
   */
  action_id: string | null;
  moved_to: string | null;
  /** Why it stopped or was handed over. Never file text. */
  reason: string | null;
  /**
   * It waits for the user now, as the core works it out when the queue is
   * read (#208): its folder to authorize, a question, or a normal flow
   * still open. The window ends a normal flow it leaves without confirming
   * by cancelling its operation.
   */
  needs_user: boolean;
  /** The current state of action_id's action, when there is one. */
  action_status: ActionStatus | null;
}

export interface DropQueueView {
  items: DropItemView[];
  /** A background pass is working on the queue. */
  running: boolean;
}
