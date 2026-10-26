-- Schema version 10: organizing a file the user drops on Traeva's icon
-- (organize-spec.md §13, v0.4, #168). Two tables of their own; the seven
-- Organize tables of 0001 do not change. Plans and actions made in the
-- automatic mode are ordinary rows there; these tables only say why they
-- ran without a confirmation page.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, STRICT tables. Metadata only: never file text.

-- The user's agreement to the automatic mode: which notice they agreed to
-- and the target range, existing folders of one authorized folder (the
-- root itself is ""), as a JSON array of relative paths. A new range or a
-- new notice is a new row; the core withdraws the open row and saves the
-- new one in one transaction, so at most one row is open at a time.
CREATE TABLE organize_auto_consents (
    consent_id      TEXT PRIMARY KEY,
    notice_version  INTEGER NOT NULL CHECK (notice_version >= 1),
    target_root_id  TEXT NOT NULL REFERENCES authorized_roots(root_id),
    scopes_json     TEXT NOT NULL CHECK (json_valid(scopes_json) AND json_array_length(scopes_json) >= 1),
    agreed_at_ms    INTEGER NOT NULL,
    withdrawn_at_ms INTEGER CHECK (withdrawn_at_ms IS NULL OR withdrawn_at_ms >= agreed_at_ms)
) STRICT;

-- One row per dropped file: when it came, what became of it, and the
-- agreement it ran under (null when the automatic mode was off, or before
-- the file was handed to the normal flow). The plan and action carry the
-- actual source and target.
CREATE TABLE organize_drop_events (
    drop_id        TEXT PRIMARY KEY,
    batch_id       TEXT NOT NULL,
    position       INTEGER NOT NULL CHECK (position >= 0),
    file_name      TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    received_at_ms INTEGER NOT NULL,
    root_id        TEXT REFERENCES authorized_roots(root_id),
    file_id        TEXT REFERENCES files(file_id),
    consent_id     TEXT REFERENCES organize_auto_consents(consent_id),
    plan_id        TEXT REFERENCES plans(plan_id),
    action_id      TEXT REFERENCES actions(action_id),
    outcome        TEXT NOT NULL CHECK (outcome IN (
                       'received', 'moved', 'already_in_place', 'handed_over',
                       'stopped', 'not_processed', 'cancelled', 'interrupted')),
    reason         TEXT CHECK (reason IS NULL OR length(reason) BETWEEN 1 AND 500),
    finished_at_ms INTEGER,
    UNIQUE (batch_id, position),
    CHECK ((outcome = 'received') = (finished_at_ms IS NULL)),
    CHECK (outcome <> 'moved' OR (consent_id IS NOT NULL AND plan_id IS NOT NULL
                                   AND action_id IS NOT NULL))
) STRICT;
