use anyhow::bail;

use super::Scheduler;
use crate::error::Result;

/// A `pman-run.timer` under `~/.config/systemd/user/`.
pub struct SystemdUserTimer;

impl Scheduler for SystemdUserTimer {
    fn install(&self) -> Result<()> {
        bail!("the systemd scheduler is not implemented yet")
    }

    fn remove(&self) -> Result<()> {
        bail!("the systemd scheduler is not implemented yet")
    }
}
