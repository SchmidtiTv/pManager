//! Error plumbing: one alias, one renderer.

/// Every fallible function in the crate returns this.
pub type Result<T = ()> = anyhow::Result<T>;

/// Render a full error chain to the log. `{:#}` walks the `anyhow` causes.
pub fn report(err: &anyhow::Error) {
    tracing::error!("{err:#}");
}
