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

`cx.db()` takes `&self` and hands back `&SqlLite` out of a `OnceCell`, so
reading `cx.paths` or `cx.view` afterwards is fine — `src/commands/list.rs`
queries the database and then uses `cx.view` in the next statement.

Note the trait import: a top-level file like this one sits directly inside
`src/commands/`, so `super` is `crate::commands` and `use super::Command;`
resolves. A file inside a group directory, such as
`src/commands/project/add.rs`, is one level deeper — there, `super` is the
group module (`crate::commands::project`), which does not define `Command`
at all. Those files must spell it out as `use crate::commands::Command;`
instead. Using `super::Command` in a group leaf is a compile error; it is a
common enough slip that it is worth knowing about before you hit it.

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
three edits against those. The only difference is the trait import described
above: use `use crate::commands::Command;`, not `use super::Command;`.

## Adding a whole group

Copy `src/commands/auto_run/mod.rs`. A group is an `Args` struct holding
`#[command(subcommand)]`, an inner `Subcommand` enum, and an `impl Command` that
matches over it. Then register the group at the root like any other command.

## Before you commit

```bash
cargo test                                  # includes Cli::command().debug_assert()
cargo fmt --check                           # formatting must be clean before review
cargo clippy --all-targets -- -D warnings   # no warnings allowed
cargo deny check                            # the licence allowlist is deliberately narrow
```

Add your command to the `SURFACE` table in `tests/cli.rs` so a later rename
cannot quietly drop it:

```rust
    (&["vacuum"], "`pman vacuum` is not implemented yet"),
```

Assert the *full* `pman <path>` message, not just a fragment. Every stub ends
in "is not implemented yet", so a check for that suffix alone passes no
matter which stub actually ran — it cannot tell `vacuum`'s stub from
`list`'s, and it would stay green even if `vacuum` were accidentally wired to
someone else's dispatch arm. The full message is the only part that names
which command actually answered.
