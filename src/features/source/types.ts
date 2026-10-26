export type SourceStatus = "unknown" | "likely" | "user_confirmed";
export type SourceRelation = "download" | "copy";
export type SourceLinkKind = "initial" | "final" | "referrer";

export interface SourceCandidateView {
  file_id: string;
  display_path: string;
  reason: string;
  can_confirm: boolean;
}

export interface SourceAssociationView {
  file_id: string;
  display_path: string;
  relation: SourceRelation;
  basis: "user_confirmed";
  note: string;
  confirmed_at_ms: number;
  current_location_confirmed: boolean;
}

export interface SourceRecordView {
  source_id: string;
  profile_id: string;
  download_id: string;
  download_state: "in_progress" | "complete" | "interrupted";
  status: SourceStatus;
  filename: string;
  initial_url: string | null;
  final_url: string | null;
  referrer: string | null;
  title: string | null;
  started_at_ms: number | null;
  completed_at_ms: number | null;
  imported_at_ms: number;
  updated_at_ms: number;
  reason: string | null;
  candidates: SourceCandidateView[];
  association: SourceAssociationView | null;
}

export interface SourceState {
  enabled: boolean;
  records: SourceRecordView[];
}

export interface SourceConnection {
  endpoint: string | null;
  pairing_token: string | null;
  enabled: boolean;
  active: boolean;
  error: string | null;
}

export interface SourceApi {
  state(): Promise<SourceState>;
  setEnabled(enabled: boolean): Promise<SourceState>;
  refresh(sourceId: string): Promise<SourceRecordView>;
  chooseFile(sourceId: string): Promise<SourceRecordView | null>;
  confirm(sourceId: string, fileId: string, relation: SourceRelation, note: string): Promise<SourceRecordView>;
  reject(sourceId: string, fileId: string): Promise<SourceRecordView>;
  unlink(sourceId: string, note: string): Promise<SourceRecordView>;
  openLink(sourceId: string, kind: SourceLinkKind): Promise<void>;
  connection(): Promise<SourceConnection>;
}
