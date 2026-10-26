-- Schema version 1: the seven minimal tables of docs/architecture.md §5.
--
-- Rules for migration files:
--   * Never edit this file after it is committed. Schema changes go in a new numbered file.
--   * No BEGIN/COMMIT here: store.rs runs each migration in its own transaction.
--   * IDs are UUID strings. Times are Unix milliseconds (UTC).
--   * *_json columns hold small JSON values whose shape is defined by Rust structs.
--     Changing that shape for stored rows also needs a migration.
--   * STRICT tables reject values of the wrong type.
--   * CHECK lists only use values that the frozen spec names.

-- A folder the user authorized. Only active or revoked; re-authorizing creates a new root_id.
CREATE TABLE authorized_roots (
    root_id         TEXT NOT NULL PRIMARY KEY,
    display_path    TEXT NOT NULL,
    evidence_json   TEXT NOT NULL CHECK (json_valid(evidence_json)), -- directory and volume evidence
    status          TEXT NOT NULL CHECK (status IN ('active', 'revoked')),
    granted_at_ms   INTEGER NOT NULL,
    revoked_at_ms   INTEGER,
    CHECK ((status = 'revoked') = (revoked_at_ms IS NOT NULL))
) STRICT;

-- One registered file: identity (file_id), current location and latest direct observation.
-- No source or classification back-pointers; future tables reference file_id instead.
CREATE TABLE files (
    file_id             TEXT NOT NULL PRIMARY KEY,
    root_id             TEXT NOT NULL REFERENCES authorized_roots (root_id),
    relative_path       TEXT NOT NULL,
    basename            TEXT NOT NULL,
    presence            TEXT NOT NULL CHECK (presence IN ('present', 'missing')),
    sha256              TEXT NOT NULL CHECK (length(sha256) = 64),
    byte_length         INTEGER NOT NULL CHECK (byte_length >= 0),
    observation_json    TEXT NOT NULL CHECK (json_valid(observation_json)), -- physical evidence, mtime, link count
    observed_at_ms      INTEGER NOT NULL,
    record_revision     INTEGER NOT NULL CHECK (record_revision >= 1),
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL
) STRICT;

-- A current location holds at most one present file. Same-content copies at other paths
-- are separate rows with their own file_id.
CREATE UNIQUE INDEX files_one_present_per_path
    ON files (root_id, relative_path) WHERE presence = 'present';

-- One model request: non-content metadata plus the validated, length-limited result.
-- Never the extracted text, a raw prompt, the raw response or credentials.
CREATE TABLE analyses (
    request_id              TEXT NOT NULL PRIMARY KEY,
    file_id                 TEXT NOT NULL REFERENCES files (file_id),
    file_sha256             TEXT NOT NULL CHECK (length(file_sha256) = 64),
    file_revision           INTEGER NOT NULL,
    provider                TEXT NOT NULL,
    model                   TEXT NOT NULL,
    requested_at_ms         INTEGER NOT NULL,
    prompt_version          TEXT NOT NULL,
    output_schema_version   TEXT NOT NULL,
    extractor_version       TEXT NOT NULL,
    app_version             TEXT NOT NULL,
    request_meta_json       TEXT NOT NULL CHECK (json_valid(request_meta_json)), -- parameters, candidate IDs, truncation, char count
    status                  TEXT NOT NULL,
    latency_ms              INTEGER,
    usage_json              TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    provider_request_id     TEXT,
    result_json             TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_code              TEXT
) STRICT;

-- An immutable ActionPlan. Everything above `lifecycle` is the execution payload and
-- never changes (see the trigger below). Consumption is not stored here: a plan is
-- consumed exactly when an actions row references it. Expiry is derived from created_at_ms.
CREATE TABLE plans (
    plan_id                 TEXT NOT NULL PRIMARY KEY,
    kind                    TEXT NOT NULL CHECK (kind IN ('organize', 'undo')),
    file_id                 TEXT NOT NULL REFERENCES files (file_id),
    source_root_id          TEXT NOT NULL REFERENCES authorized_roots (root_id),
    source_relative_path    TEXT NOT NULL,
    expected_source_json    TEXT NOT NULL CHECK (json_valid(expected_source_json)), -- full expected observation
    target_root_id          TEXT NOT NULL REFERENCES authorized_roots (root_id),
    target_segments_json    TEXT NOT NULL CHECK (json_valid(target_segments_json)), -- target folder, relative segments
    basename                TEXT NOT NULL, -- kept unchanged from the source
    folder_chain_json       TEXT NOT NULL CHECK (json_valid(folder_chain_json)), -- evidence for existing target folders
    folders_to_create_json  TEXT NOT NULL CHECK (json_valid(folders_to_create_json)),
    analysis_request_id     TEXT REFERENCES analyses (request_id),
    user_choice             TEXT CHECK (user_choice IN ('accept', 'correct', 'manual')),
    undo_of_action_id       TEXT REFERENCES actions (action_id),
    created_at_ms           INTEGER NOT NULL,
    lifecycle               TEXT NOT NULL DEFAULT 'open'
                                CHECK (lifecycle IN ('open', 'cancelled', 'superseded', 'invalidated')),
    lifecycle_changed_at_ms INTEGER,
    lifecycle_reason        TEXT,
    CHECK ((kind = 'organize') = (user_choice IS NOT NULL)),
    CHECK ((kind = 'undo') = (undo_of_action_id IS NOT NULL)),
    CHECK ((lifecycle = 'open') = (lifecycle_changed_at_ms IS NULL))
) STRICT;

CREATE TRIGGER plans_payload_is_immutable
BEFORE UPDATE OF plan_id, kind, file_id, source_root_id, source_relative_path,
    expected_source_json, target_root_id, target_segments_json, basename,
    folder_chain_json, folders_to_create_json, analysis_request_id, user_choice,
    undo_of_action_id, created_at_ms
ON plans
BEGIN
    SELECT RAISE(ABORT, 'plan payload is immutable; create a new plan instead');
END;

-- One confirmed plan becomes at most one action: plan_id is NOT NULL UNIQUE.
CREATE TABLE actions (
    action_id           TEXT NOT NULL PRIMARY KEY,
    plan_id             TEXT NOT NULL UNIQUE REFERENCES plans (plan_id),
    kind                TEXT NOT NULL CHECK (kind IN ('organize', 'undo')),
    undo_of_action_id   TEXT REFERENCES actions (action_id),
    status              TEXT NOT NULL CHECK (status IN (
                            'prepared', 'running', 'committed',
                            'failed_no_move', 'needs_user_decision', 'undone')),
    preconditions_json  TEXT NOT NULL CHECK (json_valid(preconditions_json)), -- full source/target preconditions
    created_at_ms       INTEGER NOT NULL,
    committed_at_ms     INTEGER,
    recovered_at_ms     INTEGER,
    result_json         TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    CHECK ((kind = 'undo') = (undo_of_action_id IS NOT NULL)),
    CHECK (status <> 'undone' OR kind = 'organize'),
    CHECK (status NOT IN ('committed', 'undone') OR committed_at_ms IS NOT NULL)
) STRICT;

-- An original action has at most one successful Undo.
CREATE UNIQUE INDEX actions_one_committed_undo
    ON actions (undo_of_action_id) WHERE kind = 'undo' AND status = 'committed';

-- The journal: ordered intent/result steps of one action, including directory receipts.
CREATE TABLE action_events (
    action_id       TEXT NOT NULL REFERENCES actions (action_id),
    sequence        INTEGER NOT NULL CHECK (sequence >= 1),
    step            TEXT NOT NULL,
    phase           TEXT NOT NULL CHECK (phase IN ('intent', 'result')),
    detail_json     TEXT NOT NULL CHECK (json_valid(detail_json)), -- evidence, receipt or sanitized error
    recorded_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (action_id, sequence)
) STRICT;

CREATE TRIGGER action_events_no_update BEFORE UPDATE ON action_events
BEGIN
    SELECT RAISE(ABORT, 'action_events is append-only');
END;

CREATE TRIGGER action_events_no_delete BEFORE DELETE ON action_events
BEGIN
    SELECT RAISE(ABORT, 'action_events is append-only');
END;

-- Where a file has been, appended once per intake, organize or undo.
CREATE TABLE path_history (
    path_event_id       TEXT NOT NULL PRIMARY KEY,
    file_id             TEXT NOT NULL REFERENCES files (file_id),
    from_root_id        TEXT REFERENCES authorized_roots (root_id),
    from_relative_path  TEXT,
    to_root_id          TEXT NOT NULL REFERENCES authorized_roots (root_id),
    to_relative_path    TEXT NOT NULL,
    reason              TEXT NOT NULL CHECK (reason IN ('intake', 'organize', 'undo')),
    action_id           TEXT REFERENCES actions (action_id),
    evidence_json       TEXT NOT NULL CHECK (json_valid(evidence_json)),
    recorded_at_ms      INTEGER NOT NULL,
    UNIQUE (action_id, reason),
    CHECK ((reason = 'intake') = (action_id IS NULL)),
    CHECK ((reason = 'intake') = (from_root_id IS NULL)),
    CHECK ((from_root_id IS NULL) = (from_relative_path IS NULL))
) STRICT;

CREATE TRIGGER path_history_no_update BEFORE UPDATE ON path_history
BEGIN
    SELECT RAISE(ABORT, 'path_history is append-only');
END;

CREATE TRIGGER path_history_no_delete BEFORE DELETE ON path_history
BEGIN
    SELECT RAISE(ABORT, 'path_history is append-only');
END;
