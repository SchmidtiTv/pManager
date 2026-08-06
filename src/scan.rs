//! Measuring a project: when it was last edited, and what of it is
//! regenerable. Nothing here decides anything or writes to disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Result;

/// A toolchain pManager knows how to read a project by.
///
/// This table is the whole language story. Adding one means adding one entry:
/// name it, say which files at the project root identify it, and list what it
/// writes. Both defaults — what trimming deletes and what is ignored when
/// dating a project — are derived from here, so there is no second list to
/// remember to keep in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ecosystem {
    /// What it is called, as `pmanager info` prints it.
    pub name: &'static str,
    /// Files at the project root that say a project is built with it. Any one
    /// of them is enough.
    pub markers: &'static [&'static str],
    /// Directories the toolchain rebuilds by installing what the manifest
    /// beside them asks for. These are what trimming deletes and what an
    /// archive leaves out: what pManager is willing to delete is what it
    /// refuses to spend space storing.
    pub regenerable: &'static [&'static str],
    /// Build output: written by the toolchain, and reproduced by building the
    /// sources that travel into the archive beside it.
    ///
    /// Left out of archives for the same reason as `regenerable` — storing a
    /// compiled copy of the sources next to the sources is waste. Not deleted
    /// from disk, though, which is the difference: a build that is still on
    /// disk is a build somebody may still be using, and unlike a dependency
    /// tree there is no one command that puts it back.
    pub derived: &'static [&'static str],
    /// Directories the toolchain writes to on its own that are neither.
    /// Ignored when dating a project — a build is not somebody working — but
    /// both stored and kept, because they hold things no build reproduces:
    /// editor settings, profiler recordings, somebody's own measurements.
    pub churn: &'static [&'static str],
}

impl Ecosystem {
    /// Whether `project` looks like it is built with this toolchain.
    pub fn matches(&self, project: &Path) -> bool {
        self.markers
            .iter()
            .any(|marker| project.join(marker).is_file())
    }

    /// Every directory this toolchain writes, whatever becomes of it.
    fn writes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.regenerable
            .iter()
            .chain(self.derived)
            .chain(self.churn)
            .copied()
    }
}

/// The toolchains pManager recognises.
///
/// A directory belongs in `regenerable` only if the ecosystem's own tooling
/// puts it back from the files that travel into the archive beside it, since
/// being wrong there deletes something for good. `derived` is the softer
/// version of the same claim — rebuildable, so not worth archiving, but left
/// on disk rather than deleted. Anything else belongs in `churn`, where it
/// costs a project nothing to be wrong about.
pub const ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem {
        name: "Node",
        markers: &["package.json"],
        regenerable: &["node_modules"],
        derived: &[
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".turbo",
            "dist",
            "build",
            "out",
            "coverage",
        ],
        churn: &[],
    },
    Ecosystem {
        name: "Rust",
        markers: &["Cargo.toml"],
        regenerable: &["target"],
        derived: &[],
        churn: &[],
    },
    Ecosystem {
        name: "Python",
        markers: &["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"],
        regenerable: &[".venv", "venv", "__pycache__"],
        derived: &[".pytest_cache", ".mypy_cache", ".ruff_cache", ".tox"],
        churn: &[],
    },
    Ecosystem {
        name: "Go",
        markers: &["go.mod"],
        regenerable: &["vendor"],
        derived: &[],
        churn: &[],
    },
    // Java, Kotlin and Android all arrive through Gradle, so they are one
    // entry rather than three: the manifests that identify them are the same
    // pair of build scripts, and so are the directories they write.
    //
    // `build` is regenerable here even though Node keeps it in `churn`. Gradle
    // reserves the name — every module gets one, `gradle clean` empties it, and
    // in an Android project it is the largest thing on disk by far. The two
    // meanings only meet in a repository whose root holds both a
    // `package.json` and a `build.gradle`, where Gradle's reading wins.
    //
    // `.cxx` is where the Android Gradle plugin keeps NDK object files, and
    // `captures` is where Android Studio drops profiler recordings — those are
    // somebody's measurements, not build output, so they are never deleted.
    Ecosystem {
        name: "Gradle",
        markers: &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        regenerable: &[".cxx", ".gradle", "build"],
        derived: &[],
        churn: &[".idea", ".kotlin", "captures"],
    },
    Ecosystem {
        name: "Maven",
        markers: &["pom.xml"],
        regenerable: &["target"],
        derived: &[],
        churn: &[".idea"],
    },
];

/// Directories ignored when dating a project no matter what it is built with.
/// `.git` is here rather than in an ecosystem because history is never
/// regenerable — it travels into the archive with everything else.
pub const ALWAYS_SKIP: &[&str] = &[".git", ".hg", ".svn", ".cache"];

/// The toolchains `project` is built with, by the marker files at its root.
/// A polyglot repository matches more than one.
pub fn ecosystems(project: &Path) -> Vec<&'static Ecosystem> {
    ECOSYSTEMS
        .iter()
        .filter(|eco| eco.matches(project))
        .collect()
}

/// Directories trimming may delete in `project`: what the toolchains it
/// actually uses can rebuild.
///
/// A project with no recognised marker yields nothing. That is deliberate —
/// a bare `target` directory with no `Cargo.toml` beside it is not something
/// pManager can claim to know how to rebuild, so it stays.
pub fn regenerable_dirs(project: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = ecosystems(project)
        .iter()
        .flat_map(|eco| eco.regenerable.iter().map(|dir| (*dir).to_string()))
        .collect();

    dirs.sort();
    dirs.dedup();
    dirs
}

/// Build output in `project`: rebuilt by building the sources beside it, so
/// not worth storing, but not deleted either.
///
/// A directory any matched toolchain calls regenerable is left out, so that
/// the stronger claim is the one that survives. `build` in a project that is
/// both Node and Gradle is the case: Gradle's `regenerable` wins, and it is
/// trimmed rather than merely skipped.
pub fn derived_dirs(project: &Path) -> Vec<String> {
    let matched = ecosystems(project);
    let regenerable = regenerable_dirs(project);

    let mut dirs: Vec<String> = matched
        .iter()
        .flat_map(|eco| eco.derived.iter().map(|dir| (*dir).to_string()))
        .filter(|dir| !regenerable.contains(dir))
        .collect();

    dirs.sort();
    dirs.dedup();
    dirs
}

/// Everything an archive of `project` leaves out: what trimming would delete,
/// plus the build output that trimming leaves alone.
///
/// Both are reproducible from what does get stored, which is the only reason
/// leaving them out is safe — for a project whose folder is then deleted, the
/// archive is the last copy.
pub fn archive_exclude_dirs(project: &Path) -> Vec<String> {
    let mut dirs = regenerable_dirs(project);
    dirs.extend(derived_dirs(project));
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Every directory ignored when looking for a project's last edit: everything
/// any known toolchain writes, plus version control metadata.
///
/// Deliberately not filtered by ecosystem. Skipping too much only risks dating
/// a project by an older file than it deserves, whereas counting a build as an
/// edit would keep a dead project alive forever.
pub fn skip_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = ECOSYSTEMS
        .iter()
        .flat_map(Ecosystem::writes)
        .chain(ALWAYS_SKIP.iter().copied())
        .map(str::to_string)
        .collect();

    dirs.sort();
    dirs.dedup();
    dirs
}

/// What a project calls itself, falling back to the folder name. A broken or
/// unreadable manifest falls back rather than failing.
pub fn project_name(path: &Path) -> String {
    node_name(path)
        .or_else(|| gradle_name(path))
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        })
}

/// `package.json`'s `name`, when it has a usable one.
fn node_name(path: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(path.join("package.json")).ok()?;

    serde_json::from_str::<serde_json::Value>(&manifest)
        .ok()
        .as_ref()
        .and_then(|manifest| manifest.get("name"))
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
}

/// The `rootProject.name` a Gradle settings file assigns.
///
/// Gradle projects are routinely checked out into a folder named nothing like
/// the build — `MyApp` in `~/AndroidStudioProjects/myapp-2` — so the settings
/// file is worth reading. Only a quoted literal is taken; anything else is an
/// expression that would need Gradle itself to evaluate.
fn gradle_name(path: &Path) -> Option<String> {
    for settings in ["settings.gradle", "settings.gradle.kts"] {
        let Ok(contents) = std::fs::read_to_string(path.join(settings)) else {
            continue;
        };
        if let Some(name) = contents.lines().find_map(root_project_name) {
            return Some(name);
        }
    }
    None
}

/// The name in `rootProject.name = "app"`, if `line` is that assignment.
/// Groovy and Kotlin write it identically, apart from Groovy's single quotes.
fn root_project_name(line: &str) -> Option<String> {
    let value = line
        .trim_start()
        .strip_prefix("rootProject.name")?
        .trim_start()
        .strip_prefix('=')?
        .trim_start();

    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let name = value[quote.len_utf8()..].split(quote).next()?.trim();

    (!name.is_empty()).then(|| name.to_string())
}

/// The most recently modified file under a project.
#[derive(Debug, Clone)]
pub struct Edit {
    /// The file itself, for reporting which one it was.
    pub path: PathBuf,
    /// Its modification time.
    pub at: SystemTime,
}

/// A directory that can be deleted and rebuilt.
#[derive(Debug, Clone)]
pub struct Purgeable {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
}

/// Total size of a set of purgeable directories.
pub fn total_bytes(dirs: &[Purgeable]) -> u64 {
    dirs.iter().map(|dir| dir.bytes).sum()
}

/// The newest file modification under `root`, ignoring `skip` directories.
///
/// `None` means no files outside the skipped ones — an empty checkout, which
/// the policy leaves alone. Unreadable entries are skipped rather than failing
/// the walk.
pub fn last_edit(root: &Path, skip: &[String]) -> Result<Option<Edit>> {
    let mut newest: Option<Edit> = None;

    walk(root, skip, &mut |path, metadata| {
        let Ok(at) = metadata.modified() else {
            return;
        };
        if newest.as_ref().is_none_or(|current| at > current.at) {
            newest = Some(Edit {
                path: path.to_path_buf(),
                at,
            });
        }
    })?;

    Ok(newest)
}

/// Every purge directory under `root`, at any depth — a workspace keeps one
/// per package. Not descended into once found.
pub fn purgeable(root: &Path, purge: &[String]) -> Result<Vec<Purgeable>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            // `file_type` does not follow symlinks, so a link to a directory is
            // not walked: pManager must never delete through one.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let path = entry.path();
            if matches_name(&path, purge) {
                let (files, bytes) = measure(&path);
                found.push(Purgeable { path, bytes, files });
            } else {
                stack.push(path);
            }
        }
    }

    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// File count and total size of a directory tree, ignoring what cannot be read.
pub fn measure(root: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let _ = walk(root, &[], &mut |_, metadata| {
        files += 1;
        bytes += metadata.len();
    });
    (files, bytes)
}

/// How long ago `at` was. Saturates at zero, so a file dated tomorrow reads as
/// "just now" rather than as impossibly old.
pub fn age(at: SystemTime) -> Duration {
    SystemTime::now()
        .duration_since(at)
        .unwrap_or(Duration::ZERO)
}

/// Whether `at` is older than `days`.
pub fn older_than(at: SystemTime, days: u32) -> bool {
    age(at) > Duration::from_secs(u64::from(days) * 86_400)
}

/// Whole days since `at`.
pub fn days_since(at: SystemTime) -> u64 {
    age(at).as_secs() / 86_400
}

/// Unix seconds, as stored in the database. Epoch integers rather than
/// calendar strings: every question asked of them is "how long ago", which
/// keeps a date library out of the tree.
pub fn to_epoch(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        // Before 1970: only reachable from a badly wrong clock.
        Err(err) => -(err.duration().as_secs() as i64),
    }
}

/// The point in time a stored epoch value refers to.
pub fn from_epoch(seconds: i64) -> SystemTime {
    if seconds >= 0 {
        UNIX_EPOCH + Duration::from_secs(seconds as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
    }
}

/// Now, in the form the database stores.
pub fn now_epoch() -> i64 {
    to_epoch(SystemTime::now())
}

/// A duration in the shape pTag prints it: `3d ago`, `2mo ago`, `1y ago`.
pub fn time_ago(at: SystemTime) -> String {
    let seconds = age(at).as_secs();

    match seconds {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s if s < 86_400 * 30 => format!("{}d ago", s / 86_400),
        s if s < 86_400 * 365 => format!("{}mo ago", s / (86_400 * 30)),
        s => format!("{}y ago", s / (86_400 * 365)),
    }
}

/// Walk every file under `root`, skipping directories named in `skip`.
/// Iterative, and never follows symlinks, so deep trees and link loops are
/// both safe.
fn walk(
    root: &Path,
    skip: &[String],
    visit: &mut impl FnMut(&Path, &std::fs::Metadata),
) -> Result<()> {
    // The first read is the one that reports a genuinely missing project;
    // failures further down are the unreadable corners described above.
    let mut stack = vec![std::fs::read_dir(root)?];

    while let Some(entries) = stack.pop() {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();

            if file_type.is_dir() {
                if matches_name(&path, skip) {
                    tracing::trace!(path = %path.display(), "skipping");
                    continue;
                }
                if let Ok(entries) = std::fs::read_dir(&path) {
                    stack.push(entries);
                }
            } else if file_type.is_file()
                && let Ok(metadata) = entry.metadata()
            {
                visit(&path, &metadata);
            }
        }
    }

    Ok(())
}

/// Whether the final component of `path` is one of `names`.
fn matches_name(path: &Path, names: &[String]) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    names.iter().any(|candidate| name == candidate.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dependency trees these tests build projects out of. Named rather
    /// than detected: what is being tested here is the walk, not the table.
    fn purge_dirs() -> Vec<String> {
        vec!["node_modules".to_string()]
    }

    #[test]
    fn a_project_is_trimmed_by_what_it_is_built_with() {
        let dir = tempfile::tempdir().unwrap();

        let node = dir.path().join("web");
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(node.join("package.json"), "{}").unwrap();
        assert_eq!(regenerable_dirs(&node), ["node_modules"]);

        let rust = dir.path().join("cli");
        std::fs::create_dir_all(&rust).unwrap();
        std::fs::write(rust.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(regenerable_dirs(&rust), ["target"]);

        let python = dir.path().join("scraper");
        std::fs::create_dir_all(&python).unwrap();
        std::fs::write(python.join("requirements.txt"), "requests").unwrap();
        assert_eq!(regenerable_dirs(&python), [".venv", "__pycache__", "venv"]);
    }

    #[test]
    fn a_jvm_project_is_trimmed_by_the_build_tool_that_drives_it() {
        let dir = tempfile::tempdir().unwrap();

        // Kotlin, Java and Android are the same entry: what identifies them is
        // the build script, and what they write is Gradle's.
        for marker in [
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ] {
            let project = dir.path().join(marker.replace('.', "-"));
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(project.join(marker), "").unwrap();

            let names: Vec<&str> = ecosystems(&project).iter().map(|e| e.name).collect();
            assert_eq!(names, ["Gradle"], "for {marker}");
            assert_eq!(regenerable_dirs(&project), [".cxx", ".gradle", "build"]);
        }

        let maven = dir.path().join("service");
        std::fs::create_dir_all(&maven).unwrap();
        std::fs::write(maven.join("pom.xml"), "<project/>").unwrap();
        assert_eq!(regenerable_dirs(&maven), ["target"]);
    }

    #[test]
    fn an_android_studio_recording_is_never_deleted() {
        // `captures` holds profiler recordings somebody made by hand, and
        // `.idea` is the IDE's own state. Both churn without being work, and
        // neither is something a build puts back.
        let gradle = ECOSYSTEMS
            .iter()
            .find(|eco| eco.name == "Gradle")
            .expect("the table has a Gradle entry");

        for dir in ["captures", ".idea", ".kotlin"] {
            assert!(gradle.churn.contains(&dir), "{dir} should be churn");
            assert!(!gradle.regenerable.contains(&dir), "{dir} is not rebuilt");
        }
    }

    #[test]
    fn a_polyglot_project_is_trimmed_by_each_of_its_toolchains() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let names: Vec<&str> = ecosystems(dir.path()).iter().map(|e| e.name).collect();
        assert_eq!(names, ["Node", "Rust"]);
        assert_eq!(regenerable_dirs(dir.path()), ["node_modules", "target"]);
    }

    /// Build output is worth leaving out of an archive but not worth deleting,
    /// so it has to show up in one list and not the other.
    #[test]
    fn build_output_is_left_out_of_archives_without_being_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("site");
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(node.join("package.json"), "{}").unwrap();

        assert_eq!(regenerable_dirs(&node), ["node_modules"]);
        assert_eq!(
            derived_dirs(&node),
            [
                ".next",
                ".nuxt",
                ".svelte-kit",
                ".turbo",
                "build",
                "coverage",
                "dist",
                "out"
            ]
        );
        assert_eq!(
            archive_exclude_dirs(&node),
            [
                ".next",
                ".nuxt",
                ".svelte-kit",
                ".turbo",
                "build",
                "coverage",
                "dist",
                "node_modules",
                "out"
            ]
        );
    }

    /// `build` is Node's build output and Gradle's regenerable directory. In a
    /// project that is both, the claim that deletes it has to win, or Gradle's
    /// largest directory would survive every trim.
    #[test]
    fn a_directory_two_toolchains_disagree_about_takes_the_stronger_reading() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("build.gradle"), "").unwrap();

        assert!(regenerable_dirs(dir.path()).contains(&"build".to_string()));
        assert!(
            !derived_dirs(dir.path()).contains(&"build".to_string()),
            "Gradle's reading should win, leaving `build` trimmable"
        );
        // Either way it stays out of the archive.
        assert!(archive_exclude_dirs(dir.path()).contains(&"build".to_string()));
    }

    /// Settings and recordings are not builds. Nothing reproduces them, so
    /// they are stored even though the toolchain wrote them.
    #[test]
    fn what_no_build_reproduces_is_archived() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.gradle"), "").unwrap();

        let excluded = archive_exclude_dirs(dir.path());
        for kept in [".idea", ".kotlin", "captures"] {
            assert!(
                !excluded.contains(&kept.to_string()),
                "{kept} should travel into the archive"
            );
            // Still not evidence that somebody was working, though.
            assert!(skip_dirs().contains(&kept.to_string()));
        }
    }

    #[test]
    fn an_unrecognised_project_has_nothing_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        // A `target` with no `Cargo.toml` beside it is not something pManager
        // can claim to know how to rebuild.
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("notes.md"), "just files").unwrap();

        assert!(ecosystems(dir.path()).is_empty());
        assert!(regenerable_dirs(dir.path()).is_empty());
    }

    #[test]
    fn a_marker_has_to_be_a_file_at_the_root() {
        let dir = tempfile::tempdir().unwrap();
        // A directory named like a marker, and a marker one level down:
        // neither makes this a Node project.
        std::fs::create_dir_all(dir.path().join("package.json")).unwrap();
        std::fs::create_dir_all(dir.path().join("packages/ui")).unwrap();
        std::fs::write(dir.path().join("packages/ui/Cargo.toml"), "[package]").unwrap();

        assert!(ecosystems(dir.path()).is_empty());
    }

    #[test]
    fn everything_any_toolchain_writes_is_ignored_when_dating_a_project() {
        let skip = skip_dirs();

        // Whatever the table grows, both halves of every entry land here —
        // that is what stops a build from reading as somebody working.
        for eco in ECOSYSTEMS {
            for dir in eco.regenerable.iter().chain(eco.churn) {
                assert!(
                    skip.contains(&(*dir).to_string()),
                    "{}'s {dir} is missing from the skip list",
                    eco.name
                );
            }
        }
        for dir in ALWAYS_SKIP {
            assert!(skip.contains(&(*dir).to_string()), "{dir}");
        }

        // Sorted and deduplicated, so shared names cost nothing.
        let mut sorted = skip.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(skip, sorted);
    }

    #[test]
    fn history_is_never_regenerable() {
        for eco in ECOSYSTEMS {
            assert!(
                !eco.regenerable.contains(&".git"),
                "{} would have pManager delete history",
                eco.name
            );
        }
        assert!(ALWAYS_SKIP.contains(&".git"));
    }

    /// Write `contents` to `root/relative`, creating parent directories.
    fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Backdate a file so the walk sees it as old.
    fn backdate(path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_secs(days * 86_400);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn a_project_is_named_by_its_manifest_when_it_has_one() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(project_name(dir.path()), folder);

        write(
            dir.path(),
            "package.json",
            r#"{"name": "@acme/api", "version": "1.0.0"}"#,
        );
        assert_eq!(project_name(dir.path()), "@acme/api");
    }

    #[test]
    fn a_gradle_project_is_named_by_its_settings_file() {
        let dir = tempfile::tempdir().unwrap();

        write(
            dir.path(),
            "settings.gradle",
            "rootProject.name = 'my-app'\n",
        );
        assert_eq!(project_name(dir.path()), "my-app");

        // Kotlin's settings file, and a name that is not the folder's.
        let kts = dir.path().join("app");
        write(
            &kts,
            "settings.gradle.kts",
            "pluginManagement { repositories { google() } }\n\
             rootProject.name = \"My Android App\"\n\
             include(\":app\")\n",
        );
        assert_eq!(project_name(&kts), "My Android App");

        // A Gradle build inside a Node repository still answers to its
        // `package.json`: that is the manifest a human would name it by.
        write(dir.path(), "package.json", r#"{"name": "@acme/mobile"}"#);
        assert_eq!(project_name(dir.path()), "@acme/mobile");
    }

    #[test]
    fn a_gradle_name_that_is_not_a_literal_falls_back_to_the_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        for settings in [
            // Evaluating any of these would take Gradle itself.
            "rootProject.name = property('appName')\n",
            "rootProject.name = \"\"\n",
            "include(':app')\n",
            "rootProject.nameOfSomethingElse = 'nope'\n",
            "",
        ] {
            write(dir.path(), "settings.gradle", settings);
            assert_eq!(project_name(dir.path()), folder, "for {settings:?}");
        }
    }

    #[test]
    fn a_broken_manifest_falls_back_to_the_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        for manifest in [
            r#"{"name": }"#,
            r#"{"version": "1.0.0"}"#,
            r#"{"name": ""}"#,
            "",
        ] {
            write(dir.path(), "package.json", manifest);
            assert_eq!(project_name(dir.path()), folder, "for {manifest:?}");
        }
    }

    #[test]
    fn the_newest_edited_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let old = write(dir.path(), "src/old.ts", "old");
        write(dir.path(), "src/new.ts", "new");
        backdate(&old, 40);

        let edit = last_edit(dir.path(), &skip_dirs()).unwrap().unwrap();
        assert_eq!(edit.path.file_name().unwrap(), "new.ts");
    }

    #[test]
    fn a_dependency_install_does_not_make_a_project_look_alive() {
        let dir = tempfile::tempdir().unwrap();
        let source = write(dir.path(), "src/index.ts", "source");
        backdate(&source, 90);
        // Freshly installed today, and vastly more numerous than the sources.
        write(dir.path(), "node_modules/left-pad/index.js", "dep");
        write(dir.path(), "dist/bundle.js", "built");

        let edit = last_edit(dir.path(), &skip_dirs()).unwrap().unwrap();
        assert_eq!(edit.path.file_name().unwrap(), "index.ts");
        assert!(
            older_than(edit.at, 60),
            "the project should still read stale"
        );
    }

    #[test]
    fn a_project_with_nothing_but_skipped_directories_has_no_last_edit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "node_modules/left-pad/index.js", "dep");

        assert!(last_edit(dir.path(), &skip_dirs()).unwrap().is_none());
    }

    #[test]
    fn a_missing_project_is_an_error_rather_than_an_empty_walk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(last_edit(&dir.path().join("absent"), &skip_dirs()).is_err());
    }

    #[test]
    fn nested_dependency_trees_are_all_found_and_measured() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "node_modules/a/index.js", "0123456789");
        write(dir.path(), "packages/api/node_modules/b/index.js", "01234");
        write(dir.path(), "src/index.ts", "source");

        let found = purgeable(dir.path(), &purge_dirs()).unwrap();
        assert_eq!(
            found.len(),
            2,
            "a workspace keeps one per package: {found:?}"
        );
        assert_eq!(total_bytes(&found), 15);
        assert_eq!(found.iter().map(|dir| dir.files).sum::<u64>(), 2);
    }

    #[test]
    fn a_dependency_tree_inside_another_is_not_counted_twice() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "node_modules/a/node_modules/b/index.js",
            "nested",
        );

        let found = purgeable(dir.path(), &purge_dirs()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, dir.path().join("node_modules"));
    }

    #[test]
    fn a_project_without_dependencies_has_nothing_to_purge() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "main.rs", "fn main() {}");

        assert!(purgeable(dir.path(), &purge_dirs()).unwrap().is_empty());
    }

    #[test]
    fn a_future_timestamp_reads_as_new_rather_than_ancient() {
        let tomorrow = SystemTime::now() + Duration::from_secs(86_400);
        assert_eq!(age(tomorrow), Duration::ZERO);
        assert!(!older_than(tomorrow, 1));
        assert_eq!(time_ago(tomorrow), "0s ago");
    }

    #[test]
    fn epoch_values_round_trip() {
        let now = SystemTime::now();
        let seconds = to_epoch(now);

        assert!(
            seconds > 1_700_000_000,
            "expected a plausible epoch, got {seconds}"
        );
        // Whole seconds only, so the round trip is exact to the second.
        assert_eq!(to_epoch(from_epoch(seconds)), seconds);
        assert!(age(from_epoch(now_epoch())) < Duration::from_secs(5));
    }

    #[test]
    fn ages_are_rendered_at_a_sensible_scale() {
        let ago = |seconds: u64| time_ago(SystemTime::now() - Duration::from_secs(seconds));

        assert_eq!(ago(30), "30s ago");
        assert_eq!(ago(3_600 * 5), "5h ago");
        assert_eq!(ago(86_400 * 3), "3d ago");
        assert_eq!(ago(86_400 * 90), "3mo ago");
        assert_eq!(ago(86_400 * 400), "1y ago");
    }
}
