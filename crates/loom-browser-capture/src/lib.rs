//! A fail-closed native-messaging boundary for LOOM's explicit-save browser connector.
//!
//! The browser extension is intentionally small and sends only authenticated, user-initiated
//! envelopes. This crate is the other half of that contract: it owns native-messaging framing,
//! duplicate-key rejection, protocol validation, replay/rate limits, payload integrity, and an
//! atomic sanitized-capture spool. It does not fetch URLs, execute HTML, or write anything until a
//! complete payload has passed all checks.

use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use url::Url;
use uuid::{Uuid, Version};

type HmacSha256 = Hmac<Sha256>;

pub mod lifecycle;
pub mod replay;

pub const PROTOCOL_MAJOR: u64 = 1;
pub const PROTOCOL_MINOR: u64 = 0;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_SELECTED_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PAYLOAD_CHUNK_BYTES: usize = 160 * 1024;
pub const MAX_TITLE_BYTES: usize = 512;
pub const MAX_URL_BYTES: usize = 16 * 1024;
pub const MAX_REDIRECTS: usize = 8;
pub const MAX_REDIRECT_URL_BYTES: usize = 2048;
pub const MAX_REQUESTS_PER_MINUTE: usize = 4;
pub const MAX_CLOCK_SKEW: Duration = Duration::minutes(2);

const FORBIDDEN_FIELDS: &[&str] = &[
    "cookies",
    "authorization",
    "request_body",
    "password",
    "hidden_form_values",
    "autofill",
    "local_storage",
    "session_storage",
    "service_worker_cache",
    "browser_history",
    "tab_list",
    "page_scripts",
    "remote_resources",
    "event_handlers",
];

#[derive(Debug, Error)]
pub enum HostError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("native-messaging frame is invalid: {0}")]
    Frame(String),
    #[error("host configuration is invalid: {0}")]
    Configuration(String),
    #[error("protocol rejected: {code}")]
    Rejected {
        code: String,
        request_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub key_id: String,
    pub pairing_secret: Vec<u8>,
    pub spool_root: PathBuf,
    /// Browser callers allowed to launch this host, as normalized by [`caller_from_args`]:
    /// `chrome-extension://<id>/` for Chromium browsers and `firefox:<id>` for Firefox.
    pub allowed_callers: Vec<String>,
}

/// Identifies the browser extension that launched the host from its command-line arguments.
///
/// Chromium passes the caller origin (`chrome-extension://<id>/`) as the first argument, followed
/// on Windows by `--parent-window=<handle>`. Firefox passes the path of the host manifest and then
/// the calling extension's ID. Anything else yields `None`, which the host treats as untrusted.
pub fn caller_from_args(args: &[String]) -> Option<String> {
    if let Some(origin) = args
        .first()
        .and_then(|first| first.strip_prefix("chrome-extension://"))
    {
        let id = origin.strip_suffix('/')?;
        let valid = id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte));
        return valid.then(|| format!("chrome-extension://{id}/"));
    }
    if args.len() == 2 && args[0].ends_with(".json") {
        let id = &args[1];
        let valid = !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"@._-{}".contains(&byte));
        return valid.then(|| format!("firefox:{id}"));
    }
    None
}

impl HostConfig {
    /// Fails closed unless the launching extension is on the configured allowlist. The browser's
    /// own host-manifest allowlist is the first check; this second check means a copied or
    /// re-registered host binary still refuses an extension that was never paired.
    pub fn verify_caller(&self, args: &[String]) -> Result<String, HostError> {
        let caller = caller_from_args(args).ok_or_else(|| {
            HostError::Configuration("the calling browser extension could not be identified".into())
        })?;
        if self
            .allowed_callers
            .iter()
            .any(|allowed| allowed == &caller)
        {
            Ok(caller)
        } else {
            Err(HostError::Configuration(format!(
                "browser extension {caller} is not paired with this host"
            )))
        }
    }
}

impl HostConfig {
    /// Read the development/test configuration from the environment.
    ///
    /// A production installer must replace this with a Keychain-backed provider. Refusing to
    /// start without an explicitly supplied secret prevents a native host from silently
    /// accepting unauthenticated browser traffic.
    pub fn from_env() -> Result<Self, HostError> {
        let key_id = std::env::var("LOOM_NATIVE_HOST_KEY_ID")
            .map_err(|_| HostError::Configuration("LOOM_NATIVE_HOST_KEY_ID is not set".into()))?;
        let secret_hex = std::env::var("LOOM_NATIVE_HOST_SECRET_HEX").map_err(|_| {
            HostError::Configuration("LOOM_NATIVE_HOST_SECRET_HEX is not set".into())
        })?;
        let pairing_secret = decode_hex(&secret_hex).ok_or_else(|| {
            HostError::Configuration("pairing secret must be 32 bytes of hex".into())
        })?;
        if pairing_secret.len() != 32 {
            return Err(HostError::Configuration(
                "pairing secret must be exactly 32 bytes".into(),
            ));
        }
        let spool_root = std::env::var_os("LOOM_NATIVE_HOST_SPOOL")
            .map(PathBuf::from)
            .ok_or_else(|| HostError::Configuration("LOOM_NATIVE_HOST_SPOOL is not set".into()))?;
        if key_id.is_empty() {
            return Err(HostError::Configuration("key id must not be empty".into()));
        }
        let allowed_callers = std::env::var("LOOM_NATIVE_HOST_ALLOWED_CALLERS")
            .map_err(|_| {
                HostError::Configuration("LOOM_NATIVE_HOST_ALLOWED_CALLERS is not set".into())
            })?
            .split(',')
            .map(str::trim)
            .filter(|caller| !caller.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if allowed_callers.is_empty() {
            return Err(HostError::Configuration(
                "at least one allowed browser caller is required".into(),
            ));
        }
        Ok(Self {
            key_id,
            pairing_secret,
            spool_root,
            allowed_callers,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCapture {
    pub capture_id: String,
    pub request_id: String,
    pub snapshot_state: String,
    pub source_hash: Option<String>,
    pub bytes: usize,
}

#[derive(Debug)]
pub struct NativeHost {
    config: HostConfig,
}

impl NativeHost {
    pub fn new(config: HostConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &HostConfig {
        &self.config
    }

    /// Run the stdio native-messaging loop using the host's current UTC clock.
    pub fn run<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), HostError> {
        self.run_at(reader, writer, Utc::now())
    }

    /// Deterministic variant used by fixture and device smoke tests.
    pub fn run_at<R: Read, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        now: DateTime<Utc>,
    ) -> Result<(), HostError> {
        while let Some(raw) = read_frame(reader)? {
            let value = match parse_strict_json(&raw) {
                Ok(value) => value,
                Err(_) => {
                    write_response(writer, rejected_response(None, "malformed_frame"))?;
                    continue;
                }
            };
            let request_id = value
                .get("request_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let message_type = value.get("type").and_then(Value::as_str);
            if message_type != Some("capture.request") {
                write_response(
                    writer,
                    rejected_response(request_id, "message_type_unsupported"),
                )?;
                continue;
            }
            match self.accept_request(value, reader, now) {
                Ok(accepted) => write_response(writer, accepted_response(&accepted))?,
                Err(rejection) => write_response(
                    writer,
                    rejected_response(rejection.request_id, &rejection.code),
                )?,
            }
        }
        Ok(())
    }

    fn accept_request<R: Read>(
        &mut self,
        value: Value,
        reader: &mut R,
        now: DateTime<Utc>,
    ) -> Result<AcceptedCapture, Rejection> {
        let request = validate_capture_request(&value, &self.config, now)
            .map_err(|code| self.rejection(code, request_id_from(&value)))?;
        // Consume the request, counter, and intent in the persisted ledger before reading payload
        // chunks. Browsers start a new host process per connection, so this state must outlive the
        // process. A rejected payload is not retryable with the same gesture or request ID;
        // recovery is a fresh visible save with a new counter and intent token.
        let decision = replay::consume(
            &self.config.spool_root,
            &request.session_id,
            request.counter,
            &request.request_id,
            &request.intent_token,
            MAX_REQUESTS_PER_MINUTE,
            now,
        );
        match decision {
            Ok(replay::Decision::Accept) => {}
            Ok(replay::Decision::Replay) => {
                self.discard_payloads(&value, reader);
                return Err(self.rejection("replay_rejected", Some(request.request_id.clone())));
            }
            Ok(replay::Decision::RateLimited) => {
                self.discard_payloads(&value, reader);
                return Err(
                    self.rejection("capture_rate_limited", Some(request.request_id.clone()))
                );
            }
            Err(_) => {
                self.discard_payloads(&value, reader);
                return Err(
                    self.rejection("replay_state_unavailable", Some(request.request_id.clone()))
                );
            }
        }

        let mut snapshot_bytes = Vec::with_capacity(request.snapshot_bytes.min(MAX_SNAPSHOT_BYTES));
        if request.snapshot_bytes > 0 {
            let mut expected_sequence = 0u64;
            let mut final_seen = false;
            while !final_seen {
                let raw = read_frame(reader).map_err(|_| {
                    self.rejection("payload_integrity_failed", Some(request.request_id.clone()))
                })?;
                let Some(raw) = raw else {
                    return Err(self
                        .rejection("payload_integrity_failed", Some(request.request_id.clone())));
                };
                let payload = parse_strict_json(&raw).map_err(|_| {
                    self.rejection("payload_integrity_failed", Some(request.request_id.clone()))
                })?;
                let chunk =
                    validate_payload_frame(&payload, &self.config, &request, expected_sequence)
                        .map_err(|code| self.rejection(code, Some(request.request_id.clone())))?;
                if snapshot_bytes.len() + chunk.bytes.len() > request.snapshot_bytes {
                    return Err(self
                        .rejection("payload_integrity_failed", Some(request.request_id.clone())));
                }
                snapshot_bytes.extend_from_slice(&chunk.bytes);
                expected_sequence += 1;
                final_seen = chunk.final_frame;
                if final_seen && snapshot_bytes.len() != request.snapshot_bytes {
                    return Err(self
                        .rejection("payload_integrity_failed", Some(request.request_id.clone())));
                }
            }
            let digest = format!("sha256:{}", hex_encode(&Sha256::digest(&snapshot_bytes)));
            if request.snapshot_hash.as_deref() != Some(digest.as_str()) {
                return Err(
                    self.rejection("payload_integrity_failed", Some(request.request_id.clone()))
                );
            }
            reject_untrusted_html(&snapshot_bytes)
                .map_err(|code| self.rejection(code, Some(request.request_id.clone())))?;
        }

        let accepted = AcceptedCapture {
            capture_id: format!("browser-{}", request.request_id),
            request_id: request.request_id.clone(),
            snapshot_state: request.snapshot_state.clone(),
            source_hash: request.snapshot_hash.clone(),
            bytes: snapshot_bytes.len(),
        };
        persist_capture(
            &self.config.spool_root,
            &request.value,
            &accepted,
            &snapshot_bytes,
        )
        .map_err(|_| self.rejection("capture_storage_failed", Some(request.request_id.clone())))?;
        Ok(accepted)
    }

    fn discard_payloads<R: Read>(&self, value: &Value, reader: &mut R) {
        let Some(expected) = value
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("bytes"))
            .and_then(Value::as_u64)
        else {
            return;
        };
        if expected == 0 || expected > MAX_SNAPSHOT_BYTES as u64 {
            return;
        }
        let mut consumed = 0u64;
        for _ in 0..=((MAX_SNAPSHOT_BYTES / MAX_PAYLOAD_CHUNK_BYTES) as u64) {
            let Ok(Some(frame)) = read_frame(reader) else {
                return;
            };
            let Ok(payload) = parse_strict_json(&frame) else {
                return;
            };
            let Some(bytes) = payload.get("bytes").and_then(Value::as_u64) else {
                return;
            };
            consumed = consumed.saturating_add(bytes);
            if payload.get("final").and_then(Value::as_bool) == Some(true) || consumed >= expected {
                return;
            }
        }
    }

    fn rejection(&self, code: impl Into<String>, request_id: Option<String>) -> Rejection {
        Rejection {
            code: code.into(),
            request_id,
        }
    }
}

#[derive(Debug)]
struct Rejection {
    code: String,
    request_id: Option<String>,
}

#[derive(Debug)]
struct RequestView {
    value: Value,
    request_id: String,
    session_id: String,
    counter: u64,
    intent_token: String,
    snapshot_state: String,
    snapshot_hash: Option<String>,
    snapshot_bytes: usize,
}

#[derive(Debug)]
struct PayloadChunk {
    bytes: Vec<u8>,
    final_frame: bool,
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, HostError> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(HostError::Frame("frame exceeds 256 KiB".into()));
    }
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| HostError::Frame(format!("frame body is truncated: {error}")))?;
    Ok(Some(body))
}

pub fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> Result<(), HostError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(HostError::Frame("response exceeds 256 KiB".into()));
    }
    let length = u32::try_from(body.len())
        .map_err(|_| HostError::Frame("response length does not fit native messaging".into()))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(body)?;
    writer.flush()?;
    Ok(())
}

fn write_response<W: Write>(writer: &mut W, response: Value) -> Result<(), HostError> {
    let body = serde_json::to_vec(&response)
        .map_err(|error| HostError::Frame(format!("response JSON failed: {error}")))?;
    write_frame(writer, &body)
}

fn accepted_response(accepted: &AcceptedCapture) -> Value {
    serde_json::json!({
        "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
        "type": "capture.accepted",
        "request_id": accepted.request_id,
        "capture_id": accepted.capture_id,
        "snapshot": {
            "state": accepted.snapshot_state,
            "content_hash": accepted.source_hash,
            "bytes": accepted.bytes,
        },
    })
}

fn rejected_response(request_id: Option<String>, code: &str) -> Value {
    serde_json::json!({
        "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
        "type": "capture.rejected",
        "request_id": request_id,
        "error": code,
    })
}

fn validate_capture_request(
    value: &Value,
    config: &HostConfig,
    now: DateTime<Utc>,
) -> Result<RequestView, String> {
    let root = expect_object(value, "capture_envelope")?;
    exact_keys(
        root,
        &[
            "protocol",
            "type",
            "request_id",
            "session",
            "intent",
            "source",
            "snapshot",
            "auth",
        ],
    )?;
    reject_forbidden_fields(value)?;
    let protocol = expect_object(root.get("protocol").ok_or("protocol_missing")?, "protocol")?;
    exact_keys(protocol, &["major", "minor"])?;
    if protocol.get("major").and_then(Value::as_u64) != Some(PROTOCOL_MAJOR)
        || protocol.get("minor").and_then(Value::as_u64) != Some(PROTOCOL_MINOR)
    {
        return Err("protocol_version_unsupported".into());
    }
    if root.get("type").and_then(Value::as_str) != Some("capture.request") {
        return Err("message_type_unsupported".into());
    }
    let request_id = root
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("request_id_invalid")?;
    let uuid = Uuid::parse_str(request_id).map_err(|_| "request_id_invalid")?;
    if uuid.get_version() != Some(Version::Random) {
        return Err("request_id_invalid".into());
    }

    let session = expect_object(root.get("session").ok_or("session_missing")?, "session")?;
    exact_keys(session, &["id", "counter", "issued_at", "expires_at"])?;
    let session_id = non_empty_string(session, "id", "session_id_missing")?.to_owned();
    let counter = session
        .get("counter")
        .and_then(Value::as_u64)
        .filter(|counter| *counter > 0)
        .ok_or("session_counter_invalid")?;
    validate_session_times(session, now)?;
    let intent = expect_object(root.get("intent").ok_or("intent_missing")?, "intent")?;
    exact_keys(intent, &["kind", "token", "issued_at"])?;
    if intent.get("kind").and_then(Value::as_str) != Some("user_gesture") {
        return Err("capture_not_approved".into());
    }
    let intent_token = non_empty_string(intent, "token", "capture_not_approved")?.to_owned();
    let intent_at = parse_utc(intent.get("issued_at").ok_or("capture_time_invalid")?)?;
    if (intent_at - now).abs() > MAX_CLOCK_SKEW {
        return Err("capture_time_invalid".into());
    }

    let source = expect_object(root.get("source").ok_or("source_missing")?, "source")?;
    exact_keys(
        source,
        &[
            "live_url",
            "final_url",
            "title",
            "selected_text",
            "captured_at",
            "redirects",
            "scope",
        ],
    )?;
    validate_url(source, "live_url", MAX_URL_BYTES, "source_invalid")?;
    validate_url(source, "final_url", MAX_URL_BYTES, "source_invalid")?;
    bounded_string(source, "title", MAX_TITLE_BYTES, "title_too_large")?;
    bounded_string(
        source,
        "selected_text",
        MAX_SELECTED_TEXT_BYTES,
        "selected_text_too_large",
    )?;
    if source.get("scope").and_then(Value::as_str) != Some("selection")
        && source.get("scope").and_then(Value::as_str) != Some("page")
    {
        return Err("source_invalid".into());
    }
    let redirects = source
        .get("redirects")
        .and_then(Value::as_array)
        .ok_or("source_invalid")?;
    if redirects.len() > MAX_REDIRECTS {
        return Err("source_invalid".into());
    }
    for redirect in redirects {
        validate_url_value(redirect, MAX_REDIRECT_URL_BYTES, "source_invalid")?;
    }
    let captured_at = parse_utc(source.get("captured_at").ok_or("capture_time_invalid")?)?;
    if (captured_at - now).abs() > MAX_CLOCK_SKEW {
        return Err("capture_time_invalid".into());
    }

    let snapshot = expect_object(root.get("snapshot").ok_or("snapshot_missing")?, "snapshot")?;
    exact_keys(
        snapshot,
        &[
            "state",
            "content_hash",
            "parser_id",
            "parser_version",
            "bytes",
            "failure_code",
        ],
    )?;
    let state = non_empty_string(snapshot, "state", "snapshot_invalid")?;
    if !matches!(state, "complete" | "partial" | "failed" | "not_requested") {
        return Err("snapshot_invalid".into());
    }
    let bytes = snapshot
        .get("bytes")
        .and_then(Value::as_u64)
        .ok_or("snapshot_invalid")?;
    let bytes = usize::try_from(bytes).map_err(|_| "snapshot_invalid")?;
    if bytes > MAX_SNAPSHOT_BYTES {
        return Err("payload_too_large".into());
    }
    let snapshot_hash = match state {
        "complete" | "partial" => {
            let hash = snapshot
                .get("content_hash")
                .and_then(Value::as_str)
                .ok_or("snapshot_invalid")?;
            if !valid_sha256(hash) || bytes == 0 {
                return Err("snapshot_invalid".into());
            }
            if snapshot
                .get("parser_id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
                || snapshot
                    .get("parser_version")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            {
                return Err("snapshot_invalid".into());
            }
            if state == "complete" {
                if !snapshot.get("failure_code").is_some_and(Value::is_null) {
                    return Err("snapshot_invalid".into());
                }
            } else if snapshot
                .get("failure_code")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                return Err("snapshot_invalid".into());
            }
            Some(hash.to_owned())
        }
        "failed" => {
            if !snapshot.get("content_hash").is_some_and(Value::is_null)
                || !snapshot.get("parser_id").is_some_and(Value::is_null)
                || !snapshot.get("parser_version").is_some_and(Value::is_null)
                || bytes != 0
                || snapshot
                    .get("failure_code")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            {
                return Err("snapshot_invalid".into());
            }
            None
        }
        _ => {
            if !snapshot.get("content_hash").is_some_and(Value::is_null)
                || !snapshot.get("parser_id").is_some_and(Value::is_null)
                || !snapshot.get("parser_version").is_some_and(Value::is_null)
                || bytes != 0
                || !snapshot.get("failure_code").is_some_and(Value::is_null)
            {
                return Err("snapshot_invalid".into());
            }
            None
        }
    };

    validate_auth(root, config)?;
    Ok(RequestView {
        value: value.clone(),
        request_id: request_id.to_owned(),
        session_id,
        counter,
        intent_token,
        snapshot_state: state.to_owned(),
        snapshot_hash,
        snapshot_bytes: bytes,
    })
}

fn validate_payload_frame(
    value: &Value,
    config: &HostConfig,
    request: &RequestView,
    expected_sequence: u64,
) -> Result<PayloadChunk, String> {
    let root = expect_object(value, "payload")?;
    exact_keys(
        root,
        &[
            "protocol",
            "type",
            "request_id",
            "session",
            "sequence",
            "final",
            "content_type",
            "encoding",
            "content_hash",
            "bytes",
            "body",
            "auth",
        ],
    )?;
    let protocol = expect_object(
        root.get("protocol").ok_or("payload_integrity_failed")?,
        "protocol",
    )?;
    exact_keys(protocol, &["major", "minor"])?;
    if protocol.get("major").and_then(Value::as_u64) != Some(PROTOCOL_MAJOR)
        || protocol.get("minor").and_then(Value::as_u64) != Some(PROTOCOL_MINOR)
    {
        return Err("protocol_version_unsupported".into());
    }
    if root.get("type").and_then(Value::as_str) != Some("capture.payload")
        || root.get("request_id").and_then(Value::as_str) != Some(request.request_id.as_str())
    {
        return Err("payload_integrity_failed".into());
    }
    let session = expect_object(
        root.get("session").ok_or("payload_integrity_failed")?,
        "session",
    )?;
    exact_keys(session, &["id", "counter"])?;
    if session.get("id").and_then(Value::as_str)
        != request
            .value
            .get("session")
            .and_then(|session| session.get("id"))
            .and_then(Value::as_str)
        || session.get("counter").and_then(Value::as_u64)
            != request
                .value
                .get("session")
                .and_then(|session| session.get("counter"))
                .and_then(Value::as_u64)
    {
        return Err("payload_integrity_failed".into());
    }
    if root.get("sequence").and_then(Value::as_u64) != Some(expected_sequence) {
        return Err("payload_integrity_failed".into());
    }
    let final_frame = root
        .get("final")
        .and_then(Value::as_bool)
        .ok_or("payload_integrity_failed")?;
    if root.get("content_type").and_then(Value::as_str) != Some("text/html")
        || root.get("encoding").and_then(Value::as_str) != Some("base64")
        || root.get("content_hash").and_then(Value::as_str) != request.snapshot_hash.as_deref()
    {
        return Err("payload_integrity_failed".into());
    }
    let encoded = root
        .get("body")
        .and_then(Value::as_str)
        .ok_or("payload_integrity_failed")?;
    let bytes = decode_base64(encoded).ok_or("payload_integrity_failed")?;
    let declared = root
        .get("bytes")
        .and_then(Value::as_u64)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or("payload_integrity_failed")?;
    if bytes.is_empty() || bytes.len() != declared || bytes.len() > MAX_PAYLOAD_CHUNK_BYTES {
        return Err("payload_too_large".into());
    }
    validate_auth(root, config).map_err(|_| "payload_integrity_failed".to_owned())?;
    Ok(PayloadChunk { bytes, final_frame })
}

fn validate_auth(root: &Map<String, Value>, config: &HostConfig) -> Result<(), String> {
    let auth = expect_object(root.get("auth").ok_or("auth_missing")?, "auth")?;
    exact_keys(auth, &["key_id", "algorithm", "mac"])?;
    if auth.get("key_id").and_then(Value::as_str) != Some(config.key_id.as_str())
        || auth.get("algorithm").and_then(Value::as_str) != Some("HMAC-SHA256")
    {
        return Err("authentication_failed".into());
    }
    let mac = auth
        .get("mac")
        .and_then(Value::as_str)
        .ok_or("authentication_failed")?;
    let supplied = decode_hex(mac).ok_or("authentication_failed")?;
    if supplied.len() != 32 {
        return Err("authentication_failed".into());
    }
    let canonical = canonical_without_mac(&Value::Object(root.clone()))?;
    let mut verifier = HmacSha256::new_from_slice(&config.pairing_secret)
        .map_err(|_| "authentication_failed".to_owned())?;
    verifier.update(&canonical);
    verifier
        .verify_slice(&supplied)
        .map_err(|_| "authentication_failed".to_owned())
}

pub fn canonical_without_mac(value: &Value) -> Result<Vec<u8>, String> {
    let mut unsigned = value.clone();
    let root = unsigned.as_object_mut().ok_or("capture_envelope_invalid")?;
    let auth = root
        .get_mut("auth")
        .and_then(Value::as_object_mut)
        .ok_or("auth_missing")?;
    auth.remove("mac");
    serde_json::to_vec(&unsigned).map_err(|_| "capture_envelope_invalid".into())
}

/// Commits an accepted capture to the spool.
///
/// The snapshot is written first and the metadata record last, so the metadata file is the commit
/// marker: a crash or failure can never leave metadata that claims a snapshot which is absent.
/// Every failure removes the temporary files and any snapshot this call already committed.
fn persist_capture(
    root: &Path,
    request: &Value,
    accepted: &AcceptedCapture,
    snapshot_bytes: &[u8],
) -> Result<(), io::Error> {
    fs::create_dir_all(root)?;
    let stem = &accepted.request_id;
    let metadata_path = root.join(format!("{stem}.json"));
    let snapshot_path = root.join(format!("{stem}.html"));
    let mut metadata = request.clone();
    if let Some(object) = metadata.as_object_mut() {
        object.remove("auth");
        object.insert(
            "capture_id".into(),
            Value::String(accepted.capture_id.clone()),
        );
        let accepted_at = Utc::now();
        object.insert(
            "accepted_at".into(),
            Value::String(accepted_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        );
        let requested_at = request
            .get("intent")
            .and_then(|intent| intent.get("issued_at"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        object.insert(
            "lifecycle".into(),
            serde_json::to_value(lifecycle::Lifecycle::accepted(requested_at, accepted_at))
                .map_err(io::Error::other)?,
        );
    }
    let metadata_bytes = serde_json::to_vec(&metadata).map_err(io::Error::other)?;
    if !snapshot_bytes.is_empty() {
        write_new_atomically(
            &root.join(format!(".{stem}.html.tmp")),
            &snapshot_path,
            snapshot_bytes,
        )?;
    }
    if let Err(error) = write_new_atomically(
        &root.join(format!(".{stem}.json.tmp")),
        &metadata_path,
        &metadata_bytes,
    ) {
        if !snapshot_bytes.is_empty() {
            let _ = fs::remove_file(&snapshot_path);
        }
        return Err(error);
    }
    Ok(())
}

/// Writes `bytes` to a new temporary file, syncs it, and links it to a new `destination`. The
/// temporary file is always removed; an existing destination is never replaced.
fn write_new_atomically(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    // A hard link commits without ever replacing an existing record: an earlier capture with the
    // same request ID is never overwritten, even if replay state were lost.
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::hard_link(temporary, destination)
        });
    let _ = fs::remove_file(temporary);
    result
}

fn reject_untrusted_html(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "snapshot_untrusted")?;
    let lower = text.to_ascii_lowercase();
    for needle in [
        "<script",
        "<style",
        "<iframe",
        "<object",
        "<embed",
        "<form",
        "<base",
        "<link",
        "javascript:",
        " onload=",
        " onclick=",
        " onerror=",
    ] {
        if lower.contains(needle) {
            return Err("snapshot_untrusted".into());
        }
    }
    reject_html_attributes(text)?;
    Ok(())
}

fn reject_html_attributes(text: &str) -> Result<(), String> {
    const ALLOWED_TAGS: &[&str] = &[
        "a",
        "article",
        "blockquote",
        "br",
        "code",
        "dd",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hr",
        "li",
        "main",
        "mark",
        "ol",
        "p",
        "pre",
        "section",
        "small",
        "span",
        "strong",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "ul",
    ];
    let mut cursor = 0usize;
    while let Some(relative_start) = text[cursor..].find('<') {
        let start = cursor + relative_start;
        let relative_end = text[start..]
            .find('>')
            .ok_or_else(|| "snapshot_untrusted".to_owned())?;
        let end = start + relative_end;
        let mut tag = text[start + 1..end].trim();
        let closing = tag.starts_with('/');
        if closing {
            tag = tag[1..].trim_start();
        }
        tag = tag.strip_suffix('/').unwrap_or(tag).trim_end();
        let name = tag
            .split_whitespace()
            .next()
            .ok_or_else(|| "snapshot_untrusted".to_owned())?;
        if !ALLOWED_TAGS.contains(&name.to_ascii_lowercase().as_str()) || tag != name {
            return Err("snapshot_untrusted".into());
        }
        if closing && matches!(name, "br" | "hr") {
            return Err("snapshot_untrusted".into());
        }
        cursor = end + 1;
    }
    Ok(())
}

fn validate_session_times(session: &Map<String, Value>, now: DateTime<Utc>) -> Result<(), String> {
    let issued_at = parse_utc(session.get("issued_at").ok_or("session_invalid")?)?;
    let expires_at = parse_utc(session.get("expires_at").ok_or("session_invalid")?)?;
    if expires_at <= issued_at || expires_at <= now || issued_at - now > MAX_CLOCK_SKEW {
        return Err("session_expired".into());
    }
    Ok(())
}

fn parse_utc(value: &Value) -> Result<DateTime<Utc>, String> {
    let text = value.as_str().ok_or("timestamp_invalid")?;
    if !text.ends_with('Z') {
        return Err("timestamp_invalid".into());
    }
    DateTime::parse_from_rfc3339(text)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| "timestamp_invalid".into())
}

fn validate_url(
    object: &Map<String, Value>,
    field: &str,
    maximum: usize,
    error: &str,
) -> Result<(), String> {
    validate_url_value(object.get(field).ok_or(error)?, maximum, error)
}

fn validate_url_value(value: &Value, maximum: usize, error: &str) -> Result<(), String> {
    let text = value.as_str().ok_or(error)?;
    if text.is_empty() || text.len() > maximum || text.chars().any(char::is_whitespace) {
        return Err(error.into());
    }
    let parsed = Url::parse(text).map_err(|_| error.to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(error.into());
    }
    Ok(())
}

fn bounded_string(
    object: &Map<String, Value>,
    field: &str,
    maximum: usize,
    error: &str,
) -> Result<(), String> {
    let value = object.get(field).and_then(Value::as_str).ok_or(error)?;
    if value.len() > maximum {
        return Err(error.into());
    }
    Ok(())
}

fn non_empty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    error: &str,
) -> Result<&'a str, String> {
    let value = object.get(field).and_then(Value::as_str).ok_or(error)?;
    if value.is_empty() {
        return Err(error.into());
    }
    Ok(value)
}

fn expect_object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| format!("{name}_invalid"))
}

fn exact_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<(), String> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("unknown_or_missing_field".into());
    }
    Ok(())
}

fn reject_forbidden_fields(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if FORBIDDEN_FIELDS.contains(&key.as_str()) {
                    return Err("forbidden_field".into());
                }
                reject_forbidden_fields(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                reject_forbidden_fields(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn request_id_from(value: &Value) -> Option<String> {
    value
        .get("request_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut result = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for index in (0..bytes.len()).step_by(2) {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        result.push(high << 4 | low);
    }
    Some(result)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(4) {
        return None;
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        let first = base64_value(chunk[0])?;
        let second = base64_value(chunk[1])?;
        let third = if chunk[2] == b'=' {
            0
        } else {
            base64_value(chunk[2])?
        };
        let fourth = if chunk[3] == b'=' {
            0
        } else {
            base64_value(chunk[3])?
        };
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return None;
        }
        output.push(first << 2 | second >> 4);
        if chunk[2] != b'=' {
            output.push(second << 4 | third >> 2);
        }
        if chunk[3] != b'=' {
            output.push(third << 6 | fourth);
        }
    }
    Some(output)
}

fn base64_value(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

struct StrictValue;

impl<'de> DeserializeSeed<'de> for StrictValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON key: {key}"
                )));
            }
            let value = map.next_value_seed(StrictValue)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

pub fn parse_strict_json(raw: &[u8]) -> Result<Value, HostError> {
    let mut deserializer = serde_json::Deserializer::from_slice(raw);
    let value = StrictValue
        .deserialize(&mut deserializer)
        .map_err(|error| HostError::Frame(format!("invalid JSON: {error}")))?;
    deserializer
        .end()
        .map_err(|error| HostError::Frame(format!("trailing JSON: {error}")))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    const KEY_ID: &str = "pairing-key-test-only";
    const CHROME_CALLER: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop/";
    const FIREFOX_CALLER: &str = "firefox:loom-explicit-save@alisinadevelo.local";
    const REQUEST_ID: &str = "11111111-1111-4111-8111-111111111111";
    const NOW: &str = "2027-08-23T12:34:56Z";

    fn config(root: &Path) -> HostConfig {
        HostConfig {
            key_id: KEY_ID.into(),
            pairing_secret: (0u8..32).collect(),
            spool_root: root.to_path_buf(),
            allowed_callers: vec![CHROME_CALLER.into(), FIREFOX_CALLER.into()],
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(NOW)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn request(snapshot: &str, body: &[u8], counter: u64) -> Value {
        let hash = if body.is_empty() {
            Value::Null
        } else {
            Value::String(format!("sha256:{}", hex_encode(&Sha256::digest(body))))
        };
        let (parser_id, parser_version, bytes, failure_code) = if body.is_empty() {
            (Value::Null, Value::Null, Value::from(0u64), Value::Null)
        } else {
            (
                Value::String("loom.web.sanitize".into()),
                Value::String("0.1.0".into()),
                Value::from(body.len() as u64),
                if snapshot == "partial" {
                    Value::String("snapshot_truncated".into())
                } else {
                    Value::Null
                },
            )
        };
        let mut value = serde_json::json!({
            "protocol": {"major": 1, "minor": 0},
            "type": "capture.request",
            "request_id": if counter == 2 { "22222222-2222-4222-8222-222222222222" } else { REQUEST_ID },
            "session": {"id": "session-test-1", "counter": counter, "issued_at": NOW, "expires_at": "2027-08-23T12:39:56Z"},
            "intent": {"kind": "user_gesture", "token": format!("intent-{counter}"), "issued_at": NOW},
            "source": {"live_url": "https://example.test/start", "final_url": "https://example.test/article", "title": "Synthetic", "selected_text": "A passage", "captured_at": NOW, "redirects": [], "scope": "selection"},
            "snapshot": {"state": snapshot, "content_hash": hash, "parser_id": parser_id, "parser_version": parser_version, "bytes": bytes, "failure_code": failure_code},
            "auth": {"key_id": KEY_ID, "algorithm": "HMAC-SHA256", "mac": ""},
        });
        let canonical = canonical_without_mac(&value).unwrap();
        let mut mac = HmacSha256::new_from_slice(&(0u8..32).collect::<Vec<_>>()).unwrap();
        mac.update(&canonical);
        value["auth"]["mac"] = Value::String(hex_encode(&mac.finalize().into_bytes()));
        value
    }

    fn payload(request: &Value, body: &[u8], sequence: u64, final_frame: bool) -> Value {
        let hash = request["snapshot"]["content_hash"].clone();
        let body_b64 = base64_encode(body);
        let mut frame = serde_json::json!({
            "protocol": {"major": 1, "minor": 0},
            "type": "capture.payload",
            "request_id": request["request_id"],
            "session": {"id": "session-test-1", "counter": request["session"]["counter"]},
            "sequence": sequence,
            "final": final_frame,
            "content_type": "text/html",
            "encoding": "base64",
            "content_hash": hash,
            "bytes": body.len(),
            "body": body_b64,
            "auth": {"key_id": KEY_ID, "algorithm": "HMAC-SHA256", "mac": ""},
        });
        let canonical = canonical_without_mac(&frame).unwrap();
        let mut mac = HmacSha256::new_from_slice(&(0u8..32).collect::<Vec<_>>()).unwrap();
        mac.update(&canonical);
        frame["auth"]["mac"] = Value::String(hex_encode(&mac.finalize().into_bytes()));
        frame
    }

    fn framed(values: &[Value]) -> Vec<u8> {
        let mut result = Vec::new();
        for value in values {
            let body = serde_json::to_vec(value).unwrap();
            result.extend_from_slice(&(body.len() as u32).to_le_bytes());
            result.extend_from_slice(&body);
        }
        result
    }

    fn read_responses(bytes: &[u8]) -> Vec<Value> {
        let mut cursor = Cursor::new(bytes);
        let mut responses = Vec::new();
        while let Some(frame) = read_frame(&mut cursor).unwrap() {
            responses.push(parse_strict_json(&frame).unwrap());
        }
        responses
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn caller_identity_is_parsed_for_chrome_and_firefox_launches() {
        assert_eq!(
            caller_from_args(&args(&[
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/"
            ])),
            Some(CHROME_CALLER.into())
        );
        assert_eq!(
            caller_from_args(&args(&[
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
                "--parent-window=0"
            ])),
            Some(CHROME_CALLER.into())
        );
        assert_eq!(
            caller_from_args(&args(&[
                "/Library/Application Support/Mozilla/NativeMessagingHosts/com.alisinadevelo.loom.json",
                "loom-explicit-save@alisinadevelo.local"
            ])),
            Some(FIREFOX_CALLER.into())
        );
    }

    #[test]
    fn spoofed_malformed_or_missing_callers_are_refused() {
        let directory = tempdir().unwrap();
        let config = config(directory.path());
        for launch in [
            args(&[]),
            args(&["chrome-extension://zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/"]),
            args(&["chrome-extension://abcdefghijklmnopabcdefghijklmno/"]),
            args(&["chrome-extension://abcdefghijklmnopabcdefghijklmnop"]),
            args(&["chrome-extension://pppppppppppppppppppppppppppppppp/"]),
            args(&["https://abcdefghijklmnopabcdefghijklmnop/"]),
            args(&["manifest.json", "other-extension@example.test"]),
            args(&[
                "manifest.json",
                "loom-explicit-save@alisinadevelo.local;rm -rf",
            ]),
            args(&["manifest.txt", "loom-explicit-save@alisinadevelo.local"]),
            args(&[
                "manifest.json",
                "loom-explicit-save@alisinadevelo.local",
                "extra",
            ]),
        ] {
            assert!(config.verify_caller(&launch).is_err(), "{launch:?}");
        }
        assert_eq!(
            config
                .verify_caller(&args(&[
                    "chrome-extension://abcdefghijklmnopabcdefghijklmnop/"
                ]))
                .unwrap(),
            CHROME_CALLER
        );
    }

    fn resign(mut value: Value) -> Value {
        let canonical = canonical_without_mac(&value).unwrap();
        let mut mac = HmacSha256::new_from_slice(&(0u8..32).collect::<Vec<_>>()).unwrap();
        mac.update(&canonical);
        value["auth"]["mac"] = Value::String(hex_encode(&mac.finalize().into_bytes()));
        value
    }

    fn single_response(root: &Path, request: Value) -> Value {
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(&mut Cursor::new(framed(&[request])), &mut output, now())
            .unwrap();
        read_responses(&output).remove(0)
    }

    #[test]
    fn boundary_rejects_downgrade_oversize_and_nested_secrets_without_writing() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let base = request("not_requested", &[], 1);

        let mut newer_major = base.clone();
        newer_major["protocol"]["major"] = Value::from(2u64);
        let mut older_major = base.clone();
        older_major["protocol"]["major"] = Value::from(0u64);
        let mut huge_selection = base.clone();
        huge_selection["source"]["selected_text"] =
            Value::String("x".repeat(MAX_SELECTED_TEXT_BYTES + 1));
        let mut huge_snapshot = request("complete", b"<p>safe</p>", 1);
        huge_snapshot["snapshot"]["bytes"] = Value::from(MAX_SNAPSHOT_BYTES as u64 + 1);
        let mut nested_password = base.clone();
        nested_password["source"]["password"] = Value::String("hunter2".into());

        for (name, value, expected) in [
            ("newer major", newer_major, "protocol_version_unsupported"),
            ("older major", older_major, "protocol_version_unsupported"),
            ("huge selection", huge_selection, "selected_text_too_large"),
            ("huge snapshot", huge_snapshot, "payload_too_large"),
            ("nested password", nested_password, "forbidden_field"),
        ] {
            let response = single_response(root, resign(value));
            assert_eq!(response["error"], expected, "{name}");
        }
        assert!(spool_entries(root).is_empty());

        let mut oversized_frame = Vec::new();
        oversized_frame.extend_from_slice(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes());
        oversized_frame.extend_from_slice(&vec![b' '; MAX_FRAME_BYTES + 1]);
        assert!(NativeHost::new(config(root))
            .run_at(&mut Cursor::new(oversized_frame), &mut Vec::new(), now())
            .is_err());
        assert!(spool_entries(root).is_empty());
    }

    #[test]
    fn replay_is_refused_across_host_processes_and_never_overwrites_a_capture() {
        // Browsers start a new host process per connection; model each with a fresh NativeHost.
        let directory = tempdir().unwrap();
        let root = directory.path();
        let body = b"<p>first capture</p>";
        let first = request("complete", body, 1);
        let frames = framed(&[first.clone(), payload(&first, body, 0, true)]);
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(&mut Cursor::new(frames.clone()), &mut output, now())
            .unwrap();
        assert_eq!(read_responses(&output)[0]["type"], "capture.accepted");
        let stored = fs::read(root.join(format!("{REQUEST_ID}.json"))).unwrap();

        // An exact replay of the same frames in a new process.
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(&mut Cursor::new(frames), &mut output, now())
            .unwrap();
        assert_eq!(read_responses(&output)[0]["error"], "replay_rejected");

        // The same request ID with a fresh intent and re-signed envelope in another process.
        let mut reused = first.clone();
        reused["intent"]["token"] = Value::String("intent-reused".into());
        let reused = resign(reused);
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(
                &mut Cursor::new(framed(&[reused.clone(), payload(&reused, body, 0, true)])),
                &mut output,
                now(),
            )
            .unwrap();
        assert_eq!(read_responses(&output)[0]["error"], "replay_rejected");
        assert_eq!(
            fs::read(root.join(format!("{REQUEST_ID}.json"))).unwrap(),
            stored,
            "the first capture was overwritten"
        );

        // Even if replay state were lost, the commit refuses to replace an existing record.
        fs::remove_file(root.join(replay::LEDGER_FILE)).unwrap();
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(
                &mut Cursor::new(framed(&[reused.clone(), payload(&reused, body, 0, true)])),
                &mut output,
                now(),
            )
            .unwrap();
        assert_eq!(
            read_responses(&output)[0]["error"],
            "capture_storage_failed"
        );
        assert_eq!(
            fs::read(root.join(format!("{REQUEST_ID}.json"))).unwrap(),
            stored
        );
        assert_eq!(
            spool_entries(root),
            vec![format!("{REQUEST_ID}.html"), format!("{REQUEST_ID}.json")]
        );
    }

    #[test]
    fn strict_json_rejects_duplicate_keys() {
        let error = parse_strict_json(br#"{"type":"capture.request","type":"capture.request"}"#)
            .expect_err("duplicate keys must not be accepted");
        assert!(error.to_string().contains("duplicate JSON key"));
    }

    #[test]
    fn frame_round_trip_and_limit() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"hello").unwrap();
        let mut cursor = Cursor::new(bytes);
        assert_eq!(read_frame(&mut cursor).unwrap(), Some(b"hello".to_vec()));
        let too_large = vec![0u8; MAX_FRAME_BYTES + 1];
        assert!(matches!(
            write_frame(&mut Vec::new(), &too_large),
            Err(HostError::Frame(_))
        ));
    }

    #[test]
    fn accepts_authenticated_sanitized_payload_and_writes_atomically() {
        let directory = tempdir().unwrap();
        let body = b"<article><p>safe</p></article>";
        let request = request("complete", body, 1);
        let input = framed(&[request.clone(), payload(&request, body, 0, true)]);
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        let responses = read_responses(&output);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["type"], "capture.accepted");
        assert!(directory
            .path()
            .join(format!("{REQUEST_ID}.html"))
            .is_file());
        assert!(directory
            .path()
            .join(format!("{REQUEST_ID}.json"))
            .is_file());
        assert!(!directory
            .path()
            .join(format!(".{REQUEST_ID}.html.tmp"))
            .exists());
    }

    fn accepted_capture() -> AcceptedCapture {
        AcceptedCapture {
            capture_id: format!("browser-{REQUEST_ID}"),
            request_id: REQUEST_ID.into(),
            snapshot_state: "complete".into(),
            source_hash: None,
            bytes: 11,
        }
    }

    /// Capture files in the spool. The replay ledger holds only hashes and is not a capture.
    fn spool_entries(root: &Path) -> Vec<String> {
        if !root.exists() {
            return Vec::new();
        }
        let mut names = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != replay::LEDGER_FILE)
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn failed_metadata_commit_removes_the_committed_snapshot() {
        let directory = tempdir().unwrap();
        // A non-empty directory at the metadata path makes the final metadata rename fail.
        fs::create_dir_all(directory.path().join(format!("{REQUEST_ID}.json/occupied"))).unwrap();
        let request = request("complete", b"<p>safe</p>", 1);
        assert!(persist_capture(
            directory.path(),
            &request,
            &accepted_capture(),
            b"<p>safe</p>"
        )
        .is_err());
        assert_eq!(
            spool_entries(directory.path()),
            vec![format!("{REQUEST_ID}.json")]
        );
    }

    #[test]
    fn failed_snapshot_write_leaves_no_metadata_record() {
        let directory = tempdir().unwrap();
        // A stale temporary file makes the exclusive snapshot create fail.
        let stale = directory.path().join(format!(".{REQUEST_ID}.html.tmp"));
        fs::write(&stale, b"stale").unwrap();
        let request = request("complete", b"<p>safe</p>", 1);
        assert!(persist_capture(
            directory.path(),
            &request,
            &accepted_capture(),
            b"<p>safe</p>"
        )
        .is_err());
        assert!(!directory.path().join(format!("{REQUEST_ID}.json")).exists());
        assert!(!directory.path().join(format!("{REQUEST_ID}.html")).exists());
    }

    #[test]
    fn metadata_only_capture_commits_without_a_snapshot() {
        let directory = tempdir().unwrap();
        let request = request("not_requested", &[], 1);
        persist_capture(directory.path(), &request, &accepted_capture(), &[]).unwrap();
        assert_eq!(
            spool_entries(directory.path()),
            vec![format!("{REQUEST_ID}.json")]
        );
        let stored: Value = serde_json::from_slice(
            &fs::read(directory.path().join(format!("{REQUEST_ID}.json"))).unwrap(),
        )
        .unwrap();
        assert!(stored.get("auth").is_none());
        assert_eq!(stored["capture_id"], format!("browser-{REQUEST_ID}"));
    }

    fn accept_one(root: &Path, body: &[u8]) {
        let request = request("complete", body, 1);
        let input = framed(&[request.clone(), payload(&request, body, 0, true)]);
        let mut output = Vec::new();
        NativeHost::new(config(root))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        assert_eq!(read_responses(&output)[0]["type"], "capture.accepted");
    }

    fn stored_record(root: &Path) -> Map<String, Value> {
        let bytes = fs::read(root.join(format!("{REQUEST_ID}.json"))).unwrap();
        serde_json::from_slice::<Value>(&bytes)
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    }

    fn spool_contains(root: &Path, needle: &[u8]) -> bool {
        fs::read_dir(root).unwrap().any(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap_or_default();
            bytes.windows(needle.len()).any(|window| window == needle)
        })
    }

    #[test]
    fn accepted_capture_records_requested_and_accepted_transitions() {
        let directory = tempdir().unwrap();
        accept_one(directory.path(), b"<p>lifecycle body</p>");
        let lifecycle = lifecycle::record_lifecycle(&stored_record(directory.path())).unwrap();
        assert_eq!(lifecycle.state, lifecycle::CaptureState::Accepted);
        let states = lifecycle
            .transitions
            .iter()
            .map(|transition| transition.state)
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                lifecycle::CaptureState::Requested,
                lifecycle::CaptureState::Accepted
            ]
        );
        assert_eq!(lifecycle.transitions[0].at, NOW);
    }

    #[test]
    fn lifecycle_fixture_revoke_repair_delete_leaves_no_managed_copy() {
        use lifecycle::CaptureState;

        let directory = tempdir().unwrap();
        let root = directory.path();
        let body = b"<p>private snapshot body</p>";
        accept_one(root, body);
        let sensitive: [&[u8]; 5] = [
            b"https://example.test/start",
            b"https://example.test/article",
            b"Synthetic",
            b"A passage",
            body,
        ];
        for needle in sensitive {
            assert!(spool_contains(root, needle));
        }

        let revoked = lifecycle::revoke_spool(root, now()).unwrap();
        assert_eq!(revoked.changed, vec![REQUEST_ID.to_string()]);
        assert_eq!(
            lifecycle::record_lifecycle(&stored_record(root))
                .unwrap()
                .state,
            CaptureState::Revoked
        );
        assert!(root.join(format!("{REQUEST_ID}.html")).is_file());
        let repaired = lifecycle::repair_spool(root, now()).unwrap();
        assert_eq!(repaired.changed.len(), 1);
        lifecycle::revoke_spool(root, now()).unwrap();

        // An interrupted earlier attempt left temporary files behind.
        fs::write(root.join(format!(".{REQUEST_ID}.html.tmp")), body).unwrap();
        let report = lifecycle::delete_capture(root, REQUEST_ID, now()).unwrap();
        assert!(!report.already_deleted);
        assert!(report.snapshot_removed);
        assert_eq!(report.temporary_files_removed, 1);

        for needle in sensitive {
            assert!(
                !spool_contains(root, needle),
                "managed copy kept: {needle:?}"
            );
        }
        assert_eq!(spool_entries(root), vec![format!("{REQUEST_ID}.json")]);
        let tombstone = stored_record(root);
        let mut keys = tombstone.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "capture_id",
                "lifecycle",
                "protocol",
                "request_id",
                "snapshot_content_hash"
            ]
        );
        assert_eq!(
            tombstone["snapshot_content_hash"],
            format!("sha256:{}", hex_encode(&Sha256::digest(body)))
        );
        let history = lifecycle::record_lifecycle(&tombstone)
            .unwrap()
            .transitions
            .into_iter()
            .map(|transition| transition.state)
            .collect::<Vec<_>>();
        assert_eq!(
            history,
            vec![
                CaptureState::Requested,
                CaptureState::Accepted,
                CaptureState::Revoked,
                CaptureState::Accepted,
                CaptureState::Revoked,
                CaptureState::Deleted
            ]
        );

        // Deleted is terminal: revocation and re-pairing leave it alone, and a repeated delete only
        // finishes cleanup.
        assert_eq!(lifecycle::revoke_spool(root, now()).unwrap().unchanged, 1);
        assert_eq!(lifecycle::repair_spool(root, now()).unwrap().unchanged, 1);
        fs::write(root.join(format!("{REQUEST_ID}.html")), body).unwrap();
        let again = lifecycle::delete_capture(root, REQUEST_ID, now()).unwrap();
        assert!(again.already_deleted);
        assert!(again.snapshot_removed);
        assert!(!spool_contains(root, body));
    }

    #[test]
    fn deletion_rejects_unknown_and_malformed_request_ids() {
        let directory = tempdir().unwrap();
        assert!(lifecycle::delete_capture(directory.path(), REQUEST_ID, now()).is_err());
        assert!(lifecycle::delete_capture(directory.path(), "../escape", now()).is_err());
        assert!(lifecycle::delete_capture(
            directory.path(),
            "11111111-1111-4111-8111-111111111111.json",
            now()
        )
        .is_err());
    }

    #[test]
    fn rejected_and_expired_requests_persist_nothing() {
        let directory = tempdir().unwrap();
        let mut expired = request("not_requested", &[], 1);
        expired["session"]["expires_at"] = Value::String("2027-08-23T12:00:00Z".into());
        let canonical = canonical_without_mac(&expired).unwrap();
        let mut mac = HmacSha256::new_from_slice(&(0u8..32).collect::<Vec<_>>()).unwrap();
        mac.update(&canonical);
        expired["auth"]["mac"] = Value::String(hex_encode(&mac.finalize().into_bytes()));
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(framed(&[expired])), &mut output, now())
            .unwrap();
        let code = read_responses(&output)[0]["error"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            lifecycle::rejection_state(&code),
            lifecycle::CaptureState::Expired
        );
        assert!(spool_entries(directory.path()).is_empty());
    }

    #[test]
    fn rejects_untrusted_payload_without_writing_anything() {
        let directory = tempdir().unwrap();
        let body = b"<article><script>steal()</script></article>";
        let request = request("complete", body, 1);
        let input = framed(&[request.clone(), payload(&request, body, 0, true)]);
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        let responses = read_responses(&output);
        assert_eq!(responses[0]["error"], "snapshot_untrusted");
        assert!(spool_entries(directory.path()).is_empty());
    }

    #[test]
    fn rejects_remote_or_event_attributes_even_when_tags_are_allowed() {
        let directory = tempdir().unwrap();
        let body = b"<article><a href=\"https://outside.test\">link</a></article>";
        let request = request("complete", body, 1);
        let input = framed(&[request.clone(), payload(&request, body, 0, true)]);
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        assert_eq!(read_responses(&output)[0]["error"], "snapshot_untrusted");
        assert!(spool_entries(directory.path()).is_empty());
    }

    #[test]
    fn rejects_replay_and_accepts_new_counter_after_integrity_failure() {
        let directory = tempdir().unwrap();
        let body = b"<p>safe</p>";
        let first = request("complete", body, 1);
        let mut bad = payload(&first, body, 0, true);
        bad["body"] = Value::String(base64_encode(b"tampered"));
        bad["bytes"] = Value::from(8u64);
        let second = request("complete", body, 2);
        let input = framed(&[
            first.clone(),
            bad,
            first.clone(),
            payload(&first, body, 0, true),
            second.clone(),
            payload(&second, body, 0, true),
        ]);
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        let responses = read_responses(&output);
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["error"], "payload_integrity_failed");
        assert_eq!(responses[1]["error"], "replay_rejected");
        assert_eq!(responses[2]["type"], "capture.accepted");
    }

    #[test]
    fn rejects_unknown_fields_before_authentication() {
        let directory = tempdir().unwrap();
        let mut request = request("not_requested", &[], 1);
        request["source"]["cookies"] = Value::String("secret".into());
        let input = framed(&[request]);
        let mut output = Vec::new();
        NativeHost::new(config(directory.path()))
            .run_at(&mut Cursor::new(input), &mut output, now())
            .unwrap();
        assert_eq!(read_responses(&output)[0]["error"], "forbidden_field");
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = String::new();
        for chunk in bytes.chunks(3) {
            let first = chunk[0];
            let second = *chunk.get(1).unwrap_or(&0);
            let third = *chunk.get(2).unwrap_or(&0);
            output.push(TABLE[(first >> 2) as usize] as char);
            output.push(TABLE[((first & 0x03) << 4 | second >> 4) as usize] as char);
            output.push(if chunk.len() > 1 {
                TABLE[((second & 0x0f) << 2 | third >> 6) as usize] as char
            } else {
                '='
            });
            output.push(if chunk.len() > 2 {
                TABLE[(third & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        output
    }
}
