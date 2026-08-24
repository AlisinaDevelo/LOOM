# ADR 0011: Import browser bookmarks as source-faithful metadata

- Status: Accepted for the pre-alpha connector
- Date: 2026-08-25

## Context

Chrome and Firefox both export a Netscape HTML file. LOOM users need to recover a saved URL and
the context in which it was filed, but importing an export must not silently become a web crawler
or promise an immutable snapshot. The import also needs a durable answer to whether a repeated
export was new, unchanged, merged, or conflicting.

## Decision

Read one explicitly selected regular export file, parse the bounded Netscape HTML format locally,
and preserve folder path, title, URL, `ADD_DATE`, and `LAST_MODIFIED`. Store the export locator,
format, BLAKE3 hash, and import timestamp in `bookmark_imports`; store the current per-folder URL
record and its entry hash in `bookmark_records`; store per-entry outcomes in
`bookmark_import_items`. Use a `text/x-bookmark` artifact and a `loom.bookmark` passage so the
existing source-backed search contract can return the title, URL, folder, import ID, and hash.

The operation never resolves a URL or opens a network client. It reports `remote_fetches: 0`.
Live-page capture or a saved snapshot, if ever added, is a separate explicit action with its own
consent and visible best-effort capture status.

## Consequences

- Repeated identical exports are idempotent by `(source locator, format, content hash)`.
- Changed metadata remains recoverable as a new import/version, while current records report merges
  and duplicate-URL conflicts.
- Bookmark URLs are locators and searchable evidence, not proof that the current remote page exists.
- The schema adds three canonical history tables in version 7 and migrates version 6 additively.
- A browser permission session, live fetcher, and web archive are intentionally out of scope.

## Rejected alternatives

- Fetch every imported URL: rejected because it violates intentional local scope, adds network and
  privacy risk, and cannot guarantee a faithful snapshot.
- Store only a URL list: rejected because it loses folder/title/timestamp context and import lineage.
- Treat the HTML export as an authoritative web archive: rejected because export metadata is not a
  capture of remote page bytes.
