# Browser capture protocol v1

Status: proposed for roadmap `0300` (Q05). This is a security and provenance contract for a
future explicit-save browser extension. It does not implement a browser extension, native host,
HTML sanitizer, or web archive.

## Boundary and non-goals

The browser extension is a user-operated source connector. A capture begins only from a visible
browser command such as “Save to LOOM”; LOOM never receives browsing history, background tab
events, cookies, headers, form values, passwords, local storage, or clipboard data through this
protocol. The native messaging host is the only local transport. It binds the extension to a
paired LOOM installation and writes only an accepted, source-backed capture record.

This protocol does not make an extension trustworthy if its browser or operating system is already
compromised. It limits the fields and lifetime of what a compromised page or extension can submit,
requires explicit consent, and makes every rejection visible. It does not promise a complete web
archive, legal authenticity, or a snapshot of content that the user could not access.

## Wire envelope

Native messaging supplies the outer length-prefixed JSON frame. The protocol payload is UTF-8
JSON with deterministic key ordering for authentication. Unknown top-level keys are rejected in
v1 so that a future field cannot silently change the meaning of a signed request. The parser must
reject duplicate JSON keys before canonicalization; accepting “last key wins” would make the
extension and host sign different messages.

The following fields are required for `capture.request`:

```json
{
  "protocol": {"major": 1, "minor": 0},
  "type": "capture.request",
  "request_id": "uuid",
  "session": {
    "id": "paired-session-id",
    "counter": 7,
    "issued_at": "2027-08-23T12:34:56Z",
    "expires_at": "2027-08-23T12:39:56Z"
  },
  "intent": {
    "kind": "user_gesture",
    "token": "single-use-intent-token",
    "issued_at": "2027-08-23T12:34:55Z"
  },
  "source": {
    "live_url": "https://example.test/article",
    "final_url": "https://example.test/article",
    "title": "Synthetic article",
    "selected_text": "A user-selected passage.",
    "captured_at": "2027-08-23T12:34:56Z",
    "redirects": [],
    "scope": "selection"
  },
  "snapshot": {
    "state": "complete",
    "content_hash": "sha256:...",
    "parser_id": "loom.web",
    "parser_version": "0.1.0",
    "bytes": 2048,
    "failure_code": null
  },
  "auth": {
    "key_id": "pairing-key-1",
    "algorithm": "HMAC-SHA256",
    "mac": "hex-encoded-32-byte-mac"
  }
}
```

`live_url` is the URL the user saved. `final_url` is the URL observed after the page completed
its redirects. They are retained separately even when equal. `content_hash` covers the sanitized
snapshot bytes, never the original unsanitized DOM. A `not_requested` snapshot has no hash or
parser fields; a `partial` or `failed` snapshot keeps the live URL and a typed failure reason.

The host returns a `capture.accepted` response containing the stable artifact ID, capture ID,
snapshot state, and source hash, or a `capture.rejected` response with a stable error code. A
rejection never creates a source, version, passage, snapshot, or relationship row.

When `snapshot.state` is `complete` or `partial` and `snapshot.bytes` is non-zero, the request is
followed by one or more authenticated `capture.payload` frames. Payload frames are deliberately
separate from the metadata request so that an 8 MiB best-effort snapshot does not violate the
256 KiB native-messaging envelope limit:

```json
{
  "protocol": {"major": 1, "minor": 0},
  "type": "capture.payload",
  "request_id": "same-request-uuid",
  "session": {"id": "paired-session-id", "counter": 7},
  "sequence": 0,
  "final": false,
  "content_type": "text/html",
  "encoding": "base64",
  "content_hash": "sha256:...",
  "bytes": 160000,
  "body": "base64-of-sanitized-utf8-bytes",
  "auth": {
    "key_id": "pairing-key-1",
    "algorithm": "HMAC-SHA256",
    "mac": "hex-encoded-32-byte-mac"
  }
}
```

Payload `sequence` values start at zero and increase without gaps; exactly one frame has `final:
true`. Each frame carries at most 160 KiB of raw UTF-8 bytes and remains below the 256 KiB encoded
frame limit. The host authenticates every frame, rejects an unexpected request ID, counter,
sequence, hash, content type, or body size, and writes no snapshot until the concatenated bytes
match the request hash. `failed` and `not_requested` requests have no payload frames.

## Pairing, authentication, and version negotiation

1. The native host manifest allowlists the signed extension identifier and the exact LOOM host
   executable. The host does not listen on a TCP, Unix, or HTTP socket.
2. A user starts pairing from a visible LOOM settings action. LOOM generates a random 32-byte
   pairing secret, displays a short one-time code, stores the secret in the macOS Keychain, and
   the extension stores its peer copy in non-synchronised local extension storage. Re-pairing
   replaces the old secret;
   removing the connector revokes every session derived from it.
3. The host sends `session.challenge` with a fresh random challenge, the supported protocol
   versions, an expiry, and a capability set. The extension answers `session.prove` with its
   allowlisted extension ID and an HMAC over the canonical challenge, chosen version, and client
   ID. The host creates a five-minute session only after the HMAC and allowlist checks pass.
4. Each capture request is authenticated with HMAC-SHA256 using the paired secret. The MAC covers
   the complete envelope with `auth.mac` removed, canonical JSON encoding, and UTF-8 bytes. A
   production implementation must use a constant-time comparison.
5. The host accepts only protocol major `1` and a supported minor. It rejects a lower minor when
   the negotiated feature set would change the meaning of a field; it never silently downgrades a
   request. A major mismatch is `protocol_version_unsupported`.
6. `session.counter` is strictly increasing and `request_id` is single-use within the session.
   The host rejects duplicate or older counters, expired timestamps (more than two minutes of
   clock skew), expired sessions, and reused intent tokens as `replay_rejected`.
7. `intent.kind=user_gesture` and a fresh, single-use intent token are mandatory. The extension
   may mint that token only in the visible command handler. The host rejects missing, expired, or
   already-used tokens as `capture_not_approved`; it never accepts a timer, page script, history
   event, or background worker as consent.

The pairing secret is a capability, not proof that the extension is safe. Users can revoke it,
pause browser capture, delete all connector-derived records, or operate LOOM in no-network mode.
All timestamps are UTC RFC 3339 with a `Z` suffix; the host uses its own clock for expiry and
never trusts a timestamp supplied by the page.

## Fields and exclusions

| Field | Rule | Reason |
| --- | --- | --- |
| `live_url` | Required absolute `http` or `https` URL; max 16 KiB | Preserve what the user saved. |
| `final_url` | Required when a redirect was observed; max 16 KiB | Preserve source identity drift. |
| `title` | Optional; UTF-8, max 512 bytes | Useful display metadata only. |
| `selected_text` | Optional; max 64 KiB; user-selected only | Avoid hidden page-wide collection. |
| `captured_at` | Required UTC RFC 3339 timestamp | Establish capture time without trusting page time. |
| `redirects` | At most 8 URL hops, 2 KiB each | Bound hostile redirect chains. |
| `scope` | `selection` or `page` | Make the user's requested scope explicit. |
| `snapshot.state` | `not_requested`, `complete`, `partial`, or `failed` | Separate live and saved truth. |
| `snapshot.content_hash` | SHA-256 of sanitized bytes when present | Bind derived content to stored bytes. |
| `snapshot.parser_id/version` | Required for `complete` and `partial` | Make transformations reproducible. |
| `snapshot.bytes` | 0 through 8 MiB | Bound storage and parser work. |
| `snapshot.failure_code` | Required for `partial` and `failed` | Explain limitations without inventing content. |

The extension must omit cookies, authorization headers, request bodies, password or hidden form
values, autofill data, local/session storage, service-worker caches, browser history, tab lists,
page scripts, remote resource URLs, and raw DOM event handlers. The sanitizer strips scripts,
event-handler attributes, forms, `javascript:` URLs, and external resource loads before hashing.

Selected text can still contain a secret; the UI must show the selection and let the user cancel
before transmission. A future redaction pass must be opt-in and must retain a visible transform
record rather than changing the canonical source silently.

## Redirects and snapshot truth

The extension records the clicked `live_url`, the observed `final_url`, and a bounded redirect
chain. A same-origin redirect may be captured without a second prompt. A cross-origin final URL
requires the visible capture action to remain active; otherwise the host records `partial` with
`cross_origin_redirect` and retains only the live URL and redirect metadata. Redirects are never
followed by the native host and are not used to read arbitrary URLs.

Snapshot state is an honest outcome, not a boolean archive claim:

| State | Meaning | Required user-facing message |
| --- | --- | --- |
| `complete` | Sanitized bytes were captured and hashed under the configured limits. | “Saved snapshot at capture time.” |
| `partial` | The live URL and some selected metadata were saved, but content was truncated or a page capability was unavailable. | “Partial snapshot; the live page may differ.” |
| `failed` | No snapshot bytes were accepted; the live URL and failure reason remain inspectable. | “Snapshot unavailable; open the live URL.” |
| `not_requested` | The user chose metadata/selection only. | “Live URL saved; no snapshot requested.” |

Paywalls, login walls, CSP restrictions, JavaScript-only rendering, robots or site policy, deleted
pages, and parser failures can produce `partial` or `failed`. LOOM must display the status, capture
time, hash when available, parser version, and reason. It must never describe a best-effort local
snapshot as a complete archive or as evidence that the current live page still matches.

## Resource, failure, and recovery rules

- Reject a frame larger than 256 KiB, selected text larger than 64 KiB, or sanitized snapshot bytes
  larger than 8 MiB before writing anything; return `payload_too_large`.
- Treat a missing, duplicate, out-of-order, or hash-mismatched payload frame as
  `payload_integrity_failed`; discard all uncommitted chunks and retain only the live URL plus a
  visible partial/failed reason.
- Cap redirects at eight hops and reject malformed or non-HTTP(S) URLs with `source_invalid`.
- Apply a per-session rate limit of four accepted requests per minute. Return
  `capture_rate_limited` without changing the library when the limit is exceeded.
- On parser timeout, sanitizer failure, or disk failure, retain no unverified snapshot bytes. The
  response contains a typed failure and the live URL only; a later explicit retry gets a new
  request ID and counter.
- Replaying a rejected request remains rejected. Recovery requires a new visible user gesture and
  a new request, not a retry loop in the background.
- Revocation removes the pairing secret, invalidates sessions, and marks connector records as
  unavailable until the user explicitly deletes or re-pairs them.

## Threat coverage

| Threat | Required control | Observable failure |
| --- | --- | --- |
| Malicious page | Allowlisted fields, sanitizer, no script/resource execution, byte limits | `source_invalid`, `payload_too_large`, or `snapshot` partial/failed |
| Compromised extension | Pairing secret, user gesture token, revocation, explicit scope, no history APIs | `capture_not_approved` or a user-visible capture record |
| Native messaging spoof | Browser allowlist, signed executable path, no listening socket | Session challenge/proof failure |
| Replay | Session expiry, monotonic counter, single-use request and intent IDs | `replay_rejected` |
| Downgrade | Major-version match and negotiated minor feature set | `protocol_version_unsupported` |
| Sensitive fields | Explicit exclusion list and selection preview | Field is absent; user can cancel |
| Redirect abuse | Bounded chain, same-origin default, no host-side fetch | `cross_origin_redirect` partial state |
| Oversized payload | Frame, text, snapshot, and rate limits before persistence | `payload_too_large` or `capture_rate_limited` |

The protocol does not defend against a compromised macOS account, Keychain, browser process, or
signed native host. Those are separate release threat-model and notarization gates.

## Implementation and test gate

An implementation may start only after this document and the fixture contract are reviewed. The
fixture-driven validator at `scripts/test-browser-capture-protocol.py` checks the required fields,
canonical HMAC test vector, limits, status semantics, forbidden fields, replay counter, downgrade,
missing-consent, and oversized-payload rejection cases. It is a protocol contract test, not a
claim that the browser extension exists.

Before roadmap `0301` can close, the extension and host must add cross-browser tests for redirect,
SPA, offline, denied-capture, malformed-frame, revocation, and recovery cases. A consented device
session must record permission prompts and the exact captured artifact. Any desktop screenshot
used for that evidence must be cropped to the relevant browser/LOOM panel; no full-screen capture
is an acceptable substitute.
