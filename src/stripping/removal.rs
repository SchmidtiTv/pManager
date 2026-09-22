//! Deleting what the matched profiles declare strippable.

use std::path::Path;

use anyhow::Context as _;

use crate::disk;
use crate::error::Result;
use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

/// Remove every `strippable` entry of every matched profile from `root`, and
/// report the total size of what was removed.
///
/// The entries were checked by [`crate::stripping::validation`] when the
/// profiles were loaded, so joining one onto `root` cannot escape it.
///
/// An entry that is not there is not an error: a profile lists what *may* be
/// stripped, and a Python project that never created a `.venv` is normal.
/// Anything else -- a permission error, a folder held open by an editor --
/// fails the project, so the pass does not report space it did not reclaim.
pub fn strip(root: &Path, profiles: &[StrippingProfile]) -> Result<u64> {
    let mut freed: u64 = 0;

    for profile in profiles {
        for element in &profile.strippable {
            let removed = remove(root, element)
                .with_context(|| format!("stripping the {:?} profile", profile.language))?;

            freed = freed.saturating_add(removed);
        }
    }

    Ok(freed)
}

/// Size is measured before the entry is deleted, because afterwards there is
/// nothing left to measure.
fn remove(root: &Path, element: &Element) -> Result<u64> {
    let path = root.join(&element.element);

    let present = match element.element_type {
        ElementType::File => path.is_file(),
        ElementType::Folder => path.is_dir(),
    };
    if !present {
        tracing::debug!("nothing to strip at {}", path.display());
        return Ok(0);
    }

    let size = disk::size_of(&path);

    match element.element_type {
        ElementType::File => std::fs::remove_file(&path),
        ElementType::Folder => std::fs::remove_dir_all(&path),
    }
    .with_context(|| format!("could not remove {}", path.display()))?;

    tracing::debug!("stripped {} ({size} bytes)", path.display());
    Ok(size)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::strip;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

    fn element(element_type: ElementType, element: &str) -> Element {
        Element {
            element_type,
            element: PathBuf::from(element),
        }
    }

    fn profile(strippable: Vec<Element>) -> StrippingProfile {
        StrippingProfile {
            language: "python".to_string(),
            detection: vec![element(ElementType::File, "pyproject.toml")],
            strippable,
        }
    }

    #[test]
    fn a_strippable_folder_is_removed_and_its_size_reported() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".venv")).unwrap();
        std::fs::write(tmp.path().join(".venv").join("lib"), vec![0u8; 300]).unwrap();
        std::fs::write(tmp.path().join("main.py"), "print()").unwrap();

        let freed = strip(
            tmp.path(),
            &[profile(vec![element(ElementType::Folder, ".venv")])],
        )
        .unwrap();

        assert_eq!(freed, 300);
        assert!(!tmp.path().join(".venv").exists());
        assert!(
            tmp.path().join("main.py").exists(),
            "only listed entries may be removed"
        );
    }

    #[test]
    fn every_matched_profile_contributes_to_the_total() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".venv")).unwrap();
        std::fs::write(tmp.path().join(".venv").join("lib"), vec![0u8; 200]).unwrap();
        std::fs::write(tmp.path().join("uv.lock"), vec![0u8; 100]).unwrap();

        let freed = strip(
            tmp.path(),
            &[
                profile(vec![element(ElementType::Folder, ".venv")]),
                profile(vec![element(ElementType::File, "uv.lock")]),
            ],
        )
        .unwrap();

        assert_eq!(freed, 300);
    }

    #[test]
    fn an_entry_that_is_not_there_is_normal_and_frees_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.py"), "print()").unwrap();

        let freed = strip(
            tmp.path(),
            &[profile(vec![element(ElementType::Folder, ".venv")])],
        )
        .unwrap();

        assert_eq!(freed, 0);
    }

    #[test]
    fn an_entry_of_the_wrong_type_on_disk_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        // The profile says `.venv` is a folder; here it is a file. Deleting
        // it would be acting on a guess about something the profile does not
        // actually describe.
        std::fs::write(tmp.path().join(".venv"), "surprise").unwrap();

        let freed = strip(
            tmp.path(),
            &[profile(vec![element(ElementType::Folder, ".venv")])],
        )
        .unwrap();

        assert_eq!(freed, 0);
        assert!(tmp.path().join(".venv").exists());
    }

    /// Unix-only: a read-only parent is the portable way to make a delete
    /// fail on demand. Windows has no equivalent that a test can arrange
    /// without holding the folder open from another process.
    #[cfg(unix)]
    #[test]
    fn a_failure_names_the_path_and_the_profile() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".venv")).unwrap();
        std::fs::write(root.join(".venv").join("lib"), "x").unwrap();

        let original = std::fs::metadata(&root).unwrap().permissions();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();

        let failed = strip(
            &root,
            &[profile(vec![element(ElementType::Folder, ".venv")])],
        );

        std::fs::set_permissions(&root, original).unwrap();

        let err = format!("{:#}", failed.unwrap_err());
        assert!(err.contains(".venv"), "the path must be named, got: {err}");
        assert!(
            err.contains("python"),
            "the profile must be named, got: {err}"
        );
    }
}
