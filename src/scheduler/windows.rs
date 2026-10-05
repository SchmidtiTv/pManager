//! Windows: a Task Scheduler task named `pManager-run`, registered for the
//! current user through `schtasks /XML`.
//!
//! The XML route is the only one `schtasks` offers for `StartWhenAvailable`,
//! which runs a day's pass once the machine is back if it was off at noon.

use std::path::PathBuf;

use anyhow::Context as _;
use chrono::{Duration, Local, NaiveDateTime, NaiveTime};

use super::{HOUR, Installed, Job, MINUTE, Removed, Runner, Scheduler, check, xml_escape};
use crate::error::Result;

pub const TASK: &str = "pManager-run";

pub struct TaskScheduler {
    scratch: PathBuf,
    runner: Box<dyn Runner>,
}

impl TaskScheduler {
    /// `scratch` is where the task XML is written for `schtasks` to read.
    pub fn new(scratch: PathBuf, runner: Box<dyn Runner>) -> Self {
        Self { scratch, runner }
    }

    fn exists(&self) -> Result<bool> {
        Ok(self
            .runner
            .run("schtasks", &["/Query", "/TN", TASK])?
            .success)
    }
}

impl Scheduler for TaskScheduler {
    fn describe(&self) -> String {
        format!("Task Scheduler task {TASK}")
    }

    fn install(&self, job: &Job) -> Result<Installed> {
        let existed = self.exists()?;
        let xml = render(job, next_run(Local::now().naive_local()));

        // `schtasks /XML` reads UTF-16 reliably; UTF-8 depends on the build.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        let path = self.scratch.join(format!("{TASK}.xml"));
        std::fs::write(&path, bytes)
            .with_context(|| format!("could not write {}", path.display()))?;

        let path_text = path.to_string_lossy();
        let created = check(
            &*self.runner,
            "schtasks",
            &["/Create", "/TN", TASK, "/XML", &path_text, "/F"],
        );
        let _ = std::fs::remove_file(&path);
        created?;

        Ok(if existed {
            Installed::Replaced
        } else {
            Installed::New
        })
    }

    fn remove(&self) -> Result<Removed> {
        if !self.exists()? {
            return Ok(Removed::NotInstalled);
        }

        check(&*self.runner, "schtasks", &["/Delete", "/TN", TASK, "/F"])?;
        Ok(Removed::Removed)
    }
}

/// The next noon from `now`. Starting the trigger in the future, rather than
/// on some fixed past date, keeps `StartWhenAvailable` from treating every
/// day before registration as missed and starting a pass on the spot.
fn next_run(now: NaiveDateTime) -> NaiveDateTime {
    let time = NaiveTime::from_hms_opt(HOUR, MINUTE, 0).expect("HOUR and MINUTE are a valid time");
    let today = now.date().and_time(time);
    if now < today {
        today
    } else {
        today + Duration::days(1)
    }
}

fn render(job: &Job, start: NaiveDateTime) -> String {
    let command = xml_escape(&job.program);
    let arguments = xml_escape(
        &job.args
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let start = start.format("%Y-%m-%dT%H:%M:%S");

    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>pManager daily run</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{start}</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// One argument as `CommandLineToArgvW` will split it back out: quoted when
/// it holds a space, a tab or a quote, with the backslashes in front of a
/// quote, or of the closing quote, doubled.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }

    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            c => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(c);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::{TASK, TaskScheduler, next_run, quote, render};
    use crate::scheduler::fake::FakeRunner;
    use crate::scheduler::{Installed, Job, Removed, Scheduler};

    fn job() -> Job {
        Job {
            program: r"C:\Program Files\pman\pman.exe".to_string(),
            args: vec![
                "--db".to_string(),
                r"C:\Users\me\AppData\Roaming\pmanager\pmanager.db".to_string(),
                "--strip-dir".to_string(),
                r"C:\Program Files\pman\strip\".to_string(),
                "run".to_string(),
            ],
            log: "unused".to_string(),
        }
    }

    fn at(day: u32, hour: u32, minute: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn arguments_round_trip_through_windows_command_line_rules() {
        assert_eq!(quote("run"), "run");
        assert_eq!(quote(r"C:\a\b.db"), r"C:\a\b.db");
        assert_eq!(quote(r"C:\my dir\x"), r#""C:\my dir\x""#);
        assert_eq!(quote(r"C:\my dir\"), r#""C:\my dir\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote(""), r#""""#);
    }

    #[test]
    fn the_first_run_is_the_next_noon_never_one_in_the_past() {
        assert_eq!(next_run(at(5, 9, 0)), at(5, 12, 0));
        assert_eq!(next_run(at(5, 12, 0)), at(6, 12, 0));
        assert_eq!(next_run(at(5, 23, 59)), at(6, 12, 0));
    }

    #[test]
    fn the_task_runs_daily_catches_up_and_escapes_its_command() {
        let xml = render(&job(), at(6, 12, 0));

        assert!(xml.contains("<StartBoundary>2026-10-06T12:00:00</StartBoundary>"));
        assert!(xml.contains("<DaysInterval>1</DaysInterval>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains(r"<Command>C:\Program Files\pman\pman.exe</Command>"));
        assert!(xml.contains(
            r#"<Arguments>--db C:\Users\me\AppData\Roaming\pmanager\pmanager.db --strip-dir &quot;C:\Program Files\pman\strip\\&quot; run</Arguments>"#
        ));
    }

    #[test]
    fn installing_creates_the_task_and_cleans_up_its_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        runner.fail(
            "schtasks /Query",
            1,
            "ERROR: The system cannot find the file specified.",
        );
        let scheduler = TaskScheduler::new(tmp.path().to_path_buf(), Box::new(runner.clone()));

        assert_eq!(scheduler.install(&job()).unwrap(), Installed::New);

        let xml = tmp.path().join(format!("{TASK}.xml"));
        assert_eq!(
            runner.calls()[1],
            format!("schtasks /Create /TN {TASK} /XML {} /F", xml.display())
        );
        assert!(!xml.exists(), "the scratch XML is removed afterwards");
    }

    #[test]
    fn installing_over_an_existing_task_replaces_it() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = TaskScheduler::new(tmp.path().to_path_buf(), Box::new(runner.clone()));

        assert_eq!(scheduler.install(&job()).unwrap(), Installed::Replaced);
    }

    #[test]
    fn a_rejected_task_is_an_error_and_still_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        runner.fail("schtasks /Create", 1, "ERROR: The task XML is malformed.");
        let scheduler = TaskScheduler::new(tmp.path().to_path_buf(), Box::new(runner.clone()));

        let err = scheduler.install(&job()).unwrap_err();

        assert!(format!("{err:#}").contains("malformed"), "got: {err:#}");
        assert!(!tmp.path().join(format!("{TASK}.xml")).exists());
    }

    #[test]
    fn removing_deletes_only_pmanagers_task() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        let scheduler = TaskScheduler::new(tmp.path().to_path_buf(), Box::new(runner.clone()));

        assert_eq!(scheduler.remove().unwrap(), Removed::Removed);
        assert_eq!(runner.calls()[1], format!("schtasks /Delete /TN {TASK} /F"));
    }

    #[test]
    fn removing_when_nothing_is_installed_is_a_quiet_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = FakeRunner::default();
        runner.fail(
            "schtasks /Query",
            1,
            "ERROR: The system cannot find the file specified.",
        );
        let scheduler = TaskScheduler::new(tmp.path().to_path_buf(), Box::new(runner.clone()));

        assert_eq!(scheduler.remove().unwrap(), Removed::NotInstalled);
        assert_eq!(runner.calls().len(), 1);
    }
}
