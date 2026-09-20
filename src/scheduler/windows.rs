use anyhow::bail;

use super::Scheduler;
use crate::error::Result;

/// A scheduled task registered through `schtasks`.
pub struct TaskScheduler;

impl Scheduler for TaskScheduler {
    fn install(&self) -> Result<()> {
        bail!("the Windows Task Scheduler backend is not implemented yet")
    }

    fn remove(&self) -> Result<()> {
        bail!("the Windows Task Scheduler backend is not implemented yet")
    }
}
