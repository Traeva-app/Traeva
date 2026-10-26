-- Schema version 2: Memory's saved versions (#32).
-- docs/development/shared-contract.md §4.2 says what the columns mean.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.

-- One saved version of a file: where its exact bytes came from. The bytes are
-- a file in the app-data folder, snapshots/<first two hex digits>/<sha256>,
-- written and checked before this row is inserted. Several rows may share one
-- stored copy when their bytes are the same. Rows never change and are never
-- deleted.
CREATE TABLE snapshots (
    snapshot_id         TEXT NOT NULL PRIMARY KEY,
    file_id             TEXT NOT NULL REFERENCES files (file_id),
    sequence            INTEGER NOT NULL CHECK (sequence >= 1), -- 1, 2, 3... per file_id, in capture order
    file_revision       INTEGER NOT NULL CHECK (file_revision >= 1),
    root_id             TEXT NOT NULL REFERENCES authorized_roots (root_id),
    relative_path       TEXT NOT NULL,
    -- Lowercase hex only: the stored copy's file name is made from it.
    sha256              TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    byte_length         INTEGER NOT NULL CHECK (byte_length >= 0),
    observation_json    TEXT NOT NULL CHECK (json_valid(observation_json)), -- physical evidence of the read
    captured_at_ms      INTEGER NOT NULL,
    UNIQUE (file_id, sequence)
) STRICT;

CREATE TRIGGER snapshots_no_update BEFORE UPDATE ON snapshots
BEGIN
    SELECT RAISE(ABORT, 'snapshots are immutable');
END;

CREATE TRIGGER snapshots_no_delete BEFORE DELETE ON snapshots
BEGIN
    SELECT RAISE(ABORT, 'snapshots are never deleted');
END;
