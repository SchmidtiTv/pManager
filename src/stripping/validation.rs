use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::models::stripping_index::StrippingIndexElement;
use crate::models::stripping_profile::{Element, StrippingProfile};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("language {language:?} is listed more than once in the index")]
    DuplicateLanguage { language: String },

    #[error(
        "profile file {file:?} for {language:?} must be a relative path inside the strip directory"
    )]
    FileOutsideStripDir { language: String, file: PathBuf },

    #[error("{file:?} is indexed as {indexed:?} but declares language {declared:?}")]
    LanguageMismatch {
        file: PathBuf,
        indexed: String,
        declared: String,
    },

    #[error(
        "{list} element {element:?} in the {language:?} profile must be a relative path inside the project"
    )]
    ElementOutsideRoot {
        language: String,
        list: &'static str,
        element: PathBuf,
    },
}

pub fn check_index(index: &[StrippingIndexElement]) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    for entry in index {
        if !seen.insert(entry.language.as_str()) {
            errors.push(ValidationError::DuplicateLanguage {
                language: entry.language.clone(),
            });
        }

        if !stays_inside(&entry.file) {
            errors.push(ValidationError::FileOutsideStripDir {
                language: entry.language.clone(),
                file: entry.file.clone(),
            });
        }
    }

    errors
}

pub fn check_profile(
    entry: &StrippingIndexElement,
    profile: &StrippingProfile,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if profile.language != entry.language {
        errors.push(ValidationError::LanguageMismatch {
            file: entry.file.clone(),
            indexed: entry.language.clone(),
            declared: profile.language.clone(),
        });
    }

    let lists = [
        ("detection", &profile.detection),
        ("strippable", &profile.strippable),
    ];
    for (list, elements) in lists {
        errors.extend(elements_outside_root(&profile.language, list, elements));
    }

    errors
}

fn elements_outside_root<'a>(
    language: &'a str,
    list: &'static str,
    elements: &'a [Element],
) -> impl Iterator<Item = ValidationError> + 'a {
    elements
        .iter()
        .filter(|element| !stays_inside(&element.element))
        .map(move |element| ValidationError::ElementOutsideRoot {
            language: language.to_string(),
            list,
            element: element.element.clone(),
        })
}

pub fn stays_inside(path: &Path) -> bool {
    let mut names_something = false;

    for component in path.components() {
        match component {
            Component::Normal(_) => names_something = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }

    names_something
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{ValidationError, check_index, check_profile, stays_inside};
    use crate::models::stripping_index::StrippingIndexElement;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

    fn entry(language: &str, file: &str) -> StrippingIndexElement {
        StrippingIndexElement {
            language: language.to_string(),
            file: PathBuf::from(file),
        }
    }

    fn folder(element: &str) -> Element {
        Element {
            element_type: ElementType::Folder,
            element: PathBuf::from(element),
        }
    }

    fn profile(language: &str, strippable: &[&str]) -> StrippingProfile {
        StrippingProfile {
            language: language.to_string(),
            detection: vec![folder(".venv")],
            strippable: strippable.iter().map(|element| folder(element)).collect(),
        }
    }

    #[test]
    fn relative_paths_below_the_root_stay_inside() {
        for path in [
            "node_modules",
            "./python.profile.json",
            "build/cache",
            ".venv",
        ] {
            assert!(stays_inside(Path::new(path)), "{path} should be accepted");
        }
    }

    #[test]
    fn paths_that_reach_the_root_or_beyond_are_rejected() {
        for path in ["", ".", "./", "..", "../x", "a/../../x", "/home", "/"] {
            assert!(!stays_inside(Path::new(path)), "{path} should be rejected");
        }
    }

    #[test]
    fn a_clean_index_passes() {
        let index = [
            entry("python", "./python.profile.json"),
            entry("java", "java.profile.json"),
        ];

        assert_eq!(check_index(&index), vec![]);
    }

    #[test]
    fn a_language_indexed_twice_is_reported_once_per_repeat() {
        let index = [
            entry("python", "a.profile.json"),
            entry("python", "b.profile.json"),
        ];

        assert_eq!(
            check_index(&index),
            vec![ValidationError::DuplicateLanguage {
                language: "python".to_string(),
            }]
        );
    }

    #[test]
    fn an_index_entry_outside_the_strip_directory_is_reported() {
        let index = [entry("python", "../elsewhere/python.profile.json")];

        assert_eq!(
            check_index(&index),
            vec![ValidationError::FileOutsideStripDir {
                language: "python".to_string(),
                file: PathBuf::from("../elsewhere/python.profile.json"),
            }]
        );
    }

    #[test]
    fn a_matching_profile_passes() {
        let errors = check_profile(
            &entry("python", "python.profile.json"),
            &profile("python", &[".venv"]),
        );

        assert_eq!(errors, vec![]);
    }

    #[test]
    fn a_profile_declaring_another_language_is_reported() {
        let errors = check_profile(
            &entry("python", "python.profile.json"),
            &profile("java", &["build"]),
        );

        assert_eq!(
            errors,
            vec![ValidationError::LanguageMismatch {
                file: PathBuf::from("python.profile.json"),
                indexed: "python".to_string(),
                declared: "java".to_string(),
            }]
        );
    }

    #[test]
    fn every_strippable_element_outside_the_project_is_reported() {
        let errors = check_profile(
            &entry("python", "python.profile.json"),
            &profile("python", &[".venv", "..", "/home", "."]),
        );

        let rejected: Vec<PathBuf> = errors
            .into_iter()
            .map(|error| match error {
                ValidationError::ElementOutsideRoot { list, element, .. } => {
                    assert_eq!(list, "strippable");
                    element
                }
                other => panic!("unexpected error: {other}"),
            })
            .collect();
        assert_eq!(
            rejected,
            vec![
                PathBuf::from(".."),
                PathBuf::from("/home"),
                PathBuf::from(".")
            ]
        );
    }
}
