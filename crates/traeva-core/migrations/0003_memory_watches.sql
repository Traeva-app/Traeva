-- Schema version 3: which files Memory records (#32).
-- The user turns Memory on for one chosen file; no other file is ever
-- captured. docs/development/shared-contract.md §4.2 has the rules.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.

-- One period in which Memory was on for a file. Turning it off ends the
-- period for good; turning it on again starts a new row. Rows are never
-- deleted, and only `status` and `stopped_at_ms` ever change.
CREATE TABLE memory_watches (
    watch_id        TEXT NOT NULL PRIMARY KEY,
    file_id         TEXT NOT NULL REFERENCES files (file_id),
    status          TEXT NOT NULL CHECK (status IN ('active', 'stopped')),
    started_at_ms   INTEGER NOT NULL,
    stopped_at_ms   INTEGER,
    CHECK ((status = 'stopped') = (stopped_at_ms IS NOT NULL))
) STRICT;

-- A file has at most one period that is still on.
CREATE UNIQUE INDEX memory_watches_one_active_per_file
    ON memory_watches (file_id) WHERE status = 'active';

CREATE TRIGGER memory_watches_fixed_columns
BEFORE UPDATE OF watch_id, file_id, started_at_ms ON memory_watches
BEGIN
    SELECT RAISE(ABORT, 'a watch keeps its file and start time');
END;

-- Off is final: a stopped period never changes again.
CREATE TRIGGER memory_watches_stop_is_final
BEFORE UPDATE ON memory_watches
WHEN OLD.status = 'stopped'
BEGIN
    SELECT RAISE(ABORT, 'a stopped watch stays as it is');
END;

CREATE TRIGGER memory_watches_no_delete BEFORE DELETE ON memory_watches
BEGIN
    SELECT RAISE(ABORT, 'watches are never deleted');
END;
