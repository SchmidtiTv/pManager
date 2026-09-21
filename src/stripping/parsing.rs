
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::Result;
use crate::models::stripping_index::StrippingIndexElement;
use crate::models::stripping_profile::StrippingProfile;

pub const INDEX_FILE: &str = "profiles.json";

const INDEX_SCHEMA: &str = include_str!("../../docs/schemas/stripping_index.schema.json");
const PROFILE_SCHEMA: &str = include_str!("../../docs/schemas/stripping_profile.schema.json");

pub fn read_index(strip_dir: &Path) -> Result<Vec<StrippingIndexElement>> {
    read_checked(&strip_dir.join(INDEX_FILE), INDEX_SCHEMA)
}

pub fn read_profile(path: &Path) -> Result<StrippingProfile> {
    read_checked(path, PROFILE_SCHEMA)
}

fn read_checked<T: DeserializeOwned>(path: &Path, schema: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let document: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;

    check_schema(path, &document, schema)?;

    serde_json::from_value(document)
        .with_context(|| format!("{} could not be read as a strip file", path.display()))
}

fn check_schema(path: &Path, document: &Value, schema: &str) -> Result<()> {
    let schema: Value = serde_json::from_str(schema).context("compiled-in schema is not JSON")?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|err| anyhow!("compiled-in schema is invalid: {err}"))?;

    let violations: Vec<String> = validator
        .iter_errors(document)
        .map(|err| {
            let pointer = err.instance_path().to_string();
            let pointer = if pointer.is_empty() { "/" } else { &pointer };
            format!("{pointer}: {err}")
        })
        .collect();

    if violations.is_empty() {
        return Ok(());
    }

    bail!(
        "{} does not match its schema: {}",
        path.display(),
        violations.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{INDEX_FILE, read_index, read_profile};
    use crate::models::stripping_index::StrippingIndexElement;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

    const PYTHON: &str = r#"{
        "language": "python",
        "detection": [{ "type": "file", "element": "pyproject.toml" }],
        "strippable": [
            { "type": "folder", "element": ".venv" },
            { "type": "file", "element": "uv.lock" }
        ]
    }"#;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_valid_profile_is_read_into_the_model() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "python.profile.json", PYTHON);

        let profile = read_profile(&path).unwrap();

        assert_eq!(
            profile,
            StrippingProfile {
                language: "python".to_string(),
                detection: vec![Element {
                    element_type: ElementType::File,
                    element: PathBuf::from("pyproject.toml"),
                }],
                strippable: vec![
                    Element {
                        element_type: ElementType::Folder,
                        element: PathBuf::from(".venv"),
                    },
                    Element {
                        element_type: ElementType::File,
                        element: PathBuf::from("uv.lock"),
                    },
                ],
            }
        );
    }

    #[test]
    fn a_valid_index_is_read_from_the_strip_directory() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            INDEX_FILE,
            r#"[{ "language": "python", "file": "./python.profile.json" }]"#,
        );

        let index = read_index(tmp.path()).unwrap();

        assert_eq!(
            index,
            vec![StrippingIndexElement {
                language: "python".to_string(),
                file: PathBuf::from("./python.profile.json"),
            }]
        );
    }

    #[test]
    fn a_missing_index_names_the_file_it_looked_for() {
        let tmp = tempfile::tempdir().unwrap();

        let err = read_index(tmp.path()).unwrap_err();

        assert!(format!("{err:#}").contains(INDEX_FILE), "got: {err:#}");
    }

    #[test]
    fn malformed_json_names_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "broken.profile.json", "{ \"language\": ");

        let err = read_profile(&path).unwrap_err();

        assert!(
            format!("{err:#}").contains("broken.profile.json is not valid JSON"),
            "got: {err:#}"
        );
    }

    #[test]
    fn every_schema_violation_is_reported_with_its_location() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            "bad.profile.json",
            r#"{
                "language": "python",
                "detection": [{ "type": "dir", "element": "src" }],
                "strippable": []
            }"#,
        );

        let err = format!("{:#}", read_profile(&path).unwrap_err());

        assert!(err.contains("does not match its schema"), "got: {err}");
        assert!(err.contains("/detection/0/type"), "got: {err}");
        assert!(err.contains("/strippable"), "got: {err}");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            "extra.profile.json",
            r#"{
                "language": "python",
                "detection": [{ "type": "file", "element": "pyproject.toml" }],
                "strippable": [{ "type": "folder", "element": ".venv" }],
                "strip": true
            }"#,
        );

        let err = format!("{:#}", read_profile(&path).unwrap_err());

        assert!(err.contains("strip"), "got: {err}");
    }
}
