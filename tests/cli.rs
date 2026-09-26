use std::path::{Path, PathBuf};

use assert_cmd::Command;
use chrono::{Duration, Utc};
use pmanager::database::schema;
use pmanager::database::sql_lite::SqlLite;
use pmanager::models::project::{self, Project};
use pmanager::models::state::ProjectState;
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
        .stdout(contains("--strip-dir"))
        .stdout(contains("--output"));
}

#[test]
fn an_unknown_flag_is_rejected() {
    pman().arg("--definitely-not-a-flag").assert().failure();
}

#[test]
fn an_unknown_command_is_rejected() {
    pman().arg("bogus").assert().failure();
}

#[test]
fn help_mentions_the_strip_dir_env_var() {
    // clap renders `[env: PMANAGER_STRIP_DIR=]` in `--help` only when the
    // `env = "PMANAGER_STRIP_DIR"` attribute is actually present on the
    // field. Deleting that attribute breaks no other test, so this is the
    // one guarding it stays wired up.
    pman()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("PMANAGER_STRIP_DIR"));
}

#[test]
fn running_with_no_command_fails_and_shows_usage() {
    pman().assert().failure().stderr(contains("Usage"));
}

#[test]
fn list_on_an_empty_ledger_says_so_on_stderr_and_prints_no_rows() {
    let tmp = tempfile::tempdir().unwrap();

    pman()
        .args(["list", "--db"])
        .arg(tmp.path().join("pmanager.db"))
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(contains("no projects registered"));
}

#[test]
fn list_prints_a_registered_project() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("pmanager.db");
    seed(&db_path, "demo", "C:/dev/demo");

    pman()
        .args(["list", "--db"])
        .arg(&db_path)
        .assert()
        .success()
        .stdout(contains("NAME"))
        .stdout(contains("demo"))
        .stdout(contains("C:/dev/demo"))
        .stdout(contains("registered"));
}

#[test]
fn list_as_json_types_the_idle_days_as_a_number() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("pmanager.db");
    seed(&db_path, "demo", "C:/dev/demo");

    let output = pman()
        .args(["list", "--output", "json", "--db"])
        .arg(&db_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("stdout is JSON");

    assert_eq!(parsed[0]["name"], serde_json::json!("demo"));
    assert_eq!(parsed[0]["path"], serde_json::json!("C:/dev/demo"));
    assert_eq!(parsed[0]["idle"], serde_json::json!(5));
}

#[test]
fn list_as_json_on_an_empty_ledger_is_still_parseable() {
    let tmp = tempfile::tempdir().unwrap();

    pman()
        .args(["list", "--output", "json", "--db"])
        .arg(tmp.path().join("pmanager.db"))
        .assert()
        .success()
        .stdout(contains("[]"));
}

/// Rows are written through the library rather than the CLI, because
/// `pman project add` needs the path to exist and this one is made up.
fn seed(db_path: &std::path::Path, name: &str, path: &str) {
    let db = SqlLite::open(db_path).unwrap();
    schema::apply(&db).unwrap();

    let project = Project {
        name: name.to_string(),
        path: path.into(),
        state: ProjectState::Registered,
        has_git_repo: true,
        last_change_at: Utc::now() - Duration::days(5),
        registered_at: Utc::now() - Duration::days(30),
    };

    db.execute(&project::insert(&project).unwrap()).unwrap();
}

#[test]
fn a_failing_stub_writes_nothing_to_stdout() {
    // Log output -- including the error report -- must go to stderr only, so
    // that command output on stdout stays pipeable even when a command
    // fails.
    pman()
        .args(["project", "cleanup"])
        .assert()
        .failure()
        .stdout(predicates::str::is_empty());
}

#[test]
fn an_unrelated_rust_log_directive_does_not_silence_errors() {
    // Regression test: EnvFilter::try_from_default_env() has no default
    // directive, so a RUST_LOG that scopes some other crate (e.g.
    // `git2=debug`) used to replace the whole filter and swallow every
    // `tracing::error!` from pManager itself, leaving a bare `exit 1` with
    // no output at all. The filter must always carry an ERROR default.
    pman()
        .args(["project", "cleanup"])
        .env("RUST_LOG", "git2=debug")
        .assert()
        .failure()
        .stderr(contains("`pman project cleanup` is not implemented yet"));
}

#[test]
fn a_stub_does_not_create_a_database() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("pmanager.db");

    pman()
        .arg("--db")
        .arg(&db)
        .args(["project", "cleanup"])
        .assert()
        .failure();

    assert!(!db.exists(), "a stub must not create the database");
}

#[test]
fn project_group_help_lists_its_subcommands() {
    pman()
        .args(["project", "--help"])
        .assert()
        .success()
        .stdout(contains("Commands:"))
        .stdout(contains("add"))
        .stdout(contains("remove"))
        .stdout(contains("cleanup"));
}

#[test]
fn project_group_exposes_three_nested_subcommands() {
    for leaf in ["add", "remove", "cleanup"] {
        pman()
            .args(["project", leaf, "--help"])
            .assert()
            .success()
            .stdout(contains(format!("pman project {leaf}")));
    }
}

#[test]
fn project_without_a_subcommand_fails() {
    pman().arg("project").assert().failure();
}

#[test]
fn project_add_requires_a_path() {
    pman().args(["project", "add"]).assert().failure();
}

/// `pman` pointed at a ledger inside `tmp`, so no test touches the real one.
fn pman_in(tmp: &Path) -> Command {
    let mut command = pman();
    command.arg("--db").arg(tmp.join("pmanager.db"));
    command
}

fn project_dir(tmp: &Path, name: &str) -> PathBuf {
    let dir = tmp.join(name);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();
    dir
}

fn listed_as_json(tmp: &Path) -> serde_json::Value {
    let output = pman_in(tmp)
        .args(["list", "--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    serde_json::from_slice(&output).expect("stdout is JSON")
}

#[test]
fn an_added_project_shows_up_in_list() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");

    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&dir)
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(contains("registered `demo`"));

    let listed = listed_as_json(tmp.path());
    assert_eq!(listed[0]["name"], serde_json::json!("demo"));
    assert_eq!(listed[0]["state"], serde_json::json!("registered"));
    assert_eq!(
        listed[0]["idle"],
        serde_json::json!(0),
        "a file was just written, so the project is not idle"
    );
}

#[test]
fn a_path_is_stored_in_one_normalised_form() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");

    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(dir.join("..").join("demo").join("."))
        .assert()
        .success();

    let listed = listed_as_json(tmp.path());
    let stored = listed[0]["path"].as_str().unwrap();
    assert_eq!(
        Path::new(stored),
        std::fs::canonicalize(&dir).unwrap(),
        "the roundabout spelling must be stored canonically"
    );
}

#[test]
fn adding_the_same_path_twice_names_the_existing_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");
    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&dir)
        .assert()
        .success();

    pman_in(tmp.path())
        .args(["project", "add", "--name", "again"])
        .arg(&dir)
        .assert()
        .failure()
        .stderr(contains("already registered as `demo`"));
}

#[test]
fn a_project_name_can_only_be_used_once() {
    let tmp = tempfile::tempdir().unwrap();
    let first = project_dir(tmp.path(), "first");
    let second = project_dir(tmp.path(), "second");
    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&first)
        .assert()
        .success();

    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&second)
        .assert()
        .failure()
        .stderr(contains("the name `demo` is already used"));
}

#[test]
fn adding_a_missing_directory_fails_without_creating_a_database() {
    let tmp = tempfile::tempdir().unwrap();

    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(tmp.path().join("nope"))
        .assert()
        .failure()
        .stderr(contains("nope"));

    assert!(
        !tmp.path().join("pmanager.db").exists(),
        "a rejected path must not create the ledger"
    );
}

#[test]
fn adding_without_a_name_or_a_terminal_asks_for_the_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");

    // assert_cmd gives the child no terminal, like a scheduled run.
    pman_in(tmp.path())
        .args(["project", "add"])
        .arg(&dir)
        .assert()
        .failure()
        .stderr(contains("pass --name"));
}

#[test]
fn a_blank_name_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");

    pman_in(tmp.path())
        .args(["project", "add", "--name", "   "])
        .arg(&dir)
        .assert()
        .failure()
        .stderr(contains("must not be empty"));
}

#[test]
fn a_removed_project_leaves_list_and_keeps_its_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");
    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&dir)
        .assert()
        .success();

    pman_in(tmp.path())
        .args(["project", "remove"])
        .arg(&dir)
        .assert()
        .success()
        .stderr(contains("unregistered `demo`"));

    assert_eq!(listed_as_json(tmp.path()), serde_json::json!([]));
    assert!(
        dir.join("main.rs").exists(),
        "unregistering must never delete project files"
    );
}

#[test]
fn a_project_whose_folder_is_gone_can_still_be_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");
    pman_in(tmp.path())
        .args(["project", "add", "--name", "demo"])
        .arg(&dir)
        .assert()
        .success();
    let canonical = std::fs::canonicalize(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    pman_in(tmp.path())
        .args(["project", "remove"])
        .arg(&canonical)
        .assert()
        .success();

    assert_eq!(listed_as_json(tmp.path()), serde_json::json!([]));
}

#[test]
fn removing_an_unregistered_path_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "demo");

    pman_in(tmp.path())
        .args(["project", "remove"])
        .arg(&dir)
        .assert()
        .failure()
        .stderr(contains("no project is registered at"));
}

#[test]
fn project_cleanup_reports_that_it_is_not_implemented() {
    pman()
        .args(["project", "cleanup"])
        .assert()
        .failure()
        .stderr(contains("`pman project cleanup` is not implemented yet"));
}

// ---------------------------------------------------------------------------
// `pman run`
// ---------------------------------------------------------------------------

/// A strip directory holding one profile: a Python project is recognised by
/// its `pyproject.toml` and gives up its `.venv`.
fn strip_dir(tmp: &Path) -> PathBuf {
    let dir = tmp.join("strip");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("profiles.json"),
        r#"[{ "language": "python", "file": "python.profile.json" }]"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("python.profile.json"),
        r#"{
            "language": "python",
            "detection": [{ "type": "file", "element": "pyproject.toml" }],
            "strippable": [{ "type": "folder", "element": ".venv" }]
        }"#,
    )
    .unwrap();
    dir
}

fn pman_run(tmp: &Path) -> Command {
    let mut command = pman_in(tmp);
    command.arg("--strip-dir").arg(tmp.join("strip"));
    command.arg("run");
    command
}

/// A Python project with something to strip, registered under `name`.
fn python_project(tmp: &Path, name: &str) -> PathBuf {
    let dir = tmp.join(name);
    std::fs::create_dir_all(dir.join(".venv").join("lib")).unwrap();
    std::fs::write(dir.join(".venv").join("lib").join("big"), vec![0u8; 4096]).unwrap();
    std::fs::write(dir.join("pyproject.toml"), "[project]").unwrap();
    std::fs::write(dir.join("main.py"), "print()").unwrap();

    pman_in(tmp)
        .args(["project", "add", "--name", name])
        .arg(&dir)
        .assert()
        .success();

    dir
}

/// Back-date every file in the tree, which is what `activity::last_change`
/// reads. Directory mtimes are not consulted, so they are left alone.
fn age(root: &Path, days: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
    back_date(root, when);
}

fn back_date(dir: &Path, when: std::time::SystemTime) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        // `.git` is skipped by the real activity check too, and git writes
        // its object files read-only, so they cannot be back-dated anyway.
        if path.file_name() == Some(std::ffi::OsStr::new(".git")) {
            continue;
        }
        if path.is_dir() {
            back_date(&path, when);
        } else {
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
    }
}

fn state_of(tmp: &Path, name: &str) -> String {
    let listed = listed_as_json(tmp);
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == serde_json::json!(name))
        .unwrap_or_else(|| panic!("`{name}` should still be in the ledger"));

    row["state"].as_str().unwrap().to_string()
}

#[test]
fn a_dry_run_prints_the_plan_and_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 70);

    pman_run(tmp.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(contains("demo"))
        .stdout(contains("strip"))
        .stdout(contains("dry-run"));

    assert!(
        dir.join(".venv").exists(),
        "--dry-run must not delete anything"
    );
    assert_eq!(
        state_of(tmp.path(), "demo"),
        "registered",
        "--dry-run must not move the ledger row"
    );
}

#[test]
fn a_cold_project_with_a_matching_profile_is_stripped() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 70);

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("strip"))
        .stdout(contains("ok"));

    assert!(!dir.join(".venv").exists(), "the .venv should be gone");
    assert!(
        dir.join("main.py").exists(),
        "only the profile's strippable entries may be removed"
    );
    assert_eq!(state_of(tmp.path(), "demo"), "stripped");
}

#[test]
fn a_project_still_in_use_is_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("none"))
        .stdout(contains("active"));

    assert!(dir.join(".venv").exists());
    assert_eq!(state_of(tmp.path(), "demo"), "registered");
}

#[test]
fn a_cold_project_no_profile_recognises_says_so_rather_than_looking_idle() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = tmp.path().join("mystery");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "no marker here").unwrap();
    pman_in(tmp.path())
        .args(["project", "add", "--name", "mystery"])
        .arg(&dir)
        .assert()
        .success();
    age(&dir, 70);

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("no profile"));

    assert_eq!(state_of(tmp.path(), "mystery"), "registered");
}

#[test]
fn a_dirty_repository_is_held_back_until_a_hundred_and_sixty_days() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");

    let repository = git2::Repository::init(&dir).unwrap();
    std::fs::write(dir.join(".gitignore"), ".venv/\n").unwrap();
    let mut index = repository.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
    let who = git2::Signature::now("test", "test@example.com").unwrap();
    repository
        .commit(Some("HEAD"), &who, &who, "seed", &tree, &[])
        .unwrap();
    // Uncommitted work: this is what defers the archive.
    std::fs::write(dir.join("notes.md"), "half-finished").unwrap();

    age(&dir, 130);
    pman_run(tmp.path()).assert().success();

    assert!(
        dir.exists() && !tmp.path().join("demo.pproject").exists(),
        "130 days with uncommitted work is short of the 160-day threshold"
    );

    age(&dir, 170);
    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("archive"));

    assert!(!dir.exists(), "past 160 days the folder is packed away");
    assert!(tmp.path().join("demo.pproject").is_file());
    assert_eq!(state_of(tmp.path(), "demo"), "archived");
}

#[test]
fn an_archived_project_is_not_touched_again() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 200);

    pman_run(tmp.path()).assert().success();
    let archive = tmp.path().join("demo.pproject");
    let packed = std::fs::read(&archive).unwrap();

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("already archived"));

    assert_eq!(
        std::fs::read(&archive).unwrap(),
        packed,
        "a second pass must not repack an archived project"
    );
}

#[test]
fn a_registered_project_past_both_thresholds_is_stripped_before_it_is_packed() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 200);

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("strip+archive"));

    let archive = tmp.path().join("demo.pproject");
    assert!(archive.is_file());
    assert!(
        std::fs::metadata(&archive).unwrap().len() < 4096,
        "the 4 KiB .venv must be stripped before packing, not compressed into the archive"
    );
}

#[test]
fn a_project_whose_folder_is_gone_is_reported_not_repaired() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    std::fs::remove_dir_all(&dir).unwrap();

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("missing"));

    assert_eq!(
        state_of(tmp.path(), "demo"),
        "registered",
        "the dead row is `pman project cleanup`'s to remove, not run's"
    );
}

#[test]
fn run_on_an_empty_ledger_succeeds_and_prints_no_rows() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(contains("no projects registered"));
}

#[test]
fn an_invalid_profile_fails_the_pass_before_any_project_is_touched() {
    let tmp = tempfile::tempdir().unwrap();
    let strip = strip_dir(tmp.path());
    std::fs::write(
        strip.join("python.profile.json"),
        r#"{ "language": "python", "detection": [], "strippable": [] }"#,
    )
    .unwrap();
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 200);

    pman_run(tmp.path()).assert().failure();

    assert!(
        dir.join(".venv").exists(),
        "a broken profile must stop the pass before anything is deleted"
    );
}

#[test]
fn run_as_json_keeps_the_freed_bytes_as_a_number() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 70);

    let output = pman_run(tmp.path())
        .args(["--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("stdout is JSON");

    assert_eq!(parsed[0]["name"], serde_json::json!("demo"));
    assert_eq!(parsed[0]["action"], serde_json::json!("strip"));
    assert_eq!(parsed[0]["result"], serde_json::json!("ok"));
    assert_eq!(parsed[0]["freed"], serde_json::json!(4096));
}

#[test]
fn a_pmanagerignore_keeps_a_project_cold_through_a_whole_pass() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 70);

    // Written last, so it is the newest file in the tree by a wide margin:
    // without the ignore rule this alone would make the project look active.
    std::fs::create_dir(dir.join("cache")).unwrap();
    std::fs::write(dir.join("cache").join("today.bin"), "fresh").unwrap();

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("none"))
        .stdout(contains("active"));
    assert!(
        dir.join(".venv").exists(),
        "a fresh file makes the project active, so nothing is due yet"
    );

    std::fs::write(dir.join(".pmanagerignore"), "cache/\n").unwrap();
    // The ignore file itself is a file in the tree, so it has to be old too.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(70 * 24 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(dir.join(".pmanagerignore"))
        .unwrap()
        .set_modified(old)
        .unwrap();

    pman_run(tmp.path())
        .assert()
        .success()
        .stdout(contains("strip"))
        .stdout(contains("ok"));

    assert!(
        !dir.join(".venv").exists(),
        "with cache/ ignored the project is 70 days idle and due for a strip"
    );
    assert!(
        dir.join("cache").join("today.bin").exists(),
        ".pmanagerignore decides what counts as activity, never what is removed"
    );
}

#[test]
fn an_unusable_pmanagerignore_fails_that_project_and_names_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    let dir = python_project(tmp.path(), "demo");
    age(&dir, 70);
    std::fs::write(dir.join(".pmanagerignore"), "cache/{\n").unwrap();

    pman_run(tmp.path())
        .assert()
        .failure()
        .stdout(contains("failed"))
        .stderr(contains(".pmanagerignore"));

    assert!(
        dir.join(".venv").exists(),
        "a project that could not be measured must not be acted on"
    );
}

/// Unix-only: a read-only parent directory is the portable way to make a
/// delete fail on demand.
#[cfg(unix)]
#[test]
fn one_project_failing_does_not_stop_the_ones_after_it() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    strip_dir(tmp.path());
    // `select_all` orders by name, so `a-locked` is attempted first.
    let locked = python_project(tmp.path(), "a-locked");
    let healthy = python_project(tmp.path(), "b-healthy");
    age(&locked, 70);
    age(&healthy, 70);

    let original = std::fs::metadata(&locked).unwrap().permissions();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

    let assertion = pman_run(tmp.path()).assert();
    std::fs::set_permissions(&locked, original).unwrap();

    assertion
        .failure()
        // The row shows the action it was attempting, not a reassuring
        // `none`, so the table is complete without reading stderr.
        .stdout(contains("strip"))
        .stdout(contains("failed"))
        .stderr(contains("a-locked"));

    assert!(
        locked.join(".venv").exists(),
        "the locked project's strip must not have half-succeeded"
    );
    assert!(
        !healthy.join(".venv").exists(),
        "a failure must not block the projects after it"
    );
    assert_eq!(state_of(tmp.path(), "a-locked"), "registered");
    assert_eq!(state_of(tmp.path(), "b-healthy"), "stripped");
}

#[test]
fn auto_run_group_is_spelled_with_a_hyphen() {
    pman()
        .args(["auto-run", "--help"])
        .assert()
        .success()
        .stdout(contains("Commands:"));
}

#[test]
fn auto_run_group_exposes_two_nested_subcommands() {
    for leaf in ["add", "remove"] {
        pman()
            .args(["auto-run", leaf, "--help"])
            .assert()
            .success()
            .stdout(contains(format!("pman auto-run {leaf}")));
    }
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

/// Every command in the spec that is still a stub, with the arguments needed
/// to reach its body, paired with the exact message its stub must print. An
/// implemented command leaves this table for tests of its own. Every stub ends in
/// "is not implemented yet", so checking only that suffix cannot tell one
/// command's stub from another's; a command that is renamed, dropped, or
/// wired to the wrong dispatch arm by accident fails here because the full
/// `pman <path>` prefix is checked too.
const SURFACE: &[(&[&str], &str)] = &[
    (
        &["project", "cleanup"],
        "`pman project cleanup` is not implemented yet",
    ),
    (
        &["auto-run", "add"],
        "`pman auto-run add` is not implemented yet",
    ),
    (
        &["auto-run", "remove"],
        "`pman auto-run remove` is not implemented yet",
    ),
];

#[test]
fn every_documented_command_is_reachable_and_honest() {
    for (args, expected) in SURFACE {
        pman()
            .args(*args)
            .assert()
            .failure()
            .stderr(contains(*expected));
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
        vec!["list", "--help"],
    ] {
        pman().args(&args).assert().success();
    }
}
