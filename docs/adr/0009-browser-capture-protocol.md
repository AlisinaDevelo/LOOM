# ADR 0009: Bound browser capture to explicit, authenticated saves

- Status: Proposed for roadmap `0300`
- Date: 2026-08-24

## Context

LOOM may eventually accept a webpage that a user deliberately saves from a browser. A browser
connector crosses a trust boundary that the current local-file path does not have: the page can be
hostile, the extension can be compromised, redirects can change source identity, and a snapshot can
be incomplete. A native messaging transport alone does not authenticate the intended LOOM
installation or prevent replay. Treating a live URL as a complete archive would also make the
evidence contract misleading.

## Decision

Adopt the versioned explicit-save protocol in
[`docs/protocol/browser-capture-v1.md`](../protocol/browser-capture-v1.md) before implementing an
extension. The first implementation will:

- use a browser-allowlisted native messaging host with no listening socket or network path;
- require visible pairing, a Keychain-backed capability, HMAC-authenticated canonical envelopes,
  session expiry, monotonic counters, single-use request/intent IDs, and explicit user-gesture
  tokens;
- accept only an allowlisted metadata/selection/sanitized-snapshot field set with bounded sizes;
- retain the live URL, final URL, capture time, hash, parser identity, and snapshot state separately;
- carry sanitized HTML in separately authenticated, ordered payload frames so the native-messaging
  envelope remains bounded while the final content hash is verified before persistence;
- report complete, partial, failed, or not-requested snapshot outcomes without claiming archive
  completeness; and
- reject malformed, replayed, downgraded, unapproved, redirected, or oversized requests before
  persistence.

The protocol specification and fixture validator remain the review gate. The extension, sanitizer,
Rust native host, and browser-specific contract tests now have deterministic repository coverage;
Keychain pairing, signed host packaging, and a consented browser permission session remain
deliberately deferred to roadmap `0301` release gates.

## Consequences

The design adds pairing and capability lifecycle work before a capture can be accepted, but it
keeps the first connector intentional and local-first. Provenance remains inspectable when a page
changes or a snapshot cannot be saved. A compromised browser or operating system is outside this
boundary and remains an explicit release limitation. The fixture contract can be tested offline
before macOS browser permissions and native packaging are available.

## References

- [Browser capture protocol v1](../protocol/browser-capture-v1.md)
- [Threat model](../THREAT_MODEL.md)
- [Roadmap issue 0300](https://github.com/AlisinaDevelo/LOOM/issues/29)
