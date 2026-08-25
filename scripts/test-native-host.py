#!/usr/bin/env python3
"""Device smoke test for LOOM's stdio native-messaging host.

The test speaks the real four-byte little-endian native-messaging framing used by browsers. It
keeps the corpus synthetic, verifies authenticated acceptance and atomic spooling, then exercises
malformed JSON, replay, hostile HTML, and a fresh-counter recovery request. It never contacts a
network or uses a user's browser profile.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import subprocess
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path


KEY_ID = "pairing-key-device-test"
SECRET = bytes(range(32))


def canonical(value: dict) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()


def sign(value: dict) -> dict:
    unsigned = json.loads(json.dumps(value))
    unsigned["auth"].pop("mac", None)
    value["auth"]["mac"] = hmac.new(SECRET, canonical(unsigned), hashlib.sha256).hexdigest()
    return value


def request(request_id: str, counter: int, body: bytes, *, state: str = "complete") -> dict:
    now = datetime.now(timezone.utc).replace(microsecond=0)
    digest = f"sha256:{hashlib.sha256(body).hexdigest()}" if body else None
    snapshot = {
        "state": state,
        "content_hash": digest,
        "parser_id": "loom.web.sanitize" if body else None,
        "parser_version": "0.1.0" if body else None,
        "bytes": len(body),
        "failure_code": None,
    }
    if state == "partial":
        snapshot["failure_code"] = "snapshot_truncated"
    if state in {"failed", "not_requested"}:
        snapshot = {
            "state": state,
            "content_hash": None,
            "parser_id": None,
            "parser_version": None,
            "bytes": 0,
            "failure_code": "offline" if state == "failed" else None,
        }
    return sign(
        {
            "protocol": {"major": 1, "minor": 0},
            "type": "capture.request",
            "request_id": request_id,
            "session": {
                "id": "session-device-test",
                "counter": counter,
                "issued_at": now.isoformat().replace("+00:00", "Z"),
                "expires_at": (now + timedelta(minutes=5)).isoformat().replace("+00:00", "Z"),
            },
            "intent": {
                "kind": "user_gesture",
                "token": f"intent-{counter}",
                "issued_at": now.isoformat().replace("+00:00", "Z"),
            },
            "source": {
                "live_url": "https://example.test/device",
                "final_url": "https://example.test/device",
                "title": "Device native-host fixture",
                "selected_text": "synthetic evidence",
                "captured_at": now.isoformat().replace("+00:00", "Z"),
                "redirects": [],
                "scope": "selection",
            },
            "snapshot": snapshot,
            "auth": {"key_id": KEY_ID, "algorithm": "HMAC-SHA256", "mac": ""},
        }
    )


def payload(message: dict, body: bytes, *, tamper_mac: bool = False) -> dict:
    frame = sign(
        {
            "protocol": {"major": 1, "minor": 0},
            "type": "capture.payload",
            "request_id": message["request_id"],
            "session": {
                "id": message["session"]["id"],
                "counter": message["session"]["counter"],
            },
            "sequence": 0,
            "final": True,
            "content_type": "text/html",
            "encoding": "base64",
            "content_hash": message["snapshot"]["content_hash"],
            "bytes": len(body),
            "body": base64.b64encode(body).decode(),
            "auth": {"key_id": KEY_ID, "algorithm": "HMAC-SHA256", "mac": ""},
        }
    )
    if tamper_mac:
        frame["auth"]["mac"] = "0" * 64
    return frame


def frame(value: bytes) -> bytes:
    return len(value).to_bytes(4, "little") + value


def framed_json(value: dict) -> bytes:
    return frame(canonical(value))


def read_frames(raw: bytes) -> list[dict]:
    output: list[dict] = []
    offset = 0
    while offset < len(raw):
        length = int.from_bytes(raw[offset : offset + 4], "little")
        offset += 4
        body = raw[offset : offset + length]
        offset += length
        output.append(json.loads(body))
    return output


def run_host(host: Path, input_bytes: bytes, spool: Path) -> list[dict]:
    environment = {
        **os.environ,
        "LOOM_NATIVE_HOST_KEY_ID": KEY_ID,
        "LOOM_NATIVE_HOST_SECRET_HEX": SECRET.hex(),
        "LOOM_NATIVE_HOST_SPOOL": str(spool),
    }
    result = subprocess.run(
        [str(host)], input=input_bytes, capture_output=True, env=environment, check=True
    )
    if result.stderr:
        raise AssertionError(f"native host wrote unexpected stderr: {result.stderr.decode()}")
    return read_frames(result.stdout)


def extension_messages() -> tuple[dict, list[dict]]:
    script = r'''
import {buildCaptureMessages} from "./browser-extension/src/capture.js";
const now = new Date().toISOString();
const secretBase64 = Buffer.from([...Array(32).keys()]).toString("base64");
const result = await buildCaptureMessages({
  page: {
    liveUrl: "https://example.test/node-extension",
    finalUrl: "https://example.test/node-extension",
    title: "Node extension interoperability",
    selectedText: "synthetic evidence",
    html: "<article><p>node-extension-safe</p></article>",
    online: true,
    scope: "selection",
    redirects: [],
  },
  session: {
    id: "session-node-extension",
    keyId: "pairing-key-device-test",
    secretBase64,
    counter: 0,
    issuedAt: now,
    expiresAt: new Date(Date.now() + 300000).toISOString(),
  },
  intentToken: "intent-node-extension",
  requestId: "44444444-4444-4444-8444-444444444444",
  capturedAt: now,
});
process.stdout.write(JSON.stringify(result));
'''
    result = subprocess.run(
        ["node", "--input-type=module", "-e", script], capture_output=True, text=True, check=True
    )
    payload = json.loads(result.stdout)
    return payload["request"], payload["payloads"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True, type=Path)
    args = parser.parse_args()
    if not args.host.is_file():
        raise SystemExit(f"native host does not exist: {args.host}")

    # First prove that the actual extension builder, rather than a second Python encoder, can
    # authenticate and deliver a capture to the Rust host.
    extension_request, extension_payloads = extension_messages()
    with tempfile.TemporaryDirectory(prefix="loom-native-host-extension-") as temporary:
        extension_spool = Path(temporary)
        responses = run_host(
            args.host,
            b"".join([framed_json(extension_request), *(framed_json(payload) for payload in extension_payloads)]),
            extension_spool,
        )
        if len(responses) != 1 or responses[0].get("type") != "capture.accepted":
            raise AssertionError(f"Node extension interoperability failed: {responses!r}")
        if not (extension_spool / f"{extension_request['request_id']}.html").is_file():
            raise AssertionError("Node extension capture was accepted without a snapshot spool")

    safe_body = b"<article><p>device-safe</p></article>"
    hostile_body = b"<article><script>steal()</script></article>"
    first = request("11111111-1111-4111-8111-111111111111", 1, safe_body)
    replay = payload(first, safe_body)
    hostile = request("22222222-2222-4222-8222-222222222222", 2, hostile_body)
    recovered = request("33333333-3333-4333-8333-333333333333", 3, safe_body)
    malformed = b'{"type":"capture.request","type":"capture.request"}'
    input_bytes = b"".join(
        [
            frame(malformed),
            framed_json(first),
            framed_json(replay),
            framed_json(first),
            framed_json(replay),
            framed_json(hostile),
            framed_json(payload(hostile, hostile_body)),
            framed_json(recovered),
            framed_json(payload(recovered, safe_body, tamper_mac=True)),
        ]
    )

    with tempfile.TemporaryDirectory(prefix="loom-native-host-device-") as temporary:
        spool = Path(temporary)
        responses = run_host(args.host, input_bytes, spool)
        errors = [response.get("error") for response in responses if response.get("type") == "capture.rejected"]
        accepted = [response for response in responses if response.get("type") == "capture.accepted"]
        expected_errors = ["malformed_frame", "replay_rejected", "snapshot_untrusted", "payload_integrity_failed"]
        if errors != expected_errors:
            raise AssertionError(f"unexpected rejection sequence: {errors!r}")
        if len(accepted) != 1 or accepted[0]["request_id"] != first["request_id"]:
            raise AssertionError(f"unexpected accepted responses: {accepted!r}")
        if not (spool / f"{first['request_id']}.html").is_file():
            raise AssertionError("accepted sanitized snapshot was not spooled")
        if list(spool.glob("*.html")) != [spool / f"{first['request_id']}.html"]:
            raise AssertionError("rejected/replayed snapshots left files behind")
        if list(spool.glob("*.json")) != [spool / f"{first['request_id']}.json"]:
            raise AssertionError("metadata spool is not one accepted capture")
    print("native host device contract: PASS (1 accepted, 4 deterministic rejections, recovery safe)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
