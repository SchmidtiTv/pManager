# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `pman project status <PATH>` shows a project's state, last change, idle
  days, git dirtiness and the next action with its date, measured live like
  `pman run`.
- `pman project restore <PATH>` unpacks an archived project to its original
  path, verifies it, marks it `registered` and deletes the `.pproject`.
  Unpacked files get the current time, so the next run does not archive the
  project again straight away.
- JSON schemas for the `-o json` output of `list`, `project status` and
  `project restore` in `docs/schemas/`.
- Exit code `3` for a path that is not registered.

### Changed

- `status`, `restore` and `remove` find a project by its original path even
  after the folder is gone, whatever the spelling of its parent folder (`/`,
  letter case, Windows 8.3 short names).
- `pman project remove` on an unregistered path exits with `3` instead of `1`.

### Fixed

- A restore whose `.pproject` cannot be deleted at the end no longer fails.
  The project is restored, the command exits with `0` and prints its JSON,
  and a warning names the archive to delete by hand.

## [0.1.0] - 2026-10-20

### Added

- Project ledger in SQLite with the states `registered`, `stripped` and
  `archived`.
- `pman project add`, `pman project remove` and `pman project cleanup` to
  manage registrations.
- `pman list` to show every registered project and its idle days.
- `pman run [--dry-run]` to strip projects idle for 60 days and archive them
  to a 7z `<name>.pproject` after 120 days, or 160 days with uncommitted git
  changes.
- Strip profiles in `strip/` next to the executable, with starter profiles
  for C++/CMake, C#, Java (Gradle and Maven), JavaScript, Python and Rust.
- `.pmanagerignore` for files that do not count as a change.
- `pman auto-run add` and `pman auto-run remove` to schedule the background
  run on Windows, macOS (launchd) and Linux (systemd).
- `pman migrate-v1` to import a pManager v1 database.
- Global options `-v`, `-q`, `--db`, `--strip-dir` (or `PMANAGER_STRIP_DIR`)
  and `-o table|json`.
- The CLI ships as both `pman` and `pmanager`, with an embedded icon on
  Windows.

[unreleased]: https://github.com/SchmidtiTv/pManager/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SchmidtiTv/pManager/releases/tag/v0.1.0
