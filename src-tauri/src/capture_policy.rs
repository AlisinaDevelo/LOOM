//! Persisted intentional-capture policy.
//!
//! The policy is a privacy boundary, so every failure fails closed: a missing file is the
//! documented first-run default, but any existing file that cannot be read, is not a regular file,
//! is oversized, malformed, or of an unknown version loads as paused and records the reason. A
//! failed write or commit leaves both the in-memory and on-disk policy unchanged.

use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Current policy file version. Files written before versioning carry no field and read as 1.
/// Older LOOM builds ignore the field, so downgrade keeps the paused flag and exclusions.
pub(crate) const POLICY_VERSION: u32 = 1;
/// Upper bound on the policy file; a real policy is a few hundred bytes.
pub(crate) const MAX_POLICY_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapturePolicy {
    #[serde(default = "legacy_version")]
    pub(crate) version: u32,
    pub(crate) paused: bool,
    pub(crate) excluded_apps: Vec<String>,
}

const fn legacy_version() -> u32 {
    1
}

impl Default for CapturePolicy {
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            paused: false,
            excluded_apps: Vec::new(),
        }
    }
}

/// The policy in force plus, when the stored policy could not be trusted, why it was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedPolicy {
    pub(crate) policy: CapturePolicy,
    pub(crate) error: Option<String>,
}

impl LoadedPolicy {
    fn rejected(reason: String) -> Self {
        Self {
            policy: CapturePolicy {
                paused: true,
                ..CapturePolicy::default()
            },
            error: Some(reason),
        }
    }

    /// Changes the paused flag, committing the candidate before it replaces the live policy.
    ///
    /// While the stored policy is rejected, capture stays paused and the rejected file is not
    /// overwritten; the user must re-state exclusions first.
    pub(crate) fn set_paused(&mut self, path: &Path, paused: bool) -> Result<(), String> {
        if let Some(error) = &self.error {
            if paused {
                return Ok(());
            }
            return Err(format!(
                "capture stays paused because the saved policy was rejected ({error}); \
                 review and save the excluded apps before resuming"
            ));
        }
        let candidate = CapturePolicy {
            paused,
            ..self.policy.clone()
        };
        save(path, &candidate)?;
        self.policy = candidate;
        Ok(())
    }

    /// Replaces the exclusions, committing the candidate before it replaces the live policy.
    ///
    /// Saving exclusions over a rejected policy is an explicit user re-statement: the rejected file
    /// is kept beside the policy for inspection, and capture remains paused until resumed.
    pub(crate) fn set_exclusions(
        &mut self,
        path: &Path,
        excluded_apps: Vec<String>,
    ) -> Result<(), String> {
        let candidate = CapturePolicy {
            excluded_apps: normalize_exclusions(excluded_apps),
            ..self.policy.clone()
        };
        if self.error.is_some() {
            preserve_rejected(path)?;
        }
        save(path, &candidate)?;
        self.policy = candidate;
        self.error = None;
        Ok(())
    }
}

pub(crate) fn policy_path(capture_root: &Path) -> PathBuf {
    capture_root
        .parent()
        .unwrap_or(capture_root)
        .join("capture-policy.json")
}

pub(crate) fn rejected_path(path: &Path) -> PathBuf {
    path.with_extension("rejected.json")
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

pub(crate) fn normalize_exclusions(excluded_apps: Vec<String>) -> Vec<String> {
    let mut values = excluded_apps
        .into_iter()
        .map(|app| app.trim().to_ascii_lowercase())
        .filter(|app| !app.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

/// Loads the committed policy. Only the committed path is read; a leftover temporary file is
/// never mistaken for policy.
pub(crate) fn load(path: &Path) -> LoadedPolicy {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return LoadedPolicy {
                policy: CapturePolicy::default(),
                error: None,
            }
        }
        Err(error) => return LoadedPolicy::rejected(format!("could not read policy: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return LoadedPolicy::rejected("policy is not a regular file".into());
    }
    if metadata.len() > MAX_POLICY_BYTES {
        return LoadedPolicy::rejected(format!("policy exceeds the {MAX_POLICY_BYTES}-byte limit"));
    }
    let mut bytes = Vec::new();
    let read = File::open(path).and_then(|file| {
        file.take(MAX_POLICY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| ())
    });
    if let Err(error) = read {
        return LoadedPolicy::rejected(format!("could not read policy: {error}"));
    }
    if bytes.len() as u64 > MAX_POLICY_BYTES {
        return LoadedPolicy::rejected(format!("policy exceeds the {MAX_POLICY_BYTES}-byte limit"));
    }
    let policy: CapturePolicy = match serde_json::from_slice(&bytes) {
        Ok(policy) => policy,
        Err(error) => return LoadedPolicy::rejected(format!("policy is malformed: {error}")),
    };
    if policy.version != POLICY_VERSION {
        return LoadedPolicy::rejected(format!(
            "policy version {} is not supported",
            policy.version
        ));
    }
    LoadedPolicy {
        policy: CapturePolicy {
            excluded_apps: normalize_exclusions(policy.excluded_apps),
            ..policy
        },
        error: None,
    }
}

/// Durably writes `policy` to a temporary file and atomically renames it over the committed path.
/// On failure the temporary file is removed and the committed file is left untouched.
pub(crate) fn save(path: &Path, policy: &CapturePolicy) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(policy).map_err(|error| error.to_string())?;
    let temporary = temporary_path(path);
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not save capture policy: {error}"));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not commit capture policy: {error}"));
    }
    Ok(())
}

fn preserve_rejected(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => fs::copy(path, rejected_path(path))
            .map(|_| ())
            .map_err(|error| format!("could not preserve the rejected capture policy: {error}")),
        Ok(_) => {
            Err("the rejected capture policy is not a regular file; remove it manually".into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "could not inspect the rejected capture policy: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let directory = tempdir().unwrap();
        let path = directory.path().join("capture-policy.json");
        (directory, path)
    }

    fn saved(paused: bool, apps: &[&str]) -> CapturePolicy {
        CapturePolicy {
            version: POLICY_VERSION,
            paused,
            excluded_apps: apps.iter().map(|app| app.to_string()).collect(),
        }
    }

    #[test]
    fn missing_policy_uses_first_run_default() {
        let (_directory, path) = setup();
        let loaded = load(&path);
        assert_eq!(loaded.policy, CapturePolicy::default());
        assert_eq!(loaded.error, None);
    }

    #[test]
    fn legacy_unversioned_policy_is_accepted() {
        let (_directory, path) = setup();
        fs::write(&path, br#"{"paused":true,"excluded_apps":["Safari"]}"#).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.policy, saved(true, &["safari"]));
    }

    #[test]
    fn invalid_existing_policies_fail_closed() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", Vec::new()),
            ("truncated", br#"{"paused":false,"excluded_"#.to_vec()),
            ("malformed", b"not json".to_vec()),
            (
                "wrong type",
                br#"{"paused":"no","excluded_apps":[]}"#.to_vec(),
            ),
            (
                "unknown field",
                br#"{"paused":false,"excluded_apps":[],"x":1}"#.to_vec(),
            ),
            (
                "unknown version",
                br#"{"version":2,"paused":false,"excluded_apps":[]}"#.to_vec(),
            ),
            ("oversized", vec![b' '; MAX_POLICY_BYTES as usize + 1]),
        ];
        for (name, bytes) in cases {
            let (_directory, path) = setup();
            fs::write(&path, &bytes).unwrap();
            let loaded = load(&path);
            assert!(loaded.policy.paused, "{name} must load paused");
            assert!(loaded.error.is_some(), "{name} must report an error");
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "{name} must not be rewritten"
            );
        }
    }

    #[test]
    fn non_regular_policy_fails_closed() {
        let (_directory, path) = setup();
        fs::create_dir(&path).unwrap();
        let loaded = load(&path);
        assert!(loaded.policy.paused);
        assert!(loaded.error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_and_unreadable_policies_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let (directory, path) = setup();
        let target = directory.path().join("elsewhere.json");
        fs::write(&target, br#"{"paused":false,"excluded_apps":[]}"#).unwrap();
        symlink(&target, &path).unwrap();
        assert!(load(&path).policy.paused);

        let (_other, unreadable) = setup();
        fs::write(&unreadable, br#"{"paused":false,"excluded_apps":[]}"#).unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = File::open(&unreadable).is_ok();
        let loaded = load(&unreadable);
        if !readable_anyway {
            assert!(loaded.policy.paused);
            assert!(loaded.error.is_some());
        }
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn leftover_temporary_file_is_never_loaded() {
        let (_directory, path) = setup();
        fs::write(
            temporary_path(&path),
            br#"{"paused":false,"excluded_apps":[]}"#,
        )
        .unwrap();
        save(&path, &saved(true, &["safari"])).unwrap();
        assert_eq!(load(&path).policy, saved(true, &["safari"]));
        assert!(!temporary_path(&path).exists());
    }

    #[test]
    fn successful_updates_survive_restart() {
        let (_directory, path) = setup();
        let mut live = load(&path);
        live.set_exclusions(
            &path,
            vec![" Safari ".into(), "safari".into(), "Mail".into()],
        )
        .unwrap();
        live.set_paused(&path, true).unwrap();
        let restarted = load(&path);
        assert_eq!(restarted.error, None);
        assert_eq!(restarted.policy, live.policy);
        assert_eq!(restarted.policy, saved(true, &["mail", "safari"]));
    }

    #[test]
    fn failed_write_leaves_memory_and_disk_unchanged() {
        let (_directory, path) = setup();
        let mut live = load(&path);
        live.set_exclusions(&path, vec!["safari".into()]).unwrap();
        live.set_paused(&path, true).unwrap();
        let before_disk = fs::read(&path).unwrap();
        let before_memory = live.clone();

        // A directory at the temporary path makes the write fail.
        fs::create_dir(temporary_path(&path)).unwrap();
        assert!(live.set_exclusions(&path, Vec::new()).is_err());
        assert!(live.set_paused(&path, false).is_err());
        assert_eq!(live, before_memory);
        assert_eq!(fs::read(&path).unwrap(), before_disk);
        fs::remove_dir(temporary_path(&path)).unwrap();
        assert_eq!(load(&path).policy, saved(true, &["safari"]));
    }

    #[test]
    fn failed_commit_leaves_memory_and_disk_unchanged() {
        let (directory, _unused) = setup();
        // A non-empty directory at the committed path makes the final rename fail.
        let path = directory.path().join("blocked").join("capture-policy.json");
        fs::create_dir_all(path.join("occupied")).unwrap();
        let mut live = LoadedPolicy {
            policy: saved(true, &["safari"]),
            error: None,
        };
        let before = live.clone();
        assert!(live.set_exclusions(&path, Vec::new()).is_err());
        assert!(live.set_paused(&path, false).is_err());
        assert_eq!(live, before);
        assert!(path.join("occupied").is_dir());
        assert!(!temporary_path(&path).exists());
    }

    #[test]
    fn rejected_policy_cannot_resume_or_be_silently_overwritten() {
        let (_directory, path) = setup();
        let corrupt = br#"{"paused":false,"excluded_apps":["bank"]"#.to_vec();
        fs::write(&path, &corrupt).unwrap();
        let mut live = load(&path);
        assert!(live.policy.paused);

        assert!(live.set_paused(&path, false).is_err());
        live.set_paused(&path, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        assert!(load(&path).error.is_some());

        live.set_exclusions(&path, vec!["bank".into()]).unwrap();
        assert_eq!(live.error, None);
        assert!(
            live.policy.paused,
            "re-stating exclusions must not resume capture"
        );
        assert_eq!(fs::read(rejected_path(&path)).unwrap(), corrupt);
        assert_eq!(load(&path).policy, saved(true, &["bank"]));

        live.set_paused(&path, false).unwrap();
        assert_eq!(load(&path).policy, saved(false, &["bank"]));
    }
}
