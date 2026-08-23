# ADR 0001: Local-first and evidence-first retrieval

- Status: Accepted
- Date: 2026-08-23

## Context

The first useful LOOM result must let a user recover the source object that supports it. A network
service, generated summary, or opaque ranking score is not a sufficient source contract. The current
product slice also needs to work without an account or a network dependency.

## Decision

Keep the supported path local-first and make source-backed evidence the result contract. Index only
a path explicitly selected by the user. Return the source locator, content hash, excerpt, and exact
character/line anchor with a result. Treat the local SQLite records as a retrieval aid, not as a
replacement for the original source.

The current path will not add cloud sync, accounts, telemetry, or external model calls as hidden
dependencies.

## Consequences

Positive:

- A result can be checked against a concrete source file.
- The first path has a small, inspectable trust boundary.
- Offline operation is possible after the local database is available.

Negative:

- Moved, changed, or deleted files return a stale-source error; a complete rescan hides them, but
  their historical extracted passages remain stored until a purge policy exists.
- Local storage must be protected by the host and its backup policy.
- Source-backed retrieval is less convenient than a system that silently copies every source.

## Out of scope

Always-on capture, browser capture, cloud sync, multi-device sync, and generated answers are not
part of this decision. They require separate consent, privacy, threat, and evidence decisions.
