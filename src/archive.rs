//! Creating and extracting archives: 7z containers named `.pArchived`.
//!
//! Compression is pure Rust ([`sevenz_rust2`]) — no `7z` binary needs to be
//! installed. Archives use LZMA2 in solid mode, as 7-Zip itself does.
//!
//! Nothing here reads the extension: [`extract`] takes whatever path it is
//! given and the format is recognised from the file's own signature. The name
//! is a label, not a contract, which is what lets [`EXTENSION`] change without
//! stranding the archives written under the old one.
//!
//! Entry names are relative to the *parent* of the source folder, so archiving
//! `Code\my-app` yields an archive containing `my-app/…`; a project comes back
//! under its own name, whether pManager or 7-Zip unpacks it.

use std::path::{Component, Path, PathBuf};

use sevenz_rust2::{ArchiveWriter, encoder_options::Lzma2Options};

use crate::error::{Error, Result};

/// Extension pManager gives the archives it writes.
///
/// The container is still 7z — only the name differs, so 7-Zip and every other
/// tool that sniffs the file signature reads one of these unchanged. The point
/// is that an archived project is recognisable as pManager's work at a glance,
/// and does not sit in a folder looking like a `.7z` somebody downloaded.
///
/// Archives written before this extension existed are restored from the path
/// recorded in the database, so a `.7z` from an older version keeps working.
pub const EXTENSION: &str = "pArchived";

/// Compression level accepted by LZMA2.
pub const MIN_LEVEL: u32 = 0;
pub const MAX_LEVEL: u32 = 9;
/// 7-Zip's own default (`-mx=6`).
pub const DEFAULT_LEVEL: u32 = 6;

/// Uncompressed bytes each worker thread takes at a time.
///
/// Only used when `threads > 1`; larger chunks compress better, smaller ones
/// parallelize better. Clamped up to the dictionary size by the encoder.
const MT_CHUNK_SIZE: u64 = 64 * 1024 * 1024;

/// How an archive should be built.
#[derive(Debug, Clone)]
pub struct Options {
    /// LZMA2 compression level, 0 (fastest) to 9 (smallest).
    pub level: u32,
    /// Worker threads. More threads trade compression ratio for speed.
    pub threads: u32,
    /// File and directory names to skip, matched against each path component.
    pub exclude: Vec<String>,
    /// Overwrite the destination if it already exists.
    pub overwrite: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            level: DEFAULT_LEVEL,
            threads: 1,
            exclude: Vec::new(),
            overwrite: false,
        }
    }
}

/// What a completed archive run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// The folder that was archived.
    pub source: PathBuf,
    /// The archive that was written.
    pub archive: PathBuf,
    /// Number of files stored.
    pub file_count: u64,
    /// Total size of those files before compression.
    pub source_bytes: u64,
    /// Size of the archive on disk.
    pub archive_bytes: u64,
}

/// Archive `source` into `dest`.
///
/// `source` must be a directory. The destination's parent directories are
/// created as needed.
pub fn create(source: &Path, dest: &Path, options: &Options) -> Result<Summary> {
    let source = canonical_dir(source)?;
    let dest = normalized(dest)?;

    if dest.exists() && !options.overwrite {
        return Err(Error::Archive(format!(
            "`{}` already exists; pass --force to overwrite it",
            dest.display()
        )));
    }

    // Writing the archive into the folder being archived would race the walk
    // against its own output.
    if dest.starts_with(&source) {
        return Err(Error::Archive(format!(
            "destination `{}` is inside the folder being archived; use --output to write it elsewhere",
            dest.display()
        )));
    }

    let (file_count, source_bytes) = measure(&source, options)?;
    if file_count == 0 {
        return Err(Error::Archive(format!(
            "`{}` contains no files to archive",
            source.display()
        )));
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tracing::info!(
        source = %source.display(),
        dest = %dest.display(),
        files = file_count,
        level = options.level,
        "creating archive"
    );

    match compress(&source, &dest, options) {
        Ok(()) => {}
        Err(err) => {
            // A half-written archive is worse than none: it looks usable.
            if let Err(cleanup) = std::fs::remove_file(&dest)
                && cleanup.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %dest.display(), error = %cleanup, "could not remove partial archive");
            }
            return Err(err);
        }
    }

    let archive_bytes = std::fs::metadata(&dest)?.len();

    Ok(Summary {
        source,
        archive: dest,
        file_count,
        source_bytes,
        archive_bytes,
    })
}

/// Read a finished archive back and confirm it holds what was walked.
///
/// Only the 7z header is parsed, so this costs a seek rather than a full
/// decompression. It catches a truncated archive *before* the source folder is
/// deleted, which is the only moment the check is worth anything.
pub fn verify(summary: &Summary) -> Result<()> {
    let archive = sevenz_rust2::Archive::open(&summary.archive).map_err(|err| {
        Error::Archive(format!(
            "`{}` could not be read back after writing ({err}); the source folder was left alone",
            summary.archive.display()
        ))
    })?;

    let files = archive
        .files
        .iter()
        .filter(|entry| !entry.is_directory)
        .count() as u64;

    if files != summary.file_count {
        return Err(Error::Archive(format!(
            "`{}` holds {files} files but `{}` had {}; the source folder was left alone",
            summary.archive.display(),
            summary.source.display(),
            summary.file_count
        )));
    }

    Ok(())
}

/// Extract an archive into `into`, returning the folder it produced.
///
/// Staged first, then moved into place as one rename, so a failed extraction
/// can never half-overwrite anything. An existing destination is refused rather
/// than merged into.
pub fn extract(source: &Path, into: &Path) -> Result<PathBuf> {
    let name = source
        .file_stem()
        .ok_or_else(|| Error::Archive(format!("`{}` has no name", source.display())))?
        .to_string_lossy()
        .into_owned();

    let destination = into.join(&name);
    if destination.exists() {
        return Err(Error::Archive(format!(
            "`{}` already exists; move it aside or restore elsewhere with --into",
            destination.display()
        )));
    }

    // Beside the destination, not in a temp dir: a cross-filesystem rename is
    // a copy, and a project can be tens of gigabytes.
    let staging = into.join(format!(".pmanager-restore-{name}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let result = unpack(source, &staging, &name, &destination);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Extract into `staging` and move the result to `destination`.
fn unpack(source: &Path, staging: &Path, name: &str, destination: &Path) -> Result<PathBuf> {
    tracing::info!(
        source = %source.display(),
        destination = %destination.display(),
        "extracting archive"
    );

    sevenz_rust2::decompress_file(source, staging).map_err(|err| {
        Error::Archive(format!("could not extract `{}`: {err}", source.display()))
    })?;

    // pManager's archives hold one top-level folder, but a renamed archive
    // carries the old name, so trust the contents over the file name.
    let mut entries: Vec<PathBuf> = std::fs::read_dir(staging)?
        .flatten()
        .map(|entry| entry.path())
        .collect();

    if entries.is_empty() {
        return Err(Error::Archive(format!(
            "`{}` extracted to nothing",
            source.display()
        )));
    }

    let root = match entries.len() {
        1 if entries[0].is_dir() => entries.remove(0),
        // Several entries: the archive holds the contents rather than the
        // folder, so staging itself is the project.
        _ => staging.to_path_buf(),
    };

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::rename(&root, destination).map_err(|err| {
        Error::Archive(format!(
            "extracted `{}` but could not move it to `{}`: {err}",
            name,
            destination.display()
        ))
    })?;

    Ok(destination.to_path_buf())
}

/// Render a byte count for humans: `1.2 MiB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Compress the tree, turning a panic inside the compressor into an error.
///
/// Compression happens in-process, so a bug in it unwinds through `create`,
/// past the loop in `pmanager run`, and out of `main` — one bad project takes
/// the whole sweep with it, and the archive it was halfway through writing is
/// left on disk looking finished. Nothing has been deleted at this point, so
/// the failure is recoverable: caught here it is one failed project, reported
/// like any other, and the run carries on to the rest.
///
/// This is why the release profile unwinds rather than aborts on panic.
fn compress(source: &Path, dest: &Path, options: &Options) -> Result<()> {
    guard(source, || write_archive(source, dest, options))
}

/// Run `work`, turning a panic inside it into an [`Error::Archive`].
fn guard(source: &Path, work: impl FnOnce() -> Result<()>) -> Result<()> {
    // `AssertUnwindSafe` because everything the closure touches is created
    // inside it: whatever state the panic left the writer and the half-written
    // file in is dropped with them, and `create` deletes the file afterwards.
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));

    attempt.unwrap_or_else(|payload| {
        let detail = panic_message(&*payload);
        tracing::error!(source = %source.display(), detail, "the compressor panicked");
        Err(Error::Archive(format!(
            "compressing `{}` failed inside the compression library ({detail}); \
             the source folder was left alone",
            source.display()
        )))
    })
}

/// What a caught panic was about, as far as the payload says.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "no message"
    }
}

/// Compress the tree. Split out so `create` can clean up after a failure.
fn write_archive(source: &Path, dest: &Path, options: &Options) -> Result<()> {
    let mut writer = ArchiveWriter::create(dest).map_err(archive_error)?;
    writer.set_content_methods(vec![lzma2_options(options).into()]);

    // Pushing from the parent keeps the folder name in the archive; the filter
    // prunes every sibling, so nothing outside `source` is walked.
    let root = source.parent().unwrap_or(source);
    let exclude = options.exclude.clone();
    let (source_owned, dest_owned) = (source.to_path_buf(), dest.to_path_buf());

    writer
        .push_source_path(root, move |path| {
            if path == root {
                return true;
            }
            if !path.starts_with(&source_owned) || path == dest_owned {
                return false;
            }
            let keep = !is_excluded(path, &source_owned, &exclude);
            if !keep {
                tracing::debug!(path = %path.display(), "excluded");
            }
            keep
        })
        .map_err(archive_error)?;

    // `finish` flushes the 7z footer; a failure here means a truncated file.
    writer.finish()?;
    Ok(())
}

fn lzma2_options(options: &Options) -> Lzma2Options {
    let level = options.level.clamp(MIN_LEVEL, MAX_LEVEL);
    if options.threads > 1 {
        Lzma2Options::from_level_mt(level, options.threads, MT_CHUNK_SIZE)
    } else {
        Lzma2Options::from_level(level)
    }
}

/// Count the files that will be stored and their total size.
///
/// Walked separately from the compression pass so the summary is known up
/// front; the cost is one extra `stat` per file.
fn measure(source: &Path, options: &Options) -> Result<(u64, u64)> {
    let mut stack = vec![source.to_path_buf()];
    let (mut count, mut bytes) = (0u64, 0u64);

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if is_excluded(&path, source, &options.exclude) {
                continue;
            }

            // `file_type` does not follow symlinks, so links are skipped rather
            // than silently duplicating their target (or looping forever).
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                count += 1;
                bytes += entry.metadata()?.len();
            }
        }
    }

    Ok((count, bytes))
}

/// Whether any path component below `root` matches an exclude pattern.
fn is_excluded(path: &Path, root: &Path, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }

    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| match component {
        Component::Normal(name) => exclude.iter().any(|pattern| name == pattern.as_str()),
        _ => false,
    })
}

/// Resolve `path` and confirm it is a directory we can archive.
///
/// Also the way a folder is spelled before it goes into the database, so that
/// `.\Tools\Blender` and `C:\Tools\Blender` are recognised as the same
/// registration.
pub fn canonical_dir(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound {
                kind: "folder",
                name: path.display().to_string(),
            }
        } else {
            Error::Io(err)
        }
    })?;

    if !canonical.is_dir() {
        return Err(Error::Archive(format!(
            "`{}` is not a folder",
            path.display()
        )));
    }

    Ok(simplified(canonical))
}

/// Resolve a path that need not exist yet into the same shape
/// [`canonical_dir`] produces, so the two can be compared.
///
/// `canonicalize` only works on existing paths, so this canonicalizes the
/// deepest ancestor that does exist and re-appends the rest. Without it the
/// two sides can spell the same location differently — on Windows a canonical
/// path is extended-length (`\\?\C:\…`) and may expand an 8.3 short name
/// (`RUNNER~1` -> `runneradmin`), so a plain `starts_with` misses.
fn normalized(path: &Path) -> Result<PathBuf> {
    let absolute = absolute(path)?;

    let mut trailing = Vec::new();
    let mut candidate = absolute.as_path();

    loop {
        if let Ok(canonical) = candidate.canonicalize() {
            let mut resolved = simplified(canonical);
            for name in trailing.iter().rev() {
                resolved.push(name);
            }
            return Ok(resolved);
        }

        let (Some(parent), Some(name)) = (candidate.parent(), candidate.file_name()) else {
            // Nothing along the path exists; the absolute form is the best we
            // can do (and is what the comparison will see on both sides).
            return Ok(absolute);
        };
        trailing.push(name.to_os_string());
        candidate = parent;
    }
}

/// Absolute form of a path that need not exist yet.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Drop Windows' `\\?\` extended-length prefix so paths stay readable in
/// output and in the database. Only plain drive paths are rewritten; UNC and
/// device paths keep the prefix, because removing it there changes meaning.
#[cfg(windows)]
fn simplified(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };

    match prefix.kind() {
        Prefix::VerbatimDisk(letter) => {
            let mut simplified = PathBuf::from(format!("{}:\\", letter as char));
            simplified.extend(components.filter(|part| !matches!(part, Component::RootDir)));
            simplified
        }
        _ => path,
    }
}

#[cfg(not(windows))]
fn simplified(path: PathBuf) -> PathBuf {
    path
}

fn archive_error(err: sevenz_rust2::Error) -> Error {
    Error::Archive(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small tree: `a.txt`, `sub/b.txt`, `.git/config`.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join("sub").join("b.txt"), "world!").unwrap();
        std::fs::write(root.join(".git").join("config"), "junk").unwrap();
        dir
    }

    #[test]
    fn archives_a_folder() {
        let dir = fixture();
        let source = dir.path().join("photos");
        let dest = dir.path().join("out").join("photos.pArchived");

        let summary = create(&source, &dest, &Options::default()).unwrap();

        assert!(dest.is_file());
        assert_eq!(summary.file_count, 3);
        assert_eq!(summary.source_bytes, 5 + 6 + 4);
        assert_eq!(
            summary.archive_bytes,
            std::fs::metadata(&dest).unwrap().len()
        );
    }

    #[test]
    fn writes_a_real_7z_container() {
        let dir = fixture();
        let dest = dir.path().join("photos.pArchived");
        create(&dir.path().join("photos"), &dest, &Options::default()).unwrap();

        // 7z signature: '7z' BC AF 27 1C
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(&bytes[..6], &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]);
    }

    #[test]
    fn an_archive_extracts_whatever_it_is_called() {
        let dir = fixture();

        // `.7z` is what earlier versions wrote, and the database still points
        // at those; extraction goes by the file's signature, not its name.
        for name in ["photos.7z", "photos.pArchived", "photos"] {
            // Written to a directory of their own: an archive called `photos`
            // would otherwise land on the folder it is archiving.
            let dest = dir.path().join("archives").join(name);
            create(&dir.path().join("photos"), &dest, &Options::default()).unwrap();

            let into = dir.path().join(format!("into-{name}"));
            let restored = extract(&dest, &into).unwrap();

            assert_eq!(
                restored,
                into.join("photos"),
                "`{name}` should come back as the folder it was"
            );
            assert_eq!(
                std::fs::read_to_string(restored.join("a.txt")).unwrap(),
                "hello"
            );
        }
    }

    #[test]
    fn the_archive_round_trips_with_the_folder_name_intact() {
        let dir = fixture();
        let dest = dir.path().join("photos.pArchived");
        create(&dir.path().join("photos"), &dest, &Options::default()).unwrap();

        let out = dir.path().join("restored");
        sevenz_rust2::decompress_file(&dest, &out).unwrap();

        // Entry names are relative to the source's parent, so the folder
        // itself is part of every path — same as `7z a photos.7z photos`.
        assert_eq!(
            std::fs::read_to_string(out.join("photos/a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("photos/sub/b.txt")).unwrap(),
            "world!"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("photos/.git/config")).unwrap(),
            "junk"
        );
    }

    #[test]
    fn excluded_files_are_absent_from_the_archive() {
        let dir = fixture();
        let dest = dir.path().join("photos.pArchived");
        let options = Options {
            exclude: vec![".git".to_string(), "b.txt".to_string()],
            ..Options::default()
        };
        create(&dir.path().join("photos"), &dest, &options).unwrap();

        let out = dir.path().join("restored");
        sevenz_rust2::decompress_file(&dest, &out).unwrap();

        assert!(out.join("photos/a.txt").exists());
        assert!(!out.join("photos/.git").exists());
        assert!(!out.join("photos/sub/b.txt").exists());
    }

    #[test]
    fn exclusions_are_matched_by_component() {
        let dir = fixture();
        let options = Options {
            exclude: vec![".git".to_string()],
            ..Options::default()
        };

        let summary = create(
            &dir.path().join("photos"),
            &dir.path().join("photos.pArchived"),
            &options,
        )
        .unwrap();

        assert_eq!(summary.file_count, 2, "the .git folder should be skipped");
        assert_eq!(summary.source_bytes, 5 + 6);
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let dir = fixture();
        let source = dir.path().join("photos");
        let dest = dir.path().join("photos.pArchived");

        create(&source, &dest, &Options::default()).unwrap();
        assert!(matches!(
            create(&source, &dest, &Options::default()),
            Err(Error::Archive(_))
        ));

        let overwrite = Options {
            overwrite: true,
            ..Options::default()
        };
        create(&source, &dest, &overwrite).unwrap();
    }

    #[test]
    fn refuses_to_write_into_the_source_folder() {
        let dir = fixture();
        let source = dir.path().join("photos");

        let err = create(&source, &source.join("self.pArchived"), &Options::default()).unwrap_err();
        assert!(err.to_string().contains("inside the folder"), "{err}");
    }

    /// The destination guard has to survive the source and destination being
    /// spelled differently. A symlink is the portable stand-in for what
    /// Windows does natively with `\\?\` prefixes and 8.3 short names.
    #[cfg(unix)]
    #[test]
    fn refuses_to_write_into_the_source_folder_by_another_name() {
        let dir = fixture();
        let source = dir.path().join("photos");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let err = create(&source, &link.join("self.pArchived"), &Options::default()).unwrap_err();
        assert!(err.to_string().contains("inside the folder"), "{err}");
    }

    #[test]
    fn rejects_missing_and_non_directory_sources() {
        let dir = fixture();

        assert!(matches!(
            create(
                &dir.path().join("absent"),
                &dir.path().join("x.pArchived"),
                &Options::default()
            ),
            Err(Error::NotFound { .. })
        ));

        assert!(matches!(
            create(
                &dir.path().join("photos").join("a.txt"),
                &dir.path().join("x.pArchived"),
                &Options::default()
            ),
            Err(Error::Archive(_))
        ));
    }

    #[test]
    fn rejects_an_empty_folder() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("empty");
        std::fs::create_dir(&source).unwrap();

        let err = create(
            &source,
            &dir.path().join("x.pArchived"),
            &Options::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no files"), "{err}");
    }

    /// Whatever the compressor panics with, the message it panicked with is
    /// what the failed project's line has to end up saying.
    #[test]
    fn a_panic_payload_is_reported_as_its_message() {
        let str_payload = std::panic::catch_unwind(|| panic!("static message")).unwrap_err();
        assert_eq!(panic_message(&*str_payload), "static message");

        let string_payload =
            std::panic::catch_unwind(|| panic!("index out of bounds: {}", 7)).unwrap_err();
        assert_eq!(
            panic_message(&*string_payload),
            "index out of bounds: 7",
            "a formatted panic arrives as a String, not a &str"
        );

        let odd_payload = std::panic::catch_unwind(|| std::panic::panic_any(7u8)).unwrap_err();
        assert_eq!(panic_message(&*odd_payload), "no message");
    }

    /// A panic in the compressor has to come back as a failed project rather
    /// than as a dead process — that is what lets `pmanager run` report it and
    /// carry on to the projects behind it.
    #[test]
    fn a_panic_while_compressing_becomes_an_error() {
        let source = Path::new("/projects/photos");

        // Otherwise the hook prints the backtrace of a panic raised on purpose.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = guard(source, || {
            panic!("index out of bounds: the len is 12853521 but the index is 1624971702")
        });
        std::panic::set_hook(hook);

        let err = result.unwrap_err();
        assert!(err.to_string().contains("index out of bounds"), "{err}");
        assert!(err.to_string().contains("photos"), "{err}");
        assert!(err.to_string().contains("left alone"), "{err}");
        assert_eq!(err.exit_code(), crate::error::exit::ARCHIVE);
    }

    /// The guard is only there for panics; ordinary results pass straight
    /// through it, errors included.
    #[test]
    fn the_guard_leaves_ordinary_results_alone() {
        let source = Path::new("/projects/photos");

        assert!(guard(source, || Ok(())).is_ok());

        let err = guard(source, || Err(Error::Archive("disk full".into()))).unwrap_err();
        assert_eq!(err.to_string(), "disk full");
    }

    #[test]
    fn byte_sizes_are_human_readable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MiB");
    }
}
