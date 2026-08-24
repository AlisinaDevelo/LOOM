use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{error::LoomError, Result};

/// A filesystem observation hint. Hints are never treated as canonical source truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationEventKind {
    Created,
    Modified,
    Removed,
    Renamed,
    Overflow,
}

/// One bounded event from a future native watcher or a polling adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationEvent {
    pub kind: ObservationEventKind,
    pub path: PathBuf,
    #[serde(default)]
    pub previous_path: Option<PathBuf>,
}

/// Deterministic, scope-checked event output for one debounce window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationPlan {
    pub events_received: u64,
    pub paths_coalesced: u64,
    pub full_rescan: bool,
    pub changed_paths: Vec<PathBuf>,
    pub removed_paths: Vec<PathBuf>,
}

/// Coalesces a bounded event window and rejects paths outside the approved root.
///
/// A rename contributes both a removal and a change. A later create/modify wins over a removal
/// for the same path, while a later removal wins over an earlier change. Overflow or an event
/// window larger than `max_events` requests a conservative full rescan.
pub fn coalesce_events(
    approved_root: impl AsRef<Path>,
    events: &[ObservationEvent],
    max_events: usize,
) -> Result<ObservationPlan> {
    let requested_root = approved_root.as_ref();
    let root = fs::canonicalize(requested_root)
        .map_err(|source| crate::error::io_error(requested_root, source))?;
    let mut changed = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut full_rescan = events.len() > max_events;

    for event in events {
        let path = scoped_path(&root, &event.path)?;
        match event.kind {
            ObservationEventKind::Created | ObservationEventKind::Modified => {
                changed.insert(path.clone());
                removed.remove(&path);
            }
            ObservationEventKind::Removed => {
                changed.remove(&path);
                removed.insert(path);
            }
            ObservationEventKind::Renamed => {
                let previous = event.previous_path.as_ref().ok_or_else(|| {
                    LoomError::InvalidPath("rename observation is missing its previous path".into())
                })?;
                let previous = scoped_path(&root, previous)?;
                changed.remove(&previous);
                removed.insert(previous);
                removed.remove(&path);
                changed.insert(path);
            }
            ObservationEventKind::Overflow => {
                full_rescan = true;
            }
        }
    }

    if full_rescan {
        changed.clear();
        removed.clear();
    }

    Ok(ObservationPlan {
        events_received: events.len() as u64,
        paths_coalesced: (changed.len() + removed.len()) as u64,
        full_rescan,
        changed_paths: changed.into_iter().collect(),
        removed_paths: removed.into_iter().collect(),
    })
}

fn scoped_path(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(LoomError::InvalidPath(format!(
            "observation path is outside the approved root: {}",
            path.display()
        )));
    }
    let normalized = if path.exists() {
        fs::canonicalize(path).map_err(|source| crate::error::io_error(path, source))?
    } else {
        let parent = path.parent().ok_or_else(|| {
            LoomError::InvalidPath(format!(
                "observation path has no parent: {}",
                path.display()
            ))
        })?;
        let name = path.file_name().ok_or_else(|| {
            LoomError::InvalidPath(format!(
                "observation path has no filename: {}",
                path.display()
            ))
        })?;
        fs::canonicalize(parent)
            .map_err(|source| crate::error::io_error(parent, source))?
            .join(name)
    };
    if !normalized.starts_with(root) {
        return Err(LoomError::InvalidPath(format!(
            "observation path resolves outside the approved root: {}",
            path.display()
        )));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{coalesce_events, ObservationEvent, ObservationEventKind};

    #[test]
    fn coalesces_changes_removes_and_renames_deterministically() {
        let directory = tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let old = root.join("old.md");
        let new = root.join("new.md");
        fs::write(&old, "old").unwrap();
        fs::write(&new, "new").unwrap();

        let plan = coalesce_events(
            &root,
            &[
                ObservationEvent {
                    kind: ObservationEventKind::Modified,
                    path: old.clone(),
                    previous_path: None,
                },
                ObservationEvent {
                    kind: ObservationEventKind::Removed,
                    path: old.clone(),
                    previous_path: None,
                },
                ObservationEvent {
                    kind: ObservationEventKind::Renamed,
                    path: new.clone(),
                    previous_path: Some(old.clone()),
                },
                ObservationEvent {
                    kind: ObservationEventKind::Modified,
                    path: new.clone(),
                    previous_path: None,
                },
            ],
            20,
        )
        .unwrap();

        assert_eq!(plan.events_received, 4);
        assert_eq!(plan.paths_coalesced, 2);
        assert!(!plan.full_rescan);
        assert_eq!(plan.changed_paths, vec![new]);
        assert_eq!(plan.removed_paths, vec![old]);
    }

    #[test]
    fn overflow_or_large_batches_request_full_rescan() {
        let directory = tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let path = root.join("notes.md");
        fs::write(&path, "notes").unwrap();
        let event = ObservationEvent {
            kind: ObservationEventKind::Overflow,
            path: root.clone(),
            previous_path: None,
        };

        let overflow = coalesce_events(&root, std::slice::from_ref(&event), 20).unwrap();
        assert!(overflow.full_rescan);
        assert_eq!(overflow.paths_coalesced, 0);

        let large = coalesce_events(&root, &[event.clone(), event], 1).unwrap();
        assert!(large.full_rescan);
        assert_eq!(large.events_received, 2);
    }

    #[test]
    fn rejects_out_of_scope_and_incomplete_rename_hints() {
        let directory = tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let outside = tempdir().unwrap().path().join("outside.md");
        let out_of_scope = ObservationEvent {
            kind: ObservationEventKind::Modified,
            path: outside,
            previous_path: None,
        };
        assert!(coalesce_events(&root, &[out_of_scope], 20).is_err());

        let incomplete = ObservationEvent {
            kind: ObservationEventKind::Renamed,
            path: root.join("new.md"),
            previous_path: None,
        };
        assert!(coalesce_events(&root, &[incomplete], 20).is_err());
    }
}
