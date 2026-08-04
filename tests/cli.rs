//! End-to-end tests driving the built binary the way a user would.
//!
//! Every test that touches configuration passes an explicit `--config` inside a
//! temporary directory, so the suite never reads or writes the real
//! `~/.config/vmanager`.

use assert_cmd::Command;
use predicates::prelude::*;

/// A `vmanager` invocation with a clean environment.
fn vmanager() -> Command {
    let mut cmd = Command::cargo_bin("vmanager").expect("binary is built");
    cmd.env_remove("VMANAGER_CONFIG")
        .env_remove("RUST_LOG")
        .env("NO_COLOR", "1");
    cmd
}

#[test]
fn prints_version() {
    vmanager()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn prints_help_with_exit_code_zero() {
    vmanager()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("Exit codes:"));
}

#[test]
fn bare_invocation_shows_help() {
    vmanager()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn unknown_flag_is_a_usage_error() {
    vmanager().arg("--definitely-not-a-flag").assert().code(2);
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    vmanager().arg("frobnicate").assert().code(2);
}

#[test]
fn config_path_echoes_the_override() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    vmanager()
        .args(["config", "path", "--config"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains(path.to_str().unwrap()));
}

#[test]
fn config_path_honours_the_environment_variable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("from-env.toml");

    vmanager()
        .env("VMANAGER_CONFIG", &path)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("from-env.toml"));
}

#[test]
fn config_show_emits_parsable_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "data_dir = \"/srv/vmanager\"\n").unwrap();

    let output = vmanager()
        .args(["config", "show", "--config"])
        .arg(&path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rendered = String::from_utf8(output).unwrap();
    assert!(
        rendered.contains("/srv/vmanager"),
        "config show should reflect the file: {rendered}"
    );
    toml::from_str::<toml::Table>(&rendered).expect("output is valid TOML");
}

#[test]
fn config_init_creates_a_file_and_refuses_to_clobber_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");

    vmanager()
        .args(["config", "init", "--config"])
        .arg(&path)
        .assert()
        .success();
    assert!(path.exists());

    vmanager()
        .args(["config", "init", "--config"])
        .arg(&path)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--force"));

    vmanager()
        .args(["config", "init", "--force", "--config"])
        .arg(&path)
        .assert()
        .success();
}

#[test]
fn quiet_suppresses_progress_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    vmanager()
        .args(["config", "init", "--quiet", "--config"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn invalid_config_exits_with_the_config_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "this is not = = toml\n").unwrap();

    vmanager()
        .args(["config", "show", "--config"])
        .arg(&path)
        .assert()
        .code(3)
        .stderr(predicate::str::contains("invalid config file"))
        .stderr(predicate::str::contains("caused by:"));
}

#[test]
fn completions_are_generated_for_every_supported_shell() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        vmanager()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("vmanager"));
    }
}

#[test]
fn completions_reject_an_unknown_shell() {
    vmanager().args(["completions", "csh"]).assert().code(2);
}

#[test]
fn logs_go_to_stderr_not_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // `-vv` enables debug logging, which reports the missing config file.
    vmanager()
        .args(["config", "show", "-vv", "--config"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("DEBUG").not())
        .stderr(predicate::str::contains("no config file"));
}
