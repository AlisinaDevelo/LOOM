//! Finite, observed-stable directory discovery. This is not a filesystem snapshot
//! or a durable cursor. No content is read and no partial file list is returned.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{error::io_error, LoomError, Result};

#[cfg(not(unix))]
use portable_backend as backend;
#[cfg(unix)]
use unix_backend as backend;

#[derive(Clone, Copy)]
pub(crate) struct DiscoveryLimits {
    pub(crate) files: usize,
    pub(crate) entries: usize,
    pub(crate) directories: usize,
    pub(crate) depth: usize,
    pub(crate) path_bytes: usize,
    pub(crate) retained_path_bytes: usize,
    pub(crate) elapsed: Duration,
}

impl DiscoveryLimits {
    pub(crate) fn for_files(files: usize) -> Self {
        Self {
            files,
            entries: 65_536,
            directories: 4_096,
            depth: 32,
            path_bytes: 4_096,
            retained_path_bytes: 8 * 1024 * 1024,
            elapsed: Duration::from_secs(5),
        }
    }

    pub(crate) fn fingerprint(self, hasher: &mut blake3::Hasher) {
        hasher.update(b"loom.discovery.bounded.v1\0");
        for value in [
            self.files,
            self.entries,
            self.directories,
            self.depth,
            self.path_bytes,
            self.retained_path_bytes,
        ] {
            hasher.update(&(value as u64).to_le_bytes());
        }
        hasher.update(&self.elapsed.as_secs().to_le_bytes());
        hasher.update(&self.elapsed.subsec_nanos().to_le_bytes());
    }
}

struct Budget {
    limits: DiscoveryLimits,
    started: Instant,
    entries: usize,
    directories: usize,
    retained_path_bytes: usize,
    files: Vec<PathBuf>,
}

impl Budget {
    fn new(limits: DiscoveryLimits) -> Self {
        Self {
            limits,
            started: Instant::now(),
            entries: 0,
            directories: 0,
            retained_path_bytes: 0,
            files: Vec::new(),
        }
    }

    fn check_time(&self) -> Result<()> {
        if self.started.elapsed() >= self.limits.elapsed {
            return Err(limit("cooperative time limit"));
        }
        Ok(())
    }

    fn check_path(&self, path: &Path) -> Result<usize> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        if bytes > self.limits.path_bytes {
            return Err(limit("path-byte limit"));
        }
        Ok(bytes)
    }

    fn entry(&mut self, path: &Path) -> Result<()> {
        self.check_time()?;
        self.check_path(path)?;
        if self.entries >= self.limits.entries {
            return Err(limit("entry limit"));
        }
        self.entries += 1;
        Ok(())
    }

    fn child(
        &self,
        root: &Path,
        parent: &Path,
        name: &std::ffi::OsStr,
    ) -> Result<(PathBuf, PathBuf)> {
        // Bound cumulative component bytes before constructing either path.
        // Native directory names are single components, never `..` or `/`.
        let root_bytes = root.as_os_str().as_encoded_bytes();
        let separator = usize::from(
            !root_bytes.is_empty()
                && !root_bytes
                    .last()
                    .is_some_and(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\')),
        );
        let bytes = root_bytes
            .len()
            .checked_add(separator)
            .and_then(|bytes| bytes.checked_add(parent.as_os_str().as_encoded_bytes().len()))
            .and_then(|bytes| bytes.checked_add(usize::from(!parent.as_os_str().is_empty())))
            .and_then(|bytes| bytes.checked_add(name.as_encoded_bytes().len()))
            .ok_or_else(|| limit("path-byte limit"))?;
        if bytes > self.limits.path_bytes {
            return Err(limit("path-byte limit"));
        }
        let relative = parent.join(name);
        let path = root.join(&relative);
        Ok((relative, path))
    }

    fn retain(&mut self, path: &Path) -> Result<()> {
        let bytes = self.check_path(path)?;
        if bytes
            > self
                .limits
                .retained_path_bytes
                .saturating_sub(self.retained_path_bytes)
        {
            return Err(limit("retained-path-byte limit"));
        }
        self.retained_path_bytes += bytes;
        Ok(())
    }

    fn directory(&mut self, path: &Path, depth: usize) -> Result<()> {
        self.check_time()?;
        if depth > self.limits.depth {
            return Err(limit("depth limit"));
        }
        if self.directories >= self.limits.directories {
            return Err(limit("directory limit"));
        }
        self.retain(path)?;
        self.directories += 1;
        Ok(())
    }

    fn file(&mut self, path: PathBuf) -> Result<()> {
        if self.files.len() >= self.limits.files {
            return Err(limit(&format!("{}-file request limit", self.limits.files)));
        }
        self.retain(&path)?;
        self.files.push(path);
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<PathBuf>> {
        self.check_time()?;
        // Compare components, not a raw flattened pathname: the old sorted
        // DFS visits directory `a/` before adjacent regular file `a.txt`.
        self.files
            .sort_unstable_by(|a, b| a.components().cmp(b.components()));
        self.check_time()?;
        Ok(self.files)
    }
}

fn limit(message: &str) -> LoomError {
    LoomError::InvalidPath(format!(
        "directory discovery exceeded {message}; no complete file list"
    ))
}

fn changed(path: &Path) -> LoomError {
    LoomError::SourceChanged(path.display().to_string())
}

/// Each probe runs outside SQLite. Files are metadata-only observations; later
/// extraction still has to enforce consent, source identity, byte/hash and CAS
/// fences. Time is checked between syscalls, not by preempting a filesystem.
pub(crate) fn walk(
    root: &Path,
    limits: DiscoveryLimits,
    mut probe: impl FnMut(&Path) -> Result<()>,
) -> Result<Vec<PathBuf>> {
    let mut budget = Budget::new(limits);
    budget.directory(root, 0)?;
    let canonical = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    backend::walk(root, &mut budget, &mut probe)?;
    if fs::canonicalize(root).map_err(|error| io_error(root, error))? != canonical {
        return Err(changed(root));
    }
    budget.finish()
}

#[cfg(unix)]
mod unix_backend {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

    use rustix::{
        fd::{AsFd, OwnedFd},
        fs::{open, openat, statat, AtFlags, Dir, FileType, Mode, OFlags, Stat},
    };

    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Stamp {
        device: u64,
        inode: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }

    impl Stamp {
        // `Stat` field widths differ by Unix target. Normalize exactly like
        // MetadataExt, including a signed platform dev_t's raw bit pattern.
        #[allow(clippy::unnecessary_cast)]
        fn from_stat(value: &Stat) -> Self {
            Self {
                device: value.st_dev as u64,
                inode: value.st_ino as u64,
                modified: (value.st_mtime as i64, value.st_mtime_nsec as i64),
                changed: (value.st_ctime as i64, value.st_ctime_nsec as i64),
            }
        }

        fn at_path(path: &Path) -> Result<Self> {
            let value = fs::symlink_metadata(path).map_err(|error| {
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) || error.raw_os_error() == Some(libc::ELOOP)
                {
                    changed(path)
                } else {
                    io_error(path, error)
                }
            })?;
            if !value.is_dir() || value.file_type().is_symlink() {
                return Err(changed(path));
            }
            Ok(Self {
                device: value.dev(),
                inode: value.ino(),
                modified: (value.mtime(), value.mtime_nsec()),
                changed: (value.ctime(), value.ctime_nsec()),
            })
        }
    }

    struct Frame {
        relative: PathBuf,
        stream: Dir,
        stamp: Stamp,
        depth: usize,
    }

    fn flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }

    fn namespace_error(path: &Path, error: rustix::io::Errno) -> LoomError {
        if matches!(
            error,
            rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP
        ) {
            changed(path)
        } else {
            io_error(path, error.into())
        }
    }

    // Reopen a visited directory from the original root descriptor, one
    // component at a time. NOFOLLOW on a multi-component open is insufficient.
    fn reopen(
        root: &OwnedFd,
        root_path: &Path,
        relative: &Path,
        budget: &Budget,
    ) -> Result<Option<OwnedFd>> {
        let mut parent: Option<OwnedFd> = None;
        for component in relative.components() {
            budget.check_time()?;
            let fd = parent.as_ref().map_or(root.as_fd(), AsFd::as_fd);
            let next = openat(fd, component.as_os_str(), flags(), Mode::empty())
                .map_err(|error| namespace_error(&root_path.join(relative), error))?;
            parent = Some(next);
        }
        Ok(parent)
    }

    pub(super) fn walk(
        root: &Path,
        budget: &mut Budget,
        probe: &mut impl FnMut(&Path) -> Result<()>,
    ) -> Result<()> {
        let stamp = Stamp::at_path(root)?;
        budget.check_time()?;
        let root_fd =
            open(root, flags(), Mode::empty()).map_err(|error| io_error(root, error.into()))?;
        let stream = Dir::read_from(&root_fd).map_err(|error| io_error(root, error.into()))?;
        if Stamp::from_stat(
            &stream
                .stat()
                .map_err(|error| io_error(root, error.into()))?,
        ) != stamp
        {
            return Err(changed(root));
        }
        let mut stack = vec![Frame {
            relative: PathBuf::new(),
            stream,
            stamp,
            depth: 0,
        }];
        let mut visited = Vec::new();
        probe(root)?;
        while let Some(frame) = stack.last_mut() {
            budget.check_time()?;
            let Some(entry) = frame.stream.read() else {
                let after = frame
                    .stream
                    .stat()
                    .map_err(|error| io_error(root, error.into()))?;
                if Stamp::from_stat(&after) != frame.stamp {
                    return Err(changed(&root.join(&frame.relative)));
                }
                let frame = stack.pop().expect("frame checked above");
                let path = root.join(&frame.relative);
                visited.push((frame.relative, frame.stamp));
                probe(&path)?;
                continue;
            };
            let entry = entry.map_err(|error| io_error(root, error.into()))?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            let (relative, path) = budget.child(
                root,
                &frame.relative,
                std::ffi::OsStr::from_bytes(name.to_bytes()),
            )?;
            budget.entry(&path)?;
            let fd = frame
                .stream
                .fd()
                .map_err(|error| io_error(&path, error.into()))?;
            let before = statat(fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| namespace_error(&path, error))?;
            probe(&path)?;
            budget.check_time()?;
            match FileType::from_raw_mode(before.st_mode) {
                FileType::Directory => {
                    let depth = frame.depth + 1;
                    budget.directory(&relative, depth)?;
                    let child = openat(fd, name, flags(), Mode::empty())
                        .map_err(|error| namespace_error(&path, error))?;
                    let stream = Dir::new(child).map_err(|error| io_error(&path, error.into()))?;
                    let stamp = Stamp::from_stat(&before);
                    if Stamp::from_stat(
                        &stream
                            .stat()
                            .map_err(|error| io_error(&path, error.into()))?,
                    ) != stamp
                    {
                        return Err(changed(&path));
                    }
                    stack.push(Frame {
                        relative,
                        stream,
                        stamp,
                        depth,
                    });
                }
                FileType::RegularFile => budget.file(path)?,
                _ => {} // Links and special files count against entry limits; never opened.
            }
        }
        // A directory can change after its EOF but before another subtree
        // finishes. Verify every visited directory again from the pinned root.
        for (relative, stamp) in visited {
            budget.check_time()?;
            let fd = reopen(&root_fd, root, &relative, budget)?;
            let fd = fd.as_ref().map_or(root_fd.as_fd(), AsFd::as_fd);
            let after = rustix::fs::fstat(fd)
                .map_err(|error| io_error(root.join(&relative), error.into()))?;
            if Stamp::from_stat(&after) != stamp {
                return Err(changed(&root.join(relative)));
            }
        }
        if Stamp::at_path(root)? != stamp {
            return Err(changed(root));
        }
        Ok(())
    }
}

// Portable core builds keep bounded path-based enumeration. This fallback is
// NOT the descriptor-pinned Unix boundary and is not a sandbox against an
// adversarial concurrent ancestor replacement. macOS uses the backend above.
#[cfg(any(not(unix), test))]
mod portable_backend {
    use super::*;

    struct Frame {
        path: PathBuf,
        stream: fs::ReadDir,
        modified: std::time::SystemTime,
        depth: usize,
    }

    fn modified(path: &Path) -> Result<std::time::SystemTime> {
        let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(changed(path));
        }
        metadata.modified().map_err(|error| io_error(path, error))
    }

    fn frame(path: PathBuf, depth: usize) -> Result<Frame> {
        let before = modified(&path)?;
        let stream = fs::read_dir(&path).map_err(|error| io_error(&path, error))?;
        if modified(&path)? != before {
            return Err(changed(&path));
        }
        Ok(Frame {
            path,
            stream,
            modified: before,
            depth,
        })
    }

    pub(super) fn walk(
        root: &Path,
        budget: &mut Budget,
        probe: &mut impl FnMut(&Path) -> Result<()>,
    ) -> Result<()> {
        let mut stack = vec![frame(root.to_path_buf(), 0)?];
        let mut visited = Vec::new();
        probe(root)?;
        while let Some(parent) = stack.last_mut() {
            budget.check_time()?;
            let Some(entry) = parent.stream.next() else {
                let parent = stack.pop().expect("frame checked above");
                if modified(&parent.path)? != parent.modified {
                    return Err(changed(&parent.path));
                }
                probe(&parent.path)?;
                visited.push((parent.path, parent.modified));
                continue;
            };
            let entry = entry.map_err(|error| io_error(&parent.path, error))?;
            let relative = parent
                .path
                .strip_prefix(root)
                .map_err(|_| changed(&parent.path))?;
            let (_, path) = budget.child(root, relative, &entry.file_name())?;
            budget.entry(&path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
            probe(&path)?;
            budget.check_time()?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let depth = parent.depth + 1;
                budget.directory(&path, depth)?;
                stack.push(frame(path, depth)?);
            } else if metadata.is_file() {
                budget.file(path)?;
            }
        }
        for (path, before) in visited {
            budget.check_time()?;
            if modified(&path)? != before {
                return Err(changed(&path));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn run(root: &Path, limits: DiscoveryLimits) -> Result<Vec<PathBuf>> {
        walk(root, limits, |_| Ok(()))
    }

    #[test]
    fn deterministic_component_order_matches_previous_sorted_dfs() {
        let temporary = tempdir().unwrap();
        let root = temporary.path();
        for name in [
            "z.md",
            "a.txt",
            "a/9.md",
            "a/1.md",
            "a/b.txt",
            "a/b/β.md",
            "α.md",
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        let expected = walkdir::WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        for _ in 0..32 {
            assert_eq!(run(root, DiscoveryLimits::for_files(7)).unwrap(), expected);
        }
    }

    #[test]
    fn empty_root_does_not_charge_dot_entries_or_a_file() {
        let directory = tempdir().unwrap();
        let mut limits = DiscoveryLimits::for_files(0);
        limits.entries = 0;
        limits.directories = 1;
        limits.depth = 0;
        assert!(run(directory.path(), limits).unwrap().is_empty());
    }

    #[test]
    fn directories_and_depth_have_independent_limits() {
        let directory = tempdir().unwrap();
        fs::create_dir(directory.path().join("child")).unwrap();
        let mut limits = DiscoveryLimits::for_files(10);
        limits.directories = 1;
        assert!(run(directory.path(), limits)
            .unwrap_err()
            .to_string()
            .contains("directory limit"));
        limits.directories = 2;
        limits.depth = 0;
        assert!(run(directory.path(), limits)
            .unwrap_err()
            .to_string()
            .contains("depth limit"));
        limits.depth = 1;
        assert!(run(directory.path(), limits).unwrap().is_empty());
    }

    #[test]
    fn file_and_path_budgets_refuse_without_a_partial_list() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let file = root.join("a.bin"); // Unsupported regular files still count.
        fs::write(&file, "not parsed").unwrap();
        let mut limits = DiscoveryLimits::for_files(0);
        assert!(run(root, limits)
            .unwrap_err()
            .to_string()
            .contains("0-file request limit"));
        limits.files = 1;
        limits.path_bytes = file.as_os_str().as_encoded_bytes().len() - 1;
        let mut probes = 0;
        let outcome = walk(root, limits, |_| {
            probes += 1;
            Ok(())
        });
        assert!(outcome.unwrap_err().to_string().contains("path-byte limit"));
        assert_eq!(
            probes, 1,
            "oversized child path is refused before entry probe"
        );
        limits.path_bytes += 1;
        limits.retained_path_bytes = root.as_os_str().as_encoded_bytes().len()
            + file.as_os_str().as_encoded_bytes().len()
            - 1;
        assert!(run(root, limits)
            .unwrap_err()
            .to_string()
            .contains("retained-path-byte limit"));
        limits.retained_path_bytes += 1;
        assert_eq!(run(root, limits).unwrap(), vec![file]);
    }

    #[test]
    fn cooperative_time_and_probe_errors_return_no_list() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("a.md"), "content").unwrap();
        let mut limits = DiscoveryLimits::for_files(10);
        limits.elapsed = Duration::ZERO;
        assert!(run(directory.path(), limits)
            .unwrap_err()
            .to_string()
            .contains("time limit"));
        limits.elapsed = Duration::from_secs(5);
        for stop_at in [1, 2] {
            let mut probes = 0;
            let outcome = walk(directory.path(), limits, |_| {
                probes += 1;
                if probes == stop_at {
                    return Err(LoomError::JobQueue("probe stopped".into()));
                }
                Ok(())
            });
            assert!(outcome.unwrap_err().to_string().contains("probe stopped"));
            assert_eq!(probes, stop_at);
        }
        let mut budget = Budget::new(limits);
        budget.started = Instant::now() - Duration::from_secs(6);
        assert!(budget
            .check_time()
            .unwrap_err()
            .to_string()
            .contains("time limit"));
    }

    #[test]
    fn portable_fallback_is_compiled_and_has_the_same_finite_bounds() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("a.md"), "content").unwrap();
        let mut limits = DiscoveryLimits::for_files(1);
        let mut budget = Budget::new(limits);
        budget.directory(directory.path(), 0).unwrap();
        portable_backend::walk(directory.path(), &mut budget, &mut |_| Ok(())).unwrap();
        assert_eq!(
            budget.finish().unwrap(),
            vec![directory.path().join("a.md")]
        );
        limits.entries = 0;
        let mut budget = Budget::new(limits);
        budget.directory(directory.path(), 0).unwrap();
        assert!(
            portable_backend::walk(directory.path(), &mut budget, &mut |_| Ok(()))
                .unwrap_err()
                .to_string()
                .contains("entry limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_special_files_and_unsupported_entries_count_without_being_opened() {
        use std::os::unix::{fs::symlink, net::UnixListener};
        let directory = tempdir().unwrap();
        let root = directory.path().join("selected");
        fs::create_dir(&root).unwrap();
        let outside = directory.path().join("outside.md");
        fs::write(&outside, "outside source").unwrap();
        symlink(&outside, root.join("link.md")).unwrap();
        symlink(&root, root.join("cycle")).unwrap();
        let _socket = UnixListener::bind(root.join("socket")).unwrap();
        fs::write(root.join("regular.bin"), "unsupported").unwrap();
        let mut limits = DiscoveryLimits::for_files(1);
        limits.entries = 4;
        let mut paths = Vec::new();
        assert_eq!(
            walk(&root, limits, |path| {
                paths.push(path.to_path_buf());
                Ok(())
            })
            .unwrap(),
            vec![root.join("regular.bin")]
        );
        assert!(!paths.iter().any(|path| path == &outside));
        limits.entries = 3;
        assert!(run(&root, limits)
            .unwrap_err()
            .to_string()
            .contains("entry limit"));
        assert!(
            run(&root.join("cycle"), limits).is_err(),
            "selected root links are refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_directory_replacement_between_stat_and_open_is_refused() {
        use std::os::unix::fs::symlink;
        for replacement in ["symlink", "directory"] {
            let directory = tempdir().unwrap();
            let root = directory.path().join("selected");
            let child = root.join("child");
            fs::create_dir_all(&child).unwrap();
            let outside = directory.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("secret.md"), "unselected content").unwrap();
            let mut replaced = false;
            let mut saw_unselected_entry = false;
            let outcome = walk(&root, DiscoveryLimits::for_files(10), |path| {
                if path == child && !replaced {
                    replaced = true;
                    fs::rename(&child, directory.path().join("old-child")).unwrap();
                    if replacement == "symlink" {
                        symlink(&outside, &child).unwrap();
                    } else {
                        fs::create_dir(&child).unwrap();
                    }
                }
                saw_unselected_entry |= path.ends_with("secret.md");
                Ok(())
            });
            assert!(outcome.is_err(), "{replacement}: {outcome:?}");
            assert!(replaced);
            assert!(
                !saw_unselected_entry,
                "unselected directory must never be enumerated"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_replacement_and_mutation_after_child_eof_abort_discovery() {
        for action in ["root", "child-after-eof", "child-after-root-eof"] {
            let directory = tempdir().unwrap();
            let root = directory.path().join("selected");
            let child = root.join("child");
            fs::create_dir_all(&child).unwrap();
            fs::write(child.join("old.md"), "old content").unwrap();
            let mut child_visits = 0;
            let mut root_visits = 0;
            let mut mutated = false;
            let outcome = walk(&root, DiscoveryLimits::for_files(10), |path| {
                if path == root {
                    root_visits += 1;
                }
                if action == "root" && path == root && !mutated {
                    fs::rename(&root, directory.path().join("old-root")).unwrap();
                    fs::create_dir(&root).unwrap();
                    mutated = true;
                }
                if action == "child-after-eof" && path == child {
                    child_visits += 1;
                    if child_visits == 2 {
                        fs::write(child.join("new.md"), "added after child EOF").unwrap();
                        mutated = true;
                    }
                }
                if action == "child-after-root-eof" && path == root && root_visits == 2 {
                    fs::remove_file(child.join("old.md")).unwrap();
                    fs::remove_dir(&child).unwrap();
                    mutated = true;
                }
                Ok(())
            });
            assert!(mutated, "{action}: injection did not run");
            assert!(outcome.is_err(), "{action}: {outcome:?}");
            assert!(
                matches!(outcome, Err(LoomError::SourceChanged(_))),
                "{action}: {outcome:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_non_utf8_ordering_is_lossless() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let directory = tempdir().unwrap();
        let first = directory
            .path()
            .join(OsString::from_vec(b"x\x80.md".to_vec()));
        let second = directory
            .path()
            .join(OsString::from_vec(b"x\x81.md".to_vec()));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        let mut paths = vec![second.clone(), first.clone()];
        paths.sort_unstable_by(|a, b| a.components().cmp(b.components()));
        assert_eq!(paths, vec![first, second]);
    }

    // APFS refuses these byte sequences (EILSEQ); run the physical-name case
    // on other Unix filesystems, not as a silently skipped macOS fixture.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn discovers_physical_non_utf8_names_without_lossy_sorting() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let directory = tempdir().unwrap();
        let first = directory
            .path()
            .join(OsString::from_vec(b"x\x80.md".to_vec()));
        let second = directory
            .path()
            .join(OsString::from_vec(b"x\x81.md".to_vec()));
        fs::write(&second, "second").unwrap();
        fs::write(&first, "first").unwrap();
        assert_eq!(
            run(directory.path(), DiscoveryLimits::for_files(2)).unwrap(),
            vec![first, second]
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_bound_child() {
        if std::env::var_os("LOOM_TEST_DISCOVERY_FDS").is_none() {
            return;
        }
        let descriptors = || fs::read_dir("/dev/fd").unwrap().count();
        let directory = tempdir().unwrap();
        let mut path = directory.path().to_path_buf();
        for _ in 0..8 {
            path.push("child");
            fs::create_dir(&path).unwrap();
        }
        fs::write(path.join("a.md"), "content").unwrap();
        let baseline = descriptors();
        let mut maximum = baseline;
        walk(directory.path(), DiscoveryLimits::for_files(1), |_| {
            maximum = maximum.max(descriptors());
            Ok(())
        })
        .unwrap();
        assert!(
            maximum <= baseline + 34,
            "{baseline} -> {maximum}: unexpected live descriptors"
        );
        assert_eq!(descriptors(), baseline);
        for _ in 0..64 {
            let mut limits = DiscoveryLimits::for_files(1);
            limits.depth = 31;
            assert!(run(directory.path(), limits).is_err());
            assert_eq!(descriptors(), baseline, "error path leaked a descriptor");
        }
        println!(
            "discovery fds: baseline={baseline}, observed_max={maximum}, error_repetitions=64"
        );
    }

    #[cfg(unix)]
    #[test]
    fn actual_subprocess_proves_descriptor_bound_and_cleanup() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "discovery::tests::descriptor_bound_child",
                "--exact",
                "--nocapture",
            ])
            .env("LOOM_TEST_DISCOVERY_FDS", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
