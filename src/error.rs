//! Error plumbing: one alias, one renderer.

use std::path::PathBuf;
use std::process::ExitCode;

/// Every fallible function in the crate returns this.
pub type Result<T = ()> = anyhow::Result<T>;

/// The path is not in the ledger. Has its own exit code so callers such as
/// the IDE plugin can tell "not managed" apart from a failure.
#[derive(Debug, thiserror::Error)]
#[error("no project is registered at {}", path.display())]
pub struct NotRegistered {
    pub path: PathBuf,
}

pub const NOT_REGISTERED_EXIT_CODE: u8 = 3;

/// Render a full error chain to the log. `{:#}` walks the `anyhow` causes.
pub fn report(err: &anyhow::Error) {
    tracing::error!("{err:#}");
}

pub fn exit_code(err: &anyhow::Error) -> ExitCode {
    if err.chain().any(|cause| cause.is::<NotRegistered>()) {
        ExitCode::from(NOT_REGISTERED_EXIT_CODE)
    } else {
        ExitCode::FAILURE
    }
}
