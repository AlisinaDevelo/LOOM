#!/usr/bin/env python3
"""Offline contract tests for the proposed browser-capture protocol.

The tests validate the versioned fixture and its rejection cases. They do not claim that a
browser extension, native host, sanitizer, or macOS permission flow exists.
"""

from __future__ import annotations

import copy
import hashlib
import hmac
import json
import re
import unittest
import uuid
from pathlib import Path
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parents[1]
FIXTURE_PATH = ROOT / "tests" / "fixtures" / "browser-capture" / "protocol-v1.json"
FIXTURE = json.loads(FIXTURE_PATH.read_text())
LIMITS = FIXTURE["limits"]
FORBIDDEN = set(FIXTURE["forbidden_fields"])
UUID_PATTERN = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)
HASH_PATTERN = re.compile(r"^sha256:[0-9a-f]{64}$")


class ProtocolError(ValueError):
    """A deterministic protocol rejection used by the fixture validator."""


def canonical_without_mac(message: dict) -> bytes:
    signed = copy.deepcopy(message)
    signed["auth"].pop("mac", None)
    return json.dumps(signed, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def reject_duplicate_keys(raw: str) -> None:
    def pairs(pairs: list[tuple[str, object]]) -> dict:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ProtocolError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    json.loads(raw, object_pairs_hook=pairs)


def expected_mac(message: dict) -> str:
    key = bytes.fromhex(FIXTURE["hmac_test_key_hex"])
    return hmac.new(key, canonical_without_mac(message), hashlib.sha256).hexdigest()


def reject_forbidden_fields(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN:
                raise ProtocolError(f"forbidden field: {key}")
            reject_forbidden_fields(child)
    elif isinstance(value, list):
        for child in value:
            reject_forbidden_fields(child)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ProtocolError(message)


def validate_url(value: object, field: str, maximum: int) -> None:
    require(isinstance(value, str), f"{field} must be a string")
    require(0 < len(value.encode()) <= maximum, f"{field} exceeds its byte limit")
    parsed = urlparse(value)
    require(parsed.scheme in {"http", "https"} and bool(parsed.netloc), f"{field} is not HTTP(S)")


def validate_snapshot(snapshot: dict) -> None:
    require(set(snapshot) == {"state", "content_hash", "parser_id", "parser_version", "bytes", "failure_code"}, "snapshot shape changed")
    state = snapshot["state"]
    require(state in {"complete", "partial", "failed", "not_requested"}, "unknown snapshot state")
    require(isinstance(snapshot["bytes"], int) and 0 <= snapshot["bytes"] <= LIMITS["max_snapshot_bytes"], "snapshot bytes out of bounds")
    if state in {"complete", "partial"}:
        require(isinstance(snapshot["content_hash"], str) and HASH_PATTERN.fullmatch(snapshot["content_hash"]), "snapshot hash missing or malformed")
        require(isinstance(snapshot["parser_id"], str) and snapshot["parser_id"], "snapshot parser id missing")
        require(isinstance(snapshot["parser_version"], str) and snapshot["parser_version"], "snapshot parser version missing")
        if state == "complete":
            require(snapshot["failure_code"] is None, "complete snapshot cannot carry a failure")
        else:
            require(isinstance(snapshot["failure_code"], str) and snapshot["failure_code"], "partial snapshot needs a reason")
    elif state == "failed":
        require(snapshot["content_hash"] is None, "failed snapshot cannot carry a hash")
        require(snapshot["parser_id"] is None and snapshot["parser_version"] is None, "failed snapshot cannot claim a parser")
        require(snapshot["bytes"] == 0, "failed snapshot cannot retain bytes")
        require(isinstance(snapshot["failure_code"], str) and snapshot["failure_code"], "failed snapshot needs a reason")
    else:
        require(snapshot["content_hash"] is None, "not-requested snapshot cannot carry a hash")
        require(snapshot["parser_id"] is None and snapshot["parser_version"] is None, "not-requested snapshot cannot claim a parser")
        require(snapshot["bytes"] == 0 and snapshot["failure_code"] is None, "not-requested snapshot has derived state")


def validate_request(message: dict, *, verify_auth: bool = False) -> None:
    require(set(message) == set(FIXTURE["required_capture_fields"]), "capture envelope has unknown or missing fields")
    reject_forbidden_fields(message)
    protocol = message["protocol"]
    require(protocol == {"major": 1, "minor": 0}, "protocol version is unsupported or downgraded")
    require(message["type"] == "capture.request", "message type is not capture.request")
    require(isinstance(message["request_id"], str) and UUID_PATTERN.fullmatch(message["request_id"]), "request id is not a v4 UUID")

    session = message["session"]
    require(set(session) == {"id", "counter", "issued_at", "expires_at"}, "session shape changed")
    require(isinstance(session["id"], str) and session["id"], "session id missing")
    require(isinstance(session["counter"], int) and session["counter"] > 0, "session counter is invalid")

    intent = message["intent"]
    require(set(intent) == {"kind", "token", "issued_at"}, "intent shape changed")
    require(intent["kind"] == "user_gesture" and isinstance(intent["token"], str) and intent["token"], "capture is not user-approved")

    source = message["source"]
    require(set(source) == {"live_url", "final_url", "title", "selected_text", "captured_at", "redirects", "scope"}, "source shape changed")
    validate_url(source["live_url"], "live_url", LIMITS["max_url_bytes"])
    validate_url(source["final_url"], "final_url", LIMITS["max_url_bytes"])
    require(isinstance(source["title"], str) and len(source["title"].encode()) <= LIMITS["max_title_bytes"], "title exceeds its byte limit")
    require(isinstance(source["selected_text"], str) and len(source["selected_text"].encode()) <= LIMITS["max_selected_text_bytes"], "selected text exceeds its byte limit")
    require(source["scope"] in {"selection", "page"}, "capture scope is invalid")
    require(isinstance(source["redirects"], list) and len(source["redirects"]) <= LIMITS["max_redirects"], "redirect chain is too long")
    for redirect in source["redirects"]:
        validate_url(redirect, "redirect", LIMITS["max_redirect_url_bytes"])

    validate_snapshot(message["snapshot"])
    auth = message["auth"]
    require(set(auth) == {"key_id", "algorithm", "mac"}, "auth shape changed")
    require(isinstance(auth["key_id"], str) and auth["key_id"], "auth key id missing")
    require(auth["algorithm"] == "HMAC-SHA256", "auth algorithm is unsupported")
    require(isinstance(auth["mac"], str) and re.fullmatch(r"[0-9a-f]{64}", auth["mac"]), "auth mac is malformed")
    frame_bytes = len(json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode())
    require(frame_bytes <= LIMITS["max_frame_bytes"], "capture frame exceeds its byte limit")
    if verify_auth:
        require(hmac.compare_digest(auth["mac"], expected_mac(message)), "capture MAC does not verify")


def accept_counters(messages: list[dict]) -> None:
    seen: set[tuple[str, int]] = set()
    last_by_session: dict[str, int] = {}
    for message in messages:
        session = message["session"]
        key = (session["id"], session["counter"])
        if key in seen or session["counter"] <= last_by_session.get(session["id"], 0):
            raise ProtocolError("replay_rejected")
        seen.add(key)
        last_by_session[session["id"]] = session["counter"]


class BrowserCaptureProtocolTests(unittest.TestCase):
    def setUp(self) -> None:
        self.message = copy.deepcopy(FIXTURE["capture_request"])

    def test_fixture_has_documented_limits_and_forbidden_fields(self) -> None:
        self.assertEqual(FIXTURE["schema_version"], 1)
        self.assertEqual(FIXTURE["protocol"], {"major": 1, "minor": 0})
        self.assertEqual(FIXTURE["limits"]["max_frame_bytes"], 256 * 1024)
        self.assertIn("cookies", FORBIDDEN)
        self.assertIn("page_scripts", FORBIDDEN)

    def test_canonical_hmac_vector_verifies(self) -> None:
        self.assertEqual(expected_mac(self.message), FIXTURE["expected_mac"])
        validate_request(self.message, verify_auth=True)

    def test_duplicate_keys_are_rejected_before_authentication(self) -> None:
        with self.assertRaisesRegex(ProtocolError, "duplicate JSON key"):
            reject_duplicate_keys('{"type":"capture.request","type":"capture.request"}')

    def test_snapshot_states_are_explicit_and_source_backed(self) -> None:
        for snapshot in FIXTURE["snapshot_examples"].values():
            validate_snapshot(snapshot)
        self.assertEqual(FIXTURE["snapshot_examples"]["failed"]["state"], "failed")
        self.assertEqual(FIXTURE["snapshot_examples"]["not_requested"]["content_hash"], None)

    def test_replay_counter_must_increase(self) -> None:
        newer = copy.deepcopy(self.message)
        newer["session"]["counter"] = 8
        accept_counters([self.message, newer])
        with self.assertRaisesRegex(ProtocolError, "replay_rejected"):
            accept_counters([self.message, copy.deepcopy(self.message)])
        older = copy.deepcopy(self.message)
        older["session"]["counter"] = 6
        with self.assertRaisesRegex(ProtocolError, "replay_rejected"):
            accept_counters([self.message, older])

    def test_downgrade_and_major_change_are_rejected(self) -> None:
        for version in ({"major": 0, "minor": 9}, {"major": 2, "minor": 0}):
            candidate = copy.deepcopy(self.message)
            candidate["protocol"] = version
            with self.assertRaisesRegex(ProtocolError, "unsupported|downgraded"):
                validate_request(candidate)

    def test_missing_or_automatic_capture_is_rejected(self) -> None:
        candidate = copy.deepcopy(self.message)
        candidate["intent"] = {"kind": "timer", "token": "", "issued_at": "2027-08-23T12:34:55Z"}
        with self.assertRaisesRegex(ProtocolError, "user-approved"):
            validate_request(candidate)

    def test_forbidden_sensitive_field_is_rejected(self) -> None:
        candidate = copy.deepcopy(self.message)
        candidate["source"]["cookies"] = "must-not-cross-boundary"
        with self.assertRaisesRegex(ProtocolError, "forbidden field"):
            validate_request(candidate)

    def test_oversized_selection_and_snapshot_are_rejected(self) -> None:
        candidate = copy.deepcopy(self.message)
        candidate["source"]["selected_text"] = "x" * (LIMITS["max_selected_text_bytes"] + 1)
        with self.assertRaisesRegex(ProtocolError, "selected text"):
            validate_request(candidate)
        candidate = copy.deepcopy(self.message)
        candidate["snapshot"]["bytes"] = LIMITS["max_snapshot_bytes"] + 1
        with self.assertRaisesRegex(ProtocolError, "snapshot bytes"):
            validate_request(candidate)

    def test_fixture_request_id_is_uuid(self) -> None:
        self.assertEqual(str(uuid.UUID(self.message["request_id"])), self.message["request_id"])


if __name__ == "__main__":
    unittest.main()
