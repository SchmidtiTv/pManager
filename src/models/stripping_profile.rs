//! `<language>.profile.json`: how to recognise a project and what to strip.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementType {
    File,
    Folder,
}

/// One `{ "type", "element" }` entry. `element` is relative to the project
/// root; `stripping::validation` rejects anything that could leave it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    #[serde(rename = "type")]
    pub element_type: ElementType,
    pub element: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrippingProfile {
    pub language: String,
    pub detection: Vec<Element>,
    pub strippable: Vec<Element>,
}
