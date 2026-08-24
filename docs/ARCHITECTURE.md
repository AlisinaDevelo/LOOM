# Architecture

LOOM is a pre-alpha desktop and CLI application. The current architecture keeps a small,
source-backed retrieval path stable while leaving semantic retrieval and additional extractors as
separate, rebuildable layers.

## Current flow

```text
explicit file or directory selection
        |
        v
backend-owned selection -> bounded traversal (no symlink following)
        |
        v
no-follow, contained stable read -> line-ending normalization -> BLAKE3 hash
        |
        v
artifact version -> passages with text/page/pixel anchors
        |
        +--> SQLite canonical tables
        |
        +--> SQLite FTS5 lexical index
                    |
                    v
              source-backed search hit
              (path, hash, excerpt, anchor)
```

The source file remains the authority for opening the original. LOOM stores extracted passage text
and metadata in the local database so searches can run without rereading every source file. The
current file ingestion path does not upload, copy, or synchronize source content to a service.
Image OCR is a local macOS Vision call; only derived text, fixed-point region geometry, and
provider/model metadata enter the canonical database.

## Components

### loom-core

The Rust core owns ingestion, passage segmentation, the SQLite schema, FTS5 queries, statistics, and
evidence-bearing search results. Text uses `loom.text` 0.1.0; PDF uses `loom.pdf` 0.1.0; image OCR
uses `loom.ocr` 0.1.0 with a native provider boundary.

Ingestion accepts an explicitly selected regular file or directory, supports UTF-8 .txt/.md/.markdown,
bounded text-based PDFs, and common PNG/JPEG/GIF/WebP images, does not follow symlinks, limits a file
to 8 MiB, and limits one traversal to 20,000 files. Reads use a no-follow descriptor on Unix,
canonical root containment, file identity, size, modification-time, and post-read checks. Files
that change during a read are retried and reported if
a stable read cannot be obtained. A complete directory rescan marks disappeared, unsupported, or
unreadable prior sources missing so they no longer search. Text is normalized to LF before passage
offsets are computed. Passages target 1,000 characters with 120 characters of overlap.

### loom-cli

The CLI indexes a selected path, searches the configured database, reports statistics, inspects a
stored extraction, checks or repairs the derived FTS5 projection, and runs the retrieval smoke
benchmark. Its default database is .loom/library.sqlite3; callers can provide another path.

### Tauri shell and UI

The Tauri layer opens the application-data SQLite database and exposes the narrow commands
index_selected_folder, cancel_indexing, reconcile_approved_roots, list_source_roots,
revoke_source_root, search, library_stats, and open_artifact. Folder selection happens inside the
Rust command through the native dialog; the webview does not provide a path. Persisted roots are
exact read-only locators with
available/missing/denied/moved/unsafe/revoked status. Re-selection through the native picker is the
current relaunch recovery path; a future sandboxed build must add persistent security-scoped
bookmarks rather than widening permissions. Opening requires the artifact ID, version ID, and content
hash returned by search, then rereads and verifies current source bytes before handing the path to the
host. The React UI can search, inspect structured source-backed evidence, inspect/revoke scopes, and
request that verified open operation. Tauri capability configuration is a security boundary and must
be reviewed with command changes. See the
[Tauri capabilities documentation](https://v2.tauri.app/security/capabilities/).

### Benchmark fixture

benchmarks/retrieval/v0/ is a synthetic, rights-clean smoke fixture. It is intentionally separate
from user data and is not a claim about production retrieval quality.

## Persistence

SQLite is the canonical store. The current schema is version 5 and contains source roots, logical
artifacts, locators, content versions, passages, reserved relationships, durable indexing-job
checkpoints, and extraction metadata. Each passage has a JSON text, PDF-page, or image-region
anchor plus scalar offsets. Versions 2–4 migrate transactionally while preserving populated
canonical rows; see [SCHEMA_COMPATIBILITY.md](SCHEMA_COMPATIBILITY.md).

SQLite FTS5 is an external-content virtual table over passages. Insert, update, and delete triggers
keep the lexical index synchronized with canonical passage rows. The disposable `fts5vocab` row and
instance projections provide term and indexed-document digests for the health command. Search uses
sanitized FTS5 terms and phrases and BM25 ranking, then projects matches into structured source-text
segments and exact character/line anchors. Highlight state is data, not an in-band source-text
marker. See the [SQLite FTS5 reference](https://www.sqlite.org/fts5.html).

The connection uses foreign keys, a five-second busy timeout, WAL journaling, NORMAL synchronous
mode, in-memory temporary storage, and SQLite trusted-schema hardening. These are operational
choices for the local database, not a promise of crash-proof or encrypted storage. SQLite documents
the WAL trade-offs in its [WAL reference](https://www.sqlite.org/wal.html).

The current schema is version 5. Opening validates a known marker's required tables and columns,
refuses a missing, malformed, or unknown version marker instead of rewriting it, and supports
reviewed versions 2–4 transactional migrations. The disposable FTS5 projection is rebuilt from
canonical passages on open; canonical rows are never reconstructed from FTS5. Pre-alpha version 1
databases are explicitly rejected because their content-version uniqueness contract did not include
extractor identity. A content observation is keyed by source artifact, byte hash, extractor, and
extractor version so changed extraction logic cannot silently reuse old passages. Each indexing
unit advances a fingerprint-bound checkpoint in the same transaction as canonical writes, allowing
an interrupted or cancelled scan to resume without making a partially committed artifact searchable.
The returned report carries the durable job ID plus discovered, attempted, indexed, unchanged,
skipped, failed, and cancelled counts. The desktop stop command sets a cooperative token; the
worker observes it between units, marks the checkpoint interrupted with `cancelled by request`,
and leaves already committed versions intact.

`fts_health` creates a scratch FTS5 tokenizer projection from canonical passages and compares its
expected vocabulary digest, canonical passage digest, and indexed-document coverage with the
disposable projection. `repair_fts` runs the FTS5 `rebuild` command inside one transaction and
returns the before/after health reports; it never updates canonical source rows.

PDF ingestion uses the pure-Rust `pdf-extract` provider over source bytes already admitted by the
stable-read boundary. Each page becomes a local `pdf_page` anchor with page number, character and
line span; parser/page warnings and page count are stored on the immutable artifact version.
Encrypted, malformed, image-only, over-page-limit, and over-byte-limit PDFs fail closed with a
bounded report. The original path remains the render/open authority; LOOM never copies a PDF just
to display a page.

Image ingestion uses macOS Vision through the isolated `loom-ocr-macos` crate. The provider
returns owned Rust values only: OCR text, confidence, provider/model identity, and normalized
lower-left rectangles. The core converts rectangles once into clamped top-left pixel bounds after
EXIF orientation, stores fixed-point confidence/scale and extraction metadata, and never stores
Objective-C objects or source image bytes. OCR is an explicit local policy: disabling it
transactionally deletes every `loom.ocr` version and passage while retaining the source locator;
re-enabling and re-indexing recovers the derived records. Malformed images, empty OCR results, and
non-macOS provider absence fail closed with a visible bounded report.

Approved-root observation is deliberately conservative. The coalescer accepts only absolute,
in-scope hints, debounces duplicate create/modify/remove/rename events, and turns overflow or large
batches into a full rescan. The desktop startup command rechecks every enabled persisted root by
content hash; event hints never become source truth, and an unavailable root is reported without
widening permissions. Native watcher integration can replace the hint source without changing this
reconciliation boundary.

## Invariants

- Indexing starts only from a path explicitly supplied by the user or the UI picker.
- A ready content version is identified by its artifact, content hash, extractor identity, and
  extractor version; only an unchanged projection is reused.
- The hash is BLAKE3 over the source bytes read from disk.
- Passage anchors refer to normalized text and never split a Unicode scalar boundary.
- A search result resolves through an active artifact locator and retains its source hash and
  anchor.
- Opening a result succeeds only while the active version and current source hash still match the
  result; changed bytes return an explicit stale-source error.
- Query text is treated as user input; FTS5 operators are not accepted as an escape hatch into a
  different query language.

## Boundaries and planned extensions

The current SQLite records are the authority for source identity, text, versions, and evidence. An
eventual semantic index may add vectors or other derived representations, but it must be rebuildable
from canonical records and must not replace the source-backed result contract. Browser capture,
external model providers, cloud sync, and managed copies are not implemented in this slice.

## References

- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite Write-Ahead Logging](https://www.sqlite.org/wal.html)
- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
