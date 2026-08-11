//! Talking to vManager: whether it is installed, and switching the versions
//! it manages on pManager's behalf.
//!
//! pManager ages project dependency trees out; vManager ages old tool
//! versions out. They stay separate binaries with separate databases —
//! nothing here reads vManager's database or writes to its links directly.
//! The one thing that crosses the boundary is `vmanager use`: pManager knows
//! which version a project wants (see [`crate::db::VersionPin`]), vManager
//! knows how to make a fixed path resolve to it, and [`switch`] is the
//! subprocess call that connects the two. See `commands::r#use`.
//!
//! Detection works the same way [`crate::git`] detects git: there is no
//! separate "is it installed" check, just running the command and treating
//! every way that can fail — not installed, not on PATH, refusing to run —
//! as "not available" rather than as an error.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// The version string `vmanager --version` printed, or `None` if it could
/// not be run.
pub fn version() -> Option<String> {
    version_of("vmanager")
}

/// Whether the `vmanager` binary can be run from PATH.
pub fn is_available() -> bool {
    version().is_some()
}

/// Ask vManager to point `folder`'s `current` link at `version`.
///
/// Streams vmanager's own stdout/stderr straight through rather than
/// capturing them — `vmanager use` is a one-line report either way, and
/// pManager has nothing useful to add to it. Mirrors how `restore.rs` runs a
/// project's install command.
pub fn switch(folder: &Path, version: &str) -> Result<()> {
    switch_with("vmanager", folder, version)
}

/// [`switch`], with the binary name as a parameter so tests can point it at
/// something other than the real `vmanager`.
fn switch_with(binary: &str, folder: &Path, version: &str) -> Result<()> {
    let status = Command::new(binary)
        .arg("use")
        .arg(folder)
        .arg(version)
        .status()
        .map_err(|err| Error::Other(anyhow::anyhow!("could not run vmanager: {err}")))?;

    if !status.success() {
        return Err(Error::Other(anyhow::anyhow!(
            "vmanager exited with {status}"
        )));
    }
    Ok(())
}

/// Runs `<binary> --version` and returns its trimmed stdout, or `None` for
/// every way that can fail. Takes the binary name as a parameter so tests can
/// point it at something other than the real `vmanager`.
fn version_of(binary: &str) -> Option<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .inspect_err(|err| tracing::debug!(error = %err, "could not run vmanager"))
        .ok()?;

    if !output.status.success() {
        tracing::trace!(status = ?output.status, "vmanager --version failed");
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.trim_end_matches(['\n', '\r']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_not_on_path_is_reported_rather_than_failing() {
        assert_eq!(
            version_of("pmanager-vmanager-detection-test-does-not-exist"),
            None
        );
    }

    #[test]
    fn a_binary_on_path_that_prints_a_version_is_reported() {
        // Any binary works here — this is testing the parsing and exit-code
        // handling in `version_of`, not vmanager specifically. `cargo` is on
        // PATH in every environment that can build this crate at all.
        assert!(version_of("cargo").is_some());
    }

    #[test]
    fn switching_with_a_binary_not_on_path_is_an_error() {
        let err = switch_with(
            "pmanager-vmanager-detection-test-does-not-exist",
            Path::new("/tools/node"),
            "20.11",
        )
        .unwrap_err();
        assert!(err.to_string().contains("could not run vmanager"), "{err}");
    }
}
