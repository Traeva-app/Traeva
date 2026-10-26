-- Schema version 7: Memory's works (families), record chains, candidates
-- saved after a replacement, the user's decisions about them, version
-- relations between real snapshots, recorded coverage gaps, and the periods
-- in which Memory's watcher ran (#31 M1). docs/development/shared-contract.md
-- §4.2b and §4.3 have the cross-lane rules; family.rs and lineage.rs explain
-- the code. Nothing here uses a table of migrations 0004 to 0006, and the
-- Organize tables, `snapshots` and `memory_watches` are not changed.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.
--
-- Words:
--   family     one work: the file records that belong together. No file
--              record is ever merged into another.
--   chain      one record task: the watches of memory_watches that go on
--              from one another (Memory turned on, turned on again, or moved
--              to the object that replaced the file). Its candidate switch,
--              quota and gaps belong to it.
--   candidate  an object Memory saved at a chain's place after the file there
--              was replaced. Saving it confirms nothing.
--
-- Relation statuses (#31 material §6). M1 writes the first five; disputed
-- and superseded are listed now because a CHECK list cannot be widened
-- without rebuilding the table:
--   confirmed_system  confirmed by a checked system rule
--   confirmed_user    confirmed by the user, as it was shown to them
--   candidate         seen, not decided
--   unknown           not enough to decide (a relation with no parent)
--   rejected          the user said it is unrelated; never proposed again
--   disputed          evidence conflicts (M2)
--   superseded        replaced by a later decision (M2)
--
-- Merging works and correcting decisions (M2) will add their own tables in a
-- later migration; the values they need in the CHECK lists below are here.
--
-- Rows made below for watches from before this version (#32) reuse the UUID
-- of a watch as their ID. IDs are unique per table: never infer anything
-- from two tables holding the same ID.

-- One work. Never changes.
CREATE TABLE memory_families (
    family_id       TEXT NOT NULL PRIMARY KEY,
    created_at_ms   INTEGER NOT NULL
) STRICT;

-- One record task.
CREATE TABLE memory_chains (
    chain_id          TEXT NOT NULL PRIMARY KEY,
    family_id         TEXT NOT NULL REFERENCES memory_families (family_id),
    -- The file Memory was turned on for. Its Memory send permission (#33)
    -- covers the versions on this chain's line.
    origin_file_id    TEXT NOT NULL REFERENCES files (file_id),
    -- The newest snapshot this chain saved (a version or a candidate); null
    -- before the first. New gaps and candidates are placed after it. It only
    -- moves forward.
    tail_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    created_at_ms     INTEGER NOT NULL
) STRICT;

CREATE INDEX memory_chains_by_family ON memory_chains (family_id);

-- An object saved at a chain's place after a replacement, in the order seen.
--   reason   replaced              found at the place by candidate capture
--            present_when_enabled  at the place when the user turned candidate
--                                  capture on, and saved at once (D2)
--            read_on_decision      candidate capture was off; read when the user
--                                  decided about the object at the place
--   after_snapshot_id   the chain's newest snapshot when it was found: where
--                       "continue" starts by default (material §6)
--   first_snapshot_id   its first snapshot, saved by that same capture
--   state    pending until the user decides; a decision is final
CREATE TABLE memory_candidates (
    candidate_id       TEXT NOT NULL PRIMARY KEY,
    chain_id           TEXT NOT NULL REFERENCES memory_chains (chain_id),
    file_id            TEXT NOT NULL REFERENCES files (file_id),
    seq                INTEGER NOT NULL CHECK (seq >= 1),
    reason             TEXT NOT NULL
        CHECK (reason IN ('replaced', 'present_when_enabled', 'read_on_decision')),
    after_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    first_snapshot_id  TEXT NOT NULL REFERENCES snapshots (snapshot_id),
    state              TEXT NOT NULL
        CHECK (state IN ('pending', 'continued', 'branch', 'unknown', 'independent')),
    found_at_ms        INTEGER NOT NULL,
    decided_at_ms      INTEGER,
    UNIQUE (chain_id, seq),
    UNIQUE (chain_id, file_id),
    CHECK ((state = 'pending') = (decided_at_ms IS NULL)),
    CHECK (reason <> 'read_on_decision' OR state IN ('continued', 'branch', 'unknown')),
    CHECK (after_snapshot_id IS NULL OR after_snapshot_id <> first_snapshot_id)
) STRICT;

CREATE INDEX memory_candidates_pending ON memory_candidates (chain_id) WHERE state = 'pending';
CREATE INDEX memory_candidates_by_file ON memory_candidates (file_id);

-- One choice the user made on one confirmation page. Choices submitted
-- together share batch_id; page_token is the version of the page they saw.
-- candidate_id is null for a choice about the object at the place that was
-- never read (it is then on the row of the candidate it became, if read).
-- relocate: M2.
CREATE TABLE memory_decisions (
    decision_id         TEXT NOT NULL PRIMARY KEY,
    chain_id            TEXT NOT NULL REFERENCES memory_chains (chain_id),
    batch_id            TEXT NOT NULL,
    choice              TEXT NOT NULL CHECK (choice IN ('continue', 'branch', 'unknown',
                            'independent', 'defer', 'relocate')),
    candidate_id        TEXT REFERENCES memory_candidates (candidate_id),
    parent_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    page_token          TEXT NOT NULL
        CHECK (length(page_token) = 64 AND page_token NOT GLOB '*[^0-9a-f]*'),
    decided_at_ms       INTEGER NOT NULL,
    CHECK (choice <> 'continue' OR parent_snapshot_id IS NOT NULL),
    CHECK (choice IN ('continue', 'branch') OR parent_snapshot_id IS NULL)
) STRICT;

CREATE UNIQUE INDEX memory_decisions_once_per_batch
    ON memory_decisions (batch_id, candidate_id) WHERE candidate_id IS NOT NULL;
CREATE INDEX memory_decisions_by_candidate ON memory_decisions (candidate_id);

-- The chain of each watch and how the watch joined it.
--   started      Memory was turned on for a file no chain records
--   from_before  a watch from before this version (#32)
--   restarted    Memory was turned on again for the chain's file after it was off
--   continued    the user confirmed that the object at the place continues the
--                work (decision_id says which choice)
--   relocated    (M2) the user chose the file's new place
-- A watch is followed by at most one later watch. A stopped watch is never
-- reopened (0003): going on always means a new watch row.
CREATE TABLE memory_chain_watches (
    watch_id           TEXT NOT NULL PRIMARY KEY REFERENCES memory_watches (watch_id),
    chain_id           TEXT NOT NULL REFERENCES memory_chains (chain_id),
    reason             TEXT NOT NULL
        CHECK (reason IN ('started', 'from_before', 'restarted', 'continued', 'relocated')),
    previous_watch_id  TEXT REFERENCES memory_watches (watch_id),
    decision_id        TEXT REFERENCES memory_decisions (decision_id),
    linked_at_ms       INTEGER NOT NULL,
    CHECK ((reason IN ('started', 'from_before')) = (previous_watch_id IS NULL)),
    CHECK ((reason IN ('continued', 'relocated')) = (decision_id IS NOT NULL)),
    CHECK (previous_watch_id IS NULL OR previous_watch_id <> watch_id)
) STRICT;

CREATE UNIQUE INDEX memory_chain_watches_one_successor
    ON memory_chain_watches (previous_watch_id) WHERE previous_watch_id IS NOT NULL;
CREATE INDEX memory_chain_watches_by_chain ON memory_chain_watches (chain_id);

-- The user's permission to save candidates when the chain's place is
-- replaced (material §3). Off unless turned on; chains never get it by
-- themselves. One row per period: off is final, on again is a new row.
-- included_file_id: the already-replaced object saved at once when the
-- switch was turned on (D2).
CREATE TABLE memory_candidate_switches (
    switch_id         TEXT NOT NULL PRIMARY KEY,
    chain_id          TEXT NOT NULL REFERENCES memory_chains (chain_id),
    notice_version    INTEGER NOT NULL CHECK (notice_version >= 1),
    enabled_at_ms     INTEGER NOT NULL,
    included_file_id  TEXT REFERENCES files (file_id),
    disabled_at_ms    INTEGER,
    disabled_reason   TEXT CHECK (disabled_reason IN ('user', 'stopped', 'occupant_independent',
                          'relocated', 'corrected', 'merged')),
    CHECK ((disabled_at_ms IS NULL) = (disabled_reason IS NULL))
) STRICT;

CREATE UNIQUE INDEX memory_candidate_switches_one_on
    ON memory_candidate_switches (chain_id) WHERE disabled_at_ms IS NULL;

-- A file record's relation to a work. basis says how the row came to be and
-- is kept apart from the status, so a user's confirmation never reads as
-- system evidence:
--   memory_on          the user turned Memory on for a file with no work
--   from_before        Memory was on for it before this version (#32)
--   candidate_capture  saved at a chain's place after a replacement
--   user_decision      read when the user decided about the object at the
--                      place with candidate capture off
--   relocation, merge, correction (M2)
CREATE TABLE memory_members (
    member_id      TEXT NOT NULL PRIMARY KEY,
    family_id      TEXT NOT NULL REFERENCES memory_families (family_id),
    file_id        TEXT NOT NULL REFERENCES files (file_id),
    status         TEXT NOT NULL CHECK (status IN ('confirmed_system', 'confirmed_user',
                       'candidate', 'unknown', 'rejected', 'disputed', 'superseded')),
    basis          TEXT NOT NULL CHECK (basis IN ('memory_on', 'from_before',
                       'candidate_capture', 'user_decision', 'relocation', 'merge', 'correction')),
    decision_id    TEXT REFERENCES memory_decisions (decision_id),
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL
) STRICT;

-- One live row per (work, file); a file is a formal member of one work at a
-- time, while a candidate may point at several.
CREATE UNIQUE INDEX memory_members_one_live_row
    ON memory_members (family_id, file_id) WHERE status <> 'superseded';
CREATE UNIQUE INDEX memory_members_one_family_per_file
    ON memory_members (file_id)
    WHERE status IN ('confirmed_system', 'confirmed_user', 'disputed');

-- A version relation between real snapshots, never between names.
--   continuation  the child version goes on from the parent version
--   branch        the child is the first version of a branch from the parent
--                 (the fork point); a null parent means the fork is unknown
--   unplaced      same work, order unknown: no parent
-- A null parent is never used to compare versions. skipped_items counts the
-- objects seen between the two ends that are not on this line (judged
-- unrelated, unknown or a branch): such a pair is "a difference", never one
-- save.
CREATE TABLE memory_edges (
    edge_id             TEXT NOT NULL PRIMARY KEY,
    family_id           TEXT NOT NULL REFERENCES memory_families (family_id),
    kind                TEXT NOT NULL CHECK (kind IN ('continuation', 'branch', 'unplaced')),
    parent_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    child_snapshot_id   TEXT NOT NULL REFERENCES snapshots (snapshot_id),
    status              TEXT NOT NULL CHECK (status IN ('confirmed_system', 'confirmed_user',
                            'candidate', 'unknown', 'rejected', 'disputed', 'superseded')),
    skipped_items       INTEGER NOT NULL DEFAULT 0 CHECK (skipped_items >= 0),
    decision_id         TEXT REFERENCES memory_decisions (decision_id),
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL,
    CHECK (parent_snapshot_id IS NULL OR parent_snapshot_id <> child_snapshot_id),
    CHECK (kind <> 'continuation' OR parent_snapshot_id IS NOT NULL),
    CHECK (kind <> 'unplaced' OR parent_snapshot_id IS NULL),
    CHECK (parent_snapshot_id IS NOT NULL OR status IN ('unknown', 'disputed', 'superseded'))
) STRICT;

-- A version has at most one live placement (so at most one confirmed parent);
-- a parent may have many branches.
CREATE UNIQUE INDEX memory_edges_one_placement
    ON memory_edges (child_snapshot_id)
    WHERE status IN ('confirmed_system', 'confirmed_user', 'disputed', 'unknown');
CREATE INDEX memory_edges_by_family ON memory_edges (family_id);
CREATE INDEX memory_edges_by_parent ON memory_edges (parent_snapshot_id);

-- A recorded coverage gap (D3): a stretch in which Memory knows it may not
-- have seen every save. It lies after after_snapshot_id, the chain's newest
-- snapshot when it began (null: before the first); every version pair that
-- starts at that snapshot is marked `gap`, however the gap ended.
--   memory_off           Memory was off between two watches of the chain
--   paused               candidate capture was paused at its 20/20 quota
--   capture_failed       the place could not be captured; problem = ErrorCode
--   replaced_uncaptured  the place held an object Memory did not read
--   lost                 the object at the place was judged unrelated: the work
--                        is lost, waiting for its new place (open = lost now)
-- Time when the watcher itself was not running is not a row here: it is
-- worked out from memory_runs. A start and an end are each written once,
-- never at every pass: `changed` ends one when the place's situation turns
-- into another (a failure with another problem, or the place now holding an
-- unread object instead of nothing). When storage itself is broken a gap may
-- not be saved at all (contract §5).
CREATE TABLE memory_gaps (
    gap_id             TEXT NOT NULL PRIMARY KEY,
    chain_id           TEXT NOT NULL REFERENCES memory_chains (chain_id),
    kind               TEXT NOT NULL CHECK (kind IN ('memory_off', 'paused', 'capture_failed',
                           'replaced_uncaptured', 'lost')),
    after_snapshot_id  TEXT REFERENCES snapshots (snapshot_id),
    problem            TEXT,
    started_at_ms      INTEGER NOT NULL,
    ended_at_ms        INTEGER,
    end_reason         TEXT CHECK (end_reason IN ('captured', 'recovered', 'changed',
                           'resumed', 'released', 'decided', 'continued', 'stopped',
                           'relocated', 'corrected', 'merged')),
    CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL)),
    CHECK (ended_at_ms IS NULL OR ended_at_ms >= started_at_ms),
    CHECK ((kind = 'capture_failed') = (problem IS NOT NULL)),
    CHECK (kind <> 'memory_off' OR ended_at_ms IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX memory_gaps_one_open_per_kind
    ON memory_gaps (chain_id, kind) WHERE ended_at_ms IS NULL;
CREATE INDEX memory_gaps_by_chain ON memory_gaps (chain_id);
CREATE INDEX memory_gaps_by_anchor ON memory_gaps (after_snapshot_id);

-- One run of Memory's watcher: from the first look (or capture) of a start
-- of Traeva to the last time it was seen running. The time from one run's
-- last_seen_at_ms to the next run's started_at_ms is a recorded gap: Traeva
-- was quit or crashed (#34 student decision, 2026-10-09). last_seen_at_ms is
-- written at least every 30 seconds while the watcher runs, so a crash makes
-- the gap look up to that much longer, never shorter. Time before the first
-- run (databases from before this version) is "not recorded".
CREATE TABLE memory_runs (
    run_id            TEXT NOT NULL PRIMARY KEY,
    started_at_ms     INTEGER NOT NULL,
    last_seen_at_ms   INTEGER NOT NULL,
    CHECK (last_seen_at_ms >= started_at_ms)
) STRICT;

-- ---------------------------------------------------------------------------
-- Watches from before this version (#32). Each file Memory was ever on for
-- becomes its own work with one chain. Its watches join that chain in the
-- order they were made; the later ones as "restarted", each with the
-- memory_off gap before it (it never ends before it starts, even when the
-- clock was set back between the two watches). No chain gets candidate
-- capture (material §3: existing watches do not get it). Runs before the
-- triggers below exist.
-- ---------------------------------------------------------------------------

INSERT INTO memory_families (family_id, created_at_ms)
SELECT w.watch_id, w.started_at_ms
FROM memory_watches AS w
WHERE w.rowid = (SELECT min(f.rowid) FROM memory_watches AS f WHERE f.file_id = w.file_id);

INSERT INTO memory_chains (chain_id, family_id, origin_file_id, tail_snapshot_id, created_at_ms)
SELECT w.watch_id, w.watch_id, w.file_id,
       (SELECT s.snapshot_id FROM snapshots AS s
        WHERE s.file_id = w.file_id ORDER BY s.rowid DESC LIMIT 1),
       w.started_at_ms
FROM memory_watches AS w
WHERE w.rowid = (SELECT min(f.rowid) FROM memory_watches AS f WHERE f.file_id = w.file_id);

INSERT INTO memory_members (member_id, family_id, file_id, status, basis, decision_id,
                            created_at_ms, updated_at_ms)
SELECT chain_id, family_id, origin_file_id, 'confirmed_system', 'from_before', NULL,
       created_at_ms, created_at_ms
FROM memory_chains;

INSERT INTO memory_chain_watches (watch_id, chain_id, reason, previous_watch_id, decision_id,
                                  linked_at_ms)
SELECT w.watch_id, c.chain_id,
       CASE WHEN w.watch_id = c.chain_id THEN 'from_before' ELSE 'restarted' END,
       (SELECT p.watch_id FROM memory_watches AS p
        WHERE p.file_id = w.file_id AND p.rowid < w.rowid ORDER BY p.rowid DESC LIMIT 1),
       NULL, w.started_at_ms
FROM memory_watches AS w
JOIN memory_chains AS c ON c.origin_file_id = w.file_id;

INSERT INTO memory_gaps (gap_id, chain_id, kind, after_snapshot_id, problem,
                         started_at_ms, ended_at_ms, end_reason)
SELECT w.watch_id, link.chain_id, 'memory_off',
       (SELECT s.snapshot_id FROM snapshots AS s
        WHERE s.file_id = w.file_id AND s.captured_at_ms < w.started_at_ms
        ORDER BY s.rowid DESC LIMIT 1),
       NULL, min(previous.stopped_at_ms, w.started_at_ms), w.started_at_ms, 'resumed'
FROM memory_chain_watches AS link
JOIN memory_watches AS w ON w.watch_id = link.watch_id
JOIN memory_watches AS previous ON previous.watch_id = link.previous_watch_id
WHERE link.reason = 'restarted';

-- ---------------------------------------------------------------------------
-- Rows that never change and are never deleted
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_families_no_update BEFORE UPDATE ON memory_families
BEGIN SELECT RAISE(ABORT, 'a family never changes'); END;
CREATE TRIGGER memory_families_no_delete BEFORE DELETE ON memory_families
BEGIN SELECT RAISE(ABORT, 'families are never deleted'); END;

CREATE TRIGGER memory_decisions_no_update BEFORE UPDATE ON memory_decisions
BEGIN SELECT RAISE(ABORT, 'a decision is never changed'); END;
CREATE TRIGGER memory_decisions_no_delete BEFORE DELETE ON memory_decisions
BEGIN SELECT RAISE(ABORT, 'decisions are never deleted'); END;

CREATE TRIGGER memory_chain_watches_no_update BEFORE UPDATE ON memory_chain_watches
BEGIN SELECT RAISE(ABORT, 'a watch keeps its chain'); END;
CREATE TRIGGER memory_chain_watches_no_delete BEFORE DELETE ON memory_chain_watches
BEGIN SELECT RAISE(ABORT, 'chain links are never deleted'); END;

-- ---------------------------------------------------------------------------
-- Rows with one allowed change
-- ---------------------------------------------------------------------------

CREATE TRIGGER memory_chains_fixed_columns
BEFORE UPDATE OF chain_id, family_id, origin_file_id, created_at_ms ON memory_chains
BEGIN SELECT RAISE(ABORT, 'a chain keeps its work and first file'); END;
CREATE TRIGGER memory_chains_tail_moves_forward
BEFORE UPDATE OF tail_snapshot_id ON memory_chains
WHEN NEW.tail_snapshot_id IS NULL
  OR (OLD.tail_snapshot_id IS NOT NULL
      AND (SELECT rowid FROM snapshots WHERE snapshot_id = NEW.tail_snapshot_id)
          <= (SELECT rowid FROM snapshots WHERE snapshot_id = OLD.tail_snapshot_id))
BEGIN SELECT RAISE(ABORT, 'a chain''s newest version only moves forward'); END;
CREATE TRIGGER memory_chains_no_delete BEFORE DELETE ON memory_chains
BEGIN SELECT RAISE(ABORT, 'chains are never deleted'); END;

CREATE TRIGGER memory_candidate_switches_fixed_columns
BEFORE UPDATE OF switch_id, chain_id, notice_version, enabled_at_ms, included_file_id
ON memory_candidate_switches
BEGIN SELECT RAISE(ABORT, 'a switch period keeps what it allowed'); END;
CREATE TRIGGER memory_candidate_switches_off_is_final
BEFORE UPDATE ON memory_candidate_switches WHEN OLD.disabled_at_ms IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'a switch turned off stays off'); END;
CREATE TRIGGER memory_candidate_switches_no_delete BEFORE DELETE ON memory_candidate_switches
BEGIN SELECT RAISE(ABORT, 'switch periods are never deleted'); END;

CREATE TRIGGER memory_candidates_fixed_columns
BEFORE UPDATE OF candidate_id, chain_id, file_id, seq, reason, after_snapshot_id,
                 first_snapshot_id, found_at_ms ON memory_candidates
BEGIN SELECT RAISE(ABORT, 'a candidate keeps what was seen'); END;
CREATE TRIGGER memory_candidates_decided_is_final
BEFORE UPDATE ON memory_candidates WHEN OLD.state <> 'pending'
BEGIN SELECT RAISE(ABORT, 'a decided candidate stays decided'); END;
CREATE TRIGGER memory_candidates_no_delete BEFORE DELETE ON memory_candidates
BEGIN SELECT RAISE(ABORT, 'candidates are never deleted'); END;

CREATE TRIGGER memory_members_fixed_columns
BEFORE UPDATE OF member_id, family_id, file_id, basis, created_at_ms ON memory_members
BEGIN SELECT RAISE(ABORT, 'a membership keeps its file, work and basis'); END;
CREATE TRIGGER memory_members_status_steps
BEFORE UPDATE OF status ON memory_members
WHEN NOT (
       OLD.status = NEW.status
    OR (OLD.status = 'candidate' AND NEW.status IN
          ('confirmed_system', 'confirmed_user', 'unknown', 'rejected', 'superseded'))
    OR (OLD.status = 'unknown' AND NEW.status IN
          ('confirmed_system', 'confirmed_user', 'rejected', 'superseded'))
    OR (OLD.status IN ('confirmed_system', 'confirmed_user')
          AND NEW.status IN ('disputed', 'superseded'))
    OR (OLD.status = 'disputed' AND NEW.status IN
          ('confirmed_system', 'confirmed_user', 'superseded'))
    OR (OLD.status = 'rejected' AND NEW.status = 'superseded'))
BEGIN SELECT RAISE(ABORT, 'this status change is not allowed'); END;
CREATE TRIGGER memory_members_no_delete BEFORE DELETE ON memory_members
BEGIN SELECT RAISE(ABORT, 'memberships are never deleted'); END;

CREATE TRIGGER memory_edges_fixed_columns
BEFORE UPDATE OF edge_id, family_id, kind, parent_snapshot_id, child_snapshot_id,
                 skipped_items, decision_id, created_at_ms ON memory_edges
BEGIN SELECT RAISE(ABORT, 'a relation keeps its two versions'); END;
CREATE TRIGGER memory_edges_status_steps
BEFORE UPDATE OF status ON memory_edges
WHEN NOT (
       OLD.status = NEW.status
    OR (OLD.status = 'candidate' AND NEW.status IN
          ('confirmed_system', 'confirmed_user', 'unknown', 'rejected', 'superseded'))
    OR (OLD.status = 'unknown' AND NEW.status IN ('rejected', 'superseded'))
    OR (OLD.status IN ('confirmed_system', 'confirmed_user')
          AND NEW.status IN ('disputed', 'superseded'))
    OR (OLD.status = 'disputed' AND NEW.status IN
          ('confirmed_system', 'confirmed_user', 'superseded'))
    OR (OLD.status = 'rejected' AND NEW.status = 'superseded'))
BEGIN SELECT RAISE(ABORT, 'this status change is not allowed'); END;
-- Every relation points from an older snapshot to a newer one, so relations
-- can never form a cycle.
CREATE TRIGGER memory_edges_parent_first
BEFORE INSERT ON memory_edges
WHEN NEW.parent_snapshot_id IS NOT NULL
 AND (SELECT rowid FROM snapshots WHERE snapshot_id = NEW.parent_snapshot_id)
     >= (SELECT rowid FROM snapshots WHERE snapshot_id = NEW.child_snapshot_id)
BEGIN SELECT RAISE(ABORT, 'a parent version is saved before its child'); END;
CREATE TRIGGER memory_edges_no_delete BEFORE DELETE ON memory_edges
BEGIN SELECT RAISE(ABORT, 'relations are never deleted'); END;

CREATE TRIGGER memory_gaps_fixed_columns
BEFORE UPDATE OF gap_id, chain_id, kind, after_snapshot_id, problem, started_at_ms
ON memory_gaps
BEGIN SELECT RAISE(ABORT, 'a gap keeps where and how it began'); END;
CREATE TRIGGER memory_gaps_end_once
BEFORE UPDATE ON memory_gaps WHEN OLD.ended_at_ms IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'an ended gap stays as it is'); END;
CREATE TRIGGER memory_gaps_no_delete BEFORE DELETE ON memory_gaps
BEGIN SELECT RAISE(ABORT, 'gaps are never deleted'); END;

CREATE TRIGGER memory_runs_fixed_columns
BEFORE UPDATE OF run_id, started_at_ms ON memory_runs
BEGIN SELECT RAISE(ABORT, 'a run keeps when it started'); END;
CREATE TRIGGER memory_runs_seen_moves_forward
BEFORE UPDATE OF last_seen_at_ms ON memory_runs
WHEN NEW.last_seen_at_ms < OLD.last_seen_at_ms
BEGIN SELECT RAISE(ABORT, 'a run''s last time seen only moves forward'); END;
CREATE TRIGGER memory_runs_no_delete BEFORE DELETE ON memory_runs
BEGIN SELECT RAISE(ABORT, 'runs are never deleted'); END;

-- ---------------------------------------------------------------------------
-- Chain rules the database enforces
-- ---------------------------------------------------------------------------

-- A file's watches all belong to one chain.
CREATE TRIGGER memory_chain_watches_one_chain_per_file
BEFORE INSERT ON memory_chain_watches
WHEN EXISTS (
    SELECT 1
    FROM memory_chain_watches AS link
    JOIN memory_watches AS earlier ON earlier.watch_id = link.watch_id
    WHERE earlier.file_id = (SELECT file_id FROM memory_watches WHERE watch_id = NEW.watch_id)
      AND link.chain_id <> NEW.chain_id)
BEGIN SELECT RAISE(ABORT, 'a file''s watches all belong to one chain'); END;

-- A chain has at most one watch on: it records one place at a time.
CREATE TRIGGER memory_chain_watches_one_on_per_chain
BEFORE INSERT ON memory_chain_watches
WHEN EXISTS (
    SELECT 1
    FROM memory_chain_watches AS link
    JOIN memory_watches AS w ON w.watch_id = link.watch_id
    WHERE link.chain_id = NEW.chain_id AND w.status = 'active' AND w.watch_id <> NEW.watch_id)
BEGIN SELECT RAISE(ABORT, 'a chain has at most one watch on'); END;
