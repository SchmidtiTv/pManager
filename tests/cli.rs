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
        .stderr(contains("`pman project add` is not implemented yet"));
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
