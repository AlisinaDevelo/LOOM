//! Capture consent and retention lifecycle (roadmap `0310`).
//!
//! Every browser capture moves through an explicit state machine. The state decides what LOOM may
//! hold: a request is only in memory until the host decides; only an accepted capture is written to
//! the spool; a revoked capture stays on disk but unavailable; a deleted capture keeps a tombstone
//! with its identifiers, state history, and snapshot hash, never its URL, title, selection, or bytes.
//! Rejected and expired requests are never persisted. Credentials and referrers are never accepted
//! in any state (see the forbidden-field list in the protocol).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Lifecycle states. `Rejected`, `Expired`, and `Deleted` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Requested,
    Accepted,
    Rejected,
    Expired,
    Revoked,
    Deleted,
}

impl CaptureState {
    pub const ALL: [CaptureState; 6] = [
        CaptureState::Requested,
        CaptureState::Accepted,
        CaptureState::Rejected,
        CaptureState::Expired,
        CaptureState::Revoked,
        CaptureState::Deleted,
    ];

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Expired | Self::Deleted)
    }

    pub fn handling(self) -> StateHandling {
        match self {
            Self::Requested => StateHandling {
                persisted: false,
                url: Retention::HeldUntilDecision,
                snapshot: Retention::HeldUntilDecision,
                user_message: "Waiting for LOOM to accept this save.",
            },
            Self::Accepted => StateHandling {
                persisted: true,
                url: Retention::Stored,
                snapshot: Retention::Stored,
                user_message: "Saved to LOOM.",
            },
            Self::Rejected => StateHandling {
                persisted: false,
                url: Retention::NotStored,
                snapshot: Retention::NotStored,
                user_message: "LOOM rejected this save; nothing was stored.",
            },
            Self::Expired => StateHandling {
                persisted: false,
                url: Retention::NotStored,
                snapshot: Retention::NotStored,
                user_message: "This save expired before LOOM accepted it; nothing was stored.",
            },
            Self::Revoked => StateHandling {
                persisted: true,
                url: Retention::StoredUnavailable,
                snapshot: Retention::StoredUnavailable,
                user_message:
                    "Browser pairing was revoked; this capture is unavailable until you delete it or pair again.",
            },
            Self::Deleted => StateHandling {
                persisted: true,
                url: Retention::Removed,
                snapshot: Retention::Removed,
                user_message:
                    "Deleted; only the capture ID, state history, and snapshot hash remain.",
            },
        }
    }
}

/// What LOOM holds for one kind of capture data in a given state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    /// Held in host memory only while the request is validated; never written.
    HeldUntilDecision,
    /// Never written to disk.
    NotStored,
    /// Written to the local spool.
    Stored,
    /// Still on disk, but must not be shown, searched, or exported.
    StoredUnavailable,
    /// Removed from disk; at most a hash remains in the tombstone.
    Removed,
}

/// Per-state data handling. Credentials and referrers are never accepted in any state, so they have
/// no field here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StateHandling {
    pub persisted: bool,
    pub url: Retention,
    pub snapshot: Retention,
    pub user_message: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureEvent {
    Accept,
    Reject,
    Expire,
    Revoke,
    Repair,
    Delete,
}

impl CaptureEvent {
    pub const ALL: [CaptureEvent; 6] = [
        CaptureEvent::Accept,
        CaptureEvent::Reject,
        CaptureEvent::Expire,
        CaptureEvent::Revoke,
        CaptureEvent::Repair,
        CaptureEvent::Delete,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    #[error("capture cannot go from {from:?} on {event:?}")]
    InvalidTransition {
        from: CaptureState,
        event: CaptureEvent,
    },
    #[error("capture record is missing or unreadable: {0}")]
    Record(String),
    #[error("capture storage failed: {0}")]
    Io(String),
}

impl From<io::Error> for LifecycleError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// The complete transition table. Anything not listed is rejected.
pub fn next_state(from: CaptureState, event: CaptureEvent) -> Result<CaptureState, LifecycleError> {
    use CaptureEvent as E;
    use CaptureState as S;
    match (from, event) {
        (S::Requested, E::Accept) => Ok(S::Accepted),
        (S::Requested, E::Reject) => Ok(S::Rejected),
        (S::Requested, E::Expire) => Ok(S::Expired),
        (S::Accepted, E::Revoke) => Ok(S::Revoked),
        (S::Accepted, E::Delete) => Ok(S::Deleted),
        (S::Revoked, E::Repair) => Ok(S::Accepted),
        (S::Revoked, E::Delete) => Ok(S::Deleted),
        _ => Err(LifecycleError::InvalidTransition { from, event }),
    }
}

/// Classifies a host rejection code: stale sessions and intents expire, everything else is rejected.
pub fn rejection_state(code: &str) -> CaptureState {
    match code {
        "session_expired" | "capture_time_invalid" => CaptureState::Expired,
        _ => CaptureState::Rejected,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub state: CaptureState,
    pub at: String,
}

/// The lifecycle stored in each spool metadata record under `lifecycle`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    pub state: CaptureState,
    pub transitions: Vec<Transition>,
}

impl Lifecycle {
    /// The lifecycle of a newly accepted capture: requested at the user gesture, then accepted.
    pub fn accepted(requested_at: &str, accepted_at: DateTime<Utc>) -> Self {
        Self {
            state: CaptureState::Accepted,
            transitions: vec![
                Transition {
                    state: CaptureState::Requested,
                    at: requested_at.to_owned(),
                },
                Transition {
                    state: CaptureState::Accepted,
                    at: timestamp(accepted_at),
                },
            ],
        }
    }

    pub fn apply(&mut self, event: CaptureEvent, at: DateTime<Utc>) -> Result<(), LifecycleError> {
        let state = next_state(self.state, event)?;
        self.state = state;
        self.transitions.push(Transition {
            state,
            at: timestamp(at),
        });
        Ok(())
    }
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Fields a tombstone keeps. Everything else in the metadata record is removed on deletion.
const TOMBSTONE_FIELDS: &[&str] = &["capture_id", "request_id", "protocol", "lifecycle"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeletionReport {
    pub request_id: String,
    /// True when the capture was already deleted and this call only finished any leftover cleanup.
    pub already_deleted: bool,
    pub snapshot_removed: bool,
    pub temporary_files_removed: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TransitionReport {
    pub changed: Vec<String>,
    pub unchanged: u64,
}

fn metadata_path(root: &Path, request_id: &str) -> PathBuf {
    root.join(format!("{request_id}.json"))
}

fn snapshot_path(root: &Path, request_id: &str) -> PathBuf {
    root.join(format!("{request_id}.html"))
}

fn valid_request_id(request_id: &str) -> bool {
    uuid::Uuid::parse_str(request_id).is_ok_and(|id| id.hyphenated().to_string() == request_id)
}

fn read_record(path: &Path) -> Result<Map<String, Value>, LifecycleError> {
    let bytes = fs::read(path).map_err(|error| LifecycleError::Record(error.to_string()))?;
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(LifecycleError::Record(format!(
            "not a capture record: {}",
            path.display()
        ))),
    }
}

/// Reads a record's lifecycle. Records written before lifecycles existed are accepted captures.
pub fn record_lifecycle(record: &Map<String, Value>) -> Result<Lifecycle, LifecycleError> {
    match record.get("lifecycle") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| LifecycleError::Record(error.to_string())),
        None => {
            let accepted_at = record
                .get("accepted_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(Lifecycle {
                state: CaptureState::Accepted,
                transitions: vec![Transition {
                    state: CaptureState::Accepted,
                    at: accepted_at,
                }],
            })
        }
    }
}

fn write_record(path: &Path, record: &Map<String, Value>) -> Result<(), LifecycleError> {
    let bytes =
        serde_json::to_vec(record).map_err(|error| LifecycleError::Io(error.to_string()))?;
    let temporary = path.with_extension("json.lifecycle.tmp");
    let result = fs::write(&temporary, bytes)
        .and_then(|()| fs::File::open(&temporary)?.sync_all())
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(LifecycleError::from)
}

/// Deletes one capture: the metadata record becomes a tombstone first, then the snapshot and any
/// temporary files for the capture are removed. Retrying after an interruption finishes the cleanup.
pub fn delete_capture(
    root: &Path,
    request_id: &str,
    at: DateTime<Utc>,
) -> Result<DeletionReport, LifecycleError> {
    if !valid_request_id(request_id) {
        return Err(LifecycleError::Record(format!(
            "invalid request ID: {request_id}"
        )));
    }
    let path = metadata_path(root, request_id);
    let record = read_record(&path)?;
    let mut lifecycle = record_lifecycle(&record)?;
    let already_deleted = lifecycle.state == CaptureState::Deleted;
    if !already_deleted {
        lifecycle.apply(CaptureEvent::Delete, at)?;
        let mut tombstone = Map::new();
        for field in TOMBSTONE_FIELDS {
            if let Some(value) = record.get(*field) {
                tombstone.insert((*field).into(), value.clone());
            }
        }
        if let Some(hash) = record
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("content_hash"))
            .filter(|hash| hash.is_string())
        {
            tombstone.insert("snapshot_content_hash".into(), hash.clone());
        }
        tombstone.insert(
            "lifecycle".into(),
            serde_json::to_value(&lifecycle)
                .map_err(|error| LifecycleError::Io(error.to_string()))?,
        );
        write_record(&path, &tombstone)?;
    }

    let mut report = DeletionReport {
        request_id: request_id.to_owned(),
        already_deleted,
        ..DeletionReport::default()
    };
    match fs::remove_file(snapshot_path(root, request_id)) {
        Ok(()) => report.snapshot_removed = true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for temporary in [
        root.join(format!(".{request_id}.html.tmp")),
        root.join(format!(".{request_id}.json.tmp")),
        path.with_extension("json.lifecycle.tmp"),
    ] {
        match fs::remove_file(&temporary) {
            Ok(()) => report.temporary_files_removed += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(report)
}

fn transition_spool(
    root: &Path,
    from: CaptureState,
    event: CaptureEvent,
    at: DateTime<Utc>,
) -> Result<TransitionReport, LifecycleError> {
    let mut report = TransitionReport::default();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error.into()),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if valid_request_id(stem) {
            paths.push((stem.to_owned(), path));
        }
    }
    paths.sort();
    for (request_id, path) in paths {
        let mut record = read_record(&path)?;
        let mut lifecycle = record_lifecycle(&record)?;
        if lifecycle.state != from {
            report.unchanged += 1;
            continue;
        }
        lifecycle.apply(event, at)?;
        record.insert(
            "lifecycle".into(),
            serde_json::to_value(&lifecycle)
                .map_err(|error| LifecycleError::Io(error.to_string()))?,
        );
        write_record(&path, &record)?;
        report.changed.push(request_id);
    }
    Ok(report)
}

/// Marks every accepted capture unavailable after the browser pairing is revoked.
pub fn revoke_spool(root: &Path, at: DateTime<Utc>) -> Result<TransitionReport, LifecycleError> {
    transition_spool(root, CaptureState::Accepted, CaptureEvent::Revoke, at)
}

/// Makes revoked captures available again after the user explicitly pairs the browser again.
pub fn repair_spool(root: &Path, at: DateTime<Utc>) -> Result<TransitionReport, LifecycleError> {
    transition_spool(root, CaptureState::Revoked, CaptureEvent::Repair, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Fixture {
        transitions: Vec<FixtureCase>,
    }

    #[derive(Deserialize)]
    struct FixtureCase {
        from: CaptureState,
        event: CaptureEvent,
        to: Option<CaptureState>,
    }

    #[test]
    fn transition_fixture_covers_every_state_and_event() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../fixtures/lifecycle-v1.json")).unwrap();
        assert_eq!(
            fixture.transitions.len(),
            CaptureState::ALL.len() * CaptureEvent::ALL.len()
        );
        for state in CaptureState::ALL {
            for event in CaptureEvent::ALL {
                let case = fixture
                    .transitions
                    .iter()
                    .find(|case| case.from == state && case.event == event)
                    .unwrap_or_else(|| panic!("fixture misses {state:?} + {event:?}"));
                assert_eq!(
                    next_state(state, event).ok(),
                    case.to,
                    "{state:?} + {event:?}"
                );
            }
        }
    }

    #[test]
    fn terminal_states_accept_no_event() {
        for state in CaptureState::ALL
            .into_iter()
            .filter(|state| state.is_terminal())
        {
            for event in CaptureEvent::ALL {
                assert!(next_state(state, event).is_err());
            }
        }
    }

    #[test]
    fn only_accepted_revoked_and_deleted_are_persisted() {
        for state in CaptureState::ALL {
            let handling = state.handling();
            assert_eq!(
                handling.persisted,
                matches!(
                    state,
                    CaptureState::Accepted | CaptureState::Revoked | CaptureState::Deleted
                ),
                "{state:?}"
            );
            if !handling.persisted {
                assert_ne!(handling.url, Retention::Stored);
                assert_ne!(handling.snapshot, Retention::Stored);
            }
        }
    }

    #[test]
    fn protocol_documents_every_state_message() {
        let protocol = include_str!("../../../docs/protocol/browser-capture-v1.md");
        for state in CaptureState::ALL {
            let message = state.handling().user_message;
            assert!(
                protocol.contains(message),
                "protocol is missing the {state:?} message: {message}"
            );
        }
    }

    #[test]
    fn rejection_codes_map_to_rejected_or_expired() {
        assert_eq!(rejection_state("session_expired"), CaptureState::Expired);
        assert_eq!(
            rejection_state("capture_time_invalid"),
            CaptureState::Expired
        );
        assert_eq!(rejection_state("replay_rejected"), CaptureState::Rejected);
        assert_eq!(
            rejection_state("snapshot_untrusted"),
            CaptureState::Rejected
        );
    }
}
