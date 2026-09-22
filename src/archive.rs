//! Packing a project folder into a `.pproject`, and checking the result.
//!
//! The format is plain 7z with LZMA2 and no password. Decoding is not behind
//! a cargo feature in `sevenz-rust2` -- only the encoder is -- so [`verify`]
//! works with the trimmed feature set in `Cargo.toml`, and a future
//! `pman restore` will need no manifest change either.

use std::path::Path;

use anyhow::{Context as _, ensure};
use walkdir::WalkDir;

use crate::error::Result;

/// Pack everything under `root` into a 7z archive at `dest`.
///
/// Paths inside the archive are relative to `root`, so unpacking into an
/// empty folder reproduces the project rather than nesting it.
pub fn pack(root: &Path, dest: &Path) -> Result<()> {
    sevenz_rust2::compress_to_path(root, dest)
        .with_context(|| format!("could not pack {} into {}", root.display(), dest.display()))
}

/// The number of entries [`pack`] will write for `root`.
///
/// Every file and every folder below `root` becomes one entry; `root` itself
/// does not, because it is the archive. Symlinks are neither followed nor
/// packed, so they are not counted.
pub fn entry_count(root: &Path) -> Result<usize> {
    let mut count = 0;

    for entry in WalkDir::new(root) {
        let entry = entry.with_context(|| format!("could not read {}", root.display()))?;

        if entry.depth() > 0 && (entry.file_type().is_dir() || entry.file_type().is_file()) {
            count += 1;
        }
    }

    Ok(count)
}

/// Read `archive` back and confirm it holds `expected` entries.
///
/// This is the step that earns the right to delete the source folder. Reading
/// the header back proves the file is a complete, parseable 7z archive and
/// not a truncated write, and the count proves nothing was dropped on the way
/// in.
pub fn verify(archive: &Path, expected: usize) -> Result<()> {
    let read = sevenz_rust2::Archive::open(archive)
        .with_context(|| format!("could not read back the archive {}", archive.display()))?;

    ensure!(
        read.files.len() == expected,
        "{} holds {} entries but {expected} were packed",
        archive.display(),
        read.files.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{entry_count, pack, verify};

    fn tree(root: &Path) {
        std::fs::create_dir_all(root.join("src").join("deep")).unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "pub fn a() {}").unwrap();
        std::fs::write(root.join("src").join("deep").join("b.rs"), "// b").unwrap();
    }

    #[test]
    fn a_packed_tree_reads_back_with_every_entry_at_its_relative_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        tree(&root);
        let dest = tmp.path().join("project.pproject");

        pack(&root, &dest).unwrap();

        let read = sevenz_rust2::Archive::open(&dest).unwrap();
        let mut names: Vec<PathBuf> = read
            .files
            .iter()
            .map(|entry| PathBuf::from(entry.name()))
            .collect();
        names.sort();

        assert_eq!(
            names,
            vec![
                PathBuf::from("main.rs"),
                PathBuf::from("src"),
                PathBuf::from("src/deep"),
                PathBuf::from("src/deep/b.rs"),
                PathBuf::from("src/lib.rs"),
            ],
            "entries must be relative to the project root, not absolute"
        );
    }

    #[test]
    fn the_count_walked_before_packing_is_the_count_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        tree(&root);
        let dest = tmp.path().join("project.pproject");

        let expected = entry_count(&root).unwrap();
        pack(&root, &dest).unwrap();

        verify(&dest, expected).expect("a freshly packed archive must verify");
    }

    #[test]
    fn a_count_that_does_not_match_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        tree(&root);
        let dest = tmp.path().join("project.pproject");
        pack(&root, &dest).unwrap();

        let err = verify(&dest, 99).unwrap_err();

        assert!(
            format!("{err:#}").contains("but 99 were packed"),
            "got: {err:#}"
        );
    }

    #[test]
    fn a_truncated_archive_does_not_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        tree(&root);
        let dest = tmp.path().join("project.pproject");
        let expected = entry_count(&root).unwrap();
        pack(&root, &dest).unwrap();

        let packed = std::fs::read(&dest).unwrap();
        std::fs::write(&dest, &packed[..packed.len() / 2]).unwrap();

        assert!(
            verify(&dest, expected).is_err(),
            "a half-written archive must never be accepted as complete"
        );
    }

    #[test]
    fn an_empty_project_packs_and_verifies_as_zero_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let dest = tmp.path().join("project.pproject");

        let expected = entry_count(&root).unwrap();
        assert_eq!(expected, 0);

        pack(&root, &dest).unwrap();
        verify(&dest, expected).unwrap();
    }
}
