//! Which loaded profiles apply to a project root.

use std::path::Path;

use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

/// Every profile whose `detection` list recognises `root`.
///
/// More than one can match, and all of them are used: a repository holding a
/// Python service and its TypeScript client has both `.venv` and
/// `node_modules` to strip, and picking a single winner would leave one of
/// them on disk forever.
pub fn matching(root: &Path, profiles: &[StrippingProfile]) -> Vec<StrippingProfile> {
    profiles
        .iter()
        .filter(|profile| matches(root, profile))
        .cloned()
        .collect()
}

/// A profile matches when *any* of its detection entries is present. The
/// entries are alternatives -- a Python project has a `pyproject.toml` or a
/// `.venv`, and need not have both.
pub fn matches(root: &Path, profile: &StrippingProfile) -> bool {
    profile
        .detection
        .iter()
        .any(|element| present(root, element))
}

fn present(root: &Path, element: &Element) -> bool {
    let path = root.join(&element.element);

    match element.element_type {
        ElementType::File => path.is_file(),
        ElementType::Folder => path.is_dir(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::matching;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

    fn element(element_type: ElementType, element: &str) -> Element {
        Element {
            element_type,
            element: PathBuf::from(element),
        }
    }

    fn profile(language: &str, detection: Vec<Element>) -> StrippingProfile {
        StrippingProfile {
            language: language.to_string(),
            detection,
            strippable: vec![element(ElementType::Folder, "build")],
        }
    }

    fn python() -> StrippingProfile {
        profile(
            "python",
            vec![
                element(ElementType::File, "pyproject.toml"),
                element(ElementType::Folder, ".venv"),
            ],
        )
    }

    fn node() -> StrippingProfile {
        profile("node", vec![element(ElementType::File, "package.json")])
    }

    fn languages(root: &Path, profiles: &[StrippingProfile]) -> Vec<String> {
        matching(root, profiles)
            .into_iter()
            .map(|profile| profile.language)
            .collect()
    }

    #[test]
    fn a_root_with_no_marker_matches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();

        assert_eq!(
            languages(tmp.path(), &[python(), node()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn any_one_detection_entry_is_enough() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".venv")).unwrap();

        assert_eq!(
            languages(tmp.path(), &[python(), node()]),
            vec!["python".to_string()],
            "the second detection entry alone must match"
        );
    }

    #[test]
    fn every_matching_profile_is_returned_not_just_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "[project]").unwrap();
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();

        assert_eq!(
            languages(tmp.path(), &[python(), node()]),
            vec!["python".to_string(), "node".to_string()]
        );
    }

    #[test]
    fn a_marker_of_the_wrong_type_does_not_match() {
        let tmp = tempfile::tempdir().unwrap();
        // A folder called pyproject.toml, and a file called .venv: both names
        // are right and both types are wrong.
        std::fs::create_dir(tmp.path().join("pyproject.toml")).unwrap();
        std::fs::write(tmp.path().join(".venv"), "not a folder").unwrap();

        assert_eq!(languages(tmp.path(), &[python()]), Vec::<String>::new());
    }
}
