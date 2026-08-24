# Contributing to LOOM

LOOM is pre-alpha. Contributions should keep the current local, evidence-first boundary clear
and should not imply support for features that are not implemented.

## Before opening a change

1. Read the relevant code and the documentation in docs/.
2. For a behavior change, open or reference an issue describing the user-visible result and
   evidence required to verify it.
3. Keep changes focused. Do not include generated output such as target/, dist/,
   node_modules/, coverage, logs, or local databases.

## Local checks

~~~text
npm ci
npm run check
cargo test --workspace --locked
~~~

For retrieval changes, also run the rights-clean smoke fixture:

~~~text
cargo run --locked -p loom-cli -- benchmark \
  --corpus benchmarks/retrieval/v0/corpus \
  --queries benchmarks/retrieval/v0/queries.jsonl
~~~

Report only checks that actually ran. A benchmark on the synthetic fixture is not evidence of
real-world retrieval quality.

## Evidence and privacy requirements

- Keep results source-backed. New result fields should preserve the source locator, content
  hash, excerpt, and exact anchor where applicable.
- Do not add private documents, personal paths, secrets, or proprietary fixtures to the
  repository.
- New fixtures must have documented rights and provenance. The retrieval fixture is synthetic
  and CC0-licensed.
- Explain changes to indexing limits, parsing, persistence, or source opening in the relevant
  design, privacy, threat-model, or ADR document.
- Do not add network calls, telemetry, accounts, or external model providers to the current
  local-only path without a documented design decision and review.

## Commits and pull requests

Use short Conventional Commit subjects such as:

- feat(core): add anchor validation
- fix(cli): report unreadable files
- docs: clarify local storage

A pull request should state:

- what changed and why;
- which files or interfaces are affected;
- how it was tested;
- any known limitation or migration concern;
- whether documentation or an ADR was updated.

Keep public text factual and concise. Do not include assistant, model, bot, or generated-by
metadata in commits or documentation.

## Completion evidence contract

An issue is not complete because CI is green. Before closure, the pull request and issue must
link every acceptance criterion to a retained evidence or artifact ID, record the exact commit,
device, OS, architecture, and toolchain, and include the relevant happy-path, negative/privacy,
failure, recovery, and resource checks performed on the target device. The change must be pushed,
reviewed, and merged to protected `main`; the same reproduction must then be rerun against the
merged SHA. Record logs, measurements, artifact digests, limitations, and the merged SHA. If
required hardware or a platform surface is unavailable, record an explicit blocked/no-go result;
do not substitute a CI result.

## License

By contributing, you agree that your contribution is provided under the
[Mozilla Public License 2.0](LICENSE), unless a different written arrangement applies.
Please preserve existing copyright and license notices.
