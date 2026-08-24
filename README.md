# LOOM

Recover the exact source object, locally.

LOOM is a pre-alpha, local-first retrieval tool for finding passages in a user-selected collection
of text and Markdown files. The current slice is deliberately small: it preserves the source path,
content hash, excerpt, and exact text anchor with every result.

> [!WARNING] LOOM is pre-alpha software. Do not rely on it as a backup, records system, security
> boundary, or production data-loss prevention system. The current local database is not encrypted
> at rest.

## Current supported slice

- An explicitly selected file or directory is indexed; directory traversal does not follow symlinks.
  The desktop picker runs in the Rust backend, so the webview cannot submit an arbitrary path to the
  index command.
- Supported inputs are UTF-8 .txt, .md, .markdown, and bounded text-based .pdf files. Files larger
  than 8 MiB, PDFs over 2,048 pages, and traversals larger than 20,000 files are rejected or
  reported with an explicit bounded outcome.
- SQLite is the canonical store. SQLite FTS5 provides lexical retrieval over indexed passages.
- `loom-cli fts-health` compares canonical passage hashes and tokenizer vocabulary with the
  disposable FTS5 projection; `loom-cli fts-repair` rebuilds it transactionally and reports before/
  after digests.
- `loom-cli inspect path/to/source` prints the stored extractor identity, PDF page count/warnings,
  and exact anchors for an indexed source.
- Results include the original path, BLAKE3 content hash, structured excerpt, and exact
  character/line anchor. Literal source characters cannot become highlight instructions.
- A complete rescan hides deleted or unreadable sources. Opening a result verifies its artifact,
  version, and current BLAKE3 hash before handing the path to another application.
- The CLI and Tauri desktop UI use the same Rust core and local database.
- Saved desktop scopes persist as exact read-only locators. Missing, denied, moved, unsafe, and
  revoked roots are visible; re-selection is explicit and never broadens access.
- Indexing reports a stable run ID, bounded attempted/indexed/skipped/failed/cancelled counts, and
  can be stopped at a safe unit boundary; complete artifact versions remain searchable and the
  durable checkpoint resumes the remainder.
- The retrieval smoke fixture is the rights-clean synthetic corpus in benchmarks/retrieval/v0/.

Image OCR, embeddings, a semantic index, browser capture, cloud sync, accounts,
multi-device sync, and mobile clients are not part of this supported slice.

## Quick start

Requirements: Rust 1.88 or newer and Node.js 24 or newer.

```text
npm ci
npm run check
cargo test --workspace --locked
```

Index a selected folder and search it from the CLI:

```text
cargo run --locked -p loom-cli -- --database .loom/library.sqlite3 index ./notes
cargo run --locked -p loom-cli -- --database .loom/library.sqlite3 search "retry anomaly"
cargo run --locked -p loom-cli -- --database .loom/library.sqlite3 stats
```

Start the desktop development shell:

```text
npm run tauri -- dev
```

Run the retrieval smoke benchmark:

```text
cargo run --locked -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v0/corpus \
  --queries benchmarks/retrieval/v0/queries.jsonl
```

At the current pre-alpha baseline, the shared checkout passes npm run check and cargo test
--workspace --locked. These are local checks, not a release or compatibility guarantee.

## Five-year execution program

LOOM's public roadmap spans 20 rolling quarters from 2026-08-23 through 2031-08-23. The tracked
[manifest](roadmap/roadmap.json) contains 154 active issues across 13 product phases, with concrete
outcomes, measurable acceptance criteria, labels, quarterly placement, phase parents, and explicit
prerequisites. Four overlapping planning items are retained only as sanitized consolidation
records; no implementation is claimed for them.

Validate the complete graph offline:

```text
make roadmap-check
```

Maintainers can compare the manifest with GitHub without writing by running
`python3 scripts/roadmap.py --repository AlisinaDevelo/LOOM`. The reconciliation tool adopts both
the original LOOM markers and legacy Forge sync markers, but always renders public v2 issue bodies
without private routing metadata.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Data model](docs/DATA_MODEL.md)
- [Schema compatibility](docs/SCHEMA_COMPATIBILITY.md)
- [Privacy](docs/PRIVACY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Evaluation](docs/EVALUATION.md)
- [v0.1 activation gate](docs/ACTIVATION_GATE.md)
- [Participant worksheet](docs/studies/v0.1-participant-worksheet.md)
- [Architecture decision records](docs/adr/README.md)
- [Product direction](docs/PRODUCT.md)
- [Research direction](docs/RESEARCH.md)
- [Roadmap](docs/ROADMAP.md)
- [Machine-readable roadmap](roadmap/roadmap.json)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

The canonical retrieval primitive is SQLite FTS5; its tokenizer, ranking, and query behavior are
documented by [SQLite](https://www.sqlite.org/fts5.html).

## License

LOOM is distributed under the [Mozilla Public License 2.0](LICENSE). See
[CITATION.cff](CITATION.cff) for software citation metadata.
