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

#[cfg(test)]
mod tests {
    use super::for_this_platform;

    /// `for_this_platform` and the `Scheduler` it returns are reachable from
    /// nothing else yet — real scheduling is a later spec. This test pins the
    /// trait shape and confirms the stub fails loudly, naming the systemd
    /// backend specifically, rather than silently succeeding or failing for
    /// some unrelated reason.
    #[test]
    fn the_platform_scheduler_is_honestly_unimplemented() {
        let scheduler =
            for_this_platform().expect("a scheduler should be available on this platform");

        let install_err = scheduler
            .install()
            .expect_err("install must not silently succeed");
        assert!(
            format!("{install_err:#}").contains("the systemd scheduler is not implemented yet"),
            "install error should name the systemd scheduler as not implemented, got: {install_err:#}"
        );

        let remove_err = scheduler
            .remove()
            .expect_err("remove must not silently succeed");
        assert!(
            format!("{remove_err:#}").contains("the systemd scheduler is not implemented yet"),
            "remove error should name the systemd scheduler as not implemented, got: {remove_err:#}"
        );
    }
}
