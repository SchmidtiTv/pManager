# pman CLI Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete `pman` command surface as working, self-documenting stubs, so that adding a real command later is a three-edit mechanical operation.

**Architecture:** A library crate (`src/lib.rs`) holds everything; `src/main.rs` only maps the result to an `ExitCode`. Commands are clap-derive `Args` structs, one per file, each implementing a `Command` trait that receives a shared `Cx` context. Nested command groups (`project`, `auto-run`) repeat the identical enum-plus-dispatch pattern one level down.

**Tech Stack:** Rust edition 2024, clap 4 (derive + env), anyhow, tracing + tracing-subscriber (env-filter), rusqlite (bundled), dirs, dialoguer; assert_cmd + predicates + tempfile for tests.

**Spec:** `docs/superpowers/specs/2026-09-20-pman-cli-skeleton-design.md`

## Global Constraints

- Package is `pmanager`; the single binary target is `pman`. Never rename either.
- Edition `2024`, toolchain rustc 1.98.1. `publish = false`.
- **Synchronous only.** `tokio` has been removed. No `async fn`, no runtime, no `.await` anywhere in this plan.
- `cargo deny check` must report `advisories ok, bans ok, licenses ok, sources ok` after every task. The `deny.toml` allowlist is deliberately narrow (`Apache-2.0`, `MIT`, `MPL-2.0`, `Unicode-3.0`, `Zlib`). If a new dependency trips it, **disable the offending optional feature rather than widening the allowlist**, and stop and ask before changing `deny.toml`.
- Stub bodies use `anyhow::bail!`, never `todo!()` or `unimplemented!()` — a CLI must fail, not panic. (This refines the spec, which said `todo!()`.)
- Stub message format is exactly: ``bail!("`pman <full command path>` is not implemented yet")``.
- All log output goes to **stderr**, never stdout, so command output stays pipeable.
- Every commit message ends with the trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/main.rs` | Two lines: call `pmanager::run()`, return its `ExitCode`. |
| `src/lib.rs` | Module declarations; `run()` / `try_run()` top-level wiring. |
| `src/error.rs` | `Result` alias over anyhow; renders an error chain to the log. |
| `src/cli.rs` | `Cli` root parser, `GlobalArgs`, tracing initialisation. |
| `src/paths.rs` | Pure resolution of the database path and strip directory. |
| `src/context.rs` | `Cx` — the object every command receives; lazy SQLite handle. |
| `src/commands/mod.rs` | `Command` trait, root `Commands` enum, root dispatch. |
| `src/commands/{list,reload,run,restore}.rs` | One flat command each. |
| `src/commands/project/` | `project` group: `mod.rs` + `add`/`remove`/`cleanup`. |
| `src/commands/auto_run/` | `auto-run` group: `mod.rs` + `add`/`remove`. |
| `src/scheduler/` | `Scheduler` trait plus `#[cfg]`-gated platform modules. |
| `tests/cli.rs` | Black-box surface tests via `assert_cmd`. |
| `docs/adding-a-command.md` | The three-edit recipe, written once the pattern exists. |

---

### Task 1: Binary wiring, errors, logging, global args

Produces a runnable `pman` with `--help`, `--version` and the four global flags. No subcommands yet.

**Files:**
- Modify: `Cargo.toml` (add the `env` feature to clap)
- Modify: `.gitignore`
- Create: `src/lib.rs`, `src/error.rs`, `src/cli.rs`
- Modify: `src/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pmanager::run() -> std::process::ExitCode`
  - `pmanager::error::Result<T = ()>` (alias for `anyhow::Result<T>`)
  - `pmanager::error::report(err: &anyhow::Error)`
  - `pmanager::cli::Cli` with field `global: GlobalArgs`
  - `pmanager::cli::GlobalArgs { verbose: u8, quiet: bool, db: Option<PathBuf>, strip_dir: Option<PathBuf> }`
  - `pmanager::cli::init_tracing(verbose: u8, quiet: bool)`

- [ ] **Step 1: Add the clap `env` feature**

`--strip-dir` reads `PMANAGER_STRIP_DIR`, which the derive macro only supports with clap's `env` feature.

```bash
cd ~/dev/rust/pManager
cargo add clap --features derive,env
```

- [ ] **Step 2: Write the failing tests**

Create `tests/cli.rs`:

```rust
use assert_cmd::Command;
use predicates::str::contains;

fn pman() -> Command {
    Command::cargo_bin("pman").expect("binary `pman` should build")
}

#[test]
fn version_flag_prints_the_package_version() {
    pman()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_every_global_flag() {
    pman()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--verbose"))
        .stdout(contains("--quiet"))
        .stdout(contains("--db"))
        .stdout(contains("--strip-dir"));
}

#[test]
fn an_unknown_flag_is_rejected() {
    pman().arg("--definitely-not-a-flag").assert().failure();
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL — the binary prints `Hello, world!` and knows no flags.

- [ ] **Step 4: Write `src/error.rs`**

```rust
//! Error plumbing: one alias, one renderer.

/// Every fallible function in the crate returns this.
pub type Result<T = ()> = anyhow::Result<T>;

/// Render a full error chain to the log. `{:#}` walks the `anyhow` causes.
pub fn report(err: &anyhow::Error) {
    tracing::error!("{err:#}");
}
```

- [ ] **Step 5: Write `src/cli.rs`**

```rust
//! The root parser, its global flags, and log initialisation.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "pman",
    version,
    about = "Ages out dormant local projects to reclaim disk space",
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
}

#[derive(Args, Debug)]
pub struct GlobalArgs {
    /// Increase log verbosity (-v debug, -vv trace)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Log errors only
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Override the database path
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Override the strip-profile directory
    #[arg(long, global = true, value_name = "PATH", env = "PMANAGER_STRIP_DIR")]
    pub strip_dir: Option<PathBuf>,
}

/// Install the log subscriber. `RUST_LOG` wins over the flags when set.
///
/// Writes to stderr so that command output on stdout stays pipeable, and uses
/// `try_init` so repeated calls inside one test process are harmless.
pub fn init_tracing(verbose: u8, quiet: bool) {
    let fallback = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback));

    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_env_filter(filter)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    /// clap's own validator for malformed derive setups: conflicting argument
    /// names, duplicate shorts, bad defaults. Rerun as every command is added.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
```

- [ ] **Step 6: Write `src/lib.rs`**

```rust
//! pManager — ages out dormant local projects to reclaim disk space.

pub mod cli;
pub mod error;

use std::process::ExitCode;

use clap::Parser;

/// Process entry point. Never returns an error; it reports and converts.
pub fn run() -> ExitCode {
    match try_run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error::report(&err);
            ExitCode::FAILURE
        }
    }
}

fn try_run() -> error::Result {
    let cli = cli::Cli::parse();
    cli::init_tracing(cli.global.verbose, cli.global.quiet);
    Ok(())
}
```

- [ ] **Step 7: Replace `src/main.rs`**

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    pmanager::run()
}
```

- [ ] **Step 8: Ignore IDE files**

```bash
cd ~/dev/rust/pManager
printf '/.idea\n' >> .gitignore
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS — 3 integration tests, 1 unit test.

- [ ] **Step 10: Verify licences still pass**

Run: `cd ~/dev/rust/pManager && cargo deny check`
Expected: `advisories ok, bans ok, licenses ok, sources ok`

- [ ] **Step 11: Commit (this is the repository's initial commit)**

```bash
cd ~/dev/rust/pManager
git add .gitignore Cargo.toml Cargo.lock deny.toml src tests docs
git commit -m "feat: wire up pman binary with global flags and logging" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Path resolution

**Files:**
- Create: `src/paths.rs`
- Modify: `src/lib.rs` (add `pub mod paths;`)

**Interfaces:**
- Consumes: `pmanager::error::Result`
- Produces:
  - `pmanager::paths::Paths { pub db: PathBuf, pub strip_dir: PathBuf }`
  - `Paths::resolve(db: Option<PathBuf>, strip_dir: Option<PathBuf>) -> Result<Paths>`
  - `Paths::resolve_with(db: Option<PathBuf>, strip_dir: Option<PathBuf>, data_dir: &Path, exe_dir: &Path) -> Paths`

The pure `resolve_with` exists so the defaults can be tested without touching the real home directory. The environment variable is **not** read here — clap's `env` attribute already folded it into the `Option`.

- [ ] **Step 1: Write the failing tests**

Append to `src/paths.rs` (write the test module first, above the implementation, so the file compiles only once Step 3 lands):

```rust
#[cfg(test)]
mod tests {
    use super::Paths;
    use std::path::{Path, PathBuf};

    #[test]
    fn defaults_derive_from_data_dir_and_exe_dir() {
        let paths = Paths::resolve_with(None, None, Path::new("/data"), Path::new("/opt/pman"));
        assert_eq!(paths.db, PathBuf::from("/data/pmanager/pmanager.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/opt/pman/strip"));
    }

    #[test]
    fn explicit_overrides_win() {
        let paths = Paths::resolve_with(
            Some(PathBuf::from("/tmp/my.db")),
            Some(PathBuf::from("/tmp/strip")),
            Path::new("/data"),
            Path::new("/opt/pman"),
        );
        assert_eq!(paths.db, PathBuf::from("/tmp/my.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/tmp/strip"));
    }

    #[test]
    fn overrides_are_independent() {
        let paths = Paths::resolve_with(
            Some(PathBuf::from("/tmp/my.db")),
            None,
            Path::new("/data"),
            Path::new("/opt/pman"),
        );
        assert_eq!(paths.db, PathBuf::from("/tmp/my.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/opt/pman/strip"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test paths`
Expected: FAIL — `paths` is not a module of the crate.

- [ ] **Step 3: Write the implementation above the test module in `src/paths.rs`**

```rust
//! Where pManager keeps its database and its strip profiles.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The SQLite ledger.
    pub db: PathBuf,
    /// Directory holding `profiles.json` and the `*.profile.json` files.
    pub strip_dir: PathBuf,
}

impl Paths {
    /// Resolve against the real user data directory and the running executable.
    pub fn resolve(db: Option<PathBuf>, strip_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = dirs::data_dir().context("could not determine the user data directory")?;

        let exe = std::env::current_exe().context("could not determine the executable path")?;
        let exe_dir = exe
            .parent()
            .context("executable path has no parent directory")?
            .to_path_buf();

        Ok(Self::resolve_with(db, strip_dir, &data_dir, &exe_dir))
    }

    /// The pure half, so defaults are testable without touching `$HOME`.
    ///
    /// The strip directory sits beside the executable, matching the installed
    /// layout. Under `cargo run` that is `target/debug/strip`, which `cargo
    /// clean` destroys — hence `--strip-dir` and `PMANAGER_STRIP_DIR`.
    pub fn resolve_with(
        db: Option<PathBuf>,
        strip_dir: Option<PathBuf>,
        data_dir: &Path,
        exe_dir: &Path,
    ) -> Self {
        Self {
            db: db.unwrap_or_else(|| data_dir.join("pmanager").join("pmanager.db")),
            strip_dir: strip_dir.unwrap_or_else(|| exe_dir.join("strip")),
        }
    }
}
```

- [ ] **Step 4: Declare the module in `src/lib.rs`**

Add `pub mod paths;` to the module block, keeping it alphabetical:

```rust
pub mod cli;
pub mod error;
pub mod paths;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS — 3 new unit tests in `paths`.

- [ ] **Step 6: Commit**

```bash
cd ~/dev/rust/pManager
git add src/lib.rs src/paths.rs
git commit -m "feat: resolve database and strip-profile paths" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: The command context

**Files:**
- Create: `src/context.rs`
- Modify: `src/lib.rs` (add `pub mod context;`)

**Interfaces:**
- Consumes: `pmanager::paths::Paths`, `pmanager::error::Result`
- Produces:
  - `pmanager::context::Cx { pub paths: Paths }`
  - `Cx::new(paths: Paths) -> Cx`
  - `Cx::db(&mut self) -> Result<&rusqlite::Connection>`

The connection is opened on first use so that `--help` and the stubs never leave a stray database file behind. That is why every command takes `&mut Cx`.

- [ ] **Step 1: Write the failing tests**

Create `src/context.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::Cx;
    use crate::paths::Paths;
    use std::path::Path;

    fn cx_in(dir: &Path) -> Cx {
        Cx::new(Paths::resolve_with(
            Some(dir.join("nested").join("pmanager.db")),
            Some(dir.join("strip")),
            Path::new("/unused"),
            Path::new("/unused"),
        ))
    }

    #[test]
    fn constructing_a_context_does_not_touch_the_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let cx = cx_in(tmp.path());
        assert!(!cx.paths.db.exists(), "database must not be created eagerly");
    }

    #[test]
    fn asking_for_the_database_creates_it_and_its_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cx = cx_in(tmp.path());

        cx.db().expect("opening the database should succeed");

        assert!(cx.paths.db.exists(), "database file should now exist");
    }

    #[test]
    fn the_connection_is_reused_across_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cx = cx_in(tmp.path());

        cx.db().unwrap().execute_batch("CREATE TABLE marker (id INTEGER)").unwrap();

        // A second call must hand back the same live connection, so the
        // temporary table created above is still visible.
        let count: i64 = cx
            .db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test context`
Expected: FAIL — `context` is not a module of the crate.

- [ ] **Step 3: Write the implementation above the test module**

```rust
//! `Cx` — the single value every command receives.

use anyhow::Context as _;
use rusqlite::Connection;

use crate::error::Result;
use crate::paths::Paths;

pub struct Cx {
    pub paths: Paths,
    db: Option<Connection>,
}

impl Cx {
    pub fn new(paths: Paths) -> Self {
        Self { paths, db: None }
    }

    /// The SQLite ledger, opened on first use.
    ///
    /// Opening is deferred so that `pman --help` and the unimplemented stubs
    /// never create a database as a side effect.
    pub fn db(&mut self) -> Result<&Connection> {
        if self.db.is_none() {
            if let Some(parent) = self.paths.db.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("could not create database directory {}", parent.display())
                })?;
            }

            let conn = Connection::open(&self.paths.db).with_context(|| {
                format!("could not open database {}", self.paths.db.display())
            })?;

            self.db = Some(conn);
        }

        Ok(self.db.as_ref().expect("connection was just stored"))
    }
}
```

- [ ] **Step 4: Declare the module in `src/lib.rs`**

```rust
pub mod cli;
pub mod context;
pub mod error;
pub mod paths;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS — 3 new unit tests in `context`.

- [ ] **Step 6: Commit**

```bash
cd ~/dev/rust/pManager
git add src/lib.rs src/context.rs
git commit -m "feat: add command context with lazily opened database" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: The `Command` trait, dispatch, and the first two commands

This task establishes the pattern every later command copies. `list` and `reload` are both argument-free, which keeps the first pass focused on the plumbing.

**Files:**
- Create: `src/commands/mod.rs`, `src/commands/list.rs`, `src/commands/reload.rs`
- Modify: `src/lib.rs` (declare the module; dispatch from `try_run`)
- Modify: `src/cli.rs` (add the `command` field)
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `Cx`, `Result`, `Cli`
- Produces:
  - `pmanager::commands::Command` — `fn run(&self, cx: &mut Cx) -> Result<()>`
  - `pmanager::commands::Commands` — the root `Subcommand` enum
  - `pmanager::commands::list::List`, `pmanager::commands::reload::Reload`
  - `Cli::command: Commands`

- [ ] **Step 1: Write the failing tests**

Append to `tests/cli.rs`:

```rust
#[test]
fn running_with_no_command_fails_and_shows_usage() {
    pman().assert().failure().stderr(contains("Usage"));
}

#[test]
fn list_reports_that_it_is_not_implemented() {
    pman()
        .arg("list")
        .assert()
        .failure()
        .stderr(contains("`pman list` is not implemented yet"));
}

#[test]
fn reload_reports_that_it_is_not_implemented() {
    pman()
        .arg("reload")
        .assert()
        .failure()
        .stderr(contains("`pman reload` is not implemented yet"));
}

#[test]
fn a_stub_does_not_create_a_database() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("pmanager.db");

    pman()
        .arg("--db")
        .arg(&db)
        .arg("list")
        .assert()
        .failure();

    assert!(!db.exists(), "a stub must not create the database");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL — `list` and `reload` are unrecognised arguments.

- [ ] **Step 3: Write `src/commands/mod.rs`**

```rust
//! Command dispatch.
//!
//! Adding a command is three edits: a new file here implementing [`Command`],
//! one variant on [`Commands`], and one arm in the match below. Forgetting the
//! arm is a compile error, which is the point.

pub mod list;
pub mod reload;

use clap::Subcommand;

use crate::context::Cx;
use crate::error::Result;

/// Every leaf command implements this. Group modules implement it too, by
/// delegating to their own inner enum.
pub trait Command {
    fn run(&self, cx: &mut Cx) -> Result<()>;
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List all registered projects
    List(list::List),

    /// Re-read strip profiles from disk
    Reload(reload::Reload),
}

impl Command for Commands {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match self {
            Commands::List(c) => c.run(cx),
            Commands::Reload(c) => c.run(cx),
        }
    }
}
```

- [ ] **Step 4: Write `src/commands/list.rs`**

```rust
use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct List {}

impl Command for List {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman list` is not implemented yet")
    }
}
```

- [ ] **Step 5: Write `src/commands/reload.rs`**

```rust
use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Reload {}

impl Command for Reload {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman reload` is not implemented yet")
    }
}
```

- [ ] **Step 6: Add the subcommand field to `src/cli.rs`**

Add the import and the field to `Cli`:

```rust
use crate::commands::Commands;
```

```rust
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Commands,
}
```

- [ ] **Step 7: Dispatch from `src/lib.rs`**

```rust
pub mod cli;
pub mod commands;
pub mod context;
pub mod error;
pub mod paths;

use std::process::ExitCode;

use clap::Parser;

use crate::commands::Command as _;

pub fn run() -> ExitCode {
    match try_run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error::report(&err);
            ExitCode::FAILURE
        }
    }
}

fn try_run() -> error::Result {
    let cli = cli::Cli::parse();
    cli::init_tracing(cli.global.verbose, cli.global.quiet);

    let paths = paths::Paths::resolve(cli.global.db.clone(), cli.global.strip_dir.clone())?;
    let mut cx = context::Cx::new(paths);

    cli.command.run(&mut cx)
}
```

- [ ] **Step 8: Add `tempfile` to the integration test's imports**

At the top of `tests/cli.rs` no import is needed — `tempfile::tempdir()` is called by full path in the test above. Confirm `tempfile` is present under `[dev-dependencies]` in `Cargo.toml`.

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS — including `cli_definition_is_valid`, which now validates the subcommand tree.

- [ ] **Step 10: Commit**

```bash
cd ~/dev/rust/pManager
git add src/lib.rs src/cli.rs src/commands tests/cli.rs
git commit -m "feat: add Command trait, dispatch, and list/reload stubs" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: `run` and `restore`

The first commands carrying arguments: a flag and a positional.

**Files:**
- Create: `src/commands/run.rs`, `src/commands/restore.rs`
- Modify: `src/commands/mod.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `Command`, `Cx`, `Result`
- Produces: `commands::run::Run { pub dry_run: bool }`, `commands::restore::Restore { pub name: String }`

- [ ] **Step 1: Write the failing tests**

Append to `tests/cli.rs`:

```rust
#[test]
fn run_reports_that_it_is_not_implemented() {
    pman()
        .arg("run")
        .assert()
        .failure()
        .stderr(contains("`pman run` is not implemented yet"));
}

#[test]
fn run_accepts_dry_run() {
    pman()
        .args(["run", "--dry-run"])
        .assert()
        .failure()
        .stderr(contains("not implemented yet"));
}

#[test]
fn restore_requires_a_name() {
    pman().arg("restore").assert().failure().stderr(contains("NAME"));
}

#[test]
fn restore_reports_that_it_is_not_implemented() {
    pman()
        .args(["restore", "my-project"])
        .assert()
        .failure()
        .stderr(contains("`pman restore` is not implemented yet"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL — `run` and `restore` are unrecognised.

- [ ] **Step 3: Write `src/commands/run.rs`**

```rust
use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Run {
    /// Report what would happen without stripping or archiving anything
    #[arg(long)]
    pub dry_run: bool,
}

impl Command for Run {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman run` is not implemented yet")
    }
}
```

- [ ] **Step 4: Write `src/commands/restore.rs`**

```rust
use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Restore {
    /// Name of the archived project to restore
    #[arg(value_name = "NAME")]
    pub name: String,
}

impl Command for Restore {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman restore` is not implemented yet")
    }
}
```

- [ ] **Step 5: Register both in `src/commands/mod.rs`**

Add to the module list:

```rust
pub mod list;
pub mod reload;
pub mod restore;
pub mod run;
```

Add to the enum:

```rust
    /// Evaluate every project and execute what is due
    Run(run::Run),

    /// Restore an archived project
    Restore(restore::Restore),
```

Add to the match:

```rust
            Commands::Run(c) => c.run(cx),
            Commands::Restore(c) => c.run(cx),
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
cd ~/dev/rust/pManager
git add src/commands tests/cli.rs
git commit -m "feat: add run and restore stubs" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: The `project` command group

Proves the pattern nests. `project/mod.rs` implements `Command` by delegating to its own enum, exactly as the root does.

**Files:**
- Create: `src/commands/project/mod.rs`, `add.rs`, `remove.rs`, `cleanup.rs`
- Modify: `src/commands/mod.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `Command`, `Cx`, `Result`
- Produces:
  - `commands::project::Project { pub command: ProjectCommand }`
  - `commands::project::ProjectCommand`
  - `commands::project::add::Add { pub path: PathBuf, pub name: Option<String> }`
  - `commands::project::remove::Remove { pub path: PathBuf }`
  - `commands::project::cleanup::Cleanup {}`

- [ ] **Step 1: Write the failing tests**

Append to `tests/cli.rs`:

```rust
#[test]
fn project_group_help_lists_its_subcommands() {
    pman()
        .args(["project", "--help"])
        .assert()
        .success()
        .stdout(contains("add"))
        .stdout(contains("remove"))
        .stdout(contains("cleanup"));
}

#[test]
fn project_without_a_subcommand_fails() {
    pman().arg("project").assert().failure();
}

#[test]
fn project_add_requires_a_path() {
    pman().args(["project", "add"]).assert().failure();
}

#[test]
fn project_add_reports_that_it_is_not_implemented() {
    pman()
        .args(["project", "add", "/tmp/somewhere"])
        .assert()
        .failure()
        .stderr(contains("`pman project add` is not implemented yet"));
}

#[test]
fn project_add_accepts_a_name_flag() {
    pman()
        .args(["project", "add", "/tmp/somewhere", "--name", "thing"])
        .assert()
        .failure()
        .stderr(contains("not implemented yet"));
}

#[test]
fn project_remove_reports_that_it_is_not_implemented() {
    pman()
        .args(["project", "remove", "/tmp/somewhere"])
        .assert()
        .failure()
        .stderr(contains("`pman project remove` is not implemented yet"));
}

#[test]
fn project_cleanup_reports_that_it_is_not_implemented() {
    pman()
        .args(["project", "cleanup"])
        .assert()
        .failure()
        .stderr(contains("`pman project cleanup` is not implemented yet"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL — `project` is unrecognised.

- [ ] **Step 3: Write `src/commands/project/mod.rs`**

```rust
//! The `project` group. Same shape as the root: an enum plus a match.

pub mod add;
pub mod cleanup;
pub mod remove;

use clap::{Args, Subcommand};

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Project {
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Subcommand, Debug)]
pub enum ProjectCommand {
    /// Register a project
    Add(add::Add),

    /// Unregister a project
    Remove(remove::Remove),

    /// Drop rows whose path no longer exists
    Cleanup(cleanup::Cleanup),
}

impl Command for Project {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match &self.command {
            ProjectCommand::Add(c) => c.run(cx),
            ProjectCommand::Remove(c) => c.run(cx),
            ProjectCommand::Cleanup(c) => c.run(cx),
        }
    }
}
```

- [ ] **Step 4: Write `src/commands/project/add.rs`**

The `--name` flag exists so `auto-run` and scripts never block on a terminal. When it is absent the real implementation will prompt with `dialoguer`; the stub does not prompt, because prompting and then refusing to work would be hostile.

```rust
use std::path::PathBuf;

use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Add {
    /// Path to the project directory
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Project name; prompted for interactively when omitted
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

impl Command for Add {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project add` is not implemented yet")
    }
}
```

- [ ] **Step 5: Write `src/commands/project/remove.rs`**

```rust
use std::path::PathBuf;

use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Remove {
    /// Path of the project to unregister
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

impl Command for Remove {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project remove` is not implemented yet")
    }
}
```

- [ ] **Step 6: Write `src/commands/project/cleanup.rs`**

```rust
use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Cleanup {}

impl Command for Cleanup {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project cleanup` is not implemented yet")
    }
}
```

- [ ] **Step 7: Register the group in `src/commands/mod.rs`**

Add `pub mod project;` to the module list, then the variant:

```rust
    /// Manage project registrations
    Project(project::Project),
```

and the arm:

```rust
            Commands::Project(c) => c.run(cx),
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
cd ~/dev/rust/pManager
git add src/commands tests/cli.rs
git commit -m "feat: add project command group" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: The `auto-run` group and the scheduler abstraction

**Files:**
- Create: `src/commands/auto_run/mod.rs`, `add.rs`, `remove.rs`
- Create: `src/scheduler/mod.rs`, `src/scheduler/systemd.rs`, `src/scheduler/windows.rs`
- Modify: `src/commands/mod.rs`, `src/lib.rs`
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `Command`, `Cx`, `Result`
- Produces:
  - `commands::auto_run::AutoRun { pub command: AutoRunCommand }`
  - `pmanager::scheduler::Scheduler` — `fn install(&self) -> Result<()>`, `fn remove(&self) -> Result<()>`
  - `pmanager::scheduler::for_this_platform() -> Result<Box<dyn Scheduler>>`

The clap derive turns the `AutoRun` variant into the subcommand `auto-run` automatically; no rename attribute is needed.

**`src/scheduler/windows.rs` is `#[cfg(windows)]`, so it is never compiled on this Linux machine and `cargo test` cannot type-check it.** Keep it trivial. Anything non-trivial there must wait for a Windows build or a `--target` cross-check.

- [ ] **Step 1: Write the failing tests**

Append to `tests/cli.rs`:

```rust
#[test]
fn auto_run_group_is_spelled_with_a_hyphen() {
    pman()
        .args(["auto-run", "--help"])
        .assert()
        .success()
        .stdout(contains("add"))
        .stdout(contains("remove"));
}

#[test]
fn auto_run_add_reports_that_it_is_not_implemented() {
    pman()
        .args(["auto-run", "add"])
        .assert()
        .failure()
        .stderr(contains("`pman auto-run add` is not implemented yet"));
}

#[test]
fn auto_run_remove_reports_that_it_is_not_implemented() {
    pman()
        .args(["auto-run", "remove"])
        .assert()
        .failure()
        .stderr(contains("`pman auto-run remove` is not implemented yet"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL — `auto-run` is unrecognised.

- [ ] **Step 3: Write `src/scheduler/mod.rs`**

```rust
//! Installing pManager's periodic `run` into the host's scheduler.
//!
//! The implementations are deliberately empty: scheduling is specified
//! separately. The trait and the platform split exist now so that the
//! `auto-run` commands have a stable thing to call.

#[cfg(unix)]
pub mod systemd;
#[cfg(windows)]
pub mod windows;

use crate::error::Result;

pub trait Scheduler {
    /// Install the recurring `pman run`.
    fn install(&self) -> Result<()>;

    /// Remove it again.
    fn remove(&self) -> Result<()>;
}

/// The scheduler for the platform this binary was built for.
pub fn for_this_platform() -> Result<Box<dyn Scheduler>> {
    #[cfg(unix)]
    {
        Ok(Box::new(systemd::SystemdUserTimer))
    }

    #[cfg(windows)]
    {
        Ok(Box::new(windows::TaskScheduler))
    }

    #[cfg(not(any(unix, windows)))]
    {
        anyhow::bail!("no scheduler is available for this platform")
    }
}
```

- [ ] **Step 4: Write `src/scheduler/systemd.rs`**

```rust
use anyhow::bail;

use super::Scheduler;
use crate::error::Result;

/// A `pman-run.timer` under `~/.config/systemd/user/`.
pub struct SystemdUserTimer;

impl Scheduler for SystemdUserTimer {
    fn install(&self) -> Result<()> {
        bail!("the systemd scheduler is not implemented yet")
    }

    fn remove(&self) -> Result<()> {
        bail!("the systemd scheduler is not implemented yet")
    }
}
```

- [ ] **Step 5: Write `src/scheduler/windows.rs`**

```rust
use anyhow::bail;

use super::Scheduler;
use crate::error::Result;

/// A scheduled task registered through `schtasks`.
pub struct TaskScheduler;

impl Scheduler for TaskScheduler {
    fn install(&self) -> Result<()> {
        bail!("the Windows Task Scheduler backend is not implemented yet")
    }

    fn remove(&self) -> Result<()> {
        bail!("the Windows Task Scheduler backend is not implemented yet")
    }
}
```

- [ ] **Step 6: Write `src/commands/auto_run/mod.rs`**

```rust
//! The `auto-run` group. clap derives the hyphenated name from `AutoRun`.

pub mod add;
pub mod remove;

use clap::{Args, Subcommand};

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct AutoRun {
    #[command(subcommand)]
    pub command: AutoRunCommand,
}

#[derive(Subcommand, Debug)]
pub enum AutoRunCommand {
    /// Install the scheduled background run
    Add(add::Add),

    /// Remove the scheduled background run
    Remove(remove::Remove),
}

impl Command for AutoRun {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match &self.command {
            AutoRunCommand::Add(c) => c.run(cx),
            AutoRunCommand::Remove(c) => c.run(cx),
        }
    }
}
```

- [ ] **Step 7: Write `src/commands/auto_run/add.rs`**

```rust
use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Add {}

impl Command for Add {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman auto-run add` is not implemented yet")
    }
}
```

- [ ] **Step 8: Write `src/commands/auto_run/remove.rs`**

```rust
use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Remove {}

impl Command for Remove {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman auto-run remove` is not implemented yet")
    }
}
```

- [ ] **Step 9: Register in `src/commands/mod.rs` and `src/lib.rs`**

In `src/commands/mod.rs` add `pub mod auto_run;` to the module list, then the variant:

```rust
    /// Manage the scheduled background run
    AutoRun(auto_run::AutoRun),
```

and the arm:

```rust
            Commands::AutoRun(c) => c.run(cx),
```

In `src/lib.rs` add `pub mod scheduler;` to the module block.

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cd ~/dev/rust/pManager && cargo test`
Expected: PASS

- [ ] **Step 11: Commit**

```bash
cd ~/dev/rust/pManager
git add src/commands src/lib.rs src/scheduler tests/cli.rs
git commit -m "feat: add auto-run group and scheduler abstraction" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 8: Surface lock-in and the contributor recipe

Two guards against the surface silently drifting, plus the document that makes the pattern discoverable.

**Files:**
- Modify: `tests/cli.rs`
- Create: `docs/adding-a-command.md`

**Interfaces:**
- Consumes: everything above.
- Produces: no code interfaces.

- [ ] **Step 1: Write the failing test**

Append to `tests/cli.rs`:

```rust
/// Every command in the spec, with the arguments needed to reach its body.
/// A command that is renamed or dropped by accident fails here.
const SURFACE: &[&[&str]] = &[
    &["list"],
    &["run"],
    &["restore", "a-name"],
    &["reload"],
    &["project", "add", "/tmp/x"],
    &["project", "remove", "/tmp/x"],
    &["project", "cleanup"],
    &["auto-run", "add"],
    &["auto-run", "remove"],
];

#[test]
fn every_documented_command_is_reachable_and_honest() {
    for args in SURFACE {
        pman()
            .args(*args)
            .assert()
            .failure()
            .stderr(contains("is not implemented yet"));
    }
}

#[test]
fn help_renders_at_every_level_of_the_tree() {
    for args in [
        vec!["--help"],
        vec!["project", "--help"],
        vec!["project", "add", "--help"],
        vec!["auto-run", "--help"],
        vec!["auto-run", "add", "--help"],
        vec!["run", "--help"],
        vec!["restore", "--help"],
    ] {
        pman().args(&args).assert().success();
    }
}
```

- [ ] **Step 2: Run the tests to verify they pass**

These should pass immediately — Tasks 4 to 7 already built the surface. This test exists to *lock it in*, so a green first run is the expected result, not a mistake.

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: PASS

- [ ] **Step 3: Prove the surface test actually bites**

Temporarily rename the `Reload` variant's command by adding `#[command(name = "refresh")]` above `Reload(reload::Reload)` in `src/commands/mod.rs`, then:

Run: `cd ~/dev/rust/pManager && cargo test --test cli`
Expected: FAIL on `every_documented_command_is_reachable_and_honest`.

Revert the attribute and rerun to confirm PASS. A test that cannot fail is not a test.

- [ ] **Step 4: Write `docs/adding-a-command.md`**

````markdown
# Adding a command

Three edits. The compiler catches you if you skip the third.

## 1. Create the file

`src/commands/vacuum.rs`:

```rust
use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Vacuum {
    /// Compact the database file after pruning
    #[arg(long)]
    pub compact: bool,
}

impl Command for Vacuum {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let db = cx.db()?;
        // ...
        bail!("`pman vacuum` is not implemented yet")
    }
}
```

## 2. Add the variant

In `src/commands/mod.rs`, add `pub mod vacuum;` and:

```rust
    /// Prune and compact the database
    Vacuum(vacuum::Vacuum),
```

The doc comment becomes the command's help text. The variant name becomes the
command name, lower-kebab-cased: `AutoRun` is invoked as `auto-run`.

## 3. Add the dispatch arm

```rust
            Commands::Vacuum(c) => c.run(cx),
```

Omit this and the build fails with a non-exhaustive match. That is deliberate:
a missing arm should never become a command that silently does nothing.

## Adding a command inside a group

Identical, one level down. `src/commands/project/mod.rs` owns its own
`ProjectCommand` enum and its own match; a new `project` subcommand is the same
three edits against those.

## Adding a whole group

Copy `src/commands/auto_run/mod.rs`. A group is an `Args` struct holding
`#[command(subcommand)]`, an inner `Subcommand` enum, and an `impl Command` that
matches over it. Then register the group at the root like any other command.

## Before you commit

```bash
cargo test          # includes Cli::command().debug_assert()
cargo deny check    # the licence allowlist is deliberately narrow
```

Add your command to the `SURFACE` table in `tests/cli.rs` so a later rename
cannot quietly drop it.
````

- [ ] **Step 5: Add the new command to the surface table**

The doc instructs contributors to extend `SURFACE`. No change is needed now — `vacuum` is an example, not a real command. Confirm `SURFACE` lists exactly the nine commands from the spec.

- [ ] **Step 6: Final verification**

```bash
cd ~/dev/rust/pManager
cargo test
cargo clippy --all-targets -- -D warnings
cargo deny check
cargo run -- --help
```

Expected: tests pass, clippy clean, `advisories ok, bans ok, licenses ok, sources ok`, and the help output lists `list`, `project`, `run`, `auto-run`, `restore`, `reload`.

- [ ] **Step 7: Commit**

```bash
cd ~/dev/rust/pManager
git add tests/cli.rs docs/adding-a-command.md
git commit -m "test: lock in the command surface; document the pattern" \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Deferred to later specs

Database schema and migrations; project-type detection; the 60/120/160-day
calculation; `.pmanagerignore` handling; strip-profile format and parsing; 7z
pack and unpack; the `dialoguer` name prompt in `project add`; real systemd
timer and Task Scheduler implementations.
