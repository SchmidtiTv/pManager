//! End-to-end tests driving the built binary the way a user would.
//!
//! Every test passes an explicit `--config` inside a temporary directory, so
//! the suite never reads or writes the real `~/.config/pmanager`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use assert_cmd::Command;
use predicates::prelude::*;

/// A `pmanager` invocation with a clean environment.
fn pmanager() -> Command {
    let mut cmd = Command::cargo_bin("pmanager").expect("binary is built");
    cmd.env_remove("PMANAGER_CONFIG")
        .env_remove("RUST_LOG")
        .env("NO_COLOR", "1");
    cmd
}

/// A temporary home with its own config file and database.
fn isolated() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    // A TOML *literal* string (single quotes) — a basic string would treat the
    // backslashes in a Windows path as escape sequences and fail to parse.
    std::fs::write(
        &config,
        format!("data_dir = '{}'\n", dir.path().join("data").display()),
    )
    .unwrap();
    (dir, config)
}

/// A Node project last edited `idle_days` ago, with a dependency tree.
fn project(root: &Path, name: &str, idle_days: u64) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(
        path.join("package.json"),
        format!(r#"{{"name": "{name}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    std::fs::write(path.join("src/index.js"), "console.log('hi');").unwrap();

    // Dependencies are installed today; they must not count as activity.
    let dependency = path.join("node_modules/left-pad");
    std::fs::create_dir_all(&dependency).unwrap();
    std::fs::write(dependency.join("index.js"), "x".repeat(4096)).unwrap();

    let when = SystemTime::now() - Duration::from_secs(idle_days * 86_400);
    for file in ["package.json", "src/index.js"] {
        std::fs::File::options()
            .write(true)
            .open(path.join(file))
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    path
}

/// Track `path` and assert it worked.
fn track(config: &Path, path: &Path) {
    pmanager()
        .args(["project", "add"])
        .arg(path)
        .args(["--config"])
        .arg(config)
        .assert()
        .success();
}

// ── Command line surface ─────────────────────────────────────────────────────

#[test]
fn prints_version() {
    pmanager()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn prints_help_with_exit_code_zero() {
    pmanager()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("Exit codes:"))
        .stdout(predicate::str::contains("How a project ages:"));
}

#[test]
fn bare_invocation_shows_help() {
    pmanager()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn unknown_flag_is_a_usage_error() {
    pmanager().arg("--definitely-not-a-flag").assert().code(2);
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    pmanager().arg("frobnicate").assert().code(2);
}

// ── Completions ──────────────────────────────────────────────────────────────

#[test]
fn completion_scripts_are_written_to_stdout() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        pmanager()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("pmanager"));
    }
}

#[test]
fn the_completion_script_covers_the_subcommands() {
    pmanager()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("restore"))
        .stdout(predicate::str::contains("schedule"))
        .stdout(predicate::str::contains("--config"));
}

#[test]
fn completions_do_not_need_a_usable_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "this is not: valid toml\n").unwrap();

    // A shell profile runs this at startup; a broken config must not break the
    // shell, and `list` next door proves the file really is unusable.
    pmanager()
        .args(["completions", "bash", "--config"])
        .arg(&config)
        .assert()
        .success();

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .code(3);
}

#[test]
fn an_unknown_shell_is_a_usage_error() {
    pmanager().args(["completions", "tcsh"]).assert().code(2);
}

#[test]
fn the_shell_can_be_left_to_the_environment() {
    pmanager()
        .arg("completions")
        .env("SHELL", "/bin/bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("pmanager"));
}

// ── Configuration ────────────────────────────────────────────────────────────

#[test]
fn the_config_environment_variable_is_honoured() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("from-env.toml");
    std::fs::write(&path, "trim_days = 90\narchive_days = 30\n").unwrap();

    // A file only PMANAGER_CONFIG points at still has to be read: the run
    // rejects the inverted thresholds rather than using the defaults.
    pmanager()
        .env("PMANAGER_CONFIG", &path)
        .arg("run")
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least `trim_days`"));
}

#[test]
fn invalid_config_exits_with_the_config_code() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "trim_days = \"not a number\"\n").unwrap();

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .code(3);
}

#[test]
fn an_unknown_config_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "trm_days = 30\n").unwrap();

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .code(3)
        .stderr(predicate::str::contains("trm_days"));
}

#[test]
fn logs_go_to_stderr_not_stdout() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "api", 1));

    pmanager()
        .args(["-vv", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("DEBUG").not());
}

// ── Tracking ─────────────────────────────────────────────────────────────────

#[test]
fn nothing_is_tracked_to_begin_with() {
    let (_dir, config) = isolated();

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no projects tracked yet"));
}

#[test]
fn a_project_is_named_by_its_manifest_and_reported_on_when_tracked() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "folder-name", 1);
    std::fs::write(path.join("package.json"), r#"{"name": "@acme/api-server"}"#).unwrap();

    pmanager()
        .args(["project", "add"])
        .arg(&path)
        .args(["--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("tracking @acme/api-server"))
        .stdout(predicate::str::contains("to reclaim once it goes stale"));
}

#[test]
fn tracking_the_same_project_twice_updates_rather_than_duplicates() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 1);

    track(&config, &path);
    pmanager()
        .args(["project", "add"])
        .arg(&path)
        .args(["--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("updated api"));

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 project(s)"));
}

#[test]
fn tracking_something_that_is_not_there_is_not_found() {
    let (dir, config) = isolated();

    pmanager()
        .args(["project", "add"])
        .arg(dir.path().join("no-such-project"))
        .args(["--config"])
        .arg(&config)
        .assert()
        .code(4);
}

#[test]
fn an_untracked_project_cannot_be_inspected() {
    let (_dir, config) = isolated();

    pmanager()
        .args(["info", "ghost", "--config"])
        .arg(&config)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("not found"));
}

// ── The sweep ────────────────────────────────────────────────────────────────

#[test]
fn a_run_without_yes_changes_nothing() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 90);
    track(&config, &path);

    pmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("would trim"))
        .stdout(predicate::str::contains("pass --yes to apply"));

    assert!(
        path.join("node_modules").is_dir(),
        "a bare run must not delete anything"
    );
}

#[test]
fn a_stale_project_has_its_dependencies_trimmed() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 90);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("trimmed 1 dependency tree(s)"));

    assert!(
        !path.join("node_modules").exists(),
        "dependencies should go"
    );
    assert!(
        path.join("src/index.js").is_file(),
        "the project should stay"
    );

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("trimmed"));
}

#[test]
fn a_project_is_trimmed_by_the_toolchain_it_is_built_with() {
    let (dir, config) = isolated();

    // A Rust project: no package.json anywhere, so the only reason `target`
    // goes is that `Cargo.toml` is sitting beside it.
    let path = dir.path().join("cli");
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("Cargo.toml"), "[package]\nname = \"cli\"\n").unwrap();
    std::fs::write(path.join("src/main.rs"), "fn main() {}").unwrap();

    let build = path.join("target/debug");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("cli"), "x".repeat(8192)).unwrap();

    let when = SystemTime::now() - Duration::from_secs(90 * 86_400);
    for file in ["Cargo.toml", "src/main.rs"] {
        std::fs::File::options()
            .write(true)
            .open(path.join(file))
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    track(&config, &path);

    pmanager()
        .args(["info"])
        .arg(&path)
        .args(["--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("Rust"));

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("trimmed 1 dependency tree(s)"));

    assert!(
        !path.join("target").exists(),
        "the build directory should go"
    );
    assert!(
        path.join("src/main.rs").is_file(),
        "the project should stay"
    );
}

#[test]
fn a_project_of_no_known_toolchain_is_never_trimmed() {
    let (dir, config) = isolated();

    // A `target` with nothing to rebuild it from is somebody's own directory.
    let path = dir.path().join("notes");
    std::fs::create_dir_all(path.join("target")).unwrap();
    std::fs::write(path.join("target/keep.txt"), "hand written").unwrap();
    std::fs::write(path.join("readme.md"), "no manifest here").unwrap();

    let when = SystemTime::now() - Duration::from_secs(90 * 86_400);
    for file in ["readme.md", "target/keep.txt"] {
        std::fs::File::options()
            .write(true)
            .open(path.join(file))
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success();

    assert!(
        path.join("target/keep.txt").is_file(),
        "pManager cannot rebuild it, so it must not delete it"
    );
}

#[test]
fn a_lapse_threshold_below_the_archive_one_is_refused() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 400);
    track(&config, &path);

    pmanager()
        .args(["run", "--force-archive-days", "10", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force-archive-days"));

    assert!(path.join("src/index.js").is_file(), "nothing was touched");
}

#[test]
fn a_second_run_has_nothing_left_to_trim() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "api", 90));

    for _ in 0..2 {
        pmanager()
            .args(["run", "--yes", "--config"])
            .arg(&config)
            .assert()
            .success();
    }

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("0 trimmed, 0 archived"));
}

#[test]
fn a_recently_edited_project_is_left_alone() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 2);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("0 trimmed, 0 archived"));

    assert!(path.join("node_modules").is_dir());
}

#[test]
fn installing_dependencies_does_not_look_like_working_on_a_project() {
    let (dir, config) = isolated();
    // `project` writes node_modules today and backdates only the sources,
    // which is exactly the shape of a fresh `npm install` on a dead project.
    let path = project(dir.path(), "api", 200);
    track(&config, &path);

    pmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("would archive"));
}

#[test]
fn thresholds_can_be_overridden_for_one_run() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 10);
    track(&config, &path);

    // Ten days idle is nothing by default, but everything at --trim-days 5.
    pmanager()
        .args(["run", "--yes", "--trim-days", "5", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("trimmed 1 dependency tree(s)"));

    assert!(!path.join("node_modules").exists());
}

#[test]
fn inverted_thresholds_are_a_usage_error_before_anything_is_touched() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 400);
    track(&config, &path);

    pmanager()
        .args([
            "run",
            "--yes",
            "--trim-days",
            "90",
            "--archive-days",
            "30",
            "--config",
        ])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be at least --trim-days"));

    assert!(path.is_dir(), "nothing should have happened");
}

#[test]
fn no_archive_stops_at_trimming() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 400);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--no-archive", "--config"])
        .arg(&config)
        .assert()
        .success();

    assert!(path.is_dir(), "the project should still be there");
    assert!(!path.with_extension("pArchived").exists());
}

// ── Archiving and restoring ──────────────────────────────────────────────────

#[test]
fn a_long_abandoned_project_is_archived_and_can_be_restored() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "dashboard", 400);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("archived to"));

    let archive = dir.path().join("dashboard.pArchived");
    assert!(
        archive.is_file(),
        "the archive should be beside the project"
    );
    assert!(!path.exists(), "the folder should be gone once verified");

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("archived"));

    // …and back again. `--no-install` keeps npm out of the test.
    pmanager()
        .args(["restore", "dashboard", "--no-install", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("is active again"));

    assert!(
        path.join("src/index.js").is_file(),
        "the source should be back"
    );
    assert!(
        !path.join("node_modules").exists(),
        "dependencies are rebuilt, not archived"
    );
    assert!(
        archive.is_file(),
        "the archive is kept unless asked otherwise"
    );
}

#[test]
fn a_restored_project_is_not_archived_straight_back() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "dashboard", 400));

    for arguments in [
        vec!["run", "--yes"],
        vec!["restore", "dashboard", "--no-install"],
    ] {
        pmanager()
            .args(&arguments)
            .args(["--config"])
            .arg(&config)
            .assert()
            .success();
    }

    // Extraction preserves modification times, so the files are still 400 days
    // old. The restore is what the idle clock now runs from.
    pmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("would archive").not());

    assert!(dir.path().join("dashboard").is_dir());
}

#[test]
fn keep_source_writes_the_archive_but_leaves_the_project() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "dashboard", 400);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--keep-source", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("folder kept"));

    assert!(dir.path().join("dashboard.pArchived").is_file());
    assert!(path.is_dir());
}

#[test]
fn archives_can_be_collected_in_one_directory() {
    let (dir, config) = isolated();
    let vault = dir.path().join("vault");
    track(&config, &project(dir.path(), "dashboard", 400));

    pmanager()
        .args(["run", "--yes", "--archive-dir"])
        .arg(&vault)
        .args(["--config"])
        .arg(&config)
        .assert()
        .success();

    assert!(vault.join("dashboard.pArchived").is_file());
    assert!(!dir.path().join("dashboard.pArchived").exists());
}

#[test]
fn restoring_a_live_project_is_refused() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "api", 1));

    pmanager()
        .args(["restore", "api", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not archived"));
}

#[test]
fn restoring_over_an_existing_folder_is_refused() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "dashboard", 400);
    track(&config, &path);

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success();

    // Something else has taken the name in the meantime.
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/other.js"), "not the archived project").unwrap();

    pmanager()
        .args(["restore", "dashboard", "--no-install", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(
        std::fs::read_to_string(path.join("src/other.js")).unwrap(),
        "not the archived project",
        "the folder in the way must be left alone"
    );
}

// ── Reporting ────────────────────────────────────────────────────────────────

#[test]
fn info_says_what_the_next_run_would_do() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "api", 90));

    pmanager()
        .args(["info", "api", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("state"))
        .stdout(predicate::str::contains("last edit"))
        .stdout(predicate::str::contains("dependencies"))
        .stdout(predicate::str::contains("next run"))
        .stdout(predicate::str::contains("trim 1 dependency tree(s)"));
}

#[test]
fn list_filters_by_state() {
    let (dir, config) = isolated();
    track(&config, &project(dir.path(), "fresh", 1));
    track(&config, &project(dir.path(), "stale", 90));

    pmanager()
        .args(["run", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success();

    pmanager()
        .args(["list", "--state", "trimmed", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("stale"))
        .stdout(predicate::str::contains("fresh").not());
}

#[test]
fn quiet_suppresses_progress_output() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 90);

    pmanager()
        .args(["project", "add"])
        .arg(&path)
        .args(["--quiet", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    pmanager()
        .args(["run", "--yes", "--quiet", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    assert!(!path.join("node_modules").exists(), "it should still act");
}

// ── Untracking ───────────────────────────────────────────────────────────────

#[test]
fn untracking_leaves_the_project_on_disk() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "api", 90);
    track(&config, &path);

    pmanager()
        .args(["project", "remove", "api", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("untracked api"));

    assert!(
        path.join("node_modules").is_dir(),
        "nothing should be touched"
    );

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no projects tracked yet"));
}

#[test]
fn a_vanished_project_is_reported_and_then_pruned() {
    let (dir, config) = isolated();
    let path = project(dir.path(), "moved", 1);
    track(&config, &path);
    std::fs::remove_dir_all(&path).unwrap();

    pmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("folder is gone"));

    // Reporting only, until asked.
    pmanager()
        .args(["project", "prune", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("pass --yes"));

    pmanager()
        .args(["project", "prune", "--yes", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("untracked 1 missing project(s)"));

    pmanager()
        .args(["list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no projects tracked yet"));
}

// ── Scheduling ───────────────────────────────────────────────────────────────

#[cfg(not(windows))]
#[test]
fn scheduling_off_windows_explains_itself() {
    let (_dir, config) = isolated();

    pmanager()
        .args(["schedule", "status", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Task Scheduler"));
}
