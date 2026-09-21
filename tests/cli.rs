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
        .stderr(contains("`pman run` is not implemented yet"));
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
    (&["run"], "`pman run` is not implemented yet"),
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
