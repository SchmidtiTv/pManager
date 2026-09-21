pub mod parsing;
pub mod validation;

use std::path::Path;

use anyhow::bail;

use crate::error::Result;
use crate::models::stripping_profile::StrippingProfile;

pub fn load(strip_dir: &Path) -> Result<Vec<StrippingProfile>> {
    let index = parsing::read_index(strip_dir)?;

    let mut problems: Vec<String> = validation::check_index(&index)
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut profiles = Vec::with_capacity(index.len());

    for entry in &index {
        if !validation::stays_inside(&entry.file) {
            continue;
        }

        match parsing::read_profile(&strip_dir.join(&entry.file)) {
            Ok(profile) => {
                problems.extend(
                    validation::check_profile(entry, &profile)
                        .iter()
                        .map(ToString::to_string),
                );
                profiles.push(profile);
            }
            Err(err) => problems.push(format!("{err:#}")),
        }
    }

    if problems.is_empty() {
        return Ok(profiles);
    }

    bail!(
        "the strip profiles in {} are invalid:\n  - {}",
        strip_dir.display(),
        problems.join("\n  - ")
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::load;
    use super::parsing::INDEX_FILE;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).unwrap();
    }

    fn profile(language: &str, strippable: &str) -> String {
        format!(
            r#"{{
                "language": "{language}",
                "detection": [{{ "type": "file", "element": "marker" }}],
                "strippable": [{{ "type": "folder", "element": "{strippable}" }}]
            }}"#
        )
    }

    #[test]
    fn profiles_are_loaded_in_index_order() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            INDEX_FILE,
            r#"[
                { "language": "python", "file": "./python.profile.json" },
                { "language": "node", "file": "node.profile.json" }
            ]"#,
        );
        write(
            tmp.path(),
            "python.profile.json",
            &profile("python", ".venv"),
        );
        write(
            tmp.path(),
            "node.profile.json",
            &profile("node", "node_modules"),
        );

        let profiles = load(tmp.path()).unwrap();

        let languages: Vec<&str> = profiles.iter().map(|p| p.language.as_str()).collect();
        assert_eq!(languages, vec!["python", "node"]);
    }

    #[test]
    fn an_empty_index_loads_no_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), INDEX_FILE, "[]");

        assert_eq!(load(tmp.path()).unwrap(), vec![]);
    }

    #[test]
    fn every_problem_across_profiles_is_reported_together() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            INDEX_FILE,
            r#"[
                { "language": "python", "file": "python.profile.json" },
                { "language": "node", "file": "node.profile.json" },
                { "language": "java", "file": "missing.profile.json" },
                { "language": "go", "file": "../go.profile.json" }
            ]"#,
        );
        write(tmp.path(), "python.profile.json", &profile("python", ".."));
        write(
            tmp.path(),
            "node.profile.json",
            &profile("deno", "node_modules"),
        );

        let err = format!("{:#}", load(tmp.path()).unwrap_err());

        assert!(err.contains("strippable element \"..\""), "got: {err}");
        assert!(err.contains("declares language \"deno\""), "got: {err}");
        assert!(err.contains("missing.profile.json"), "got: {err}");
        assert!(err.contains("\"../go.profile.json\""), "got: {err}");
    }

    #[test]
    fn an_index_entry_outside_the_strip_directory_is_never_read() {
        let tmp = tempfile::tempdir().unwrap();
        let strip_dir = tmp.path().join("strip");
        std::fs::create_dir(&strip_dir).unwrap();
        write(
            &strip_dir,
            INDEX_FILE,
            r#"[{ "language": "go", "file": "../go.profile.json" }]"#,
        );
        // Valid on its own: the only complaint must be about its location.
        write(tmp.path(), "go.profile.json", &profile("go", "vendor"));

        let err = format!("{:#}", load(&strip_dir).unwrap_err());

        assert!(err.contains("inside the strip directory"), "got: {err}");
        assert_eq!(err.matches("\n  - ").count(), 1, "got: {err}");
    }
}
