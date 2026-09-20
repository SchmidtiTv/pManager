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

    pman().arg("--db").arg(&db).arg("list").assert().failure();

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
    pman()
        .arg("restore")
        .assert()
        .failure()
        .stderr(contains("NAME"));
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
