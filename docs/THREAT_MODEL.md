# Threat model

This is a compact threat model for the current pre-alpha local path. It describes implemented
boundaries and known limitations; it is not a security certification.

## Assets

- source file contents and the user's source paths;
- canonical SQLite records, WAL files, and temporary database state;
- content hashes, passage anchors, and search excerpts;
- source-to-source relationship records and their evidence metadata;
- the Tauri command surface and the ability to open an original path.

## Actors and assumptions

The user is trusted to select the collection and protect the host. The model considers:

- accidental over-selection of a sensitive directory;
- a malicious or malformed local text file;
- another local process or user able to read the database;
- a compromised dependency or build artifact;
- future connectors or model providers that cross the local boundary.

The current threat model does not claim to protect against a compromised operating system, malware,
privileged local access, a malicious desktop environment, secure-erasure requirements, or a remote
service attack. There is no supported remote service in this slice.

## Trust boundaries

1. The CLI supplies an explicit path, or the Rust backend owns a native folder picker; the desktop
   webview cannot submit a path to the index command.
2. Persisted roots retain only the exact selected locator and a read-only contract. Missing, denied,
   moved, unsafe, and revoked states are visible; no fallback path is inferred.
3. The indexer traverses and reads local files, then writes normalized text and metadata to SQLite.
4. PDF/text/image extractors derive canonical passage text from explicitly selected bytes; page or
   pixel-region anchors, parser/OCR warnings, provider/model identity, confidence, and extraction
   metadata remain inspectable. FTS5 and its vocabulary projections derive a lexical search
   structure from canonical passage rows; health/repair diagnostics never promote derived state to
   authority.
5. Tauri IPC exposes indexing, cooperative cancellation, scope status/revocation, search,
   statistics, and source opening to the UI.
6. The host external application receives an original path when the user requests opening it.
7. Relationship creation validates both artifact endpoints and optional passage evidence before
   writing the canonical graph row; the bounded read API exposes endpoint versions and hashes.
8. A future semantic index or connector would be a new boundary and requires a separate review.

## Current risks and mitigations

|Risk|Current mitigation|Remaining limitation|
|---|---|---|
|Indexing an unintended tree|Backend-owned desktop selection; no frontend path argument; no-follow traversal and canonical containment|The user or CLI can still choose a broad or sensitive path|
|Resource exhaustion|8 MiB/file and 20,000-file limits; unit cancellation preserves a resumable checkpoint safely|Large collections still consume local CPU, memory, and disk|
|Reading changing bytes|No-follow descriptor, root containment, device/inode, size/mtime, post-read checks, and retries|Network or hostile filesystems can still deny service|
|Unsupported or malformed text/image|Extension allowlist, UTF-8 requirement, bounded image dimensions, Vision fail-closed errors|No malware scanning or general document sandbox|
|FTS query injection|Search terms and phrases are compiled as safe FTS5 input|Search is lexical and does not defend a compromised database|
|Stale evidence|Complete rescans hide missing/failed sources; open is bound to artifact/version/hash and rehashes current bytes|Historical extracted text remains until a purge feature exists|
|Local data disclosure|No network path in the current slice; OCR provider is local-only; UI displays source-backed results|SQLite and WAL files are unencrypted|
|Persisted scope drift|Exact locators, status inspection, explicit re-selection, and revocation hide active artifacts|Direct-distribution build lacks security-scoped bookmarks|
|Tauri command overreach|Narrow command set; backend-owned selection; no arbitrary path command; hash-bound open|Capabilities and host permissions need release hardening|
|Dependency compromise|Locked dependency resolution and ordinary build review|No reproducible-build or signed-release guarantee yet|
|Relationship spoofing or graph disclosure|Typed known kinds, preserved unknown strings, explicit origin/method/confidence, endpoint existence checks, endpoint-bound passage evidence, bounded reads, and no relationship write command in the webview|A compromised local process can still write SQLite directly; independent audit and encrypted storage are not implemented|
|Browser connector over-collection|`activeTab` plus explicit command only; no host permissions, history listeners, or network path; sanitized attributes are discarded|A consented Chrome/Firefox permission session and paired native host remain unverified|

## Security requirements for extensions

Before adding browser capture, embeddings, network access, accounts, or managed copies, the design
must specify the following. PDF and image extraction are already bounded to explicitly selected
source bytes and remain subject to the limits above:

- explicit user consent and collection scope;
- least-privilege capabilities and process boundaries;
- secret and credential handling;
- content retention and deletion behavior;
- provenance and evidence preservation;
- resource limits and cancellation;
- tests for malformed and adversarial inputs;
- a reviewable failure mode when derived data cannot be mapped back to its source.

The browser-capture specification is recorded in
[ADR 0009](adr/0009-browser-capture-protocol.md) and
[the protocol contract](protocol/browser-capture-v1.md). It adds pairing, authenticated
envelopes, replay/downgrade rejection, explicit user-gesture tokens, field exclusions, redirect
limits, and visible best-effort snapshot states before an extension is implemented.

## References

- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)
