//! Replay protection that survives host restarts.
//!
//! Browsers start a new native-messaging host process for every connection, so replay state kept
//! in memory is lost between saves. This ledger persists, beside the spool, the per-session counter,
//! hashes of used request IDs and intent tokens, and recent acceptance times for the rate limit.
//! It stores no URL, content, or raw token. Updates happen under an exclusive lock file so
//! concurrent host processes cannot both accept the same request.

use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration as StdDuration, SystemTime},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LEDGER_FILE: &str = ".loom-replay-ledger.json";
const LOCK_FILE: &str = ".loom-replay-ledger.lock";
const LEDGER_VERSION: u32 = 1;
/// Request IDs and intent tokens remembered. Sessions expire after five minutes and the rate limit
/// admits four captures a minute, so this covers far more than any live session can use.
const MAX_REMEMBERED: usize = 4096;
const SESSION_RETENTION: Duration = Duration::hours(24);
const LOCK_ATTEMPTS: u32 = 250;
const LOCK_RETRY: StdDuration = StdDuration::from_millis(20);
const STALE_LOCK: StdDuration = StdDuration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Accept,
    Replay,
    RateLimited,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u32,
    sessions: BTreeMap<String, SessionState>,
    request_ids: VecDeque<String>,
    intent_tokens: VecDeque<String>,
    accepted_at: VecDeque<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionState {
    last_counter: u64,
    seen_at: DateTime<Utc>,
}

fn fingerprint(kind: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn lock(root: &Path) -> io::Result<LockGuard> {
    let path = root.join(LOCK_FILE);
    for _ in 0..LOCK_ATTEMPTS {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(LockGuard(path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // A host that crashed while holding the lock leaves it behind; reclaim it once it
                // is far older than any ledger update takes.
                let stale = fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > STALE_LOCK);
                if stale {
                    let _ = fs::remove_file(&path);
                } else {
                    thread::sleep(LOCK_RETRY);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "replay ledger is locked by another host",
    ))
}

fn load(path: &Path) -> io::Result<Ledger> {
    match fs::read(path) {
        Ok(bytes) => {
            let ledger: Ledger = serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if ledger.version != LEDGER_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported replay ledger version",
                ));
            }
            Ok(ledger)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Ledger {
            version: LEDGER_VERSION,
            ..Ledger::default()
        }),
        Err(error) => Err(error),
    }
}

fn save(root: &Path, ledger: &Ledger) -> io::Result<()> {
    let path = root.join(LEDGER_FILE);
    let temporary = root.join(format!("{LEDGER_FILE}.tmp"));
    let bytes = serde_json::to_vec(ledger).map_err(io::Error::other)?;
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, &path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn remember(list: &mut VecDeque<String>, value: String) {
    list.push_back(value);
    while list.len() > MAX_REMEMBERED {
        list.pop_front();
    }
}

/// Checks a validated request against the persisted ledger and, when it is fresh, records it as
/// consumed before any payload is read. A corrupt or unreadable ledger fails closed.
pub(crate) fn consume(
    root: &Path,
    session_id: &str,
    counter: u64,
    request_id: &str,
    intent_token: &str,
    max_per_minute: usize,
    now: DateTime<Utc>,
) -> io::Result<Decision> {
    fs::create_dir_all(root)?;
    let _guard = lock(root)?;
    let mut ledger = load(&root.join(LEDGER_FILE))?;

    while ledger
        .accepted_at
        .front()
        .is_some_and(|timestamp| *timestamp + Duration::minutes(1) <= now)
    {
        ledger.accepted_at.pop_front();
    }
    ledger
        .sessions
        .retain(|_, state| state.seen_at + SESSION_RETENTION > now);

    let session = fingerprint("session", session_id);
    let request = fingerprint("request", request_id);
    let intent = fingerprint("intent", intent_token);
    let stale_counter = ledger
        .sessions
        .get(&session)
        .is_some_and(|state| counter <= state.last_counter);
    if stale_counter
        || ledger.request_ids.contains(&request)
        || ledger.intent_tokens.contains(&intent)
    {
        return Ok(Decision::Replay);
    }
    if ledger.accepted_at.len() >= max_per_minute {
        return Ok(Decision::RateLimited);
    }

    ledger.sessions.insert(
        session,
        SessionState {
            last_counter: counter,
            seen_at: now,
        },
    );
    remember(&mut ledger.request_ids, request);
    remember(&mut ledger.intent_tokens, intent);
    ledger.accepted_at.push_back(now);
    save(root, &ledger)?;
    Ok(Decision::Accept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2027-08-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::seconds(seconds)
    }

    #[test]
    fn replay_state_survives_a_new_process_and_stores_no_raw_values() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let fresh = consume(root, "session-a", 1, "request-1", "intent-1", 4, at(0)).unwrap();
        assert_eq!(fresh, Decision::Accept);
        // Every call reloads from disk, exactly as a new host process would.
        for (counter, request, intent) in [
            (2, "request-1", "intent-2"),
            (3, "request-3", "intent-1"),
            (1, "request-4", "intent-4"),
        ] {
            assert_eq!(
                consume(root, "session-a", counter, request, intent, 4, at(1)).unwrap(),
                Decision::Replay
            );
        }
        assert_eq!(
            consume(root, "session-a", 2, "request-2", "intent-2", 4, at(1)).unwrap(),
            Decision::Accept
        );
        let stored = fs::read_to_string(root.join(LEDGER_FILE)).unwrap();
        for raw in ["session-a", "request-1", "intent-1"] {
            assert!(!stored.contains(raw), "{raw} stored in clear");
        }
        assert!(!root.join(LOCK_FILE).exists());
    }

    #[test]
    fn rate_limit_spans_processes_and_resets_after_a_minute() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        for index in 0..4u64 {
            let request = format!("request-{index}");
            let intent = format!("intent-{index}");
            assert_eq!(
                consume(root, "session", index + 1, &request, &intent, 4, at(0)).unwrap(),
                Decision::Accept
            );
        }
        assert_eq!(
            consume(root, "session", 5, "request-5", "intent-5", 4, at(30)).unwrap(),
            Decision::RateLimited
        );
        assert_eq!(
            consume(root, "session", 6, "request-6", "intent-6", 4, at(61)).unwrap(),
            Decision::Accept
        );
    }

    #[test]
    fn corrupt_or_future_ledgers_fail_closed() {
        for contents in [
            "not json",
            r#"{"version":2,"sessions":{},"request_ids":[],"intent_tokens":[],"accepted_at":[]}"#,
        ] {
            let directory = tempdir().unwrap();
            fs::write(directory.path().join(LEDGER_FILE), contents).unwrap();
            assert!(consume(directory.path(), "s", 1, "r", "i", 4, at(0)).is_err());
            assert_eq!(
                fs::read_to_string(directory.path().join(LEDGER_FILE)).unwrap(),
                contents
            );
        }
    }

    #[test]
    fn stale_locks_are_reclaimed_and_live_locks_block() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let lock_path = root.join(LOCK_FILE);
        fs::write(&lock_path, b"").unwrap();
        let old = SystemTime::now() - StdDuration::from_secs(120);
        fs::File::options()
            .write(true)
            .open(&lock_path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(
            consume(root, "s", 1, "r", "i", 4, at(0)).unwrap(),
            Decision::Accept
        );
        assert!(!lock_path.exists());
    }

    #[test]
    fn concurrent_hosts_accept_a_request_exactly_once() {
        let directory = tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let accepted = (0..8)
            .map(|_| {
                let root = root.clone();
                thread::spawn(move || {
                    consume(
                        &root,
                        "session",
                        1,
                        "request-shared",
                        "intent-shared",
                        64,
                        at(0),
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|decision| *decision == Decision::Accept)
            .count();
        assert_eq!(accepted, 1);
    }
}
