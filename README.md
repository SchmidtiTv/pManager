# pManager

## Why

Local projects pile up, and most of their size is not source code but
re-creatable stuff like `node_modules`, `.venv` or build output. Projects you
have not touched in months still eat that space, and cleaning them up by hand
is tedious and easy to forget. pManager does it automatically: dormant
projects are stripped first, then archived, without losing anything you
cannot get back.

## What it does

Ages out dormant local projects to reclaim disk space. Rust CLI, shipped as
`pmanager` and `pman`, with a local SQLite database holding every registered
project, its state, path and (once archived) archive path.

## Lifecycle

Days are counted since the last change in the project folder.

| Age | Action |
| --- | --- |
| 60 days | Strip: delete dependency/build folders defined by the matching strip profile (e.g. `node_modules`, `.venv`) |
| 120 days | Archive: pack the folder into a 7z `<name>.pproject` next to it and remove the folder |
| 160 days | Archive projects with uncommitted git changes (held back until then) |

A `.pmanagerignore` in a project lists files that do not count as a change.
The project type is decided by the strip profiles at strip time, not at
registration.

## Commands

| Command | Description |
| --- | --- |
| `pman list` | List all registered projects |
| `pman project add <PATH> [--name <NAME>]` | Register a project (name is prompted if omitted) |
| `pman project remove <PATH>` | Unregister a project; files and archives stay on disk |
| `pman project cleanup` | Drop entries whose folder no longer exists |
| `pman run [--dry-run]` | Strip/archive every project that is due |
| `pman auto-run add` | Install the scheduled background run |
| `pman auto-run remove` | Remove the scheduled background run |
| `pman migrate-v1` | Import projects from a pManager v1 database |

Global options: `-v`/`-vv`, `-q`, `--db <PATH>`, `--strip-dir <PATH>`
(or `PMANAGER_STRIP_DIR`), `-o table|json`.

## Strip profiles

Profiles live in a `strip` folder next to the executable:

```
pmanager.exe
strip/
  profiles.json          index of all profiles
  python.profile.json    one profile per language
```

`profiles.json`:

```json
[
  { "language": "python", "file": "./python.profile.json" }
]
```

`python.profile.json`:

```json
{
  "language": "python",
  "detection": [
    { "type": "file", "element": "pyproject.toml" },
    { "type": "folder", "element": ".venv" }
  ],
  "strippable": [
    { "type": "folder", "element": ".venv" },
    { "type": "file", "element": "uv.lock" }
  ]
}
```

## Development

```sh
cargo build --release
cargo test
cargo clippy --all-targets
cargo fmt --check
```

See `docs/adding-a-command.md` for adding a subcommand.
