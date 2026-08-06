//! A spinner that says the program is still working.
//!
//! Compressing a large project takes minutes with nothing printed until it
//! finishes, which is indistinguishable from a hang. This draws a spinner and a
//! running clock while that happens.
//!
//! The work does not cooperate: `sevenz-rust2` compresses in one uninterrupted
//! call and never yields, so nothing the main thread could do would keep an
//! animation moving. The spinner therefore lives on its own thread and paints
//! on a timer, and the main thread compresses without knowing it is there.
//!
//! Output goes to stderr. Every progress line is erased before the spinner
//! stops, so what remains on the terminal is exactly what would have been there
//! without it, and stdout — the part a pipe sees — is never touched.

use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Spinner frames, in order. ASCII so a bare Windows console renders them.
const FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// How long between repaints.
///
/// Fast enough to read as motion, slow enough that the write and flush cost
/// nothing next to the work being waited on.
const TICK: Duration = Duration::from_millis(120);

/// A running spinner. Stops and erases itself when dropped.
///
/// Dropping is the only way to stop it, so it cannot outlive the work it
/// describes even if that work returns early with `?` or panics.
pub struct Spinner {
    stop: Arc<AtomicBool>,
    /// `None` when the spinner is doing nothing, so there is no thread to join.
    handle: Option<JoinHandle<()>>,
}

impl Spinner {
    /// Start a spinner labelled `message`, or nothing at all if `enabled` is
    /// false.
    ///
    /// The caller decides `enabled` because the reasons to stay quiet — piped
    /// output, `--json`, `--quiet`, a log level that would scribble over the
    /// line anyway — are the caller's to know.
    pub fn start(enabled: bool, message: impl Into<String>) -> Self {
        if !enabled {
            return Self {
                stop: Arc::new(AtomicBool::new(true)),
                handle: None,
            };
        }
        Self::start_with(message, TICK, std::io::stderr())
    }

    /// The spinner proper, over any writer and tick length.
    ///
    /// Split out so tests can watch what gets painted without a terminal and
    /// without waiting a real tick for each frame.
    fn start_with(
        message: impl Into<String>,
        tick: Duration,
        mut writer: impl Write + Send + 'static,
    ) -> Self {
        let message = message.into();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let mut frame = 0usize;
            // What the last paint left on screen, so it can be wiped exactly.
            let mut painted = 0usize;

            loop {
                // Waiting first means work that finishes inside one tick never
                // paints, so a fast project does not flash a spinner at you.
                std::thread::park_timeout(tick);
                if flag.load(Ordering::Relaxed) {
                    break;
                }

                let line = line(FRAMES[frame % FRAMES.len()], &message, started.elapsed());
                // A dead terminal is not worth failing a sweep over.
                let _ = write!(writer, "\r{line}");
                let _ = writer.flush();
                painted = line.chars().count();
                frame += 1;
            }

            if painted > 0 {
                let _ = write!(writer, "\r{:painted$}\r", "");
                let _ = writer.flush();
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            // The thread is parked between frames; waking it means a drop
            // returns in microseconds rather than waiting out the tick.
            handle.thread().unpark();
            // Joining is what guarantees the line is erased before the caller
            // prints the result underneath it.
            let _ = handle.join();
        }
    }
}

/// One rendered progress line, without the carriage return that positions it.
fn line(frame: char, message: &str, elapsed: Duration) -> String {
    format!("{frame} {message}  {}", clock(elapsed))
}

/// Elapsed time, in the largest unit that keeps the number small.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
}

/// Whether a spinner would be visible and welcome on this stream.
///
/// Only a terminal gets one: a redirected stream keeps the carriage returns as
/// literal bytes, which is exactly the mess in a log file that `\r` avoids on
/// screen.
pub fn stderr_is_interactive() -> bool {
    std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A writer the test can read back while the spinner thread holds it.
    #[derive(Clone)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Shared {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Vec::new())))
        }

        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_disabled_spinner_paints_nothing_and_still_drops_cleanly() {
        let spinner = Spinner::start(false, "archiving thing");
        assert!(spinner.handle.is_none(), "no thread should have started");
        drop(spinner);
    }

    /// The point of the whole module: the line moves while the caller is busy.
    #[test]
    fn the_spinner_paints_while_the_caller_is_working() {
        let out = Shared::new();
        let spinner = Spinner::start_with("archiving thing", Duration::from_millis(2), out.clone());

        // Stand in for the work. The spinner thread paints throughout.
        std::thread::sleep(Duration::from_millis(80));
        drop(spinner);

        let painted = out.contents();
        assert!(painted.contains("archiving thing"), "{painted:?}");
        assert!(
            FRAMES.iter().filter(|f| painted.contains(**f)).count() > 1,
            "the spinner should have advanced through frames: {painted:?}"
        );
    }

    /// Whatever it painted must be gone, or it would sit above the result line
    /// as a permanent half-finished status.
    #[test]
    fn the_line_is_erased_when_the_spinner_stops() {
        let out = Shared::new();
        let spinner = Spinner::start_with("archiving thing", Duration::from_millis(2), out.clone());
        std::thread::sleep(Duration::from_millis(40));
        drop(spinner);

        let painted = out.contents();
        assert!(painted.ends_with('\r'), "{painted:?}");
        // The final paint is blanks: nothing of the message survives it.
        let last = painted.trim_end_matches('\r').rsplit('\r').next().unwrap();
        assert!(last.trim().is_empty(), "{last:?}");
    }

    /// Work shorter than a single tick should leave the terminal untouched.
    #[test]
    fn work_that_finishes_within_a_tick_never_paints() {
        let out = Shared::new();
        let spinner = Spinner::start_with("archiving thing", Duration::from_secs(30), out.clone());
        drop(spinner);
        assert_eq!(out.contents(), "");
    }

    #[test]
    fn the_clock_switches_to_minutes_once_there_are_some() {
        assert_eq!(clock(Duration::from_secs(0)), "0s");
        assert_eq!(clock(Duration::from_secs(59)), "59s");
        assert_eq!(clock(Duration::from_secs(60)), "1m00s");
        assert_eq!(clock(Duration::from_secs(605)), "10m05s");
    }

    #[test]
    fn a_painted_line_names_the_work_and_how_long_it_has_taken() {
        assert_eq!(
            line('/', "archiving thing", Duration::from_secs(3)),
            "/ archiving thing  3s"
        );
    }
}
