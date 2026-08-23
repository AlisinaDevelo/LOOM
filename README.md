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
- Supported inputs are UTF-8 .txt, .md, and .markdown files. Files larger than 8 MiB and traversals
  larger than 20,000 files are rejected or reported.
- SQLite is the canonical store. SQLite FTS5 provides lexical retrieval over indexed passages.
- Results include the original path, BLAKE3 content hash, structured excerpt, and exact
  character/line anchor. Literal source characters cannot become highlight instructions.
- A complete rescan hides deleted or unreadable sources. Opening a result verifies its artifact,
  version, and current BLAKE3 hash before handing the path to another application.
- The CLI and Tauri desktop UI use the same Rust core and local database.
- The retrieval smoke fixture is the rights-clean synthetic corpus in benchmarks/retrieval/v0/.

PDF/OCR extraction, embeddings, a semantic index, browser capture, cloud sync, accounts,
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

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Data model](docs/DATA_MODEL.md)
- [Privacy](docs/PRIVACY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Evaluation](docs/EVALUATION.md)
- [Architecture decision records](docs/adr/README.md)
- [Product direction](docs/PRODUCT.md)
- [Research direction](docs/RESEARCH.md)
- [Roadmap](docs/ROADMAP.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

The canonical retrieval primitive is SQLite FTS5; its tokenizer, ranking, and query behavior are
documented by [SQLite](https://www.sqlite.org/fts5.html).

## License

LOOM is distributed under the [Mozilla Public License 2.0](LICENSE). See
[CITATION.cff](CITATION.cff) for software citation metadata.
