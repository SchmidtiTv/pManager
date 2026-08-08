//! `pmanager schedule` — the daily run, registered with the platform
//! scheduler: Task Scheduler on Windows, launchd on macOS.
//!
//! pManager stays a one-shot command: there is no resident daemon to keep
//! alive or to restart after a reboot. `schedule install` hands `pmanager run
//! --yes` to whichever of those the platform already has for exactly this,
//! and which survives reboots and reports a failed run in its own history.
//!
//! Everything except the actual registration is platform independent so it
//! can be tested anywhere; that part is compiled in on Windows and macOS and
//! replaced by a clear error elsewhere.

use std::io::Write;
use std::path::Path;

use crate::commands::Context;
use crate::error::{Error, Result};

/// Task/agent name used when `--name` is not given.
const DEFAULT_TASK_NAME: &str = "pManager daily sweep";

/// Time of day the sweep runs when `--at` is not given.
const DEFAULT_TIME: &str = "03:00";

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: ScheduleCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ScheduleCommand {
    /// Register a daily `pmanager run --yes` with the platform scheduler
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
    let config = ctx.globals().config.as_deref();
    // An explicit `--config` has to travel with the task: the scheduled run
    // executes as a service-side login that may not see the same config file.
    let command = task_command(&exe, config);
    // launchd has no history of its own the way Task Scheduler does, so give
    // it somewhere to write one, alongside the rest of what pManager keeps.
    let log_path = ctx.config()?.data_dir()?.join("schedule.log");

    platform::install(&exe, config, at, name, force, &log_path)?;

    if ctx.is_json() {
        return crate::json::emit(&crate::json::Scheduled {
            task: name.to_string(),
            at: at.to_string(),
            executable: crate::json::path(&exe),
            command,
        });
    }

    if !ctx.is_quiet() {
        let mut out = std::io::stdout().lock();
        writeln!(out, "scheduled `{name}` to run daily at {at}")?;
        writeln!(out, "  {}", exe.display())?;
        writeln!(out, "remove it with `pmanager schedule uninstall`")?;
    }
    Ok(())
}

fn uninstall(ctx: &Context, name: &str) -> Result<()> {
    platform::uninstall(name)?;

    if ctx.is_json() {
        return crate::json::emit(&crate::json::Unscheduled {
            task: name.to_string(),
            removed: true,
        });
    }

    if !ctx.is_quiet() {
        writeln!(std::io::stdout().lock(), "removed `{name}`")?;
    }
    Ok(())
}

fn status(ctx: &Context, name: &str) -> Result<()> {
    let output = platform::status(name)?;

    if ctx.is_json() {
        return crate::json::emit(&crate::json::ScheduleStatus::new(name, output));
    }

    if !ctx.is_quiet() {
        std::io::stdout().lock().write_all(output.as_bytes())?;
    }
    Ok(())
}

/// The command line the platform scheduler should run.
///
/// `--yes` is the point of the whole subcommand: a bare `pmanager run` only
/// reports, which would make for a very diligent scheduled task that never did
/// anything. Installing the schedule is the deliberate act that grants the
/// daily run permission to delete.
///
/// The executable is quoted because `C:\Program Files\…` is a perfectly normal
/// place for it to live, and both `schtasks` and a shell split on spaces
/// otherwise.
fn task_command(exe: &Path, config: Option<&Path>) -> String {
    let mut command = format!("\"{}\" run --yes", exe.display());
    if let Some(config) = config {
        command.push_str(&format!(" --config \"{}\"", config.display()));
    }
    command
}

/// Validate `--at` as a 24-hour `HH:MM`, which is what both `schtasks /ST` and
/// launchd's `StartCalendarInterval` want.
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

/// Split an already-validated `HH:MM` string into its parts.
///
/// Only launchd needs the two halves — `schtasks` takes the `HH:MM` string as
/// it stands — so on a Windows release build this would otherwise be dead code.
/// The test build keeps it everywhere, since what it promises is worth checking
/// on whatever platform the suite happens to run on.
#[cfg(any(target_os = "macos", test))]
fn split_time(at: &str) -> (u32, u32) {
    let (hours, minutes) = at.split_once(':').expect("validated by parse_time");
    (
        hours.parse().expect("validated by parse_time"),
        minutes.parse().expect("validated by parse_time"),
    )
}

/// Windows: Task Scheduler, via `schtasks.exe`.
#[cfg(windows)]
mod platform {
    use super::{Error, Path, Result, task_command};

    pub fn install(
        exe: &Path,
        config: Option<&Path>,
        at: &str,
        name: &str,
        force: bool,
        _log_path: &Path,
    ) -> Result<()> {
        let command = task_command(exe, config);

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
        Ok(())
    }

    pub fn uninstall(name: &str) -> Result<()> {
        schtasks(&[
            "/Delete".to_string(),
            "/TN".to_string(),
            name.to_string(),
            "/F".to_string(),
        ])?;
        Ok(())
    }

    pub fn status(name: &str) -> Result<String> {
        schtasks(&[
            "/Query".to_string(),
            "/TN".to_string(),
            name.to_string(),
            "/V".to_string(),
            "/FO".to_string(),
            "LIST".to_string(),
        ])
    }

    /// Run `schtasks.exe` and return its stdout.
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
}

/// macOS: a per-user LaunchAgent, via `launchctl`.
///
/// A `pmanager schedule install` writes `~/Library/LaunchAgents/<label>.plist`
/// and registers it with `launchctl bootstrap`; `uninstall` reverses that with
/// `bootout` and removes the file. The label is derived from `--name` so two
/// differently-named schedules do not collide, and so `status`/`uninstall` can
/// find the file `install` wrote without keeping a registry of their own.
#[cfg(target_os = "macos")]
mod platform {
    use super::{Error, Path, Result, split_time};
    use std::path::PathBuf;

    pub fn install(
        exe: &Path,
        config: Option<&Path>,
        at: &str,
        name: &str,
        force: bool,
        log_path: &Path,
    ) -> Result<()> {
        let label = label_for(name);
        let path = plist_path(&label)?;

        if path.is_file() {
            if !force {
                return Err(Error::Other(anyhow::anyhow!(
                    "`{name}` is already scheduled; pass --force to replace it"
                )));
            }
            // Best-effort: unload the old registration before overwriting it.
            // If it is not currently loaded (e.g. after a crash) this simply
            // fails, which is fine — there is nothing to unload.
            if let Ok(uid) = console_uid() {
                let _ = launchctl(&[
                    "bootout",
                    &format!("gui/{uid}"),
                    &path.display().to_string(),
                ]);
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (hour, minute) = split_time(at);
        let contents = plist_contents(&label, exe, config, hour, minute, log_path);
        std::fs::write(&path, contents)?;

        let uid = console_uid()?;
        let output = launchctl(&[
            "bootstrap",
            &format!("gui/{uid}"),
            &path.display().to_string(),
        ])?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            let message = message.trim();
            // A plist that failed to register is not actually scheduled;
            // don't leave it behind for `status` to find and misreport.
            let _ = std::fs::remove_file(&path);
            return Err(Error::Other(anyhow::anyhow!(
                "launchctl bootstrap failed{}{}",
                if message.is_empty() { "" } else { ": " },
                message
            )));
        }

        // `bootstrap` can leave a service disabled if it was left disabled by
        // an earlier `bootout`; make sure the schedule will actually run.
        let _ = launchctl(&["enable", &format!("gui/{uid}/{label}")]);

        Ok(())
    }

    pub fn uninstall(name: &str) -> Result<()> {
        let label = label_for(name);
        let path = plist_path(&label)?;

        if !path.is_file() {
            return Err(Error::Other(anyhow::anyhow!(
                "no scheduled task named `{name}`"
            )));
        }

        if let Ok(uid) = console_uid() {
            let _ = launchctl(&[
                "bootout",
                &format!("gui/{uid}"),
                &path.display().to_string(),
            ]);
        }
        std::fs::remove_file(&path)?;
        Ok(())
    }

    pub fn status(name: &str) -> Result<String> {
        let label = label_for(name);
        let path = plist_path(&label)?;

        if !path.is_file() {
            return Err(Error::Other(anyhow::anyhow!(
                "no scheduled task named `{name}`"
            )));
        }

        let loaded = launchctl(&["list", &label])
            .map(|output| output.status.success())
            .unwrap_or(false);

        let mut text = format!(
            "Label: {label}\nPlist: {}\nLoaded: {}\n",
            path.display(),
            if loaded { "yes" } else { "no" }
        );
        if loaded && let Ok(output) = launchctl(&["list", &label]) {
            text.push_str("\n--- launchctl list ---\n");
            text.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        text.push_str("\n--- plist ---\n");
        text.push_str(&std::fs::read_to_string(&path)?);

        Ok(text)
    }

    /// Turn `--name` into a launchd label. launchd wants something
    /// identifier-like, not free text with spaces, so this lowercases and
    /// collapses everything that is not ASCII alphanumeric into a single `-`.
    fn label_for(name: &str) -> String {
        let mut slug = String::with_capacity(name.len());
        let mut last_dash = false;
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                last_dash = false;
            } else if !last_dash {
                slug.push('-');
                last_dash = true;
            }
        }
        let trimmed = slug.trim_matches('-');
        format!(
            "com.pmanager.{}",
            if trimmed.is_empty() { "task" } else { trimmed }
        )
    }

    /// `~/Library/LaunchAgents/<label>.plist` — the standard location for a
    /// per-user launchd job that should run whether or not anything is
    /// currently logged in at the console, once bootstrapped into the `gui`
    /// domain.
    fn plist_path(label: &str) -> Result<PathBuf> {
        let base = directories::BaseDirs::new().ok_or(Error::NoConfigDir)?;
        Ok(base
            .home_dir()
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist")))
    }

    /// The current user's numeric ID, needed for `launchctl`'s `gui/<uid>`
    /// domain target. There is no `unsafe`-free libc call for this in the
    /// crates already in use, and `id -u` is as reliable a source as any.
    fn console_uid() -> Result<String> {
        let output = std::process::Command::new("id")
            .arg("-u")
            .output()
            .map_err(|err| Error::Other(anyhow::anyhow!("could not run `id -u`: {err}")))?;
        if !output.status.success() {
            return Err(Error::Other(anyhow::anyhow!("`id -u` failed")));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn launchctl(arguments: &[&str]) -> Result<std::process::Output> {
        tracing::debug!(?arguments, "running launchctl");
        std::process::Command::new("launchctl")
            .args(arguments)
            .output()
            .map_err(|err| {
                Error::Other(anyhow::anyhow!(
                    "could not run launchctl: {err}; is it on PATH?"
                ))
            })
    }

    /// Escape a string for use as plist character data. Paths are not
    /// expected to contain XML metacharacters, but a project or username with
    /// one should not be able to produce a plist launchd refuses to parse.
    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn plist_contents(
        label: &str,
        exe: &Path,
        config: Option<&Path>,
        hour: u32,
        minute: u32,
        log_path: &Path,
    ) -> String {
        let mut arguments = format!(
            "\t\t<string>{}</string>\n\t\t<string>run</string>\n\t\t<string>--yes</string>\n",
            xml_escape(&exe.display().to_string())
        );
        if let Some(config) = config {
            arguments.push_str(&format!(
                "\t\t<string>--config</string>\n\t\t<string>{}</string>\n",
                xml_escape(&config.display().to_string())
            ));
        }
        let log = xml_escape(&log_path.display().to_string());

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{label}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             {arguments}\
             \t</array>\n\
             \t<key>StartCalendarInterval</key>\n\
             \t<dict>\n\
             \t\t<key>Hour</key>\n\
             \t\t<integer>{hour}</integer>\n\
             \t\t<key>Minute</key>\n\
             \t\t<integer>{minute}</integer>\n\
             \t</dict>\n\
             \t<key>StandardOutPath</key>\n\
             \t<string>{log}</string>\n\
             \t<key>StandardErrorPath</key>\n\
             \t<string>{log}</string>\n\
             </dict>\n\
             </plist>\n"
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_name_becomes_a_reverse_dns_label() {
            assert_eq!(
                label_for("pManager daily sweep"),
                "com.pmanager.pmanager-daily-sweep"
            );
        }

        #[test]
        fn punctuation_collapses_rather_than_multiplying_dashes() {
            assert_eq!(label_for("nightly!!  run"), "com.pmanager.nightly-run");
        }

        #[test]
        fn a_name_that_is_all_punctuation_still_yields_a_label() {
            assert_eq!(label_for("!!!"), "com.pmanager.task");
        }

        #[test]
        fn the_plist_carries_the_command_and_the_schedule() {
            let xml = plist_contents(
                "com.pmanager.test",
                Path::new("/usr/local/bin/pmanager"),
                Some(Path::new("/Users/me/pmanager.toml")),
                3,
                30,
                Path::new("/Users/me/.local/share/pmanager/schedule.log"),
            );
            assert!(xml.contains("<string>com.pmanager.test</string>"));
            assert!(xml.contains("<string>/usr/local/bin/pmanager</string>"));
            assert!(xml.contains("<string>run</string>"));
            assert!(xml.contains("<string>--yes</string>"));
            assert!(xml.contains("<string>--config</string>"));
            assert!(xml.contains("<string>/Users/me/pmanager.toml</string>"));
            assert!(xml.contains("<integer>3</integer>"));
            assert!(xml.contains("<integer>30</integer>"));
        }

        #[test]
        fn plist_metacharacters_in_a_path_are_escaped() {
            let xml = plist_contents(
                "com.pmanager.test",
                Path::new("/Users/me/Weird & <Path>/pmanager"),
                None,
                0,
                0,
                Path::new("/tmp/schedule.log"),
            );
            assert!(!xml.contains("Weird & <Path>"));
            assert!(xml.contains("Weird &amp; &lt;Path&gt;"));
        }
    }
}

/// Anywhere else: neither scheduler exists, so say so and point at the local
/// equivalent rather than pretending to have installed anything.
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::{Error, Path, Result};

    fn unsupported() -> Error {
        Error::Other(anyhow::anyhow!(
            "`pmanager schedule` needs Windows Task Scheduler or macOS launchd; on this \
             platform run `pmanager run --yes` from cron or a systemd timer instead"
        ))
    }

    pub fn install(
        _exe: &Path,
        _config: Option<&Path>,
        _at: &str,
        _name: &str,
        _force: bool,
        _log_path: &Path,
    ) -> Result<()> {
        Err(unsupported())
    }

    pub fn uninstall(_name: &str) -> Result<()> {
        Err(unsupported())
    }

    pub fn status(_name: &str) -> Result<String> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheduled_command_quotes_the_executable() {
        let command = task_command(Path::new(r"C:\Program Files\pmanager\pmanager.exe"), None);
        assert_eq!(
            command,
            r#""C:\Program Files\pmanager\pmanager.exe" run --yes"#
        );
    }

    #[test]
    fn an_explicit_config_travels_with_the_task() {
        let command = task_command(
            Path::new(r"C:\tools\pmanager.exe"),
            Some(Path::new(r"D:\config\pmanager.toml")),
        );
        assert_eq!(
            command,
            r#""C:\tools\pmanager.exe" run --yes --config "D:\config\pmanager.toml""#
        );
    }

    #[test]
    fn times_are_accepted_in_24_hour_form() {
        assert_eq!(parse_time("03:00").unwrap(), "03:00");
        assert_eq!(parse_time("00:00").unwrap(), "00:00");
        assert_eq!(parse_time("23:59").unwrap(), "23:59");
    }

    #[test]
    fn nonsense_times_are_rejected_before_the_scheduler_sees_them() {
        for bad in ["3pm", "24:00", "12:60", "9:00", "12", "12:5", "ab:cd", ""] {
            assert!(parse_time(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn a_validated_time_splits_cleanly() {
        assert_eq!(split_time("03:30"), (3, 30));
        assert_eq!(split_time("00:00"), (0, 0));
        assert_eq!(split_time("23:59"), (23, 59));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn scheduling_off_windows_and_macos_explains_itself() {
        let err = platform::status("anything").unwrap_err();
        assert!(err.to_string().contains("Task Scheduler"), "{err}");
        assert!(err.to_string().contains("launchd"), "{err}");
    }
}
