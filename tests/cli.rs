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
    // A TOML *literal* string (single quotes) — a basic string would treat the
    // backslashes in a Windows path as escape sequences and fail to parse.
    std::fs::write(
        &config,
        format!("data_dir = '{}'\n", dir.path().join("data").display()),
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
        .stdout(predicate::str::contains(format!(
            "schema v{}",
            vmanager::db::Database::target_version()
        )));

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

// --- managing versions -----------------------------------------------------

/// A tool folder holding one subfolder per version, each with a file in it.
fn tool_folder(root: &std::path::Path, name: &str, versions: &[&str]) -> std::path::PathBuf {
    let tool = root.join(name);
    for version in versions {
        let path = tool.join(version);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("tool.exe"), format!("{name} {version}")).unwrap();
    }
    tool
}

#[test]
fn registering_a_folder_and_sweeping_it_archives_the_oldest_versions() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["4.1", "4.2", "4.3", "4.10"]);

    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("registered"))
        .stdout(predicate::str::contains("4 version(s) present"));

    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("archived 4.1"))
        .stdout(predicate::str::contains("archived 4.2"))
        .stdout(predicate::str::contains("2 archived"));

    // The two oldest are now archives; the two newest are untouched. Note that
    // 4.10 is kept and 4.2 is not: the ordering is numeric, not alphabetic.
    assert!(tool.join("4.1.7z").is_file());
    assert!(tool.join("4.2.7z").is_file());
    assert!(!tool.join("4.1").exists());
    assert!(!tool.join("4.2").exists());
    assert!(tool.join("4.3").is_dir());
    assert!(tool.join("4.10").is_dir());

    vmanager()
        .args(["archive", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("4.1.7z"));
}

#[test]
fn a_second_sweep_has_nothing_left_to_do() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "godot", &["1.0", "2.0", "3.0"]);

    let sweep = || {
        let mut cmd = vmanager();
        cmd.args(["folder", "add"])
            .arg(&tool)
            .arg("--config")
            .arg(&config);
        cmd
    };
    sweep().assert().success();

    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 archived"));

    // The .7z left behind must not be mistaken for a version folder, so the
    // daily run settles instead of eating a version every day.
    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to archive"))
        .stdout(predicate::str::contains("0 archived"));

    assert!(tool.join("2.0").is_dir());
    assert!(tool.join("3.0").is_dir());
}

#[test]
fn a_dry_run_reports_without_touching_anything() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);

    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    vmanager()
        .args(["run", "--dry-run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("would archive 1.0"))
        .stdout(predicate::str::contains("dry run"));

    assert!(tool.join("1.0").is_dir(), "a dry run must not delete");
    assert!(!tool.join("1.0.7z").exists(), "a dry run must not write");
}

#[test]
fn keep_source_writes_the_archive_but_leaves_the_folder() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);

    vmanager()
        .args(["run", "--keep-source", "--folder"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("folder kept"));

    assert!(tool.join("1.0.7z").is_file());
    assert!(tool.join("1.0").is_dir());
}

#[test]
fn keep_is_taken_from_the_flag_then_the_folder_then_the_config() {
    let (dir, config) = isolated();

    // A per-folder override of 1 beats the configured default of 3.
    std::fs::write(
        &config,
        format!(
            "data_dir = '{}'\nkeep_versions = 3\n",
            dir.path().join("data").display()
        ),
    )
    .unwrap();

    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0", "4.0"]);
    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .args(["--keep", "1", "--config"])
        .arg(&config)
        .assert()
        .success();

    vmanager()
        .args(["run", "--dry-run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("keeping 1"))
        .stdout(predicate::str::contains("3 version(s) would be archived"));

    // …and an explicit --keep beats both.
    vmanager()
        .args(["run", "--dry-run", "--keep", "2", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("keeping 2"))
        .stdout(predicate::str::contains("2 version(s) would be archived"));
}

#[test]
fn a_keep_of_zero_is_a_usage_error() {
    let (_dir, config) = isolated();

    vmanager()
        .args(["run", "--keep", "0", "--config"])
        .arg(&config)
        .assert()
        .code(2);
}

#[test]
fn an_invalid_keep_versions_is_a_config_error() {
    let (dir, config) = isolated();
    std::fs::write(
        &config,
        format!(
            "data_dir = '{}'\nkeep_versions = 0\n",
            dir.path().join("data").display()
        ),
    )
    .unwrap();

    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least 1"));
}

#[test]
fn folders_without_version_numbers_are_left_alone() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);
    std::fs::create_dir(tool.join("current")).unwrap();
    std::fs::write(tool.join("current").join("tool.exe"), "live").unwrap();

    vmanager()
        .args(["run", "--folder"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("ignoring"))
        .stdout(predicate::str::contains("1 archived"));

    assert!(tool.join("current").is_dir(), "`current` is not a version");
    assert!(!tool.join("current.7z").exists());
}

#[test]
fn folder_list_shows_what_is_pending_and_remove_unregisters() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);

    vmanager()
        .args(["folder", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no folders registered yet"));

    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    vmanager()
        .args(["folder", "list", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("keep 2"))
        .stdout(predicate::str::contains("3 unpacked, 1 to archive"));

    vmanager()
        .args(["folder", "remove"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("unregistered"));

    vmanager()
        .args(["folder", "remove"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .code(4);
}

#[test]
fn registering_the_same_folder_twice_updates_it() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0"]);

    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("registered"));

    vmanager()
        .args(["folder", "add"])
        .arg(&tool)
        .args(["--keep", "5", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("updated"))
        .stdout(predicate::str::contains("keeping 5"));

    vmanager()
        .args(["db", "status", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("folders   1"));
}

#[test]
fn registering_a_missing_folder_is_not_found() {
    let (dir, config) = isolated();

    vmanager()
        .args(["folder", "add"])
        .arg(dir.path().join("absent"))
        .arg("--config")
        .arg(&config)
        .assert()
        .code(4);
}

#[test]
fn a_run_with_nothing_registered_says_so() {
    let (_dir, config) = isolated();

    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("no folders registered yet"));
}

#[test]
fn one_broken_folder_does_not_stop_the_others() {
    let (dir, config) = isolated();
    let healthy = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);
    let gone = dir.path().join("unplugged-drive");
    std::fs::create_dir(&gone).unwrap();

    for folder in [&healthy, &gone] {
        vmanager()
            .args(["folder", "add"])
            .arg(folder)
            .arg("--config")
            .arg(&config)
            .assert()
            .success();
    }
    std::fs::remove_dir(&gone).unwrap();

    // The missing folder fails the run, but only after the healthy one has
    // been swept — a disconnected drive must not strand every other tool.
    vmanager()
        .args(["run", "--config"])
        .arg(&config)
        .assert()
        .failure()
        .stdout(predicate::str::contains("1 archived"))
        .stderr(predicate::str::contains("not found"));

    assert!(healthy.join("1.0.7z").is_file());
}

#[test]
fn an_existing_archive_is_skipped_until_forced() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);
    std::fs::write(tool.join("1.0.7z"), b"left over from something else").unwrap();

    let sweep = || {
        let mut cmd = vmanager();
        cmd.args(["run", "--folder"])
            .arg(&tool)
            .arg("--config")
            .arg(&config);
        cmd
    };

    sweep()
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped 1.0"))
        .stdout(predicate::str::contains("--force"));
    assert!(tool.join("1.0").is_dir(), "the folder must survive a skip");

    sweep()
        .arg("--force")
        .assert()
        .success()
        .stdout(predicate::str::contains("archived 1.0"));
    assert!(!tool.join("1.0").exists());
}

#[test]
fn sweeping_an_unregistered_folder_works_without_registering_it() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);

    vmanager()
        .args(["run", "--folder"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("1 archived"));

    vmanager()
        .args(["db", "status", "--config"])
        .arg(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("folders   0"))
        .stdout(predicate::str::contains("archives  1"));
}

#[test]
fn the_archive_of_a_swept_version_restores_it() {
    let (dir, config) = isolated();
    let tool = tool_folder(dir.path(), "blender", &["1.0", "2.0", "3.0"]);

    vmanager()
        .args(["run", "--folder"])
        .arg(&tool)
        .arg("--config")
        .arg(&config)
        .assert()
        .success();

    // The whole point of archiving rather than deleting: the old version can
    // come back, folder name and contents intact.
    let restored = dir.path().join("restored");
    sevenz_rust2::decompress_file(tool.join("1.0.7z"), &restored).unwrap();
    assert_eq!(
        std::fs::read_to_string(restored.join("1.0").join("tool.exe")).unwrap(),
        "blender 1.0"
    );
}

#[test]
fn schedule_rejects_a_malformed_time() {
    vmanager()
        .args(["schedule", "install", "--at", "3pm"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("HH:MM"));
}

#[cfg(not(windows))]
#[test]
fn scheduling_off_windows_explains_itself() {
    vmanager()
        .args(["schedule", "install"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Task Scheduler"));
}
