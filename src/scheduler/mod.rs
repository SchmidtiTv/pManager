//! Installing pManager's periodic `run` into the host's scheduler.
//!
//! The implementations are deliberately empty: scheduling is specified
//! separately. The trait and the platform split exist now so that the
//! `auto-run` commands have a stable thing to call.

#[cfg(unix)]
pub mod systemd;
#[cfg(windows)]
pub mod windows;

use crate::error::Result;

pub trait Scheduler {
    /// Install the recurring `pman run`.
    fn install(&self) -> Result<()>;

    /// Remove it again.
    fn remove(&self) -> Result<()>;
}

/// The scheduler for the platform this binary was built for.
pub fn for_this_platform() -> Result<Box<dyn Scheduler>> {
    #[cfg(unix)]
    {
        Ok(Box::new(systemd::SystemdUserTimer))
    }

    #[cfg(windows)]
    {
        Ok(Box::new(self::windows::TaskScheduler))
    }

    #[cfg(not(any(unix, windows)))]
    {
        anyhow::bail!("no scheduler is available for this platform")
    }
}
