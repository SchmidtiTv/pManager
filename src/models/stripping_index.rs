//! `profiles.json`: which profile file serves which language.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrippingIndexElement {
    pub language: String,
    /// Relative to the strip directory, e.g. `./python.profile.json`.
    pub file: PathBuf,
}
