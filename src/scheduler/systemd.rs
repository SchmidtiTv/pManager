//! Linux: a systemd user timer, `pman-run.timer` plus the `pman-run.service`
//! it starts, under `~/.config/systemd/user/`.
//!
//! `Persistent=true` catches up a run missed while the machine was off. A
//! user timer only fires while the user's manager is running, which normally
//! means while they are logged in; pManager does not enable lingering.

use std::path::PathBuf;

use anyhow::Context as _;

use super::{HOUR, Installed, Job, MINUTE, Removed, Runner, Scheduler, check};
use crate::error::Result;

pub const UNIT: &str = "pman-run";

pub struct Systemd {
    dir: PathBuf,
    stamps: PathBuf,
    runner: Box<dyn Runner>,
}

impl Systemd {
    /// `dir` holds the unit files; `stamps` is where systemd records when a
    /// persistent timer last fired.
    pub fn new(dir: PathBuf, stamps: PathBuf, runner: Box<dyn Runner>) -> Self {
        Self {
            dir,
            stamps,
            runner,
        }
    }

    fn service_path(&self) -> PathBuf {
        self.dir.join(format!("{UNIT}.service"))
    }

    fn timer_path(&self) -> PathBuf {
        self.dir.join(format!("{UNIT}.timer"))
    }

    fn systemctl(&self, args: &[&str]) -> Result<()> {
        let mut full = vec!["--user"];
        full.extend_from_slice(args);
        check(&*self.runner, "systemctl", &full).map(drop)
    }
}

impl Scheduler for Systemd {
    fn describe(&self) -> String {
        format!("systemd user timer {}", self.timer_path().display())
    }

    fn install(&self, job: &Job) -> Result<Installed> {
        let service = render_service(job);
        let timer = render_timer();
        let timer_name = format!("{UNIT}.timer");

        let existing = (
            std::fs::read_to_string(self.service_path()).ok(),
            std::fs::read_to_string(self.timer_path()).ok(),
        );
        let active = self
            .runner
            .run(
                "systemctl",
                &["--user", "is-active", "--quiet", &timer_name],
            )?
            .success;
        if active
            && existing.0.as_deref() == Some(service.as_str())
            && existing.1.as_deref() == Some(timer.as_str())
        {
            return Ok(Installed::Unchanged);
        }

        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create {}", self.dir.display()))?;
        for (path, text) in [(self.service_path(), &service), (self.timer_path(), &timer)] {
            std::fs::write(&path, text)
                .with_context(|| format!("could not write {}", path.display()))?;
        }

        self.systemctl(&["daemon-reload"])?;
        self.systemctl(&["enable", &timer_name])?;
        // A restart, not a start: an already-running timer picks up the new
        // schedule. Without a stamp file nothing is "missed", so this does
        // not start a pass on the spot.
        self.systemctl(&["restart", &timer_name])?;

        Ok(if active || existing.0.is_some() || existing.1.is_some() {
            Installed::Replaced
        } else {
            Installed::New
        })
    }

    fn remove(&self) -> Result<Removed> {
        let timer_name = format!("{UNIT}.timer");
        let service = self.service_path();
        let timer = self.timer_path();
        if !service.exists() && !timer.exists() {
            return Ok(Removed::NotInstalled);
        }

        if timer.exists() {
            self.systemctl(&["disable", "--now", &timer_name])?;
        }
        for path in [&service, &timer] {
            if path.exists() {
                std::fs::remove_file(path)
                    .with_context(|| format!("could not delete {}", path.display()))?;
            }
        }
        // Left behind, the stamp would make a later `add` look like it had
        // missed every day since, and start a pass the moment it is enabled.
        let stamp = self.stamps.join(format!("stamp-{timer_name}"));
        if stamp.exists() {
            std::fs::remove_file(&stamp)
                .with_context(|| format!("could not delete {}", stamp.display()))?;
        }
        self.systemctl(&["daemon-reload"])?;

        Ok(Removed::Removed)
    }
}

fn render_service(job: &Job) -> String {
    let command: Vec<String> = std::iter::once(&job.program)
        .chain(&job.args)
        .map(|arg| quote(arg))
        .collect();

    format!(
        "[Unit]
Description=pManager daily run

[Service]
Type=oneshot
Environment=NO_COLOR=1
ExecStart={}
",
        command.join(" ")
    )
}

fn render_timer() -> String {
    format!(
        "[Unit]
Description=pManager daily run at {HOUR:02}:{MINUTE:02}

[Timer]
OnCalendar=*-*-* {HOUR:02}:{MINUTE:02}:00
Persistent=true
Unit={UNIT}.service

[Install]
WantedBy=timers.target
"
    )
}

/// One `ExecStart=` word: double-quoted, with C-style escapes, `%` doubled
/// so it is not read as a specifier, and `$` doubled so it is not expanded.
fn quote(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    for c in arg.chars() {
        match c {
            '\\' => quoted.push_str(r"\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str(r"\n"),
            '\t' => quoted.push_str(r"\t"),
            '%' => quoted.push_str("%%"),
            '$' => quoted.push_str("$$"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Systemd, UNIT, quote, render_service, render_timer};
    use crate::scheduler::fake::FakeRunner;
    use crate::scheduler::{Installed, Job, Removed, Scheduler};

    fn job() -> Job {
        Job {
            program: "/home/me/.cargo/bin/pman".to_string(),
            args: vec![
                "--db".to_string(),
                "/home/me/my db.db".to_string(),
                "run".to_string(),
            ],
            log: "/unused".to_string(),
        }
    }

    fn systemd(dir: &Path, runner: &FakeRunner) -> Systemd {
        Systemd::new(
            dir.join("user"),
            dir.join("timers"),
            Box::new(runner.clone()),
        )
    }

    fn inactive(runner: &FakeRunner) {
        runner.fail("systemctl --user is-active", 3, "");
    }

    #[test]
    fn exec_start_words_survive_spaces_quotes_specifiers_and_variables() {
        assert_eq!(quote("/a b/pman"), r#""/a b/pman""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote("100%"), r#""100%%""#);
        assert_eq!(quote("$HOME"), r#""$$HOME""#);
        assert_eq!(quote(r"C:\x"), r#""C:\\x""#);
    }

    #[test]
    fn the_units_run_the_job_daily_at_noon_and_catch_up() {
        let service = render_service(&job());
        let timer = render_timer();

        assert!(
            service.contains(
                r#"ExecStart="/home/me/.cargo/bin/pman" "--db" "/home/me/my db.db" "run""#
            )
        );
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("Environment=NO_COLOR=1"));
        assert!(timer.contains("OnCalendar=*-*-* 12:00:00"));
        assert!(timer.contains("Persistent=true"));
        assert!(timer.contains(&format!("Unit={UNIT}.service")));
    }

    #[test]
    fn a_fresh_install_writes_both_units_and_enables_the_timer() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        inactive(&runner);

        let installed = systemd(tmp.path(), &runner).install(&job()).unwrap();

        assert_eq!(installed, Installed::New);
        assert!(
            tmp.path()
                .join("user")
                .join(format!("{UNIT}.service"))
                .is_file()
        );
        assert!(
            tmp.path()
                .join("user")
                .join(format!("{UNIT}.timer"))
                .is_file()
        );
        assert_eq!(
            runner.calls(),
            vec![
                format!("systemctl --user is-active --quiet {UNIT}.timer"),
                "systemctl --user daemon-reload".to_string(),
                format!("systemctl --user enable {UNIT}.timer"),
                format!("systemctl --user restart {UNIT}.timer"),
            ]
        );
    }

    #[test]
    fn installing_the_same_job_again_touches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        inactive(&runner);
        let scheduler = systemd(tmp.path(), &runner);
        scheduler.install(&job()).unwrap();

        assert_eq!(scheduler.install(&job()).unwrap(), Installed::Unchanged);
        assert_eq!(
            runner.calls().len(),
            5,
            "only the is-active probe: {:?}",
            runner.calls()
        );
    }

    #[test]
    fn a_changed_job_is_rewritten_and_reloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        inactive(&runner);
        let scheduler = systemd(tmp.path(), &runner);
        scheduler.install(&job()).unwrap();

        let moved = Job {
            program: "/usr/bin/pman".to_string(),
            ..job()
        };

        assert_eq!(scheduler.install(&moved).unwrap(), Installed::Replaced);
        let service =
            std::fs::read_to_string(tmp.path().join("user").join(format!("{UNIT}.service")))
                .unwrap();
        assert!(service.contains(r#"ExecStart="/usr/bin/pman""#));
    }

    #[test]
    fn a_systemctl_failure_is_an_error_not_a_success() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        inactive(&runner);
        runner.fail(
            "systemctl --user daemon-reload",
            1,
            "Failed to connect to bus: No medium found",
        );

        let err = systemd(tmp.path(), &runner).install(&job()).unwrap_err();

        assert!(
            format!("{err:#}").contains("Failed to connect to bus"),
            "got: {err:#}"
        );
    }

    #[test]
    fn removing_disables_the_timer_and_deletes_its_units_and_stamp() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        inactive(&runner);
        let scheduler = systemd(tmp.path(), &runner);
        scheduler.install(&job()).unwrap();
        std::fs::create_dir_all(tmp.path().join("timers")).unwrap();
        let stamp = tmp
            .path()
            .join("timers")
            .join(format!("stamp-{UNIT}.timer"));
        std::fs::write(&stamp, "").unwrap();
        let other = tmp.path().join("user").join("other.timer");
        std::fs::write(&other, "not ours").unwrap();

        assert_eq!(scheduler.remove().unwrap(), Removed::Removed);

        assert!(
            runner
                .calls()
                .contains(&format!("systemctl --user disable --now {UNIT}.timer"))
        );
        assert!(
            !tmp.path()
                .join("user")
                .join(format!("{UNIT}.timer"))
                .exists()
        );
        assert!(
            !tmp.path()
                .join("user")
                .join(format!("{UNIT}.service"))
                .exists()
        );
        assert!(
            !stamp.exists(),
            "a stale stamp would trigger a catch-up on the next add"
        );
        assert!(other.exists(), "another unit must survive");
    }

    #[test]
    fn removing_when_nothing_is_installed_is_a_quiet_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();

        assert_eq!(
            systemd(tmp.path(), &runner).remove().unwrap(),
            Removed::NotInstalled
        );
        assert!(runner.calls().is_empty());
    }
}
