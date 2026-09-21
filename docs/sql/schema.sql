-- The pManager ledger: one row per registered project.
--
-- This file is the schema. `src/database/schema.rs` embeds it with
-- include_str! and applies it on every database open, so there is no second
-- copy to keep in sync. `PRAGMA user_version` is set separately by that
-- module; it is 1 for the shape below.
--
-- Every statement here is replayed on every open, so every statement must be
-- idempotent: IF NOT EXISTS on creates, and no bare ALTER TABLE. A change
-- that cannot be expressed idempotently is a migration and belongs behind a
-- user_version check in schema.rs instead.

CREATE TABLE IF NOT EXISTS projects (
    -- `pman restore <name>` looks a project up by name, so it identifies
    -- exactly one row. Supplied by the user during `pman project add`.
    name           TEXT    NOT NULL UNIQUE,

    -- Compared byte for byte. Registration normalises the path first: on
    -- Windows `C:\dev\x` and `C:\Dev\x` are one directory, and neither
    -- UNIQUE nor COLLATE NOCASE would also reconcile `\` against `/` or a
    -- trailing separator.
    path           TEXT    NOT NULL UNIQUE,

    -- The strip profile's `language`, detected at registration. Deliberately
    -- unconstrained: profiles are user-supplied JSON under `strip/`, so a
    -- CHECK here would turn "drop in a new profile" into "migrate the
    -- database".
    kind           TEXT    NOT NULL,

    -- registered -> stripped -> archived. A closed set this crate owns,
    -- unlike `kind`, so it is checked.
    state          TEXT    NOT NULL CHECK (state IN ('registered', 'stripped', 'archived')),

    -- Whether the folder is a git repository, checked once at registration.
    -- Whether it is *dirty* is deliberately absent: `pman run` recomputes
    -- that through git2 on every pass, and a stored copy would be stale the
    -- moment it was written.
    has_git_repo   INTEGER NOT NULL CHECK (has_git_repo IN (0, 1)),

    -- Where the 7-zipped .pproject went. NULL until the project is archived.
    archive_path   TEXT,

    -- Drives the 60/120/160-day thresholds, and the reason they survive
    -- archiving: once the folder is packed away, its mtime is gone.
    --
    -- Written by `database::timestamp::to_value`, which delegates to
    -- rusqlite's chrono ToSql, so the text is `YYYY-MM-DD HH:MM:SS.SSS+00:00`
    -- -- SQLite's own date format. `julianday('now') - julianday(...)` reads
    -- it, and `row.get::<_, DateTime<Utc>>()` parses it back.
    last_change_at TEXT    NOT NULL,

    registered_at  TEXT    NOT NULL,

    -- An archived project is the only kind with a .pproject to point at.
    CHECK ((state =  'archived' AND archive_path IS NOT NULL)
        OR (state <> 'archived' AND archive_path IS NULL))
);
