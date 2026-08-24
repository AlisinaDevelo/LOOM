# Threat model

This is a compact threat model for the current pre-alpha local path. It describes implemented
boundaries and known limitations; it is not a security certification.

## Assets

- source file contents and the user's source paths;
- canonical SQLite records, WAL files, and temporary database state;
- content hashes, passage anchors, and search excerpts;
- source-to-source relationships reserved by the schema;
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
4. FTS5 derives a lexical search structure from canonical passage rows.
5. Tauri IPC exposes indexing, scope status/revocation, search, statistics, and source opening to
   the UI.
6. The host external application receives an original path when the user requests opening it.
7. A future semantic index or connector would be a new boundary and requires a separate review.

## Current risks and mitigations

| Risk                          | Current mitigation                                                                                              | Remaining limitation                                                                   |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Indexing an unintended tree   | Backend-owned desktop selection; no frontend path argument; no-follow traversal and canonical containment       | The user or CLI can still choose a broad or sensitive path                             |
| Resource exhaustion           | 8 MiB per-file and 20,000-file traversal limits                                                                 | Large collections still consume local CPU, memory, and disk                            |
| Reading changing bytes        | No-follow descriptor, root containment, device/inode, size/mtime, post-read checks, and retries                 | Network or hostile filesystems can still deny service; this is not a forensic snapshot |
| Unsupported or malformed text | Extension allowlist and UTF-8 requirement                                                                       | No malware scanning or general document sandbox                                        |
| FTS query injection           | Search terms and phrases are compiled as safe FTS5 input                                                        | Search is lexical and does not defend a compromised database                           |
| Stale evidence                | Complete rescans hide missing/failed sources; open is bound to artifact/version/hash and rehashes current bytes | Historical extracted text remains until a purge feature exists                         |
| Local data disclosure         | No network path in the current slice; UI displays source-backed results                                         | SQLite and WAL files are unencrypted and readable by local access                      |
| Persisted scope drift         | Exact locators, status inspection, explicit re-selection, and revocation hide active artifacts                  | Direct-distribution build does not yet persist macOS security-scoped bookmarks         |
| Tauri command overreach       | Narrow command set; backend-owned selection; no arbitrary path command; hash-bound open                         | Capability configuration and host permissions still need release hardening             |
| Dependency compromise         | Locked dependency resolution and ordinary build review                                                          | No reproducible-build or signed-release guarantee yet                                  |

## Security requirements for extensions

Before adding OCR/PDF, browser capture, embeddings, network access, accounts, or managed copies, the
design must specify:

- explicit user consent and collection scope;
- least-privilege capabilities and process boundaries;
- secret and credential handling;
- content retention and deletion behavior;
- provenance and evidence preservation;
- resource limits and cancellation;
- tests for malformed and adversarial inputs;
- a reviewable failure mode when derived data cannot be mapped back to its source.

## References

- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)
