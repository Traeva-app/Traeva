-- Schema version 5: Source Memory (#27, Codex-C): Chrome download events,
-- candidates, User Confirmed relations and their append-only history.
-- docs/features/source-memory.md says what the tables mean; shared-contract.md
-- §4.5 has the cross-lane rules. Numbered 0005 by Claude-A at integration
-- (#33's 0004_memory_changes.sql came first; #65 agreement).
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.

CREATE TABLE source_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    pairing_token TEXT,
    receiver_port INTEGER CHECK (receiver_port BETWEEN 1 AND 65535)
) STRICT;
INSERT INTO source_settings (singleton) VALUES (1);

CREATE TABLE source_records (
    source_id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    download_id TEXT NOT NULL,
    root_id TEXT NOT NULL REFERENCES authorized_roots(root_id),
    event_json TEXT NOT NULL CHECK (json_valid(event_json)),
    imported_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    reason TEXT,
    UNIQUE (profile_id, download_id)
) STRICT;

CREATE TABLE source_candidates (
    source_id TEXT NOT NULL REFERENCES source_records(source_id),
    file_id TEXT NOT NULL REFERENCES files(file_id),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'rejected')),
    PRIMARY KEY (source_id, file_id)
) STRICT;

CREATE TABLE source_associations (
    source_id TEXT PRIMARY KEY REFERENCES source_records(source_id),
    file_id TEXT NOT NULL REFERENCES files(file_id),
    original_file_id TEXT NOT NULL REFERENCES files(file_id),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    relation TEXT NOT NULL CHECK (relation IN ('download', 'copy')),
    note TEXT NOT NULL,
    confirmed_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE source_history (
    history_id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES source_records(source_id),
    file_id TEXT REFERENCES files(file_id),
    kind TEXT NOT NULL CHECK (kind IN ('confirmed', 'rejected', 'unlinked', 'continued')),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    note TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX source_history_by_source ON source_history(source_id, recorded_at_ms);
CREATE TRIGGER source_history_no_update BEFORE UPDATE ON source_history
BEGIN SELECT RAISE(ABORT, 'source evidence is append-only'); END;
CREATE TRIGGER source_history_no_delete BEFORE DELETE ON source_history
BEGIN SELECT RAISE(ABORT, 'source evidence is append-only'); END;
CREATE TRIGGER source_import_time_fixed BEFORE UPDATE OF imported_at_ms ON source_records
BEGIN SELECT RAISE(ABORT, 'first successful import time is fixed'); END;
