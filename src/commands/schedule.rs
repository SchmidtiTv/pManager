//! `vmanager schedule` — the daily run, registered with Windows Task
//! Scheduler.
//!
//! vManager stays a one-shot command: there is no resident daemon to keep
//! alive or to restart after a reboot. `schedule install` hands `vmanager run`
//! to `schtasks.exe`, which is the component Windows already has for exactly
//! this, and which survives reboots and reports a failed run in its own
//! history.
//!
//! Everything except the actual `schtasks` call is platform independent so it
//! can be tested anywhere; the call itself is compiled in on Windows and
//! replaced by a clear error elsewhere.

use std::io::Write;
use std::path::Path;

use crate::commands::Context;
use crate::error::{Error, Result};

/// Task Scheduler name used when `--name` is not given.
const DEFAULT_TASK_NAME: &str = "vManager daily sweep";

/// Time of day the sweep runs when `--at` is not given.
const DEFAULT_TIME: &str = "03:00";

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: ScheduleCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ScheduleCommand {
    /// Register a daily `vmanager run` with Task Scheduler
    Install {
        /// Time of day to run, as HH:MM
        #[arg(long, value_name = "HH:MM", default_value = DEFAULT_TIME, value_parser = parse_time)]
        at: String,

        /// Name of the scheduled task
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TASK_NAME)]
        name: String,

        /// Replace an existing task with the same name
        #[arg(short, long)]
        force: bool,
    },

    /// Remove the scheduled task
    Uninstall {
        /// Name of the scheduled task
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TASK_NAME)]
        name: String,
    },

    /// Show the scheduled task, if there is one
    Status {
        /// Name of the scheduled task
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TASK_NAME)]
        name: String,
    },
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        ScheduleCommand::Install { at, name, force } => install(ctx, at, name, *force),
        ScheduleCommand::Uninstall { name } => uninstall(ctx, name),
        ScheduleCommand::Status { name } => status(ctx, name),
    }
}

fn install(ctx: &Context, at: &str, name: &str, force: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    // An explicit `--config` has to travel with the task: the scheduled run
    // executes as a service-side login that may not see the same config file.
    let command = task_command(&exe, ctx.globals().config.as_deref());

    let mut arguments = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        name.to_string(),
        "/TR".to_string(),
        command,
        "/SC".to_string(),
        "DAILY".to_string(),
        "/ST".to_string(),
        at.to_string(),
    ];
    if force {
        arguments.push("/F".to_string());
    }

    schtasks(&arguments)?;

    if !ctx.is_quiet() {
        let mut out = std::io::stdout().lock();
        writeln!(out, "scheduled `{name}` to run daily at {at}")?;
        writeln!(out, "  {}", exe.display())?;
        writeln!(out, "remove it with `vmanager schedule uninstall`")?;
    }
    Ok(())
}

fn uninstall(ctx: &Context, name: &str) -> Result<()> {
    schtasks(&[
        "/Delete".to_string(),
        "/TN".to_string(),
        name.to_string(),
        "/F".to_string(),
    ])?;

    if !ctx.is_quiet() {
        writeln!(std::io::stdout().lock(), "removed `{name}`")?;
    }
    Ok(())
}

fn status(ctx: &Context, name: &str) -> Result<()> {
    let output = schtasks(&[
        "/Query".to_string(),
        "/TN".to_string(),
        name.to_string(),
        "/V".to_string(),
        "/FO".to_string(),
        "LIST".to_string(),
    ])?;

    if !ctx.is_quiet() {
        std::io::stdout().lock().write_all(output.as_bytes())?;
    }
    Ok(())
}

/// The command line Task Scheduler should run.
///
/// The executable is quoted because `C:\Program Files\…` is a perfectly normal
/// place for it to live, and `schtasks` splits `/TR` on spaces otherwise.
fn task_command(exe: &Path, config: Option<&Path>) -> String {
    let mut command = format!("\"{}\" run", exe.display());
    if let Some(config) = config {
        command.push_str(&format!(" --config \"{}\"", config.display()));
    }
    command
}

/// Validate `--at` as a 24-hour `HH:MM`, which is what `schtasks /ST` wants.
fn parse_time(raw: &str) -> std::result::Result<String, String> {
    let (hours, minutes) = raw
        .split_once(':')
        .ok_or_else(|| format!("`{raw}` is not a time; use HH:MM, e.g. 03:00"))?;

    let invalid = |what: &str| format!("`{raw}` is not a valid time: {what}");

    if hours.len() != 2 || minutes.len() != 2 {
        return Err(invalid("use two digits for both hours and minutes"));
    }

    let hours: u32 = hours
        .parse()
        .map_err(|_| invalid("hours are not a number"))?;
    let minutes: u32 = minutes
        .parse()
        .map_err(|_| invalid("minutes are not a number"))?;

    if hours > 23 {
        return Err(invalid("hours must be 00-23"));
    }
    if minutes > 59 {
        return Err(invalid("minutes must be 00-59"));
    }

    Ok(format!("{hours:02}:{minutes:02}"))
}

/// Run `schtasks.exe` and return its stdout.
#[cfg(windows)]
fn schtasks(arguments: &[String]) -> Result<String> {
    tracing::debug!(?arguments, "running schtasks");

    let output = std::process::Command::new("schtasks")
        .args(arguments)
        .output()
        .map_err(|err| {
            Error::Other(anyhow::anyhow!(
                "could not run schtasks.exe: {err}; is it on PATH?"
            ))
        })?;

    if !output.status.success() {
        // schtasks writes its diagnostics to stderr and says useful things
        // there ("Access is denied", "The system cannot find the file
        // specified"), so pass them through rather than inventing a message.
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        return Err(Error::Other(anyhow::anyhow!(
            "schtasks failed{}{}",
            if message.is_empty() { "" } else { ": " },
            message
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Scheduling is Windows-only, so elsewhere say so and point at the local
/// equivalent rather than pretending to have installed anything.
#[cfg(not(windows))]
fn schtasks(_arguments: &[String]) -> Result<String> {
    Err(Error::Other(anyhow::anyhow!(
        "`vmanager schedule` needs Windows Task Scheduler; on this platform run \
         `vmanager run` from cron or a systemd timer instead"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheduled_command_quotes_the_executable() {
        let command = task_command(Path::new(r"C:\Program Files\vmanager\vmanager.exe"), None);
        assert_eq!(command, r#""C:\Program Files\vmanager\vmanager.exe" run"#);
    }

    #[test]
    fn an_explicit_config_travels_with_the_task() {
        let command = task_command(
            Path::new(r"C:\tools\vmanager.exe"),
            Some(Path::new(r"D:\config\vmanager.toml")),
        );
        assert_eq!(
            command,
            r#""C:\tools\vmanager.exe" run --config "D:\config\vmanager.toml""#
        );
    }

    #[test]
    fn times_are_accepted_in_24_hour_form() {
        assert_eq!(parse_time("03:00").unwrap(), "03:00");
        assert_eq!(parse_time("00:00").unwrap(), "00:00");
        assert_eq!(parse_time("23:59").unwrap(), "23:59");
    }

    #[test]
    fn nonsense_times_are_rejected_before_schtasks_sees_them() {
        for bad in ["3pm", "24:00", "12:60", "9:00", "12", "12:5", "ab:cd", ""] {
            assert!(parse_time(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn scheduling_off_windows_explains_itself() {
        let err = schtasks(&["/Query".to_string()]).unwrap_err();
        assert!(err.to_string().contains("Task Scheduler"), "{err}");
    }
}
