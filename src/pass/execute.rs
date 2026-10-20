//! The only code in a pass that deletes anything.
//!
//! Nothing here takes a dry-run flag. `--dry-run` is the absence of a call to
//! [`apply`], which is what makes the dry run trustworthy: there is no mode
//! in which this file half-runs.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

use crate::archive;
use crate::database::sql_lite::SqlLite;
use crate::disk;
use crate::error::Result;
use crate::models::project::{self, Project};
use crate::models::state::ProjectState;
use crate::pass::facts::Facts;
use crate::pass::plan::Action;

/// The extension a packed project gets, and the suffix it wears while the
/// pack is still in progress.
const ARCHIVE_EXTENSION: &str = "pproject";
const PARTIAL_SUFFIX: &str = ".part";

/// Carry out `action` for `project` and return the bytes it reclaimed.
///
/// An error leaves as little behind as the ordering allows and never a
/// half-written archive standing in for a project folder; see [`archive_it`].
pub fn apply(project: &Project, action: &Action, facts: &Facts, db: &SqlLite) -> Result<u64> {
    match action {
        Action::Nothing { .. } => Ok(0),
        Action::Strip => strip_it(project, facts, db),
        Action::Archive => archive_it(project, facts, db),
        // Stripping is committed to the ledger before the archive starts, so
        // that an archive failure still records the work that did happen and
        // the next pass does not strip a second time.
        Action::StripAndArchive => {
            let stripped = strip_it(project, facts, db)?;
            Ok(stripped.saturating_add(archive_it(project, facts, db)?))
        }
    }
}

fn strip_it(project: &Project, facts: &Facts, db: &SqlLite) -> Result<u64> {
    let freed = crate::stripping::removal::strip(&project.path, &facts.matched)
        .with_context(|| format!("could not strip `{}`", project.name))?;

    write_back(project, facts, ProjectState::Stripped, db)?;

    tracing::info!("stripped `{}`", project.name);
    Ok(freed)
}

/// The ordering here is the whole point, because a pass can be killed at any
/// line of it:
///
/// 1. pack to `<name>.pproject.part`
/// 2. verify the result by reading it back
/// 3. rename it into place
/// 4. delete the source folder
/// 5. record the new state
///
/// No step can lose data. A crash before 3 leaves the folder intact and a
/// `.part` file the next pass overwrites. A crash between 3 and 4 leaves both
/// a verified archive and the folder, and the next pass simply repacks. The
/// one narrow window is between 4 and 5 -- a single UPDATE wide -- after
/// which the next pass reports the project as `Nothing { Missing }` rather
/// than inventing a repair.
fn archive_it(project: &Project, facts: &Facts, db: &SqlLite) -> Result<u64> {
    let destination = destination(project)?;
    let partial = partial(&destination);

    let before = disk::size_of(&project.path);
    let expected = archive::entry_count(&project.path)
        .with_context(|| format!("could not walk `{}` before packing", project.name))?;

    archive::pack(&project.path, &partial)?;
    archive::verify(&partial, expected)?;

    std::fs::rename(&partial, &destination).with_context(|| {
        format!(
            "could not move {} into place at {}",
            partial.display(),
            destination.display()
        )
    })?;

    std::fs::remove_dir_all(&project.path).with_context(|| {
        format!(
            "packed `{}` to {} but could not remove the source folder {}",
            project.name,
            destination.display(),
            project.path.display()
        )
    })?;

    write_back(
        project,
        facts,
        ProjectState::Archived {
            archive_path: destination.clone(),
        },
        db,
    )?;

    tracing::info!("archived `{}` to {}", project.name, destination.display());
    Ok(before.saturating_sub(disk::size_of(&destination)))
}

/// `<parent-of-project>/<ledger name>.pproject`: the archive sits where the
/// folder used to be and stays next to its siblings.
///
/// The ledger name is used rather than the folder name because names are
/// unique in the ledger, so two projects cannot collide. It is also typed in
/// by hand during `pman project add`, so a name that cannot be a filename is
/// a real input to reject, not an internal invariant to assert.
fn destination(project: &Project) -> Result<PathBuf> {
    let parent = project
        .path
        .parent()
        .with_context(|| format!("{} has no parent directory", project.path.display()))?;

    check_archive_name(&project.name)?;

    Ok(parent.join(format!("{}.{ARCHIVE_EXTENSION}", project.name)))
}

pub fn check_archive_name(name: &str) -> Result<()> {
    if Path::new(name).file_name() != Some(name.as_ref()) {
        bail!("`{name}` cannot be used as an archive file name; rename the project");
    }
    Ok(())
}

fn partial(destination: &Path) -> PathBuf {
    let mut name = OsString::from(destination);
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

fn write_back(project: &Project, facts: &Facts, state: ProjectState, db: &SqlLite) -> Result<()> {
    let moved = Project {
        state,
        // The activity measured before anything was touched, so the pass's
        // own deletions never look like work on the project.
        last_change_at: facts.last_change_at,
        ..project.clone()
    };

    db.execute(&project::update(&moved)?)
        .with_context(|| format!("could not record the new state of `{}`", project.name))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{DateTime, Duration, Utc};

    use super::{apply, destination, partial};
    use crate::database::schema;
    use crate::database::sql_lite::SqlLite;
    use crate::models::project::{self, Project};
    use crate::models::state::ProjectState;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};
    use crate::pass::facts::Facts;
    use crate::pass::plan::{Action, Reason};

    fn measured() -> DateTime<Utc> {
        Utc::now() - Duration::days(200)
    }

    fn python() -> StrippingProfile {
        StrippingProfile {
            language: "python".to_string(),
            detection: vec![Element {
                element_type: ElementType::File,
                element: PathBuf::from("pyproject.toml"),
            }],
            strippable: vec![Element {
                element_type: ElementType::Folder,
                element: PathBuf::from(".venv"),
            }],
        }
    }

    fn facts(matched: bool) -> Facts {
        Facts {
            last_change_at: measured(),
            dirty: false,
            matched: if matched { vec![python()] } else { Vec::new() },
        }
    }

    /// A project folder with a stripable `.venv` and a source file, plus a
    /// ledger row pointing at it.
    fn seeded(root: &Path) -> (SqlLite, Project) {
        std::fs::create_dir_all(root.join(".venv").join("lib")).unwrap();
        std::fs::write(root.join(".venv").join("lib").join("big"), vec![0u8; 5000]).unwrap();
        std::fs::write(root.join("pyproject.toml"), "[project]").unwrap();
        std::fs::write(root.join("main.py"), "print()").unwrap();

        let project = Project {
            name: "demo".to_string(),
            path: root.to_path_buf(),
            state: ProjectState::Registered,
            has_git_repo: false,
            last_change_at: Utc::now() - Duration::days(999),
            registered_at: Utc::now() - Duration::days(999),
        };

        let db = SqlLite::open_in_memory().unwrap();
        schema::apply(&db).unwrap();
        db.execute(&project::insert(&project).unwrap()).unwrap();

        (db, project)
    }

    fn read_back(db: &SqlLite) -> Project {
        db.query(&project::select_all().unwrap(), project::from_row)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn nothing_does_nothing_and_leaves_the_ledger_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, project) = seeded(tmp.path());

        let freed = apply(
            &project,
            &Action::Nothing {
                reason: Reason::Active,
            },
            &facts(true),
            &db,
        )
        .unwrap();

        assert_eq!(freed, 0);
        assert_eq!(read_back(&db).state, ProjectState::Registered);
        assert!(tmp.path().join(".venv").exists());
    }

    #[test]
    fn a_strip_removes_the_entries_reports_the_size_and_moves_the_row() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, project) = seeded(tmp.path());

        let freed = apply(&project, &Action::Strip, &facts(true), &db).unwrap();

        assert_eq!(freed, 5000);
        assert!(!tmp.path().join(".venv").exists());
        assert!(tmp.path().join("main.py").exists());

        let row = read_back(&db);
        assert_eq!(row.state, ProjectState::Stripped);
        assert_eq!(
            row.last_change_at.timestamp(),
            measured().timestamp(),
            "the activity measured before the strip is what gets stored"
        );
    }

    #[test]
    fn an_archive_replaces_the_folder_with_a_verified_pproject() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("demo");
        std::fs::create_dir(&root).unwrap();
        let (db, project) = seeded(&root);

        apply(&project, &Action::Archive, &facts(true), &db).unwrap();

        let archive = tmp.path().join("demo.pproject");
        assert!(archive.is_file(), "the archive sits beside the old folder");
        assert!(!root.exists(), "the source folder is gone");
        assert!(
            !tmp.path().join("demo.pproject.part").exists(),
            "no partial file may be left behind"
        );

        assert_eq!(
            read_back(&db).state,
            ProjectState::Archived {
                archive_path: archive,
            }
        );
    }

    #[test]
    fn strip_and_archive_keeps_the_package_folder_out_of_the_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("demo");
        std::fs::create_dir(&root).unwrap();
        let (db, project) = seeded(&root);

        apply(&project, &Action::StripAndArchive, &facts(true), &db).unwrap();

        let packed = sevenz_rust2::Archive::open(tmp.path().join("demo.pproject")).unwrap();
        let names: Vec<String> = packed
            .files
            .iter()
            .map(|entry| entry.name().to_string())
            .collect();

        assert!(
            !names.iter().any(|name| name.starts_with(".venv")),
            "the 5000-byte .venv must be stripped before packing, got: {names:?}"
        );
        assert!(names.contains(&"main.py".to_string()));
    }

    #[test]
    fn the_archive_is_named_after_the_ledger_not_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let project = Project {
            name: "renamed".to_string(),
            path: tmp.path().join("on-disk"),
            state: ProjectState::Registered,
            has_git_repo: false,
            last_change_at: Utc::now(),
            registered_at: Utc::now(),
        };

        assert_eq!(
            destination(&project).unwrap(),
            tmp.path().join("renamed.pproject")
        );
    }

    #[test]
    fn a_name_that_is_not_a_file_name_is_refused_before_anything_is_packed() {
        let tmp = tempfile::tempdir().unwrap();
        let (db, mut project) = seeded(tmp.path());
        project.name = "dev/demo".to_string();

        let err = apply(&project, &Action::Archive, &facts(true), &db).unwrap_err();

        assert!(
            format!("{err:#}").contains("rename the project"),
            "got: {err:#}"
        );
        assert!(
            tmp.path().join("main.py").exists(),
            "a rejected name must not cost the project its folder"
        );
    }

    #[test]
    fn the_partial_file_sits_beside_the_archive_it_becomes() {
        assert_eq!(
            partial(Path::new("/dev/demo.pproject")),
            PathBuf::from("/dev/demo.pproject.part"),
            "suffixed, not extension-replaced: `.pproject` must survive"
        );
    }
}
