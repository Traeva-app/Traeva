// The only place the window calls Rust. Each function is one Tauri command;
// a rejected Promise carries an AppError from the core (or, for input that
// does not fit the contract, Tauri's own message). Argument names are
// snake_case, like the Rust side.

import { invoke } from "@tauri-apps/api/core";
import type {
  ActionView,
  AppState,
  BuildOutcome,
  ClarificationAnswer,
  FolderCatalogView,
  ImageSending,
  ModelSettingsInput,
  ModelSettingsView,
  OcrState,
  OperationView,
  PlanRequest,
  PlanView,
  RootView,
  UndoPreview,
} from "./types";

export function getAppState(): Promise<AppState> {
  return invoke<AppState>("get_app_state");
}

/** Opens the native folder picker; null when the user closes it. */
export function chooseRoot(): Promise<RootView | null> {
  return invoke<RootView | null>("choose_root");
}

export function revokeRoot(rootId: string): Promise<RootView> {
  return invoke<RootView>("revoke_root", { root_id: rootId });
}

/** Opens the native file picker; the file is then read in the background. */
export function chooseFile(sourceRootId: string, targetRootId: string): Promise<OperationView | null> {
  return invoke<OperationView | null>("choose_file", {
    source_root_id: sourceRootId,
    target_root_id: targetRootId,
  });
}

export function getOperation(operationId: string): Promise<OperationView> {
  return invoke<OperationView>("get_operation", { operation_id: operationId });
}

export function cancelOperation(operationId: string): Promise<OperationView> {
  return invoke<OperationView>("cancel_operation", { operation_id: operationId });
}

/** Starts (or retries) the analysis; the model is asked in the background. */
/** `sendImage`: the user just said yes to sending this file's image (§14.10, "ask each time"). */
export function startAnalysis(operationId: string, sendImage?: boolean): Promise<OperationView> {
  return invoke<OperationView>(
    "start_analysis",
    sendImage === undefined ? { operation_id: operationId } : { operation_id: operationId, send_image: sendImage },
  );
}

/** Answers the current question; the core locks the answer before asking again. */
export function answerClarification(
  operationId: string,
  answer: ClarificationAnswer,
): Promise<OperationView> {
  return invoke<OperationView>("answer_clarification", { operation_id: operationId, answer });
}

/** Leaves the analysis for picking the folder by hand. */
export function chooseManualLocation(operationId: string): Promise<OperationView> {
  return invoke<OperationView>("choose_manual_location", { operation_id: operationId });
}

export function getModelSettings(): Promise<ModelSettingsView> {
  return invoke<ModelSettingsView>("get_model_settings");
}

/** Saves the provider and model, and a new API key if given (Keychain only). */
export function saveModelSettings(input: ModelSettingsInput): Promise<ModelSettingsView> {
  return invoke<ModelSettingsView>("save_model_settings", { input });
}

/** Agrees to (or stops) sending to the model. It never allows moving anything. */
export function setAnalysisConsent(agree: boolean): Promise<ModelSettingsView> {
  return invoke<ModelSettingsView>("set_analysis_consent", { agree });
}

/** Turns sending images to the model on (ask each time / without asking) or off (§14.10). */
export function setImageSending(mode: ImageSending): Promise<ModelSettingsView> {
  return invoke<ModelSettingsView>("set_image_sending", { mode });
}

/** Where text recognition for scanned PDFs stands in this launch. */
export function getOcrState(): Promise<OcrState> {
  return invoke<OcrState>("get_ocr_state");
}

export function listFolders(rootId: string): Promise<FolderCatalogView> {
  return invoke<FolderCatalogView>("list_folders", { root_id: rootId });
}

export function buildPlan(request: PlanRequest): Promise<BuildOutcome> {
  return invoke<BuildOutcome>("build_plan", { request });
}

export function getPlan(planId: string): Promise<PlanView> {
  return invoke<PlanView>("get_plan", { plan_id: planId });
}

export function cancelPlan(planId: string): Promise<PlanView> {
  return invoke<PlanView>("cancel_plan", { plan_id: planId });
}

/** Resolves when the action is settled. Poll getPlan/getAction meanwhile. */
export function applyPlan(planId: string): Promise<ActionView> {
  return invoke<ActionView>("apply_plan", { plan_id: planId });
}

export function getAction(actionId: string): Promise<ActionView> {
  return invoke<ActionView>("get_action", { action_id: actionId });
}

export function previewUndo(actionId: string): Promise<UndoPreview> {
  return invoke<UndoPreview>("preview_undo", { action_id: actionId });
}

export function reconcileAction(actionId: string): Promise<ActionView> {
  return invoke<ActionView>("reconcile_action", { action_id: actionId });
}
