-- Schema version 6: when Source collection was last turned on (#27, Codex-C).
-- Automatic collection only takes downloads that started after this time, so
-- downloads made while Source was off are never saved on their own after it
-- is turned on again (shared-contract.md §4.5). Numbered 0006 by Claude-A at
-- integration, after 0005_sources.sql.
--
-- A database upgraded from version 5 keeps this unknown (NULL), even when
-- Source was on: an unknown time never lets automatic collection through.
-- Turning Source on sets it.
--
-- The rules of 0001_initial.sql apply: never edit this file once committed,
-- no BEGIN/COMMIT, UUID strings, Unix milliseconds, STRICT tables.

ALTER TABLE source_settings ADD COLUMN enabled_since_ms INTEGER
    CHECK (enabled_since_ms IS NULL OR enabled_since_ms >= 0);
