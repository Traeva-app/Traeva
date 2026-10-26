-- Schema version 12: following a file an app saved by replacing it (#37,
-- the student's decision of 2026-10-14, which changes the #31 rule "never
-- link files by path or name" for this one case). Apps such as TextEdit and
-- Word save by putting a new file object at the same path. For a record
-- Memory is on for, a new object at the record's exact place (same folder,
-- same relative path) is now recorded by itself as the next version of the
-- same work: the rows are the ones a user's "continue" writes (a
-- memory_decisions row, a membership, a continuation, the watch moved to
-- the new file), with status confirmed_system. This table says which of
-- those links Traeva made by itself, so the window can say so and the user
-- can undo one (revoking the relation, contract §4.3a).
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables. No table
-- of an earlier migration changes; nothing is backfilled (no link was made
-- by itself before version 12).

CREATE TABLE memory_auto_links (
    edge_id       TEXT NOT NULL PRIMARY KEY REFERENCES memory_edges (edge_id),
    member_id     TEXT NOT NULL REFERENCES memory_members (member_id),
    chain_id      TEXT NOT NULL REFERENCES memory_chains (chain_id),
    decision_id   TEXT NOT NULL UNIQUE REFERENCES memory_decisions (decision_id),
    linked_at_ms  INTEGER NOT NULL
) STRICT;

CREATE TRIGGER memory_auto_links_are_fixed
BEFORE UPDATE ON memory_auto_links
BEGIN
    SELECT RAISE(ABORT, 'an automatic link is a fixed record');
END;

CREATE TRIGGER memory_auto_links_no_delete
BEFORE DELETE ON memory_auto_links
BEGIN
    SELECT RAISE(ABORT, 'automatic links are never deleted');
END;
