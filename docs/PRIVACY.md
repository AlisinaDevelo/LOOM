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
- Directory traversal does not follow symlinks; stable reads enforce canonical root containment and
  verify descriptor/path identity before and after reading.
- Only UTF-8 .txt, .md, and .markdown files are accepted in this slice.
- No automatic cloud upload or third-party model processing occurs in the current path.
- There is no user-facing purge, retention, encryption, or source-revocation workflow yet.
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

The design is informed by the [NIST Privacy Framework](https://www.nist.gov/privacy-framework) and
the [Tauri capabilities model](https://v2.tauri.app/security/capabilities/).
