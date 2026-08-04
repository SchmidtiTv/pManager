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

// --- archives and database -------------------------------------------------

/// A config file pointing every data path at a throwaway directory, so these
/// tests never touch the real database.
fn isolated() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!("data_dir = \"{}\"\n", dir.path().join("data").display()),
    )
    .unwrap();
    (dir, config)
}

/// Build a folder with two files to archive.
fn source_folder(root: &std::path::Path) -> std::path::PathBuf {
    let source = root.join("photos");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("a.txt"), "hello").unwrap();
    std::fs::write(source.join("sub").join("b.txt"), "world!").unwrap();
    source
}

#[test]
fn creates_an_archive_and_records_it() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());
    let out = dir.path().join("photos.7z");

    vmanager()
        .args(["archive", "create"])
        .arg(&source)
        .arg("--output")
        .arg(&out)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("2 files"));

    assert!(out.is_file());

    // The run should now be visible in the database.
    vmanager()
        .args(["archive", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("photos.7z"));

    vmanager()
        .args(["db", "status", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("archives  1"));
}

#[test]
fn archive_output_defaults_to_the_folder_name() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());
    let workdir = dir.path().join("cwd");
    std::fs::create_dir(&workdir).unwrap();

    vmanager()
        .current_dir(&workdir)
        .args(["archive", "create"])
        .arg(&source)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    assert!(workdir.join("photos.7z").is_file());
}

#[test]
fn refusing_to_overwrite_uses_the_archive_exit_code() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());
    let out = dir.path().join("photos.7z");

    let create = || {
        let mut cmd = vmanager();
        cmd.args(["archive", "create"])
            .arg(&source)
            .arg("--output")
            .arg(&out)
            .arg("--config")
            .arg(&config);
        cmd
    };

    create().assert().success();
    create()
        .assert()
        .code(6)
        .stderr(predicate::str::contains("--force"));
    create().arg("--force").assert().success();
}

#[test]
fn no_record_skips_the_database() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());

    vmanager()
        .args(["archive", "create"])
        .arg(&source)
        .arg("--output")
        .arg(dir.path().join("photos.7z"))
        .args(["--no-record", "--config"])
        .arg(&config)
        .assert()
        .success();

    vmanager()
        .args(["archive", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no archives recorded yet"));
}

#[test]
fn archiving_a_missing_folder_is_not_found() {
    let (dir, config) = isolated();

    vmanager()
        .args(["archive", "create"])
        .arg(dir.path().join("absent"))
        .arg("--config")
        .arg(&config)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn forget_removes_the_record_but_keeps_the_file() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());
    let out = dir.path().join("photos.7z");

    vmanager()
        .args(["archive", "create"])
        .arg(&source)
        .arg("--output")
        .arg(&out)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    vmanager()
        .args(["archive", "forget", "1", "--config"])
        .arg(&config)
        .assert()
        .success();

    assert!(out.is_file(), "forget must not delete the archive itself");

    vmanager()
        .args(["archive", "forget", "1", "--config"])
        .arg(&config)
        .assert()
        .code(4);
}

#[test]
fn prune_drops_records_for_deleted_archives() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());
    let out = dir.path().join("photos.7z");

    vmanager()
        .args(["archive", "create"])
        .arg(&source)
        .arg("--output")
        .arg(&out)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    std::fs::remove_file(&out).unwrap();

    vmanager()
        .args(["archive", "prune", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 record(s) pruned"));

    vmanager()
        .args(["db", "status", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("archives  0"));
}

#[test]
fn db_path_does_not_create_the_database() {
    let (dir, config) = isolated();
    let db_path = dir.path().join("data").join("vmanager.db");

    vmanager()
        .args(["db", "path", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("vmanager.db"));

    assert!(!db_path.exists(), "`db path` should be read-only");

    vmanager()
        .args(["db", "init", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("schema v1"));

    assert!(db_path.is_file());
}

#[test]
fn compression_level_is_validated() {
    let (dir, config) = isolated();
    let source = source_folder(dir.path());

    vmanager()
        .args(["archive", "create"])
        .arg(&source)
        .args(["--level", "11", "--config"])
        .arg(&config)
        .assert()
        .code(2);
}
