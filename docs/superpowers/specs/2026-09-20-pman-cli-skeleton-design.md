y# pman CLI Skeleton — Design

Date: 2026-09-20
Status: proposed

## Purpose

pManager reclaims disk space on a storage-limited machine by ageing out dormant
local projects. This spec covers only the **CLI skeleton**: the command tree, how
a command is registered, what a command receives, and how errors, logging and
tests are wired. The goal is that adding a new command is a mechanical,
three-edit operation with no plumbing to rediscover.

Domain behaviour — the database schema, project-type detection, the day
thresholds, the strip-profile format, 7z pack/unpack, and systemd unit
generation — is deliberately **out of scope** and gets its own specs.

## Background

The product, for context only (not specified here):

- Projects are registered explicitly; pManager never touches an unregistered path.
- At registration the project's git repo is detected and its type classified
  (Python, C/C++/C#, Java/Kotlin/Android, JS/TS).
- Escalation from last change: **60 days** strip packages per the project's strip
  profile; **120 days** archive to a 7z'd `.pproject`. A dirty working tree does
  not block archiving, it defers it to **160 days**.
- A project's `.pmanagerignore` excludes listed files from the last-change
  calculation.
- Strip profiles live beside the executable in `strip/`, as `profiles.json` plus
  per-language `<lang>.profile.json`.
- SQLite is the ledger: one row per project — name, path, state, archive path.

## Command surface

```
pman [GLOBAL] <COMMAND>

GLOBAL
  -v, --verbose...        Increase log verbosity (-v debug, -vv trace)
  -q, --quiet             Log errors only
      --db <PATH>         Override the database path
      --strip-dir <PATH>  Override the strip-profile directory
                          [env: PMANAGER_STRIP_DIR]

COMMANDS
  list                       List all registered projects
  project add <PATH>         Register a project [--name <NAME>]
  project remove <PATH>      Unregister a project
  project cleanup            Drop rows whose path no longer exists
  run [--dry-run]            Evaluate every project, execute what is due
  auto-run add               Install the scheduled background run
  auto-run remove            Remove the scheduled background run
  restore <NAME>             Restore an archived project
  reload                     Re-read strip profiles from disk
```

`reload` re-reads `strip/profiles.json` and `strip/*.profile.json` so profile
edits take effect without reinstalling. It is a sibling of configuration, not of
`run`.

## Decisions

### Synchronous, not async

Every workload is blocking: `git2`, `rusqlite`, `sevenz-rust2` and `walkdir`
expose no async API. An async runtime would colour every function and buy
nothing. `tokio` has been removed from the manifest. If `run` becomes slow across
many projects the answer is data parallelism — `ignore`'s parallel walker or
rayon — not concurrency.

### Library plus thin binary

`src/lib.rs` holds everything; `src/main.rs` only calls into it and maps the
result to an `ExitCode`. This lets tests exercise the age calculation, the
profile parser and the DB layer directly instead of only through the binary.

### Derive enum plus a `Command` trait

clap's derive API, one file per command owning its own args struct:

```rust
pub trait Command {
    fn run(&self, cx: &mut Cx) -> Result<()>;
}
```

Rejected alternatives:

- **Macro-generated registry** — saves one line per command, but per-command clap
  attributes (`about`, `alias`, arg groups) become awkward to thread through the
  macro, and editor completion and error messages degrade inside macro bodies.
- **`inventory` auto-registration** — requires clap's builder API, so arguments
  are pulled out of `ArgMatches` by hand and type-safe parsing is lost. Its
  link-section registration is also fragile under some LTO configurations.

The surviving match arm is not friction. It is compile-time exhaustiveness: omit
it and the build fails, rather than the command silently vanishing at runtime.

## Module layout

```
src/
  main.rs              thin: pmanager::run() -> ExitCode
  lib.rs               pub mod {cli, commands, context, error, paths, scheduler}
  cli.rs               Cli root parser and global args
  context.rs           Cx
  error.rs             Result alias over anyhow, chain rendering, exit codes
  paths.rs             database path, strip-directory resolution
  commands/
    mod.rs             Command trait, Commands enum, dispatch
    list.rs
    run.rs
    restore.rs
    reload.rs
    project/
      mod.rs           ProjectCommand enum and dispatch
      add.rs  remove.rs  cleanup.rs
    auto_run/
      mod.rs           AutoRunCommand enum and dispatch
      add.rs  remove.rs
  scheduler/
    mod.rs             trait Scheduler
    systemd.rs         #[cfg(unix)]
    windows.rs         #[cfg(windows)]
tests/
  cli.rs               assert_cmd: the tree parses, help renders
```

Nested groups follow the identical pattern one level down, so the shape is the
same wherever a command is added.

## The context object

```rust
pub struct Cx {
    pub paths: Paths,
    db: Option<Connection>,
}

impl Cx {
    pub fn db(&mut self) -> Result<&Connection>;  // opened on first use
}
```

The connection is opened lazily, creating parent directories on the way, so
`pman --help` and the unimplemented stubs never create a database file as a side
effect. That laziness is the reason the trait takes `&mut Cx`.

`Paths` resolves two locations:

- **Database** — `--db`, else `dirs::data_dir()/pmanager/pmanager.db`.
- **Strip directory** — `--strip-dir`, else `$PMANAGER_STRIP_DIR`, else the
  directory containing the executable, joined with `strip/`.

The exe-relative fallback matches the installed layout, but under `cargo run` the
executable is `target/debug/pman`, so `strip/` would have to live inside `target/`
and would be destroyed by `cargo clean`. The flag and the environment variable
exist so development is workable without diverging from the installed behaviour.

## Adding a command

1. Create `commands/foo.rs` with `#[derive(Args)] pub struct Foo` and
   `impl Command for Foo`.
2. Add `Foo(foo::Foo)` to the `Commands` enum.
3. Add `Commands::Foo(c) => c.run(cx)` to the dispatch match.

Three edits, two of them a single line. Skipping step 3 is a compile error.

## Errors and exit codes

Command bodies return `anyhow::Result<()>`, so `?` and `.context()` are available
without ceremony. Typed `thiserror` errors are introduced later, per domain
module, where a caller actually needs to match on a variant.

`main` renders the full error chain through `tracing::error!` and returns
`ExitCode::FAILURE`. Every stub is:

```rust
bail!("`pman <name>` is not implemented yet")
```

Complete help text, honest failure, exit status 1.

## Logging

`tracing_subscriber::fmt` with `EnvFilter`, initialised after argument parsing so
the flags take effect. `RUST_LOG` overrides `-v`/`-q` when it is set.

## Interactive input

`project add` prompts for a name with `dialoguer`. It also accepts `--name
<NAME>`, and only prompts when the flag is absent, so `auto-run` and scripted use
are never blocked on a TTY.

## Testing

- `tests/cli.rs` (`assert_cmd`): `--help` renders at every level of the tree, the
  full command tree parses, unknown commands fail.
- A unit test calling `Cli::command().debug_assert()` — clap's own validator for
  malformed derive setups. It catches conflicting argument definitions as soon as
  a command is added.

## Out of scope

Database schema and migrations; project-type detection; the 60/120/160-day
calculation; `.pmanagerignore` handling; strip-profile format and parsing; 7z
pack and unpack; systemd unit generation. The `Scheduler` trait in `scheduler/` and its
`#[cfg]`-gated modules are created here, with `todo!()` bodies.
