# Privacy

LOOM's current pre-alpha path is local-only. It has no account, cloud service, telemetry, or network
synchronization in the supported ingestion and search flow. Local-only does not mean private by
default: the application writes indexed content and source metadata to a local SQLite database and
the host, local users, backups, and other software can access that database.

## What is processed

For a selected file or directory, the current path may store:

- the selected source-root path;
- source file paths, titles, and media types;
- source byte size and source modification time;
- a BLAKE3 content hash;
- normalized passage text and exact character/line anchors;
- extractor identity and version;
- local timestamps and indexing statistics.

Search queries are used in memory for the current search operation and are not a field in the
canonical schema. The current UI does not send them to a service.

## Where it is stored

The CLI stores the database at .loom/library.sqlite3 unless another path is supplied. The Tauri
application stores it in the operating system's application-data directory. SQLite WAL and temporary
files may be created next to or under the database according to SQLite's normal operation. The
database is not encrypted at rest.

LOOM stores extracted text for search; it does not currently copy source files into a managed
document store. Opening evidence asks the host to open the original path, which may expose that path
or content to the selected external application.

## Current controls and limits

- The user chooses the file or folder through the CLI or a backend-owned native desktop picker; the
  desktop webview cannot submit an arbitrary path to the index command.
- The desktop persists only the exact canonical locator selected by the user. It exposes the saved
  scope as read-only, reports available/missing/denied/moved/revoked states, and offers explicit
  re-selection; it never falls back to the home directory or another broader path.
- Directory traversal does not follow symlinks; stable reads enforce canonical root containment and
  verify descriptor/path identity before and after reading.
- Only UTF-8 .txt, .md, and .markdown files are accepted in this slice.
- No automatic cloud upload or third-party model processing occurs in the current path.
- Revoking a saved scope disables future reconciliation and hides its active artifacts from search;
  canonical historical rows remain until a future retention/purge policy is implemented.
- The desktop stop control requests cooperative cancellation at a bounded indexing-unit boundary;
  it does not upload, discard, or roll back a complete source version already committed locally.
- A complete rescan hides removed or unreadable sources from search, but does not erase their stored
  historical passage text.

Do not select confidential or regulated material unless you understand the local storage,
operating-system, backup, and deletion implications. Treat the database and its WAL files as
sensitive if the indexed collection is sensitive.

## Future changes

Any network connector, browser capture, model provider, semantic index, managed-copy feature, or
multi-device sync must define collection scope, consent, retention, deletion, encryption, access
control, and failure behavior before it is enabled. A derived representation must not silently
expand the collection or become the only way to recover source evidence.

The current direct-distribution build uses explicit re-selection rather than claiming a
security-scoped bookmark. A future sandboxed/notarized build must replace or supplement that path
with persistent security-scoped bookmarks and test stale bookmark recovery. The design is informed
by the [NIST Privacy Framework](https://www.nist.gov/privacy-framework) and the
[Tauri capabilities model](https://v2.tauri.app/security/capabilities/).
