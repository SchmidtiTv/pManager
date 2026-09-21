# pman run — Design

Date: 2026-09-21
Status: proposed

## Purpose

`pman run` is the part of pManager that actually reclaims disk space. For every
registered project it works out how long the project has been idle and, when a
threshold is due, strips its regenerable dependency folders or packs the whole
folder into a 7z `.pproject` and deletes the original. `pman run --dry-run`
reports the same plan without changing anything.

Success means: a scheduled `pman run` can be left unattended, never deletes
anything that is not recoverable from an archive it has verified, and a dry
run shows exactly what a real run would do.

## Decisions taken with the owner

| Question | Decision |
| --- | --- |
| Where does an archive go? | Beside the project: `<parent>/<folder>.pproject`. |
| Is the original folder deleted? | Yes, after the archive is verified. |
| Does a dirty git tree hold back stripping? | No. Stripped files are regenerable; dirty only moves archiving from 120 to 160 days. |
| Which strip profiles apply? | Every profile whose detection matches. No match: nothing is removed, the project still becomes `stripped`. |
| `.pmanagerignore` | One file in the project root, gitignore syntax. It only affects the last-change check, never what is stripped or archived. |
| Archive encryption | None. Plain LZMA2. |
| Architecture | Plan, then execute: a pure decision function; `--dry-run` prints the plan, a real run executes the same plan. |

## Decision rules

`idle` is `now - last_change` in whole days. `last_change` is recomputed on
every run (see `activity` below).

*Archive due* means `idle >= 120` when the tree is clean or not a git
repository, and `idle >= 160` when it is dirty.

| State | Condition | Action |
| --- | --- | --- |
| archived | always | `Nothing` |
| registered or stripped | folder missing | `Missing` (warn: run `pman project cleanup`) |
| registered | archive due | `StripThenArchive` |
| registered | `idle >= 60`, archive not due | `Strip` |
| stripped | archive due | `Archive` |
| stripped | `idle < 60` | `Reset` to `registered` |
| otherwise | | `Nothing` |

- The rows are evaluated top to bottom; the first match wins.
- `StripThenArchive` strips first so dependency folders never end up inside a
  `.pproject`.
- `Reset` exists because a stripped project that is being worked on again
  will have its dependency folders reinstalled; returning it to `registered`
  lets it be stripped again when it next goes idle.
- **Dirty** means git reports any modified, staged or untracked file. Ignored
  files do not count. If git cannot answer (e.g. a corrupt repository), the
  project is treated as dirty: the only effect is a later archive.
- A profile **matches** when any one of its `detection` elements exists in
  the project root with the declared type (`file` or `folder`).

## Components

| Unit | Responsibility | Depends on |
| --- | --- | --- |
| `src/lifecycle/decide.rs` (new) | Pure `decide(&ProjectState, &Facts, now) -> Action`. Owns the 60/120/160 constants. `Facts { exists, last_change, dirty, profiles }`. `Action` is `Nothing`, `Missing`, `Strip`, `StripThenArchive`, `Archive`, `Reset`. | `chrono` only |
| `src/lifecycle/facts.rs` (new) | `gather(&Project, &[StrippingProfile]) -> Result<Facts>`. Reads the disk, never writes. Does not compute last change, dirtiness or matches for a missing folder. | `activity`, `git`, `stripping::detect` |
| `src/activity.rs` (extend) | `last_change` also skips paths matched by the root `.pmanagerignore` (via `ignore::gitignore`). `pman project add` inherits this. | `ignore`, `walkdir` |
| `src/git.rs` (new) | `is_dirty(path) -> bool`. Uses git2 statuses with untracked files included and ignored files excluded. Any error: log a warning, return `true`. | `git2` |
| `src/stripping/detect.rs` (new) | `matching(&[StrippingProfile], root) -> Vec<&StrippingProfile>`. | models |
| `src/stripping/strip.rs` (new) | `strip(root, &[&StrippingProfile]) -> Result<StripReport>`; `StripReport` lists removed paths. Re-checks every element with `validation::stays_inside`; skips elements that do not exist or whose type differs; never follows symlinks (a symlinked element removes the link, never its target). | `validation` |
| `src/archive.rs` (new) | `archive(root) -> Result<PathBuf>`: create and verify the `.pproject` (steps below). Does not delete the folder; `run` does, after the ledger is updated. | `sevenz-rust2`, `walkdir` |
| `src/models/project.rs` (extend) | `update_last_change(path, at)` and `update_state(path, &ProjectState)` query builders. | `SqlBuilder` |
| `src/commands/run.rs` | Takes the run lock, loads profiles, builds the plan, executes it unless `--dry-run`, prints the result. | all of the above |

`decide` never sees a path and `strip`/`archive` never see a threshold, so the
timing rules and the file operations are tested separately.

## Run flow

1. **Lock.** Open `pmanager.run.lock` beside the database and take
   `File::try_lock`. If it is held: fail with "another `pman run` is in
   progress". The OS releases the lock when the process exits, so a crash
   never leaves a stale lock. `--dry-run` takes the lock too.
2. **Profiles.** `stripping::load(strip_dir)`. A missing or invalid index or
   profile aborts the run before any project is examined.
3. **Plan.** Read every project; for each non-archived one call
   `facts::gather`, then `decide`.
4. **Execute** (skipped by `--dry-run`), project by project:
   - store the recomputed `last_change_at` (every project whose folder
     exists and is not archived);
   - `Strip`: `strip`, then store `stripped`;
   - `Archive`: archive steps below;
   - `StripThenArchive`: both, in that order, each state stored as soon as
     its step succeeds;
   - `Reset`: store `registered`.
5. **Report** the table below and set the exit code.

### Archive steps

1. Target `<parent>/<folder>.pproject`. If it exists, fail this project; an
   existing file is never overwritten. If the folder (after any strip)
   contains a symlink, fail this project too: whether a link is stored or
   followed cannot be verified against the folder, so it is not archived and
   not deleted. The error names the first link found.
2. Compress the folder into `<target>.partial` (LZMA2, no password).
3. Verify: read the archive back entry by entry (the reader checks each
   entry's CRC) and compare the set of files, as relative path plus size,
   with a fresh walk of the folder (regular files only; empty directories
   are archived but not verified). On any mismatch or read error, delete the
   `.partial` and fail this project; the folder is untouched.
4. Rename `.partial` to `.pproject`.
5. In `run`: store `archived` with `archive_path` in the ledger, **then**
   `remove_dir_all` the folder. If deletion fails part-way, the ledger
   already points at a verified archive; log a warning naming the folder.

## Errors

- A project that fails is logged to stderr with its full error chain and the
  run continues with the next project.
- Exit code: 0 when every project succeeded; 1 with "N of M projects failed"
  otherwise.
- Profile loading, lock and database errors abort the whole run with exit 1.

## Output

Data goes to stdout through `ConsoleData`/`build_output`, one row per
project, so `--output json` works:

| Column | Values |
| --- | --- |
| NAME | project name |
| IDLE | days (`Cell::Days`) |
| DIRTY | `yes`, `no`, `-` (no repo, missing, or archived) |
| ACTION | `strip`, `archive`, `strip+archive`, `reset`, `missing`, `-` |
| RESULT | `done`, `failed`, `planned` (dry run), `-` (no action) |
| PATH | project path |

For archived and missing projects the IDLE column is computed from the
stored `last_change_at`.
Progress messages (for example "stripping `foo`: removed node_modules,
.venv") go to stderr through `tracing`.

## Testing

TDD per unit.

- `decide`: a table-driven test over every rule, including the boundaries
  59/60, 119/120, 159/160 for clean and dirty, archived, missing, reset and
  strip-then-archive.
- `activity`: `.pmanagerignore` globs, `dir/` patterns and `!` negation are
  excluded from the last change; a `.pmanagerignore` below the root has no
  effect.
- `git`: temp repositories via `git2::Repository::init`: clean; modified
  tracked file; untracked file; only an ignored file (not dirty); a broken
  `.git` (dirty).
- `detect` / `strip`: file vs folder type matching, several matching
  profiles, missing elements skipped, and (unix only) a symlinked element
  pointing outside the project removes the link while the target survives.
- `archive`: round trip (archive, extract, compare trees); an existing
  `.pproject` is refused; a folder containing a symlink is refused (unix);
  a failed verification leaves the folder and no `.partial`.
- `models::project`: `update_state` and `update_last_change` round trips,
  including the archived-path invariant.
- CLI (`tests/cli.rs`), always with a temp `--db` and `--strip-dir`, and
  projects whose files are dated 70, 130 and 170 days back: `--dry-run`
  changes nothing on disk or in the ledger and reports `planned`; a real run
  strips, archives and deletes, and `list` shows the new states; an invalid
  profile aborts before touching any project; a held lock makes a second run
  fail; JSON output parses. `run` leaves the `SURFACE` table.

## Out of scope

- `pman project cleanup`, `auto-run add/remove`, restoring a `.pproject`.
- Parallel processing of projects.
- Reporting reclaimed disk space.
- Encrypted archives. The `aes256` feature of `sevenz-rust2` stays enabled
  but unused for now.
