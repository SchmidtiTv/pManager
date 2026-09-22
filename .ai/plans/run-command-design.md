# Design — `pman run`

Status: implemented (2026-09-22). Kept as the record of why the pass is
shaped this way; `.ai/context/codemap.md` describes what is now in the tree.

`pman run` is the pass that does the work the rest of the tool exists to support: it
looks at every registered project, decides whether it has gone cold enough to strip or
archive, and carries that out. `pman run --dry-run` reports the same decisions without
touching anything.

The thresholds themselves are product spec and live in `README.md` — 60 days to strip,
120 to archive, 160 when the working tree is dirty. This file covers how the command is
built.

## Shape of a pass

The pass splits into a read half and a write half, with a plain value in between:

```
stripping::load(cx.paths.strip_dir)  ──┐
project::select_all()  ────────────────┤
                                       ▼
   for each project:  facts::gather(project, &profiles)     reads disk + git, mutates nothing
                                       ▼
                      plan::decide(project, &facts, now)    pure: no I/O, no clock
                                       ▼
                      report::console_data(&planned) → build_output → stdout
                                       ▼
        if !dry_run:  execute::apply(&planned, db)          the only code that deletes
```

Two properties follow from this split, and both are the reason for it:

- **`--dry-run` is the absence of the last step.** The flag is read in
  `commands/run.rs` and never travels below it. Nothing that deletes a folder takes a
  mode parameter.
- **Every fact is measured on the untouched tree.** A strip that removes a tracked file
  makes the repository dirty; because dirtiness was already measured, that cannot move
  the project from the 120-day threshold to the 160-day one within the same pass. The
  same ordering keeps the idle clock honest: `last_change_at` is read before any
  mutation, so the tool's own writes never reset it.

`plan::decide` being pure is what makes the threshold table testable. The 60/120/160
rules are covered by unit tests on a function that takes a `now`, with no temporary
directory and no back-dated mtimes.

## Files

New:

| File | Purpose |
| --- | --- |
| `src/pass/mod.rs` | Orchestration: load profiles, read the ledger, gather → plan → render → execute |
| `src/pass/facts.rs` | Per-project disk and git reads: fresh `last_change_at`, `dirty`, matched profiles |
| `src/pass/plan.rs` | Pure `(project, facts, now) -> Action`; the only home of the day thresholds |
| `src/pass/execute.rs` | Performs one action and writes the ledger row |
| `src/pass/report.rs` | `&[Planned] -> ConsoleData` |
| `src/git.rs` | `is_dirty(root)` and `has_repo(root)` |
| `src/archive.rs` | `pack(root, dest)` and `verify(dest, expected)` over `sevenz-rust2` |
| `src/stripping/detection.rs` | Which loaded profiles match a project root |
| `src/stripping/removal.rs` | Deletes a profile's `strippable` entries from a root |

`pass` is a top-level module rather than a `commands/run/` directory because a directory
under `commands/` means a command group with subcommands; `run` is a leaf. The name
follows the vocabulary already used in `docs/sql/schema.sql`.

Changed:

- `src/commands/run.rs` — stays a thin leaf: parses `--dry-run`, calls `pass::execute_pass`.
- `src/activity.rs` — honours `.pmanagerignore`.
- `src/models/project.rs` — no UPDATE query exists yet; the pass needs one.
- `src/models/state.rs` — `Archived { archive_path: PathBuf }` instead of `String`.
- `src/formats/console_data.rs` — new `Cell::Bytes(u64)` variant.
- `src/commands/project/add.rs` — one line, uses `git::has_repo` instead of its own
  private copy, so `git2` has a single home.
- `src/lib.rs` — declares `archive`, `git`, `pass`.
- `tests/cli.rs` — the `SURFACE` row for `run` is deleted and replaced by behavioural
  tests.

## Gathering facts

```rust
pub struct Facts {
    pub last_change_at: DateTime<Utc>,
    pub dirty: bool,
    pub matched: Vec<StrippingProfile>,
}
```

- **`last_change_at`** comes from `activity::last_change`, extended to honour
  `.pmanagerignore`. The existing `.git` skip stays, and gitignore-syntax patterns from
  `<project>/.pmanagerignore` are layered on through the `ignore` crate's
  `GitignoreBuilder`, filtered into the same `walkdir` traversal. Nothing else is
  special-cased: if a package folder should stop counting as activity, it goes in that
  file, where it is visible.
- **`dirty`** comes from `git::is_dirty`, computed fresh on every pass. The schema
  comment on `has_git_repo` states why it is never stored: a stored copy is stale the
  moment it is written. A project with no repository is never dirty.
- **`matched`** comes from `stripping::detection`: a profile matches when any of its
  `detection` entries exists at the project root with the right type.

A project whose folder no longer exists yields no facts. It is reported as
`Nothing { Missing }`, pointing at `pman project cleanup`, which owns removing dead rows.

`stripping::load` already fails the whole pass when `profiles.json` is missing or any
profile is invalid, and that behaviour is kept. The alternative — carrying on with no
profiles — would silently archive projects unstripped and produce `.pproject` files many
times larger than they should be, with no signal that anything was wrong.

## The decision table

`plan.rs`, pure, with the thresholds as its only constants:

```rust
const STRIP_AFTER_DAYS: i64 = 60;
const ARCHIVE_AFTER_DAYS: i64 = 120;
const ARCHIVE_DIRTY_AFTER_DAYS: i64 = 160;

pub enum Action {
    Nothing { reason: Reason },
    Strip,
    Archive,
    StripAndArchive,
}

pub enum Reason { Active, NoProfile, AlreadyArchived, Missing }
```

With `idle = now - facts.last_change_at` and
`archive_at = if facts.dirty { ARCHIVE_DIRTY_AFTER_DAYS } else { ARCHIVE_AFTER_DAYS }`:

| Project state | Condition | Action |
| --- | --- | --- |
| `Archived` | — | `Nothing { AlreadyArchived }` |
| any | folder missing | `Nothing { Missing }` |
| any | `idle >= archive_at`, a profile matched, state is `Registered` | `StripAndArchive` |
| any | `idle >= archive_at`, otherwise | `Archive` |
| `Registered` | `idle >= STRIP_AFTER_DAYS`, a profile matched | `Strip` |
| `Registered` | `idle >= STRIP_AFTER_DAYS`, no profile matched | `Nothing { NoProfile }` |
| any | `idle < STRIP_AFTER_DAYS` | `Nothing { Active }` |

A project that has crossed both thresholds is stripped and archived in the same pass, in
that order, so package folders never enter the archive. Restoring them is a reinstall.

`dirty` can only be true for a git repository, so a project without one always uses the
120-day threshold — which is what the README intends, since the 160-day hold exists to
protect uncommitted work.

`Nothing` carries its reason so the report can distinguish a project still in use from
one idle for 90 days that no strip profile recognises. Without the reason those two look
identical, and only the second is worth acting on.

## Executing an action

### Strip

For each matched profile, each `strippable` entry is resolved against the project root
and removed — `remove_dir_all` for a folder, `remove_file` for a file. Entries are
validated by `stripping::validation` at load time to be relative and inside the root, so
the resolved path cannot escape.

An entry that is already absent is not an error; the profile lists what *may* be
stripped, and a Python project without a `.venv` is normal. Anything else — a permission
error, a locked folder — fails the project.

Size freed is measured by walking each entry before deleting it, so the report can show
what the pass actually reclaimed.

On success the ledger row moves to `stripped`.

### Archive

The destination is `<parent-of-project>/<name>.pproject`, using the project's ledger
name, so the archive sits where the folder used to be and stays next to its siblings.
Names are unique in the ledger, so two projects cannot collide. A name that is not
usable as a filename fails that project with an error naming it — the ledger name is
supplied interactively during `pman project add`, so this is a real input, not an
internal invariant.

Ordering, which is what makes a crash survivable:

1. Pack the folder to `<name>.pproject.part` with `sevenz_rust2::compress_to_path`.
2. Verify by opening the result with `ArchiveReader` and comparing its entry count
   against the number of files walked before packing. Decoding is not behind a cargo
   feature in `sevenz-rust2` — only the encoder is — so this works with the manifest's
   current `default-features = false` set, and a future `pman restore` needs no change.
3. Rename `.part` to `.pproject`.
4. Delete the source folder.
5. Update the ledger row to `archived` with its `archive_path`.

No step can lose data. A crash before step 3 leaves the folder intact and a `.part` file
the next pass overwrites. A crash between 3 and 4 leaves both a verified archive and the
folder, and the next pass simply repacks. The one narrow window is between 4 and 5 — a
single `UPDATE` wide — after which the folder is gone, a verified archive exists, and
the ledger still says `registered` or `stripped`. The next pass reports that project as
`Nothing { Missing }` rather than inventing a repair; the archive is on disk beside where
the folder was, and the state is visible rather than silently patched.

## Errors

One project failing must not stop the pass — `pman run` usually runs unattended from a
scheduler, and a folder locked by an editor would otherwise block every project after it.

- Each project is attempted independently. A failure leaves that project's ledger row
  untouched, so the next pass retries from a known state.
- The full `anyhow` cause chain is reported through `error::report` to stderr, naming the
  project and the operation.
- The project's row in the table shows the failure, so the report is complete on its own.
- The command exits non-zero when any project failed, with a final error naming every one
  of them. Nothing is swallowed and no failure is downgraded to a warning.

## Output

`report.rs` builds a `ConsoleData` and hands it to `build_output`, exactly as `list.rs`
does; the pass never formats its own rows. The same table is produced with and without
`--dry-run`, which is what makes the dry run trustworthy — it is the identical plan.

| Column | Cell |
| --- | --- |
| `NAME` | `Cell::Text` |
| `IDLE` | `Cell::Days` |
| `ACTION` | `Cell::Text` — `strip`, `archive`, `strip+archive`, `none` |
| `FREED` | `Cell::Bytes` |
| `RESULT` | `Cell::Text` — `ok`, `failed`, or the `Nothing` reason |

`FREED` is space actually reclaimed, which differs by action: for a strip it is the total
size of the removed entries; for an archive it is the size of the source folder less the
size of the `.pproject` that replaced it; for `strip+archive` it is both together. A
`none` row reports zero.

`Cell::Bytes(u64)` is new: it renders as a human-readable size in the table and as a raw
integer in JSON, so the machine-readable view keeps the number and only the renderer
applies units — the same contract `Cell::Days` already follows.

Results go to stdout, diagnostics to stderr, per the existing stream discipline.

## Testing

Following `.ai/context/testing.md`: black-box cases in `tests/cli.rs` by default, unit
tests where the logic is pure and awkward to reach through the CLI.

`src/pass/plan.rs` — unit tests, one per row of the decision table, driving a synthetic
`now`. This is where the product's actual rules are pinned, and it needs no filesystem.

`src/stripping/detection.rs`, `src/stripping/removal.rs`, `src/archive.rs`,
`src/activity.rs` — unit tests over `tempfile::tempdir()`, as the existing `activity.rs`
tests already do. `archive.rs` gets a round trip: pack a tree, read it back, confirm the
entries. `activity.rs` gets a `.pmanagerignore` case proving an ignored file does not
count as activity.

`src/models/project.rs` — a round trip for the new UPDATE query, in the established
in-memory-database idiom, plus confirmation that `archive_path` survives as a `PathBuf`.

`tests/cli.rs` — delete the `run` row from `SURFACE` and add behavioural tests:

- `--dry-run` over a seeded cold project prints the planned action and changes neither
  the folder nor the ledger.
- A real pass over a cold project with a matching profile strips it and moves its row to
  `stripped`.
- A dirty repository idle between 120 and 160 days is not archived; the same project
  past 160 is.
- A project whose strip fails exits non-zero, names the project on stderr, and still
  processes the projects after it.

## Out of scope

- `pman restore` — unpacking a `.pproject` is its own command. This design only makes it
  possible by keeping the archive readable and its path in the ledger.
- Progress reporting. `indicatif` is already a declared dependency and a pass that packs
  several gigabytes will want it, but it is TTY-only work and `auto-run` executes
  headless. It can be added behind the `IsTerminal` check `project/add.rs` already uses,
  without changing anything here.
