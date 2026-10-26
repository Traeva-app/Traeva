-- Schema version 8: the user's decisions about whole works (#31 M2b):
-- merging two works, revoking or confirming again one relation, moving
-- members to another work, associating a file with a work; the rows each
-- decision created or changed; and records held after a correction.
-- docs/development/shared-contract.md §4.3/§4.3a have the cross-lane rules;
-- works.rs, family_merge.rs and lineage.rs explain the code. Student rules:
-- #31 material §2, §5, §6, §7 and the 2026-10-10 decisions on #31.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables. No 0007
-- table changes shape; nothing is backfilled (no work decision existed
-- before version 8).
--
-- Words:
--   live work  a work that was not merged into another. Every work ID still
--              opens its work: an absorbed one through the merges below
--              (view memory_live_families). New rows only go under live IDs.
--   work decision  one confirmed page about works, never changed. Rows of
--              memory_members and memory_edges it creates have no
--              memory_decisions row (that table needs a record chain), so
--              memory_work_changes lists them (from_status NULL), and every
--              status it changes (from -> to).
--
-- 0007 values this version leaves unused: basis 'merge' and the reasons
-- 'merged'. Merging writes no membership and stops nothing: the files of
-- the absorbed work stay members of it, and it now means the live work.

CREATE TABLE memory_work_decisions (
    work_decision_id    TEXT NOT NULL PRIMARY KEY,
    kind                TEXT NOT NULL
        CHECK (kind IN ('merge', 'revoke', 'confirm_again', 'move', 'associate')),
    -- The page's work: the one that stays (merge), the relation's work
    -- (revoke, confirm_again), the source (move), the target (associate).
    family_id           TEXT NOT NULL REFERENCES memory_families (family_id),
    -- merge: the absorbed work; move: the target; associate: the file's own
    -- work when the association merges it in.
    other_family_id     TEXT REFERENCES memory_families (family_id),
    edge_id             TEXT REFERENCES memory_edges (edge_id),
    file_id             TEXT REFERENCES files (file_id),
    choice              TEXT CHECK (choice IN ('continue', 'branch', 'unknown', 'independent')),
    parent_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    -- What the page showed as evidence; never a status (material §6).
    evidence            TEXT CHECK (evidence IN ('same_bytes', 'same_object', 'none')),
    page_token          TEXT NOT NULL UNIQUE
        CHECK (length(page_token) = 64 AND page_token NOT GLOB '*[^0-9a-f]*'),
    decided_at_ms       INTEGER NOT NULL,
    CHECK ((kind IN ('revoke', 'confirm_again')) = (edge_id IS NOT NULL)),
    CHECK ((kind = 'associate') = (file_id IS NOT NULL)),
    CHECK ((kind = 'associate') = (choice IS NOT NULL)),
    CHECK ((kind = 'associate') = (evidence IS NOT NULL)),
    CHECK (kind NOT IN ('merge', 'move') OR other_family_id IS NOT NULL),
    CHECK (kind NOT IN ('revoke', 'confirm_again') OR other_family_id IS NULL),
    CHECK (other_family_id IS NULL OR other_family_id <> family_id),
    CHECK ((coalesce(choice, '') IN ('continue', 'branch')) = (parent_snapshot_id IS NOT NULL))
) STRICT;

CREATE INDEX memory_work_decisions_by_family ON memory_work_decisions (family_id);
CREATE INDEX memory_work_decisions_by_other ON memory_work_decisions (other_family_id)
    WHERE other_family_id IS NOT NULL;

-- One merge: the absorbed work now means the live work it was merged into.
-- A work is absorbed at most once and only into a live work, so merges
-- never form a cycle. Old IDs and their rows stay as they were.
CREATE TABLE memory_family_merges (
    absorbed_family_id  TEXT NOT NULL PRIMARY KEY REFERENCES memory_families (family_id),
    into_family_id      TEXT NOT NULL REFERENCES memory_families (family_id),
    work_decision_id    TEXT NOT NULL UNIQUE
        REFERENCES memory_work_decisions (work_decision_id),
    merged_at_ms        INTEGER NOT NULL,
    CHECK (absorbed_family_id <> into_family_id)
) STRICT;

CREATE INDEX memory_family_merges_by_into ON memory_family_merges (into_family_id);

-- Every membership or relation row a work decision created (from_status
-- NULL) or whose status it changed.
CREATE TABLE memory_work_changes (
    change_id         TEXT NOT NULL PRIMARY KEY,
    work_decision_id  TEXT NOT NULL REFERENCES memory_work_decisions (work_decision_id),
    member_id         TEXT REFERENCES memory_members (member_id),
    edge_id           TEXT REFERENCES memory_edges (edge_id),
    from_status       TEXT CHECK (from_status IN ('confirmed_system', 'confirmed_user',
                          'candidate', 'unknown', 'rejected', 'disputed', 'superseded')),
    to_status         TEXT NOT NULL CHECK (to_status IN ('confirmed_system', 'confirmed_user',
                          'candidate', 'unknown', 'rejected', 'disputed', 'superseded')),
    changed_at_ms     INTEGER NOT NULL,
    CHECK ((member_id IS NULL) <> (edge_id IS NULL)),
    CHECK (from_status IS NULL OR from_status <> to_status)
) STRICT;

CREATE UNIQUE INDEX memory_work_changes_member_created ON memory_work_changes (member_id)
    WHERE from_status IS NULL AND member_id IS NOT NULL;
CREATE UNIQUE INDEX memory_work_changes_edge_created ON memory_work_changes (edge_id)
    WHERE from_status IS NULL AND edge_id IS NOT NULL;
CREATE INDEX memory_work_changes_by_member ON memory_work_changes (member_id)
    WHERE member_id IS NOT NULL;
CREATE INDEX memory_work_changes_by_edge ON memory_work_changes (edge_id)
    WHERE edge_id IS NOT NULL;
CREATE INDEX memory_work_changes_by_decision ON memory_work_changes (work_decision_id);

-- A record held after a correction took away what its recording rested on
-- (material §7 last bullet, and the 2026-10-10 decision for moves): its
-- watch was stopped by that decision; the user turns Memory on again (the
-- same record goes on) or stops it.
CREATE TABLE memory_record_holds (
    hold_id           TEXT NOT NULL PRIMARY KEY,
    chain_id          TEXT NOT NULL REFERENCES memory_chains (chain_id),
    watch_id          TEXT NOT NULL REFERENCES memory_watches (watch_id),
    -- The continuation the recording rested on; null when a move split a
    -- record at a relocation, which has no relation of its own to point at.
    edge_id           TEXT REFERENCES memory_edges (edge_id),
    work_decision_id  TEXT NOT NULL REFERENCES memory_work_decisions (work_decision_id),
    held_at_ms        INTEGER NOT NULL,
    released_at_ms    INTEGER,
    release_reason    TEXT CHECK (release_reason IN ('resumed', 'stopped')),
    CHECK ((released_at_ms IS NULL) = (release_reason IS NULL)),
    CHECK (released_at_ms IS NULL OR released_at_ms >= held_at_ms)
) STRICT;

CREATE UNIQUE INDEX memory_record_holds_one_open ON memory_record_holds (chain_id)
    WHERE released_at_ms IS NULL;

-- Which live work each work ID means now.
CREATE VIEW memory_live_families (family_id, live_family_id) AS
WITH RECURSIVE up (family_id, at) AS (
    SELECT family_id, family_id FROM memory_families
    UNION
    SELECT up.family_id, m.into_family_id
    FROM up JOIN memory_family_merges AS m ON m.absorbed_family_id = up.at)
SELECT family_id, at FROM up
WHERE at NOT IN (SELECT absorbed_family_id FROM memory_family_merges);

-- Same content (the start hint, association evidence), relations found by
-- their child version, and the works of a file.
CREATE INDEX snapshots_by_sha256 ON snapshots (sha256);
CREATE INDEX memory_edges_by_child ON memory_edges (child_snapshot_id);
CREATE INDEX memory_members_by_file ON memory_members (file_id);

-- ---------------------------------------------------------------------------
-- Rows that never change and are never deleted
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_work_decisions_no_update BEFORE UPDATE ON memory_work_decisions
BEGIN SELECT RAISE(ABORT, 'a work decision is never changed'); END;
CREATE TRIGGER memory_work_decisions_no_delete BEFORE DELETE ON memory_work_decisions
BEGIN SELECT RAISE(ABORT, 'work decisions are never deleted'); END;

CREATE TRIGGER memory_family_merges_no_update BEFORE UPDATE ON memory_family_merges
BEGIN SELECT RAISE(ABORT, 'a merge is never changed'); END;
CREATE TRIGGER memory_family_merges_no_delete BEFORE DELETE ON memory_family_merges
BEGIN SELECT RAISE(ABORT, 'merges are never deleted'); END;

CREATE TRIGGER memory_work_changes_no_update BEFORE UPDATE ON memory_work_changes
BEGIN SELECT RAISE(ABORT, 'a change row is never changed'); END;
CREATE TRIGGER memory_work_changes_no_delete BEFORE DELETE ON memory_work_changes
BEGIN SELECT RAISE(ABORT, 'change rows are never deleted'); END;

CREATE TRIGGER memory_record_holds_fixed_columns
BEFORE UPDATE OF hold_id, chain_id, watch_id, edge_id, work_decision_id, held_at_ms
ON memory_record_holds
BEGIN SELECT RAISE(ABORT, 'a hold keeps what it held'); END;
CREATE TRIGGER memory_record_holds_release_once
BEFORE UPDATE ON memory_record_holds WHEN OLD.released_at_ms IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'a released hold stays released'); END;
CREATE TRIGGER memory_record_holds_no_delete BEFORE DELETE ON memory_record_holds
BEGIN SELECT RAISE(ABORT, 'holds are never deleted'); END;

-- ---------------------------------------------------------------------------
-- Live works
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_work_decisions_about_live_works
BEFORE INSERT ON memory_work_decisions
WHEN EXISTS (SELECT 1 FROM memory_family_merges
             WHERE absorbed_family_id IN (NEW.family_id, NEW.other_family_id))
BEGIN SELECT RAISE(ABORT, 'a work decision is about live works'); END;

CREATE TRIGGER memory_family_merges_into_live
BEFORE INSERT ON memory_family_merges
WHEN EXISTS (SELECT 1 FROM memory_family_merges WHERE absorbed_family_id = NEW.into_family_id)
BEGIN SELECT RAISE(ABORT, 'a work is merged only into a live work'); END;

CREATE TRIGGER memory_family_merges_match_decision
BEFORE INSERT ON memory_family_merges
WHEN NOT EXISTS (SELECT 1 FROM memory_work_decisions AS d
                 WHERE d.work_decision_id = NEW.work_decision_id
                   AND d.kind IN ('merge', 'associate')
                   AND d.family_id = NEW.into_family_id
                   AND d.other_family_id = NEW.absorbed_family_id)
BEGIN SELECT RAISE(ABORT, 'a merge is the one its decision names'); END;

CREATE TRIGGER memory_members_into_live_work
BEFORE INSERT ON memory_members
WHEN EXISTS (SELECT 1 FROM memory_family_merges WHERE absorbed_family_id = NEW.family_id)
BEGIN SELECT RAISE(ABORT, 'new memberships go into the live work'); END;

CREATE TRIGGER memory_edges_into_live_work
BEFORE INSERT ON memory_edges
WHEN EXISTS (SELECT 1 FROM memory_family_merges WHERE absorbed_family_id = NEW.family_id)
BEGIN SELECT RAISE(ABORT, 'new relations go into the live work'); END;

CREATE TRIGGER memory_chains_into_live_work
BEFORE INSERT ON memory_chains
WHEN EXISTS (SELECT 1 FROM memory_family_merges WHERE absorbed_family_id = NEW.family_id)
BEGIN SELECT RAISE(ABORT, 'new records go into the live work'); END;

CREATE TRIGGER memory_members_no_merge_basis
BEFORE INSERT ON memory_members WHEN NEW.basis = 'merge'
BEGIN SELECT RAISE(ABORT, 'merging writes no membership'); END;

-- ---------------------------------------------------------------------------
-- Every retirement has its reason: the change row is written first
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_edges_retired_with_reason
BEFORE UPDATE OF status ON memory_edges
WHEN NEW.status <> OLD.status
 AND (NEW.status IN ('disputed', 'superseded') OR OLD.status = 'disputed')
 AND coalesce((SELECT from_status || '>' || to_status FROM memory_work_changes
               WHERE edge_id = NEW.edge_id ORDER BY rowid DESC LIMIT 1), '')
     <> OLD.status || '>' || NEW.status
BEGIN SELECT RAISE(ABORT, 'a relation is retired only by a recorded decision'); END;

-- Members only for superseded: a membership marked disputed directly stays
-- possible (#33/#34 tests do it), and M2b never disputes memberships.
CREATE TRIGGER memory_members_superseded_with_reason
BEFORE UPDATE OF status ON memory_members
WHEN NEW.status <> OLD.status AND NEW.status = 'superseded'
 AND coalesce((SELECT from_status || '>' || to_status FROM memory_work_changes
               WHERE member_id = NEW.member_id ORDER BY rowid DESC LIMIT 1), '')
     <> OLD.status || '>' || NEW.status
BEGIN SELECT RAISE(ABORT, 'a membership is retired only by a recorded decision'); END;

-- ---------------------------------------------------------------------------
-- Holds
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_record_holds_after_stop
BEFORE INSERT ON memory_record_holds
WHEN NOT EXISTS (SELECT 1 FROM memory_chain_watches AS link
                 JOIN memory_watches AS w ON w.watch_id = link.watch_id
                 WHERE link.watch_id = NEW.watch_id AND link.chain_id = NEW.chain_id
                   AND w.status = 'stopped')
BEGIN SELECT RAISE(ABORT, 'a hold is for a stopped watch of its record'); END;

-- Turning Memory on again releases the hold first, in the same transaction:
-- no code path revives a held recording silently.
CREATE TRIGGER memory_chain_watches_not_while_held
BEFORE INSERT ON memory_chain_watches
WHEN EXISTS (SELECT 1 FROM memory_record_holds
             WHERE chain_id = NEW.chain_id AND released_at_ms IS NULL)
BEGIN SELECT RAISE(ABORT, 'a held record goes on only after its hold is released'); END;
