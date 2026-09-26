//! `.pmanagerignore`: the file a project uses to say which of its contents
//! are not work on it.
//!
//! The syntax is gitignore syntax, matched by the `ignore` crate -- the same
//! matcher `ripgrep` uses -- so a pattern that works in a `.gitignore` works
//! here. That is deliberate: the file exists so a user can hold a folder back
//! from counting as activity, and a second dialect they had to learn first
//! would be a worse tool than no file at all.
//!
//! This type knows nothing about walking a tree. It answers one question
//! about one path, so it can be tested without a traversal in the way.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::error::Result;

/// The loaded rules for one project.
///
/// `Debug` reports the pattern count rather than the compiled matcher, which
/// is a glob set and prints as pages of nothing useful.
pub struct IgnoreFile {
    matcher: Gitignore,
    patterns: usize,
}

impl IgnoreFile {
    /// The file's name, in the project root.
    pub const NAME: &'static str = ".pmanagerignore";

    /// Where [`load`](Self::load) looks for it.
    pub fn path(root: &Path) -> PathBuf {
        root.join(Self::NAME)
    }

    /// Read the rules for the project at `root`.
    ///
    /// A project with no `.pmanagerignore` is the normal case and gets rules
    /// that ignore nothing.
    ///
    /// A file that *is* there and cannot be used is an error, and the error
    /// names it. Dropping the bad line instead would archive a project the
    /// user believed was protected, which is the one outcome this file exists
    /// to prevent -- so a pattern that cannot be honoured stops the pass
    /// rather than being honoured silently as "matches nothing".
    pub fn load(root: &Path) -> Result<Self> {
        let file = Self::path(root);
        if !file.is_file() {
            return Ok(Self {
                matcher: Gitignore::empty(),
                patterns: 0,
            });
        }

        let mut builder = GitignoreBuilder::new(root);
        if let Some(err) = builder.add(&file) {
            return Err(err).with_context(|| format!("could not read {}", file.display()));
        }

        let matcher = builder
            .build()
            .with_context(|| format!("{} contains an unusable pattern", file.display()))?;

        tracing::debug!(
            "{} declares {} pattern(s)",
            file.display(),
            matcher.num_ignores()
        );

        Ok(Self {
            patterns: matcher.num_ignores() as usize,
            matcher,
        })
    }

    /// Whether these rules can ignore anything at all.
    ///
    /// A caller that walks a large tree checks this once and then skips
    /// asking about every entry, which is the common case: most projects
    /// have no `.pmanagerignore`.
    pub fn is_empty(&self) -> bool {
        self.patterns == 0
    }

    /// How many patterns were loaded.
    pub fn patterns(&self) -> usize {
        self.patterns
    }

    /// Whether `path` is ignored.
    ///
    /// `is_dir` has to be passed because gitignore semantics depend on it: a
    /// trailing slash, as in `build/`, matches a directory and never a file
    /// of the same name.
    ///
    /// This asks about `path` alone and does not consult its parents. A
    /// caller that prunes ignored directories as it walks -- which is the
    /// point of ignoring a directory -- never reaches a path whose parent was
    /// ignored, so the cheaper question is the right one.
    pub fn ignores(&self, path: &Path, is_dir: bool) -> bool {
        if self.is_empty() {
            return false;
        }

        self.matcher.matched(path, is_dir).is_ignore()
    }
}

impl fmt::Debug for IgnoreFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IgnoreFile")
            .field("patterns", &self.patterns)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::IgnoreFile;

    /// A project root holding `contents` as its `.pmanagerignore`.
    fn with_rules(tmp: &Path, contents: &str) -> IgnoreFile {
        std::fs::write(IgnoreFile::path(tmp), contents).unwrap();
        IgnoreFile::load(tmp).unwrap()
    }

    #[test]
    fn a_project_without_the_file_ignores_nothing() {
        let tmp = tempfile::tempdir().unwrap();

        let rules = IgnoreFile::load(tmp.path()).unwrap();

        assert!(rules.is_empty());
        assert_eq!(rules.patterns(), 0);
        assert!(!rules.ignores(&tmp.path().join("build"), true));
        assert!(!rules.ignores(&tmp.path().join("main.rs"), false));
    }

    #[test]
    fn the_file_is_looked_for_in_the_project_root() {
        assert_eq!(
            IgnoreFile::path(Path::new("/dev/demo")),
            Path::new("/dev/demo/.pmanagerignore")
        );
    }

    #[test]
    fn a_trailing_slash_matches_the_folder_and_not_a_file_of_that_name() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "build/\n");

        assert!(rules.ignores(&tmp.path().join("build"), true));
        assert!(
            !rules.ignores(&tmp.path().join("build"), false),
            "`build/` names a directory, so a file called `build` is not it"
        );
    }

    #[test]
    fn a_bare_name_matches_at_any_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "node_modules\n");

        assert!(rules.ignores(&tmp.path().join("node_modules"), true));
        assert!(
            rules.ignores(
                &tmp.path().join("packages").join("web").join("node_modules"),
                true
            ),
            "an unanchored pattern matches anywhere below the root"
        );
    }

    #[test]
    fn a_leading_slash_anchors_the_pattern_to_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "/target\n");

        assert!(rules.ignores(&tmp.path().join("target"), true));
        assert!(
            !rules.ignores(&tmp.path().join("crates").join("target"), true),
            "`/target` is the one at the root, not every target in the tree"
        );
    }

    #[test]
    fn a_glob_matches_by_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "*.log\n");

        assert!(rules.ignores(&tmp.path().join("debug.log"), false));
        assert!(!rules.ignores(&tmp.path().join("debug.txt"), false));
    }

    #[test]
    fn a_negation_takes_a_path_back_out() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "*.log\n!keep.log\n");

        assert!(rules.ignores(&tmp.path().join("debug.log"), false));
        assert!(
            !rules.ignores(&tmp.path().join("keep.log"), false),
            "a later negation must win, as it does in a .gitignore"
        );
    }

    #[test]
    fn comments_and_blank_lines_are_not_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "# generated output\n\nbuild/\n");

        assert_eq!(rules.patterns(), 1);
        assert!(!rules.ignores(&tmp.path().join("# generated output"), false));
    }

    #[test]
    fn a_file_with_no_patterns_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = with_rules(tmp.path(), "# nothing to say yet\n");

        assert!(
            rules.is_empty(),
            "a file of only comments can ignore nothing, so callers may skip it"
        );
    }

    #[test]
    fn an_unusable_pattern_names_the_file_the_user_has_to_fix() {
        let tmp = tempfile::tempdir().unwrap();
        // An unclosed alternate group: `ignore` rejects the line rather than
        // quietly matching nothing with it.
        std::fs::write(IgnoreFile::path(tmp.path()), "build/{\n").unwrap();

        let err = IgnoreFile::load(tmp.path()).unwrap_err();

        assert!(
            format!("{err:#}").contains(IgnoreFile::NAME),
            "got: {err:#}"
        );
    }
}
