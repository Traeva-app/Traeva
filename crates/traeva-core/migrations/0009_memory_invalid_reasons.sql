-- Schema version 9: why a model answer was refused (#33), so an
-- invalid_output can be traced: the rule the answer broke, as changes.rs
-- `check_output` words it. Only Traeva's own rule text (a change number, a
-- field name, a limit), never the model's answer or the user's text.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, STRICT tables. Nothing is backfilled: invalid_output rows
-- from before version 9 keep NULL, because their reason was not recorded,
-- and none is made up.
--
-- One direction only: a reason only on an invalid_output row, at most 300
-- characters. Not the other way round: SQLite checks a new column's CHECK
-- against the rows already there, and older invalid_output rows have none.
ALTER TABLE memory_change_analyses
    ADD COLUMN invalid_reason TEXT
        CHECK (invalid_reason IS NULL
               OR (status = 'invalid_output' AND length(invalid_reason) BETWEEN 1 AND 300));
