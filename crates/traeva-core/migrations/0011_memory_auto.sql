-- Schema version 11: "Keep versions of files Traeva organizes" (#201, the
-- student's decision of 2026-10-12). One switch: after Traeva moves a file
-- it organizes, Memory starts recording that file and Traeva gives the file
-- Memory's own agreement to send (memory_consents), so every existing check
-- on sending stays as it is. These tables say which agreements and records
-- came from the switch, so turning it off withdraws exactly those.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, STRICT tables. Metadata only: never file text.

-- The switch's agreement: for which provider, model and notice. Turning it
-- on again after a change is a new row. At most one row is open: the core
-- withdraws the open row and saves the new one in one transaction, under
-- the settings lock (as for organize_auto_consents in 0010).
CREATE TABLE memory_auto_agreements (
    agreement_id    TEXT NOT NULL PRIMARY KEY,
    provider        TEXT NOT NULL,
    model           TEXT NOT NULL,
    notice_version  INTEGER NOT NULL CHECK (notice_version >= 1),
    agreed_at_ms    INTEGER NOT NULL,
    withdrawn_at_ms INTEGER CHECK (withdrawn_at_ms IS NULL OR withdrawn_at_ms >= agreed_at_ms)
) STRICT;

CREATE TRIGGER memory_auto_agreements_fixed_columns
BEFORE UPDATE OF agreement_id, provider, model, notice_version, agreed_at_ms
ON memory_auto_agreements
BEGIN
    SELECT RAISE(ABORT, 'an agreement keeps what it agreed to');
END;

CREATE TRIGGER memory_auto_agreements_withdrawal_is_final
BEFORE UPDATE ON memory_auto_agreements
WHEN OLD.withdrawn_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a withdrawn agreement stays withdrawn');
END;

-- One row per organize action after which the switch started Memory: the
-- watch it started and the per-file agreement it gave (null when the file
-- already had one of its own, or the switch was turned off meanwhile).
-- An action is handled at most once.
CREATE TABLE memory_auto_starts (
    action_id     TEXT NOT NULL PRIMARY KEY REFERENCES actions (action_id),
    agreement_id  TEXT NOT NULL REFERENCES memory_auto_agreements (agreement_id),
    file_id       TEXT NOT NULL REFERENCES files (file_id),
    watch_id      TEXT NOT NULL REFERENCES memory_watches (watch_id),
    consent_id    TEXT REFERENCES memory_consents (consent_id),
    started_at_ms INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX memory_auto_starts_one_per_consent
    ON memory_auto_starts (consent_id) WHERE consent_id IS NOT NULL;

CREATE TRIGGER memory_auto_starts_are_fixed
BEFORE UPDATE ON memory_auto_starts
BEGIN
    SELECT RAISE(ABORT, 'an automatic start is a fixed record');
END;

CREATE TRIGGER memory_auto_starts_no_delete
BEFORE DELETE ON memory_auto_starts
BEGIN
    SELECT RAISE(ABORT, 'automatic starts are never deleted');
END;
