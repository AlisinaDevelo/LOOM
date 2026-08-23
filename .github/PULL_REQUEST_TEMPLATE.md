# Pull request

## What

Describe the user-visible or engineering outcome.

## Why

Link the issue or evidence motivating the change.

## How

Summarize the implementation and important trade-offs.

## Verification

- [ ] `make check`
- [ ] Manual retrieval/source-open smoke, when behavior changes
- [ ] New or updated benchmark fixture, when ranking/extraction changes
- [ ] Documentation updated

## Data and privacy

- [ ] No new source content, paths, queries, OCR text, vectors, or secrets enter logs/telemetry
- [ ] Schema/index compatibility and rebuild behavior considered
- [ ] Retention/deletion behavior considered
- [ ] New Tauri, filesystem, parser, browser, or network permission is explained

## Scope

- [ ] The change preserves LOOM's intentional-capture and evidence-first boundaries
- [ ] Generated build/index/cache artifacts are not committed
