//! How much space something takes.
//!
//! Both halves of a pass need this: stripping reports what deleting an entry
//! reclaimed, and archiving reports the source folder it replaced. It lives
//! here rather than in either of them so there is one walker, not two.

use std::path::Path;

use walkdir::WalkDir;

/// The total size of every file under `path`, or of `path` itself when it is
/// a file.
///
/// This never fails. The number it produces is a figure in a report, not a
/// precondition for deleting anything, so an unreadable entry is logged and
/// counted as zero rather than failing a project whose strip would otherwise
/// have succeeded. Symlinks are not followed, so a link out of the project
/// contributes its own size and not its target's.
pub fn size_of(path: &Path) -> u64 {
    let mut total: u64 = 0;

    for entry in WalkDir::new(path) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!("skipping unreadable entry while measuring: {err}");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        match entry.metadata() {
            Ok(metadata) => total = total.saturating_add(metadata.len()),
            Err(err) => tracing::warn!("could not measure {}: {err}", entry.path().display()),
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::size_of;

    #[test]
    fn a_folder_is_the_sum_of_every_file_below_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), vec![0u8; 100]).unwrap();
        std::fs::create_dir(tmp.path().join("deep")).unwrap();
        std::fs::write(tmp.path().join("deep").join("b"), vec![0u8; 50]).unwrap();

        assert_eq!(size_of(tmp.path()), 150);
    }

    #[test]
    fn a_single_file_measures_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a");
        std::fs::write(&file, vec![0u8; 42]).unwrap();

        assert_eq!(size_of(&file), 42);
    }

    #[test]
    fn something_that_is_not_there_measures_zero() {
        let tmp = tempfile::tempdir().unwrap();

        assert_eq!(size_of(&tmp.path().join("nope")), 0);
    }
}
