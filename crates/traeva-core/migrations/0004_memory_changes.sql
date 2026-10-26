-- Schema version 4: Memory's changes between saved versions (#33).
-- docs/development/shared-contract.md §4.6 has the boundaries; changes.rs
-- and memory_consent.rs explain the rules.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.
--
-- No table here holds the file's lines. A change keeps the model's short
-- description (checked not to copy 60 or more characters of the text) and
-- the line numbers of its evidence; the lines themselves are read again from
-- the saved versions when someone looks.

-- The user's agreement that Memory may send the changed lines of one file to
-- the model, for this file's versions now and later. Separate from
-- Organize's consent. Withdrawing ends it for good; agreeing again is a new
-- row. Rows are never deleted.
CREATE TABLE memory_consents (
    consent_id          TEXT NOT NULL PRIMARY KEY,
    file_id             TEXT NOT NULL REFERENCES files (file_id),
    provider            TEXT NOT NULL,
    model               TEXT NOT NULL,
    notice_version      INTEGER NOT NULL CHECK (notice_version >= 1),
    agreed_at_ms        INTEGER NOT NULL,
    withdrawn_at_ms     INTEGER
) STRICT;

-- A file has at most one agreement that is not withdrawn.
CREATE UNIQUE INDEX memory_consents_one_open_per_file
    ON memory_consents (file_id) WHERE withdrawn_at_ms IS NULL;

CREATE TRIGGER memory_consents_fixed_columns
BEFORE UPDATE OF consent_id, file_id, provider, model, notice_version, agreed_at_ms
ON memory_consents
BEGIN
    SELECT RAISE(ABORT, 'an agreement keeps what it agreed to');
END;

CREATE TRIGGER memory_consents_withdrawal_is_final
BEFORE UPDATE ON memory_consents
WHEN OLD.withdrawn_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a withdrawn agreement stays withdrawn');
END;

CREATE TRIGGER memory_consents_no_delete BEFORE DELETE ON memory_consents
BEGIN
    SELECT RAISE(ABORT, 'agreements are never deleted');
END;

-- Two consecutive saved versions, compared on this Mac (no model): did the
-- text change? Written once per pair and rule version, never changed.
--   changed         the text differs in `hunks` places; the model may describe them
--   no_text_change  the bytes differ but the text is the same (line breaks, BOM)
--   unreadable      one side has no text (unsupported type or broken bytes), or
--                   its saved copy is damaged; `note` says which. A copy that is
--                   only missing is not recorded: it can come back.
CREATE TABLE memory_pairs (
    pair_id             TEXT NOT NULL PRIMARY KEY,
    before_snapshot_id  TEXT NOT NULL REFERENCES snapshots (snapshot_id),
    after_snapshot_id   TEXT NOT NULL REFERENCES snapshots (snapshot_id),
    differ_version      TEXT NOT NULL,
    outcome             TEXT NOT NULL CHECK (outcome IN ('changed', 'no_text_change', 'unreadable')),
    before_reader       TEXT NOT NULL,
    after_reader        TEXT NOT NULL,
    -- Whether each side's text was complete or cut at the reader's limit.
    before_complete     INTEGER NOT NULL CHECK (before_complete IN (0, 1)),
    after_complete      INTEGER NOT NULL CHECK (after_complete IN (0, 1)),
    hunks               INTEGER NOT NULL CHECK (hunks >= 0),
    note                TEXT,
    compared_at_ms      INTEGER NOT NULL,
    CHECK (before_snapshot_id <> after_snapshot_id),
    CHECK ((outcome = 'changed') = (hunks > 0)),
    -- A new reader (or comparison rule) compares the pair again; the old row stays.
    UNIQUE (before_snapshot_id, after_snapshot_id, differ_version, before_reader, after_reader)
) STRICT;

CREATE TRIGGER memory_pairs_no_update BEFORE UPDATE ON memory_pairs
BEGIN
    SELECT RAISE(ABORT, 'a comparison is never changed');
END;

CREATE TRIGGER memory_pairs_no_delete BEFORE DELETE ON memory_pairs
BEGIN
    SELECT RAISE(ABORT, 'comparisons are never deleted');
END;

-- One request to the model about one changed pair. Only non-content facts:
-- which agreement allowed it, provider, model, versions, sizes, outcome.
--   analyzing       sent (or about to be), no outcome yet
--   analyzed        the answer passed every check; its changes are in memory_changes
--   invalid_output  the answer came back but failed the checks; nothing kept
--   failed          no usable answer (network, provider, interrupted); error_code says why
--   cancelled       the agreement or the model setup changed: not sent, or the
--                   answer was dropped
CREATE TABLE memory_change_analyses (
    analysis_id         TEXT NOT NULL PRIMARY KEY,
    pair_id             TEXT NOT NULL REFERENCES memory_pairs (pair_id),
    consent_id          TEXT NOT NULL REFERENCES memory_consents (consent_id),
    provider            TEXT NOT NULL,
    model               TEXT NOT NULL,
    -- The model settings' generation (settings.rs) when the request started:
    -- a change of key, provider or model starts the failure count again.
    settings_generation INTEGER NOT NULL,
    prompt_version      TEXT NOT NULL,
    schema_version      TEXT NOT NULL,
    status              TEXT NOT NULL
        CHECK (status IN ('analyzing', 'analyzed', 'invalid_output', 'failed', 'cancelled')),
    -- 0 when the request could not even be built (too large, copy missing).
    sent_hunks          INTEGER NOT NULL CHECK (sent_hunks >= 0),
    left_out_hunks      INTEGER NOT NULL CHECK (left_out_hunks >= 0),
    request_bytes       INTEGER NOT NULL CHECK (request_bytes >= 0),
    started_at_ms       INTEGER NOT NULL,
    finished_at_ms      INTEGER,
    error_code          TEXT,
    retryable           INTEGER CHECK (retryable IN (0, 1)),
    -- Hunk ids the answer called minor (spacing, typos), as a JSON list.
    minor_hunks_json    TEXT CHECK (minor_hunks_json IS NULL OR json_valid(minor_hunks_json)),
    latency_ms          INTEGER,
    usage_json          TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    provider_request_id TEXT,
    CHECK ((status = 'analyzing') = (finished_at_ms IS NULL)),
    CHECK ((status = 'failed') = (error_code IS NOT NULL)),
    CHECK ((status = 'analyzed') = (minor_hunks_json IS NOT NULL))
) STRICT;

-- At most one request of a pair at a time, and at most one accepted answer.
CREATE UNIQUE INDEX memory_change_analyses_one_running
    ON memory_change_analyses (pair_id) WHERE status = 'analyzing';
CREATE UNIQUE INDEX memory_change_analyses_one_result
    ON memory_change_analyses (pair_id) WHERE status = 'analyzed';

CREATE TRIGGER memory_change_analyses_fixed_columns
BEFORE UPDATE OF analysis_id, pair_id, consent_id, provider, model, settings_generation,
                 prompt_version, schema_version, sent_hunks, left_out_hunks, request_bytes,
                 started_at_ms
ON memory_change_analyses
BEGIN
    SELECT RAISE(ABORT, 'a request keeps what was sent');
END;

-- Only a request still waiting gets its outcome; an outcome is final.
CREATE TRIGGER memory_change_analyses_outcome_is_final
BEFORE UPDATE ON memory_change_analyses
WHEN OLD.status <> 'analyzing'
BEGIN
    SELECT RAISE(ABORT, 'an outcome is final');
END;

CREATE TRIGGER memory_change_analyses_no_delete BEFORE DELETE ON memory_change_analyses
BEGIN
    SELECT RAISE(ABORT, 'requests are never deleted');
END;

-- One meaningful change the model described, checked against the diff.
-- `evidence_json` holds line numbers only: the hunks it cites (lines in the
-- before and after text) and where its quotes were found.
CREATE TABLE memory_changes (
    change_id           TEXT NOT NULL PRIMARY KEY,
    analysis_id         TEXT NOT NULL REFERENCES memory_change_analyses (analysis_id),
    position            INTEGER NOT NULL CHECK (position >= 1),
    action              TEXT NOT NULL CHECK (action IN ('added', 'removed', 'modified')),
    topic               TEXT NOT NULL,
    description         TEXT NOT NULL,
    evidence_json       TEXT NOT NULL CHECK (json_valid(evidence_json)),
    UNIQUE (analysis_id, position)
) STRICT;

CREATE TRIGGER memory_changes_no_update BEFORE UPDATE ON memory_changes
BEGIN
    SELECT RAISE(ABORT, 'a change is never changed');
END;

CREATE TRIGGER memory_changes_no_delete BEFORE DELETE ON memory_changes
BEGIN
    SELECT RAISE(ABORT, 'changes are never deleted');
END;
