# Security policy

LOOM is pre-alpha software for local experimentation. The current supported path has no account,
cloud service, or telemetry, but it is not a security boundary. The local SQLite database stores
indexed text, source paths, hashes, and anchors, and is not encrypted at rest.

## Supported scope

Security reports are currently accepted for the latest state of the main branch. There are no
stable release versions or guaranteed support windows yet.

Relevant areas include:

- file and directory selection, traversal, symlink handling, and stable reads;
- text parsing, size/count limits, and SQLite/FTS5 persistence;
- search query handling and evidence rendering;
- Tauri commands and capability configuration;
- dependency or build changes that could expose indexed content.

## Reporting a vulnerability

Please use a private
[GitHub security advisory](https://github.com/AlisinaDevelo/LOOM/security/advisories/new) when
available. Include:

- the affected commit or version;
- operating system and relevant runtime versions;
- a minimal reproduction that does not contain private source content;
- the impact, including whether indexed text, paths, or other local data could be exposed.

Redact source text, personal paths, credentials, and tokens. Do not disclose a suspected
vulnerability in a public issue before maintainers have had a reasonable opportunity to assess it.
If private advisory reporting is unavailable, open a minimal public issue requesting a private
contact route without including sensitive details.

## Current limitations

LOOM does not currently promise encryption at rest, secure deletion, malware scanning, protection
against a compromised host or dependency, protection from another local process with access to the
database, or safe handling of hostile files beyond the implemented parser and resource limits.
Do not index material that you are not prepared to store in this local database.

Security design references include
[Tauri capabilities](https://v2.tauri.app/security/capabilities/) and
[SQLite FTS5](https://www.sqlite.org/fts5.html).

## Local dependency verification

`make security-check` requires gitleaks, npm, Cargo, and cargo-audit. It scans secrets,
checks npm advisories and the locked Cargo manifest, then runs `cargo audit --deny warnings`.
Unsoundness, unmaintained dependencies, and yanked crates fail this check; a default
`cargo audit` exit code of zero alone is not sufficient. The device verifier retains this
failure and continues its independent checks. It does not turn known findings into a pass.

The tracked `.cargo/audit.toml` takes precedence over user audit configuration: it has no
ignores, no target filter, no severity cutoff, and enables fresh official RustSec data,
informational warnings, and yanked checks. The script refuses a missing policy file.

A task-local tool and advisory database can be selected with `LOOM_CARGO_AUDIT_BIN` and
`LOOM_CARGO_AUDIT_DB`. These are trusted operator/test inputs, not a security boundary:
an untrusted executable or modified database could misreport findings. The script itself
adds no exceptions. Install the auditor separately with
`cargo install cargo-audit --version 0.22.2 --locked`, and retain its version/hash,
policy hash, advisory DB revision, and complete report with evidence. No hosted Actions
result is required to run it.

The current GTK dependency path still includes `glib 0.18.5`, affected by
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
The supported Tauri dependency upgrade is tracked in
[issue #278](https://github.com/AlisinaDevelo/LOOM/issues/278); local passing functional tests
do not close that security gate. See the [dated local audit](docs/evidence/rust-audit-2026-10-03.md)
for the precise dependency snapshot and limitations.
