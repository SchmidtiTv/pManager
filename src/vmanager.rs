//! Whether vManager is installed alongside pManager.
//!
//! pManager ages project dependency trees out; vManager ages old tool
//! versions out. They are separate binaries with separate databases and
//! nothing here has either one act on the other's behalf yet — this module
//! is the first step towards that: telling whether the `vmanager` binary can
//! be run at all.
//!
//! Detection works the same way [`crate::git`] detects git: there is no
//! separate "is it installed" check, just running the command and treating
//! every way that can fail — not installed, not on PATH, refusing to run —
//! as "not available" rather than as an error.

use std::process::Command;

/// The version string `vmanager --version` printed, or `None` if it could
/// not be run.
pub fn version() -> Option<String> {
    version_of("vmanager")
}

/// Whether the `vmanager` binary can be run from PATH.
pub fn is_available() -> bool {
    version().is_some()
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
}
