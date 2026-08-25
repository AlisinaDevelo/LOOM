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
- for images, derived OCR text, pixel-region anchors, provider/model identity, automatic-language
  mode, confidence and its confirmed/low-confidence state, image hash, EXIF orientation, and
  fixed-point display scale;
- for an explicitly rebuilt semantic derivative, passage hashes, provider/model identity, vector
  dimensions/revision, and encoded local vectors;
- local timestamps and indexing statistics.
- for an explicitly selected Chrome or Firefox Netscape HTML export, the export path/hash, bookmark
  folder, title, URL, browser timestamps, per-entry hash, and import/merge/conflict history. LOOM
  does not fetch or store remote page content during this import.

Search queries are used in memory for the current search operation and are not a field in the
canonical schema. The current UI does not send them to a service.

## Where it is stored

The CLI stores the database at .loom/library.sqlite3 unless another path is supplied. The Tauri
application stores it in the operating system's application-data directory. SQLite WAL and temporary
files may be created next to or under the database according to SQLite's normal operation. The
database is not encrypted at rest. The storage inspector accounts for the database, WAL/SHM/journal
sidecars, and LOOM's fixed `cache`, `model-cache`, `thumbnails`, `ocr-scratch`, `tmp-exports`, and
`logs` directories without following symlinks.

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
- Only UTF-8 .txt/.md/.markdown, bounded text-based PDFs, and common PNG/JPEG/GIF/WebP images are
  accepted in this slice. Image OCR is local macOS Vision processing; it is disabled/purged as a
  user-visible policy and never uploads source bytes.
- No automatic cloud upload or third-party model processing occurs in the current path.
- Semantic commands run the current deterministic provider locally and never download a model or
  send passage text to a network service. The vector tables are derived and can be dropped without
  deleting canonical source records.
- Revoking a saved scope disables future reconciliation and hides its active artifacts from search;
  canonical historical rows remain until the user explicitly deletes the indexed data or applies a
  retention policy.
- The desktop stop control requests cooperative cancellation at a bounded indexing-unit boundary;
  it does not upload, discard, or roll back a complete source version already committed locally.
- A complete rescan hides removed or unreadable sources from search, but does not erase their stored
  historical passage text. Use the artifact/root/time deletion controls when erasure is intended.
- Bookmark import reads only the selected regular export file, rejects symlinks and executable URL
  schemes, preserves source/export hashes and entry outcomes, and reports `remote_fetches: 0`.
  Repeating the same export is idempotent; changed exports create a new import record and report
  merges or duplicate-URL conflicts. No network client is part of this path.
- Disabling OCR or invoking the OCR purge removes derived `loom.ocr` versions/passages but retains
  the original image locator and source bytes. Re-indexing after re-enabling recreates the derived
  records.
- The storage inspector reports approximate canonical/source, derived, SQLite-sidecar, and known
  disposable bytes by source/path. `purge-artifact`, `purge-root`, and `purge-before` delete
  canonical and derived rows transactionally, rebuild FTS5, checkpoint/vacuum SQLite, and verify
  clean state after restart in the device test suite.
- Retention is disabled by default. A user may configure 1–36,500 days and explicitly apply it;
  applying retention deletes artifacts older than the computed RFC3339 cutoff. `purge-disposable`
  removes only known disposable files and sidecars, never user-owned source files or captures.

Do not select confidential or regulated material unless you understand the local storage,
operating-system, backup, and deletion implications. Treat the database and its WAL files as
Sensitive material also includes local logs and caches. These controls are application-level
deletion, not a claim of cryptographic secure erasure from filesystem snapshots or backups.

## Future changes

Any network connector, browser capture, model provider, semantic index, managed-copy feature, or
multi-device sync must define collection scope, consent, retention, deletion, encryption, access
control, and failure behavior before it is enabled. A derived representation must not silently
expand the collection or become the only way to recover source evidence.

The current [explicit-save browser connector prototype](../browser-extension/README.md) is not an
always-on capture service: it requests no host permissions, history, tabs, cookies, or network
access, refuses to inspect a page until a local pairing exists, and sends only sanitized,
user-requested data to the named native host. The checked host refuses unpaired configuration and
spools only hash-verified sanitized bytes; Keychain pairing, a consented browser permission
session, signed packaging, and host-to-library storage integration remain separate release gates.

The current direct-distribution build uses explicit re-selection rather than claiming a
security-scoped bookmark. A future sandboxed/notarized build must replace or supplement that path
with persistent security-scoped bookmarks and test stale bookmark recovery. The design is informed
by the [NIST Privacy Framework](https://www.nist.gov/privacy-framework) and the
[Tauri capabilities model](https://v2.tauri.app/security/capabilities/).
