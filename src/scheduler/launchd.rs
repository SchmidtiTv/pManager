//! macOS: a user LaunchAgent, `~/Library/LaunchAgents/pmanager.run.plist`.
//!
//! `StartCalendarInterval` fires at noon and, if the Mac was asleep then,
//! once on wake. A Mac that was switched off at noon skips that day: launchd
//! has no catch-up for time spent powered off.

use std::path::PathBuf;

use anyhow::Context as _;

use super::{HOUR, Installed, Job, MINUTE, Removed, Runner, Scheduler, check, xml_escape};
use crate::error::Result;

pub const LABEL: &str = "pmanager.run";

/// `launchctl print`'s exit code for "Could not find service".
const NOT_LOADED: i32 = 113;

pub struct Launchd {
    dir: PathBuf,
    runner: Box<dyn Runner>,
}

impl Launchd {
    /// `dir` is the LaunchAgents directory the plist is written to.
    pub fn new(dir: PathBuf, runner: Box<dyn Runner>) -> Self {
        Self { dir, runner }
    }

    fn plist_path(&self) -> PathBuf {
        self.dir.join(format!("{LABEL}.plist"))
    }

    /// `gui/<uid>`, the domain a user agent lives in.
    fn domain(&self) -> Result<String> {
        let uid = check(&*self.runner, "id", &["-u"])?;
        Ok(format!("gui/{}", uid.stdout.trim()))
    }

    /// `launchctl print` exits 113 for a service that is not loaded. Any
    /// other failure means the question was not answered, and guessing "not
    /// loaded" would let `remove` delete the plist under a live agent.
    fn is_loaded(&self, service: &str) -> Result<bool> {
        let output = self.runner.run("launchctl", &["print", service])?;
        if output.success {
            return Ok(true);
        }
        if output.code == Some(NOT_LOADED) {
            return Ok(false);
        }

        check(&*self.runner, "launchctl", &["print", service]).map(|_| true)
    }
}

impl Scheduler for Launchd {
    fn describe(&self) -> String {
        format!("launchd agent {}", self.plist_path().display())
    }

    fn install(&self, job: &Job) -> Result<Installed> {
        let plist = render(job);
        let path = self.plist_path();
        let domain = self.domain()?;
        let service = format!("{domain}/{LABEL}");

        let loaded = self.is_loaded(&service)?;
        let existing = std::fs::read_to_string(&path).ok();
        // Reloading an identical agent buys nothing, and a bootout followed
        // straight away by a bootstrap can race launchd's own teardown.
        if loaded && existing.as_deref() == Some(plist.as_str()) {
            return Ok(Installed::Unchanged);
        }

        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create {}", self.dir.display()))?;
        if let Some(log_dir) = PathBuf::from(&job.log).parent() {
            // launchd creates the log file but not its directory.
            std::fs::create_dir_all(log_dir)
                .with_context(|| format!("could not create {}", log_dir.display()))?;
        }
        std::fs::write(&path, &plist)
            .with_context(|| format!("could not write {}", path.display()))?;

        if loaded {
            check(&*self.runner, "launchctl", &["bootout", &service])?;
        }
        let path_text = path.to_string_lossy();
        check(
            &*self.runner,
            "launchctl",
            &["bootstrap", &domain, &path_text],
        )?;

        Ok(if loaded || existing.is_some() {
            Installed::Replaced
        } else {
            Installed::New
        })
    }

    fn remove(&self) -> Result<Removed> {
        let path = self.plist_path();
        let service = format!("{}/{LABEL}", self.domain()?);

        let loaded = self.is_loaded(&service)?;
        let present = path.exists();
        if !loaded && !present {
            return Ok(Removed::NotInstalled);
        }

        if loaded {
            check(&*self.runner, "launchctl", &["bootout", &service])?;
        }
        if present {
            std::fs::remove_file(&path)
                .with_context(|| format!("could not delete {}", path.display()))?;
        }

        Ok(Removed::Removed)
    }
}

/// No `RunAtLoad`: installing the schedule must not start a pass on the spot.
/// `NO_COLOR` keeps ANSI escapes out of the log file.
fn render(job: &Job) -> String {
    let arguments: String = std::iter::once(&job.program)
        .chain(&job.args)
        .map(|arg| format!("        <string>{}</string>\n", xml_escape(arg)))
        .collect();
    let log = xml_escape(&job.log);

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{arguments}    </array>
    <key>StartCalendarInterval</key>
    <dict>
        <key>Hour</key>
        <integer>{HOUR}</integer>
        <key>Minute</key>
        <integer>{MINUTE}</integer>
    </dict>
    <key>EnvironmentVariables</key>
    <dict>
        <key>NO_COLOR</key>
        <string>1</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LABEL, Launchd, render};
    use crate::scheduler::fake::FakeRunner;
    use crate::scheduler::{Installed, Job, Removed, Scheduler};

    fn job(dir: &Path) -> Job {
        Job {
            program: "/opt/pman & co/pman".to_string(),
            args: vec![
                "--db".to_string(),
                "/d/<x>.db".to_string(),
                "run".to_string(),
            ],
            log: dir.join("logs").join("auto-run.log").display().to_string(),
        }
    }

    fn launchd(dir: &Path, runner: &FakeRunner) -> Launchd {
        runner.say("id -u", "501\n");
        Launchd::new(dir.join("LaunchAgents"), Box::new(runner.clone()))
    }

    fn not_loaded(runner: &FakeRunner) {
        runner.fail("launchctl print", 113, "Could not find service");
    }

    #[test]
    fn the_plist_runs_the_job_daily_at_noon_without_running_at_load() {
        let plist = render(&job(Path::new("/tmp")));

        assert!(plist.contains("<string>/opt/pman &amp; co/pman</string>"));
        assert!(plist.contains("<string>/d/&lt;x&gt;.db</string>"));
        assert!(plist.contains("<key>Hour</key>\n        <integer>12</integer>"));
        assert!(plist.contains("<key>Minute</key>\n        <integer>0</integer>"));
        assert!(plist.contains("<key>NO_COLOR</key>"));
        assert!(
            !plist.contains("RunAtLoad"),
            "installing must not start a pass"
        );
    }

    #[test]
    fn a_fresh_install_writes_the_plist_and_bootstraps_it() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);

        let installed = scheduler.install(&job(tmp.path())).unwrap();

        assert_eq!(installed, Installed::New);
        let plist = tmp
            .path()
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist"));
        assert!(plist.is_file());
        assert!(
            tmp.path().join("logs").is_dir(),
            "the log directory is created"
        );
        assert_eq!(
            runner.calls(),
            vec![
                "id -u".to_string(),
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl bootstrap gui/501 {}", plist.display()),
            ]
        );
    }

    #[test]
    fn installing_the_same_job_again_touches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);
        scheduler.install(&job(tmp.path())).unwrap();
        runner.say("id -u", "501\n");

        let again = scheduler.install(&job(tmp.path())).unwrap();

        assert_eq!(again, Installed::Unchanged);
        assert!(
            !runner.calls()[3..].iter().any(|c| c.contains("boot")),
            "an identical, loaded agent is left alone: {:?}",
            runner.calls()
        );
    }

    #[test]
    fn a_changed_job_replaces_the_loaded_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);
        scheduler.install(&job(tmp.path())).unwrap();
        runner.say("id -u", "501\n");

        let moved = Job {
            program: "/usr/local/bin/pman".to_string(),
            ..job(tmp.path())
        };
        let installed = scheduler.install(&moved).unwrap();

        assert_eq!(installed, Installed::Replaced);
        let calls = runner.calls();
        assert_eq!(
            calls[calls.len() - 2],
            format!("launchctl bootout gui/501/{LABEL}")
        );
        assert!(calls[calls.len() - 1].starts_with("launchctl bootstrap gui/501 "));
    }

    #[test]
    fn a_failed_bootstrap_is_an_error_not_a_success() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);
        runner.fail(
            "launchctl bootstrap",
            5,
            "Bootstrap failed: 5: Input/output error",
        );

        let err = scheduler.install(&job(tmp.path())).unwrap_err();

        assert!(
            format!("{err:#}").contains("Bootstrap failed: 5"),
            "got: {err:#}"
        );
    }

    #[test]
    fn removing_unloads_and_deletes_only_pmanagers_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);
        scheduler.install(&job(tmp.path())).unwrap();
        let other = tmp
            .path()
            .join("LaunchAgents")
            .join("com.example.other.plist");
        std::fs::write(&other, "not ours").unwrap();
        runner.say("id -u", "501\n");

        assert_eq!(scheduler.remove().unwrap(), Removed::Removed);

        assert!(
            runner
                .calls()
                .contains(&format!("launchctl bootout gui/501/{LABEL}"))
        );
        assert!(
            !tmp.path()
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist"))
                .exists()
        );
        assert!(other.exists(), "another agent's plist must survive");
    }

    #[test]
    fn an_unexpected_launchctl_failure_is_not_read_as_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        runner.fail("launchctl print", 1, "Unrecognized target specifier.");
        runner.fail("launchctl print", 1, "Unrecognized target specifier.");

        let err = scheduler.remove().unwrap_err();

        assert!(
            format!("{err:#}").contains("Unrecognized target"),
            "got: {err:#}"
        );
    }

    #[test]
    fn removing_when_nothing_is_installed_is_a_quiet_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = launchd(tmp.path(), &runner);
        not_loaded(&runner);

        assert_eq!(scheduler.remove().unwrap(), Removed::NotInstalled);
        assert!(!runner.calls().iter().any(|c| c.contains("bootout")));
    }
}
