# vManager

A command line application written in Rust.

The repository currently contains the CLI foundation — argument parsing,
configuration, logging, error handling and exit codes — with the domain
commands still to be written.

## Building

```sh
cargo build --release        # binary at target/release/vmanager
cargo run -- --help          # run it straight from the source tree
cargo install --path .       # install into ~/.cargo/bin
```

Requires Rust 1.85 or newer (edition 2024).

## Usage

```
vmanager [OPTIONS] <COMMAND>
```

### Commands

| Command | Description |
| --- | --- |
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
| `130` | Interrupted |

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
  config.rs           config file discovery, parsing and defaults
  error.rs            error type and its exit-code mapping
  logging.rs          tracing subscriber setup (stderr)
  commands/
    mod.rs            shared Context handed to every subcommand
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
```
