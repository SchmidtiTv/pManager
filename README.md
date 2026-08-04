# vManager

A command line application written in Rust.

It archives folders into `.7z` files and keeps a record of every archive in a
local SQLite database.

## Building

```sh
cargo build --release        # binary at target/release/vmanager
cargo run -- --help          # run it straight from the source tree
cargo install --path .       # install into ~/.cargo/bin
```

Requires Rust 1.93 or newer (edition 2024). SQLite is compiled in from source,
so there is nothing to install first — and no `7z` binary is needed either;
compression is pure Rust.

## Usage

```
vmanager [OPTIONS] <COMMAND>
```

### Commands

| Command | Description |
| --- | --- |
| `archive create <FOLDER>` | Compress a folder into a `.7z` archive |
| `archive list` | List archives recorded in the database |
| `archive forget <ID>` | Remove a record, leaving the archive file alone |
| `archive prune` | Drop records whose archive file is gone |
| `db path` | Print the database path (without creating it) |
| `db init` | Create the database and run pending migrations |
| `db status` | Show schema version and row counts |
| `config show` | Print the configuration actually in effect, as TOML |
| `config path` | Print the path of the configuration file |
| `config init` | Write a commented starter configuration file |
| `completions <SHELL>` | Print a shell completion script |

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
| `VMANAGER_CONFIG` | Path to the configuration file (`--config` wins) |
| `NO_COLOR` | Disables color when `--color` is left at `auto` |
| `RUST_LOG` | Fine-grained log filter, e.g. `vmanager=debug`; overrides `-v` |

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

## Archiving

```sh
vmanager archive create ~/photos                 # writes ./photos.7z
vmanager archive create ~/photos -o /backup/photos.7z
vmanager archive create ~/code -x .git -x target # skip by name, repeatable
vmanager archive create ~/big --level 9          # 0 fastest … 9 smallest
vmanager archive create ~/big --threads 8        # faster, marginally larger
```

Archives use LZMA2 in solid mode — the same defaults as 7-Zip itself — and are
readable by any 7-Zip, p7zip or Windows Explorer plugin. Entry names keep the
top-level folder (`photos/…`), matching `7z a photos.7z photos`.

Details worth knowing:

- Symlinks are skipped rather than followed, so an archive can never loop or
  silently inline a link's target.
- Empty directories are not stored.
- A partially written archive is deleted if compression fails, so a `.7z` on
  disk is always complete.
- The destination may not sit inside the folder being archived.
- `--force` is required to overwrite an existing archive.

Every successful run is recorded in the database (`--no-record` opts out). If
the database write fails the archive still succeeds — a warning is logged
instead, since the file on disk is the thing that matters.

```
$ vmanager archive list
 1  2026-08-04T11:35:41Z      4 files  294.2 KiB  /backup/photos.7z
```

## Database

A single SQLite file, by default `vmanager.db` inside the data directory
(`vmanager db path` prints it). SQLite is linked statically via `rusqlite`'s
bundled build, so there is no system package to install and no version skew
between machines.

The schema is versioned with SQLite's own `user_version` pragma and migrated on
open: each entry in `MIGRATIONS` (`src/db.rs`) runs inside a transaction, so an
interrupted upgrade leaves the database on its previous version rather than
half-migrated. A database written by a *newer* vmanager is refused rather than
downgraded. To add a table or column, append a migration — never edit one that
has shipped.

Connections open with WAL journaling, `foreign_keys = ON` and a 5 second busy
timeout, so a second `vmanager` waits its turn instead of failing.

## Configuration

`vmanager config init` writes a commented starter file to the platform
configuration directory:

| Platform | Path |
| --- | --- |
| Linux | `~/.config/vmanager/config.toml` |
| macOS | `~/Library/Application Support/vmanager/config.toml` |
| Windows | `%APPDATA%\vmanager\config.toml` |

Values are layered, lowest precedence first: built-in defaults, then the
configuration file, then environment variables, then command line flags. A
missing file is fine — the defaults apply — but an unreadable or malformed one
is an error. Unknown keys are rejected so typos surface immediately.

## Shell completions

```sh
vmanager completions bash > /etc/bash_completion.d/vmanager
vmanager completions zsh  > ~/.zfunc/_vmanager
vmanager completions fish > ~/.config/fish/completions/vmanager.fish
```

## Layout

```
src/
  main.rs             process entry point: parse, run, map errors to exit codes
  lib.rs              library crate; dispatches a parsed command
  cli.rs              clap definitions: global flags and the subcommand list
  archive.rs          7z creation: walking, filtering, compression
  db.rs               SQLite connection, migrations and queries
  config.rs           config file discovery, parsing and defaults
  error.rs            error type and its exit-code mapping
  logging.rs          tracing subscriber setup (stderr)
  commands/
    mod.rs            shared Context handed to every subcommand
    archive.rs        `vmanager archive`
    db.rs             `vmanager db`
    config.rs         `vmanager config`
    completions.rs    `vmanager completions`
tests/
  cli.rs              end-to-end tests that run the built binary
```

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

vManager itself has no license declared yet — that is a call for the repository
owner. Until one is chosen the crate is marked `publish = false` and cargo-deny
skips it (`private.ignore`); setting `license` in `Cargo.toml` afterwards lets
that setting go away.
