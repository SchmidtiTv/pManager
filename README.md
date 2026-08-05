# pManager

Keeps the disk from filling up with projects you have stopped working on.

Point pManager at your projects and once a day it deletes the `node_modules` of
the ones nobody has touched in two months, and compresses the ones nobody has
touched in six. Nothing is lost — the trimmed ones need an `npm install`, the
compressed ones a `pmanager restore`.

```
C:\Code\                before                C:\Code\                  after
  api-server\    1.2 GiB  (edited today)        api-server\           1.2 GiB
  old-dashboard\ 890 MiB  (3 months ago)        old-dashboard\         12 MiB
  scratch-cli\   410 MiB  (14 months ago)       scratch-cli.pArchived 2.1 MiB
```

Written in Rust, with a local SQLite database recording every project it
watches and what it has done to each one.

## Building

```sh
cargo build --release        # binary at target/release/pmanager
cargo run -- --help          # run it straight from the source tree
cargo install --path .       # install into ~/.cargo/bin
```

Requires Rust 1.93 or newer (edition 2024). SQLite is compiled in from source,
so there is nothing to install first — and no `7z` binary is needed either;
compression is pure Rust. `git` is used when it is on PATH and simply skipped
when it is not.

## Getting started

```sh
pmanager project add C:\Code\api-server   # track a project
pmanager list                             # see them all
pmanager run                              # see what has aged out (changes nothing)
pmanager run --yes                        # carry it out
pmanager schedule install                 # …every day at 03:00
pmanager completions powershell           # complete commands as you type them
```

## Usage

```
pmanager [OPTIONS] <COMMAND>
```

### Commands

| Command | Description |
| --- | --- |
| `run` | Act on projects that have aged out; reports only unless given `--yes` |
| `list` | Show every tracked project, one line each |
| `info <PROJECT>` | Show everything known about one project |
| `project add <PROJECT>` | Track a project folder |
| `project remove <PROJECT>` | Stop tracking one; nothing on disk is touched |
| `project prune` | Stop tracking projects whose folder is gone |
| `restore <PROJECT>` | Extract an archived project and put it back to work |
| `schedule install` | Register the daily run with Windows Task Scheduler |
| `schedule uninstall` | Remove the scheduled task |
| `schedule status` | Show the scheduled task |
| `completions [SHELL]` | Print the shell completion script |

A `<PROJECT>` is a path or the name shown by `pmanager list` — the name is how
you reach an archived project, whose folder is not there to point at.

### Global options

These are accepted before or after the subcommand.

| Option | Description |
| --- | --- |
| `-v`, `--verbose` | Increase logging verbosity; repeat for more (`-vv`, `-vvv`) |
| `-q`, `--quiet` | Silence everything except errors |
| `-c`, `--config <FILE>` | Use a specific configuration file |
| `--color <auto\|always\|never>` | Control colored output |
| `-h`, `--help` / `-V`, `--version` | Help and version |

### Environment

| Variable | Effect |
| --- | --- |
| `PMANAGER_CONFIG` | Path to the configuration file (`--config` wins) |
| `NO_COLOR` | Disables color when `--color` is left at `auto` |
| `RUST_LOG` | Fine-grained log filter, e.g. `pmanager=debug`; overrides `-v` |

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Failure |
| `2` | Invalid arguments |
| `3` | Configuration error |
| `4` | Not found |
| `5` | Database error |
| `6` | Archive error |
| `130` | Interrupted |

## How a project ages

```
active  ──60 days idle──▶  trimmed  ──180 days idle──▶  archived
   ▲                          │                            │
   └──────(edited again)──────┘         pmanager restore ──┘
```

**active** — normal, nothing is done to it.

**trimmed** — the dependency trees are deleted. The project itself is
completely untouched: sources, config, `.git`, everything. `npm install` undoes
it. This is where most of the disk is won, and it costs nothing but a reinstall
you would have done anyway after six months away.

**archived** — the whole project is a `.pArchived` and the folder is gone.
`pmanager restore` extracts it, reinstalls its dependencies and marks it active
again.

### What "idle" means

The age of a project is the most recent modification time of any file in it,
**ignoring directories that churn on their own**: `node_modules`, `.git`,
`dist`, `build`, `target`, `.venv`, `coverage` and the rest of `skip_dirs`.

That exclusion is the whole trick. An `npm install` rewrites thousands of files
without anyone opening the project, and counting those would keep a dead
project looking alive forever — which is exactly the failure mode of judging
staleness by folder timestamps.

Git activity is read separately, and shown by `list` and `info`, but it does
not change a project's age: committing is not the same as editing, and a
project can be worked on for a week without a single commit.

### What is never thrown away

The archive holds **everything except the purge directories** — including
`.git`. History, stashes, local branches and remotes all survive the round
trip, because none of that is regenerable. Only `node_modules` and whatever
else you add to `purge_dirs` is left out, on the grounds that one command
rebuilds it.

The rule is a single one, and it runs in both directions: **what pManager is
willing to delete is exactly what it refuses to store.**

### What protects the archived projects

The delete is what frees the disk, so it happens last and only after the
archive has been proven good:

1. the archive is written to a fresh `.pArchived`, and deleted again if
   compression fails — one on disk is never half-written;
2. it is read back and its file count compared against what was walked;
3. only then is the project folder removed.

A failure at any earlier point leaves the project exactly where it was. If an
archive of that name already exists, the project is skipped rather than
overwritten; pass `--force` when you do want it replaced. Use `--keep-source`
for a first run if you would rather inspect the archives before trusting the
deletion.

One project failing does not stop the others: an unplugged drive is reported,
the remaining projects are still swept, and the run ends with a non-zero exit
code so a failed scheduled run is visible in Task Scheduler's history.

### Unfinished work is held back

A project with uncommitted changes or unpushed commits is never archived,
however old it is:

```
$ pmanager run
half-done            1y ago  old enough to archive, held back: 1 uncommitted change(s)
```

Nothing would actually be *lost* by archiving it — the working tree and `.git`
both go into the archive — but a folder disappearing out from under work you
left half-finished is a surprise, and that is what uncommitted changes mean.
`--force` overrides it. Trimming is not affected: deleting `node_modules` never
touches your work.

### Restoring does not restart the clock at zero

Extracting an archive preserves modification times, so a project restored from
a two-year-old archive arrives with two-year-old files. pManager records when
it restored a project and ages it from there, so the next run does not
immediately archive it straight back.

## Tracking projects

A tracked project is **one project folder** — the one holding `package.json`,
`.git` and the source — not the directory they all live in.

```sh
pmanager project add C:\Code\api-server
pmanager project add C:\Code\web C:\Code\api        # several at once
pmanager project add C:\Code\thing --name "Thing"   # override the display name
pmanager project remove api-server
```

The name comes from `package.json`'s `name` when there is one, otherwise the
folder name. Adding a project reports what the next run would do to it, so the
first `run` is never a surprise:

```
$ pmanager project add C:\Code\old-dashboard
tracking old-dashboard  C:\Code\old-dashboard
  last edited 3mo ago, the next run would free 890.2 MiB
```

A project whose folder has been moved or deleted stays tracked until you say
otherwise, since a drive that is not plugged in looks exactly the same:

```sh
pmanager project prune          # report which are missing
pmanager project prune --yes    # stop tracking them
```

## What you have

```
$ pmanager list
NAME           EDITED     COMMITTED    STATE     NOTES
api-server     2h ago     2h ago       active    1.2 GiB to reclaim
half-done      1y ago     1y ago       active    410.0 MiB to reclaim, stale >60d, 1 uncommitted
old-dashboard  3mo ago    3mo ago      trimmed   stale >60d
scratch-cli    8mo ago    —            archived  2.1 MiB

4 project(s): 2 active, 1 trimmed, 1 archived
1.6 GiB sitting in dependency trees
890.2 MiB reclaimed so far
```

`EDITED` is the idle clock described above; `COMMITTED` is the last commit.
Both are read off the disk on every run, so they are true rather than
remembered — which is why `list` takes a moment on a large collection.

```sh
pmanager list --state trimmed   # active, trimmed or archived
pmanager list --stale           # past the trim threshold
pmanager list --dirty           # uncommitted changes
pmanager list --unpushed        # commits the remote has not seen
```

`pmanager info <PROJECT>` is the same information for one project, in full —
which file was edited last, where each dependency tree is, the branch, the
commit, the changed files — ending with what the next run would do and, when
the answer is nothing, why not:

```
$ pmanager info half-done
half-done  C:\Code\half-done
  state           active
  tracked         3mo ago
  last edit       1y ago  src/index.js
  dependencies    410.0 MiB in 1 tree(s)
                    410.0 MiB  node_modules
  branch          main  → origin/main
  last commit     1y ago  ff1b8aa  Rough draft  (Schmidti)
  untracked       1 file(s)
                    src/wip.js

  next run        nothing — past 180 days, but held back by 1 uncommitted change(s) (--force overrides)
```

## The run

`pmanager run` surveys every tracked project and **reports what it would do**.
It changes nothing without `--yes`:

```
$ pmanager run
old-dashboard        3mo ago  would trim 1 dependency tree(s), freeing 890.2 MiB
scratch-cli           1y ago  would archive to C:\Code\scratch-cli.pArchived

2 project(s) would change
nothing was changed; pass --yes to apply
```

```
$ pmanager run --yes
old-dashboard        3mo ago  trimmed 1 dependency tree(s), freed 890.2 MiB
scratch-cli           1y ago  archived to C:\Code\scratch-cli.pArchived  1204 file(s), 410.0 MiB -> 2.1 MiB, freed 407.9 MiB

1 trimmed, 1 archived, 1.3 GiB freed
```

Useful flags:

```sh
pmanager run --yes --project api-server   # one project; repeatable
pmanager run --yes --trim-days 30         # override the thresholds for this run
pmanager run --yes --no-archive           # trim only, never archive
pmanager run --yes --no-trim              # archive only, never trim
pmanager run --yes --keep-source          # write the archives, keep the folders
pmanager run --yes --force                # archive despite unfinished work
pmanager run --yes --archive-dir D:\Cold  # collect the archives in one place
pmanager run --all                        # show projects nothing happens to
pmanager run --yes --level 9 --threads 8  # compression settings for this run
```

A workspace with a `node_modules` per package has all of them trimmed, not just
the one at the top. If one cannot be deleted — a file still open in an editor —
the others still go and the survivor is picked up on the next run.

## Restoring

```sh
pmanager restore scratch-cli                 # extract, npm install, mark active
pmanager restore scratch-cli --into D:\Code  # somewhere else
pmanager restore scratch-cli --no-install    # skip the dependency install
pmanager restore scratch-cli --delete-archive
```

The archive is extracted into a staging directory beside the destination and
moved into place as one rename once it is whole, so a failed extraction can
never half-overwrite anything. An existing folder in the way is refused rather
than merged into.

**The archive is kept by default.** Deleting the only copy of a project the
moment it is unpacked, before anyone has looked at the result, would be the one
irreversible step in a design that has none.

A `.pArchived` written by pManager is an ordinary 7-Zip archive under another
name — the extension only marks it as pManager's. 7-Zip identifies it by its
contents, so a project can be recovered by hand on any machine that has never
heard of pManager; point 7-Zip at the file and it opens. The folder name is
stored inside it.

Archives written by earlier versions are named `.7z`. They keep restoring
unchanged: `pmanager restore` uses the path recorded in the database, and
extraction goes by the file's own signature rather than its name. There is
nothing to migrate, and renaming an old archive by hand is unnecessary.

## Scheduling the daily run

pManager has no resident daemon. `schedule install` hands `pmanager run --yes`
to Windows Task Scheduler, which already knows how to run something daily,
survive reboots and record the outcome.

```sh
pmanager schedule install                 # daily at 03:00
pmanager schedule install --at 21:30      # …or whenever suits
pmanager schedule install --force         # replace an existing task
pmanager schedule status
pmanager schedule uninstall
```

Installing the schedule is the deliberate act that grants the daily run
permission to delete — that is why the task carries `--yes` while typing
`pmanager run` yourself does not.

The task is named `pManager daily sweep` (`--name` changes it) and runs the
executable at its current path, so move the binary and you will want to install
it again. An explicit `--config` is carried into the task.

On non-Windows platforms these commands report that Task Scheduler is not
available; run `pmanager run --yes` from cron or a systemd timer instead.

## Shell completions

`pmanager completions <SHELL>` writes a completion script to stdout. Put it
where your shell looks for one:

```powershell
pmanager completions powershell >> $PROFILE
```

```sh
pmanager completions bash > ~/.local/share/bash-completion/completions/pmanager
pmanager completions zsh  > ~/.zfunc/_pmanager     # ~/.zfunc on $fpath
pmanager completions fish > ~/.config/fish/completions/pmanager.fish
pmanager completions elvish >> ~/.config/elvish/rc.elv
```

Then start a new shell: these files are read once, at startup. To try it
without installing anything, `eval "$(pmanager completions bash)"`.

Omit the shell and it is guessed from the environment — handy in a profile
that is shared between machines, though `$SHELL` is the login shell rather
than the current one and is not set at all under PowerShell, so naming the
shell is the reliable form.

The script is generated from the same command line definition the binary
parses with, so it always covers every current subcommand, flag and value —
including `--state`'s values and the shells listed above. It is a snapshot of
one version, though: regenerate it after upgrading pmanager.

Completions never read the configuration file or the database, so a broken
config cannot break the shell that runs this at startup.

## Database

A single SQLite file, by default `pmanager.db` inside the platform data
directory (set `data_dir` or `database` to move it). It is created and migrated
automatically on first use. SQLite is linked statically via `rusqlite`'s
bundled build, so there is no system package to install and no version skew
between machines.

The schema is versioned with SQLite's own `user_version` pragma and migrated on
open: each entry in `MIGRATIONS` (`src/db.rs`) runs inside a transaction, so an
interrupted upgrade leaves the database on its previous version rather than
half-migrated. A database written by a *newer* pmanager is refused rather than
downgraded. To add a table or column, append a migration — never edit one that
has shipped.

Connections open with WAL journaling, `foreign_keys = ON` and a 5 second busy
timeout, so a second `pmanager` waits its turn instead of failing.

One table, `projects`: where each project is in its life, when it got there,
how much trimming has reclaimed, and — for an archived one — where the archive
went and what it held. That last part is the only thing stored that cannot be
re-derived by looking at the disk, and it is what lets `list` say what
restoring a project would cost.

The database is a record, not the source of truth: the projects and archives on
disk are. Deleting it loses the registrations and the history, not a single
project.

## Configuration

Configuration is optional — every key has a default. To change one, write a
`config.toml` by hand in the platform configuration directory:

| Platform | Path |
| --- | --- |
| Linux | `~/.config/pmanager/config.toml` |
| macOS | `~/Library/Application Support/pmanager/config.toml` |
| Windows | `%APPDATA%\pmanager\config.toml` |

```toml
trim_days = 30
archive_days = 120
purge_dirs = ["node_modules", "target", ".venv"]
archive_dir = 'D:\Cold storage'
install_command = ["pnpm", "install"]
```

Values are layered, lowest precedence first: built-in defaults, then the
configuration file, then environment variables, then command line flags. A
missing file is fine — the defaults apply — but an unreadable or malformed one
is an error. Unknown keys are rejected so typos surface immediately.

| Key | Default | Meaning |
| --- | --- | --- |
| `data_dir` | platform data directory | Where pManager keeps its data |
| `database` | `pmanager.db` in `data_dir` | SQLite database file |
| `trim_days` | `60` | Days idle before the dependency trees go |
| `archive_days` | `180` | Days idle before the project is compressed |
| `purge_dirs` | `["node_modules"]` | Deleted when trimming; kept out of archives |
| `skip_dirs` | see `src/scan.rs` | Not counted when dating a project |
| `archive_dir` | beside each project | Where archives are written |
| `install_command` | `npm install` when there is a `package.json` | Run by `restore` |
| `compression_level` | `6` | LZMA2 level used by `run`, 0–9 |
| `compression_threads` | `1` | Compression threads used by `run` |

`archive_days` below `trim_days` is rejected rather than obeyed: as written it
would archive and delete every stale project without ever trimming one.

Anything in `purge_dirs` is added to `skip_dirs` automatically — a directory
pManager is willing to delete cannot also be evidence that a project is alive.
That way adding `target` to `purge_dirs` does not silently keep every Rust
project looking freshly edited.

`install_command = []` turns the reinstall step off entirely.

### Beyond Node

The defaults are Node-shaped because that is what most of this is for, but
nothing in the design is:

```toml
purge_dirs = ["node_modules", "target", ".venv", "__pycache__", "vendor"]
install_command = []          # too many ecosystems to guess; do it yourself
```

## Layout

```
src/
  main.rs             process entry point: parse, run, map errors to exit codes
  lib.rs              library crate; dispatches a parsed command
  cli.rs              clap definitions: global flags and the subcommand list
  scan.rs             measuring a project: last edit, purgeable size, ages
  git.rs              read-only git status via the `git` binary
  lifecycle.rs        the policy: states, thresholds, decide, trim, archive
  archive.rs          .pArchived (7z) creation, verification and extraction
  db.rs               SQLite connection, migrations and queries
  config.rs           config file discovery, parsing and defaults
  error.rs            error type and its exit-code mapping
  logging.rs          tracing subscriber setup (stderr)
  commands/
    mod.rs            shared Context handed to every subcommand
    run.rs            `pmanager run`
    list.rs           `pmanager list`
    info.rs           `pmanager info`
    project.rs        `pmanager project`
    restore.rs        `pmanager restore`
    schedule.rs       `pmanager schedule`
    completions.rs    `pmanager completions`
tests/
  cli.rs              end-to-end tests that run the built binary
```

`scan.rs`, `git.rs` and `lifecycle.rs` hold the policy and are filesystem-only:
no database, no printing, no clap. That is what lets the ageing rules and the
archive-then-delete sequence be tested directly, and it keeps `commands/run.rs`
down to gathering settings, looping and reporting.

`main.rs` is deliberately thin: the logic lives in the library so tests and any
future front-end can drive it without a process boundary.

### Adding a command

1. Create `src/commands/<name>.rs` with a `#[derive(clap::Args)]` struct and a
   `run(&Args, &Context) -> Result<()>` function.
2. Register the module in `src/commands/mod.rs`.
3. Add a variant to `Command` in `src/cli.rs` and a match arm in `run` in
   `src/lib.rs`.

Write output to stdout and diagnostics to stderr via `tracing`, so that piping
stdout stays useful.

## Development

```sh
cargo test              # unit + end-to-end tests
cargo clippy --all-targets
cargo fmt --check
cargo deny check        # licenses, advisories, sources (needs cargo-deny)
```

CI checks formatting, runs clippy over all targets and builds the release
binary on Windows; it also checks dependency licences and advisories. It does
**not** run the test suite — `cargo test` is a local step. Because clippy uses
`--all-targets` the tests are still compiled and type-checked on every push, so
they cannot silently rot; they just are not executed.

The git tests skip themselves when `git` is not on PATH rather than failing.

## Licensing of dependencies

Every dependency can be used commercially, including in closed-source products.
The tree resolves to MIT, Apache-2.0, MPL-2.0, Unicode-3.0 and Zlib; no crate
*requires* GPL, LGPL or AGPL terms. (A few are multi-licensed and merely
*offer* a copyleft option alongside a permissive one — `r-efi`, for instance,
is "MIT OR Apache-2.0 OR LGPL-2.1-or-later", and taking MIT is enough.)

Two things are worth knowing:

- **SQLite** itself is in the public domain, and `sevenz-rust2` is a pure-Rust
  implementation of the 7z format under Apache-2.0 — so neither the database
  nor the archiver drags in a copyleft or patent-encumbered component.
- **`option-ext`** (a transitive dependency of `directories`) is MPL-2.0. That
  is file-level copyleft: the obligation is to publish changes to *that crate's
  own files*. Using it unmodified, as here, carries no source-disclosure
  requirement for this project.

The practical obligation across the tree is the usual one — keep the copyright
and license notices when distributing binaries.

`deny.toml` encodes this as an allowlist and CI runs `cargo deny check`, so a
dependency arriving under a license outside that set fails the build instead of
slipping in unnoticed. The list holds only the licenses actually in use, so a
new one is a deliberate decision rather than something pre-approved. Dropping
MPL-2.0 from it means replacing `directories`.

pManager itself has no license declared yet — that is a call for the repository
owner. Until one is chosen the crate is marked `publish = false` and cargo-deny
skips it (`private.ignore`); setting `license` in `Cargo.toml` afterwards lets
that setting go away.

## Relationship to pTag

pManager is a port of [pTag](pTag/), a Python tool that did the same job. The
core idea is unchanged — age projects out by last edit, delete the dependency
trees, then compress what is left — and pTag's `check` is this tool's `run`.

What changed in the move:

- **`.git` is archived rather than skipped.** pTag left it out of the zip, which
  meant a deep-archived project lost its history unless everything had been
  pushed. It compensated by refusing to zip anything unpushed; pManager keeps
  that as a "don't surprise me" rule but no longer needs it to protect history.
- **Restoring records itself.** pTag would re-archive a project it had just
  restored, because unzipping preserves the old file times. pManager ages a
  project from its restore.
- **The staircase is gone.** pTag required a project to pass through *archived*
  before it could be *deep-archived*, which took two runs a threshold apart. A
  project old enough to compress is compressed.
- **`--yes` instead of prompts.** pTag asked before each deletion, which is
  awkward for a scheduled task. pManager reports by default and acts on `--yes`.
- **SQLite instead of `db.json`**, one process-wide `run` instead of per-command
  rescans, and a compiled binary instead of a Python interpreter.

pTag's Flask web UI (`pTag/server.py`) has no equivalent here.
