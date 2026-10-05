//! Installing pManager's daily `run` into the host's own scheduler.
//!
//! Every backend is compiled on every platform and chosen at run time, so all
//! three are unit-tested wherever the tests run. They touch the system only
//! through a [`Runner`], which the tests replace with a fake.

pub mod launchd;
pub mod systemd;
pub mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

use crate::error::Result;
use crate::paths::Paths;

/// When the daily run fires, in local time. See the cadence decision in
/// `.ai/DECISIONS.md`: once a day at noon, caught up after a missed run.
pub const HOUR: u32 = 12;
pub const MINUTE: u32 = 0;

/// The scheduled command, fully resolved.
///
/// It names the ledger and the strip directory explicitly, so the job does
/// not depend on a working directory, a shell, or an environment variable
/// that was set when `pman auto-run add` ran but is not set at noon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub program: String,
    pub args: Vec<String>,
    /// Where a backend without a log of its own (launchd) writes output.
    pub log: String,
}

impl Job {
    /// The job for `pman run` with these paths, run by this executable.
    ///
    /// The executable path is taken as the OS reports it, without resolving
    /// symlinks, so a job installed through a package manager's shim keeps
    /// working after an upgrade replaces what the shim points at.
    pub fn for_run(paths: &Paths) -> Result<Self> {
        let exe = std::env::current_exe().context("could not determine the executable path")?;
        let db = absolute(&paths.db)?;
        let strip_dir = absolute(&paths.strip_dir)?;
        let log = db
            .parent()
            .context("the database path has no parent directory")?
            .join("auto-run.log");

        Ok(Self {
            program: text(&exe)?,
            args: vec![
                "--db".to_string(),
                text(&db)?,
                "--strip-dir".to_string(),
                text(&strip_dir)?,
                "run".to_string(),
            ],
            log: text(&log)?,
        })
    }

    /// The command as a user would type it, for reporting.
    pub fn command_line(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("could not resolve {}", path.display()))
}

fn text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    New,
    Replaced,
    /// Already installed exactly as asked; nothing was touched.
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    Removed,
    NotInstalled,
}

pub trait Scheduler {
    /// What was installed and where, for the report.
    fn describe(&self) -> String;

    /// Install the daily job, replacing pManager's own job if there is one.
    fn install(&self, job: &Job) -> Result<Installed>;

    /// Remove pManager's job. Nothing else in the scheduler is touched.
    fn remove(&self) -> Result<Removed>;
}

/// The scheduler for the platform this binary is running on.
pub fn for_this_platform() -> Result<Box<dyn Scheduler>> {
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().context("could not determine the home directory")?;
        Ok(Box::new(launchd::Launchd::new(
            home.join("Library").join("LaunchAgents"),
            Box::new(System),
        )))
    } else if cfg!(target_os = "linux") {
        let config = dirs::config_dir().context("could not determine the config directory")?;
        let data = dirs::data_dir().context("could not determine the data directory")?;
        Ok(Box::new(systemd::Systemd::new(
            config.join("systemd").join("user"),
            data.join("systemd").join("timers"),
            Box::new(System),
        )))
    } else if cfg!(windows) {
        Ok(Box::new(windows::TaskScheduler::new(
            std::env::temp_dir(),
            Box::new(System),
        )))
    } else {
        bail!(
            "scheduled runs are not supported on {}; run `pman run` from your own scheduler",
            std::env::consts::OS
        )
    }
}

/// What a finished external command reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs the platform's scheduler tools. The seam the tests replace.
pub trait Runner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;
}

/// The real thing: `std::process::Command`, no shell in between.
pub struct System;

impl Runner for System {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("could not run `{program}`; is it installed and on PATH?"))?;

        Ok(Output {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Run a command that must succeed, naming it and what it said if it did not.
fn check(runner: &dyn Runner, program: &str, args: &[&str]) -> Result<Output> {
    let output = runner.run(program, args)?;
    if output.success {
        return Ok(output);
    }

    let code = output
        .code
        .map_or_else(|| "killed by a signal".to_string(), |c| format!("exit {c}"));
    let said = [output.stderr.trim(), output.stdout.trim()]
        .into_iter()
        .find(|text| !text.is_empty())
        .unwrap_or("no output");
    bail!("`{program} {}` failed ({code}): {said}", args.join(" "))
}

/// Escape text for an XML element body or attribute.
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::{Output, Runner};
    use crate::error::Result;

    /// Records every command and answers from a script, in order. An
    /// unscripted call succeeds with no output.
    #[derive(Clone, Default)]
    pub struct FakeRunner {
        pub calls: Rc<RefCell<Vec<String>>>,
        answers: Rc<RefCell<VecDeque<(String, Output)>>>,
    }

    impl FakeRunner {
        /// The next call whose command line starts with `prefix` gets `output`.
        pub fn answer(&self, prefix: &str, output: Output) {
            self.answers
                .borrow_mut()
                .push_back((prefix.to_string(), output));
        }

        pub fn fail(&self, prefix: &str, code: i32, stderr: &str) {
            self.answer(
                prefix,
                Output {
                    success: false,
                    code: Some(code),
                    stdout: String::new(),
                    stderr: stderr.to_string(),
                },
            );
        }

        pub fn say(&self, prefix: &str, stdout: &str) {
            self.answer(
                prefix,
                Output {
                    success: true,
                    code: Some(0),
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                },
            );
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
            let line = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls.borrow_mut().push(line.clone());

            let mut answers = self.answers.borrow_mut();
            let found = answers
                .iter()
                .position(|(prefix, _)| line.starts_with(prefix.as_str()));
            Ok(match found.and_then(|index| answers.remove(index)) {
                Some((_, output)) => output,
                None => Output {
                    success: true,
                    code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::fake::FakeRunner;
    use super::{Job, check, xml_escape};
    use crate::paths::Paths;

    #[test]
    fn the_job_names_its_ledger_and_profiles_with_absolute_paths() {
        let paths = Paths {
            db: PathBuf::from("relative/pmanager.db"),
            strip_dir: PathBuf::from("strip"),
        };

        let job = Job::for_run(&paths).unwrap();

        assert_eq!(job.args[0], "--db");
        assert_eq!(job.args[2], "--strip-dir");
        assert_eq!(job.args[4], "run");
        assert!(PathBuf::from(&job.args[1]).is_absolute());
        assert!(PathBuf::from(&job.args[3]).is_absolute());
        assert!(job.args[1].ends_with("pmanager.db"));
        assert!(PathBuf::from(&job.program).is_absolute());
        assert!(
            job.log.ends_with("auto-run.log"),
            "the log sits beside the ledger, got {}",
            job.log
        );
    }

    #[test]
    fn a_failed_command_names_itself_its_exit_code_and_what_it_said() {
        let runner = FakeRunner::default();
        runner.fail(
            "launchctl bootstrap",
            5,
            "Bootstrap failed: 5: Input/output error\n",
        );

        let err = check(&runner, "launchctl", &["bootstrap", "gui/501", "x.plist"]).unwrap_err();

        assert_eq!(
            format!("{err:#}"),
            "`launchctl bootstrap gui/501 x.plist` failed (exit 5): Bootstrap failed: 5: Input/output error"
        );
    }

    #[test]
    fn xml_special_characters_are_escaped() {
        assert_eq!(
            xml_escape(r#"C:\a & b <c> "d" 'e'"#),
            r"C:\a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"
        );
    }
}
