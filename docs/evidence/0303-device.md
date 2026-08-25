# Issue 0303 — merged-main bookmark import evidence

Issue: [#32](https://github.com/AlisinaDevelo/LOOM/issues/32)

Implementation PR: [#225](https://github.com/AlisinaDevelo/LOOM/pull/225)

Evidence PR: this change

Merged-main SHA: `7694d51c0dc5630e8308156ab4c9d1edcdf79540`

Roadmap ID: `0303`
Run date: 2026-08-25 (Europe/Rome)

This record is the closure evidence for the three acceptance criteria. The implementation was
merged before this reproduction; every command below was run against the merged `main` SHA, not a
feature branch. GitHub Actions were queued and were not used as evidence.

## Device and toolchain

| Field | Recorded value |
| --- | --- |
| OS | macOS 26.6.2, build 25G83 |
| Device | MacBookPro17,1, Apple M1, arm64, 8 GiB |
| Rust | rustc 1.96.0, cargo 1.96.0 |
| Node/npm | Node v26.7.0, npm 11.19.0 |
| Python | 3.9.6 |
| Git | merged SHA above; working tree clean after the run |
| Build resource mode | Rust checks used `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0`; generated targets were cleaned between large phases |

## Acceptance mapping

| Criterion | Retained artifact and result |
| --- | --- |
| HTML exports from two documented browsers pass fixtures | [`chrome.html`](../../crates/loom-core/tests/fixtures/bookmarks/chrome.html) and [`firefox.html`](../../crates/loom-core/tests/fixtures/bookmarks/firefox.html) are rights-clean Netscape exports. `chrome_and_firefox_exports_preserve_folder_title_url_and_timestamps` passed and checked nested folders, entity decoding, URLs, and browser timestamps. |
| Repeated import is idempotent and reports merges/conflicts | Core tests cover unchanged re-import, changed-folder duplicate URL conflict, and timestamp-only metadata merge with a distinct immutable version. Merged-main CLI smoke returned `imported: 1` then `unchanged: 1`; `bookmarks --limit 10` returned the retained folder, title, URL, timestamps, entry hash, and import ID. |
| Import never fetches remote content without an explicit action | The parser and import path contain no network client or URL resolver. Reports expose `remote_fetches: 0`; the core and UI tests assert it. The merged-main CLI smoke returned `remote_fetches: 0` on both imports. |

## Test matrix

All results below are from merged-main SHA `7694d51c0dc5630e8308156ab4c9d1edcdf79540`.

- `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo test --locked -p loom-core --lib --tests` —
  36 unit tests and 53 integration tests passed; 0 failed.
- `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo test --locked -p loom-cli` — 9 tests passed;
  0 failed.
- `npm ci --ignore-scripts --no-audit --no-fund` followed by `npm run check` — lint, 22 UI
  tests, TypeScript typecheck, and Vite production build passed.
- `npm run test:browser-extension` — 10 tests passed; 0 failed.
- `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo check --locked -p loom` — passed, including
  the Tauri command and generated permission surface.
- `python3 scripts/roadmap.py --validate-only` — valid; 154 active issues, 4 retired issues,
  20 milestones, 13 phases, 141 parent edges, and 314 dependency edges.
- `python3 -m unittest discover -s tests -v` — 20 tests passed; 0 failed.
- `markdownlint-cli2 '**/*.md' '#node_modules' '#target' '#.forge'` — 0 issues in 0 files.
- CLI smoke on a fresh temporary database and fixture — first import `imported: 1`, second
  identical import `unchanged: 1`, both with `remote_fetches: 0` and no failures.
- Final live audit after closure — 158 issues (144 open, 14 closed), 68 pull requests (4 open,
  64 closed), zero unsafe public bodies, and active `main-protection` ruleset `21332347`.
- `git diff --check` and `cargo fmt --all -- --check` — clean before the implementation merge;
  merged-main working tree remained clean during reproduction.

## Negative, privacy, recovery, and resource coverage

- Missing Netscape marker, missing `HREF`, unclosed tags, oversized URLs, executable
  `javascript:` input, symlink exports, and configured file-size limits fail closed before rows are
  written.
- Changed exports preserve current metadata, report merges or duplicate-URL conflicts, and retain
  an immutable artifact version for timestamp-only changes.
- Schema compatibility includes a populated v6-to-v7 migration that creates bookmark tables and
  preserves existing canonical rows and FTS evidence.
- The import is intentionally metadata-only: it stores the selected export path/hash and bookmark
  fields but does not fetch, snapshot, or upload any URL. The UI announces this constraint.
- The final merged-main Tauri check was run after cleaning the exact generated LOOM target with one
  build job. An earlier parallel rebuild at 708 MiB free hit `ENOSPC`; it was not counted as a
  functional failure, and the successful constrained rerun is the retained result.

## Log digests

Logs remain in `/tmp` on the test device; SHA-256 digests make the retained run identifiable without
committing generated output to the repository.

| Log | SHA-256 |
| --- | --- |
| `/tmp/loom-0303-merged-core.log` | `0fc5df9d895e87e7133b37412b04880d3c9785192669266145b783b7ef273f70` |
| `/tmp/loom-0303-merged-cli.log` | `fe4e56e2a01f08e78f1ad93884ad9d83da8c8bdcd5fde39f352561bb20edac2d` |
| `/tmp/loom-0303-merged-frontend.log` | `e36b1cf59e509beef05aa057663a6e9a7ae9d96a00478a4e539791d9c74528f3` |
| `/tmp/loom-0303-merged-browser.log` | `8de1e459fa32bc6f67a99a1c8e5c031e853686f006f997a38c864af4a61abd0b` |
| `/tmp/loom-0303-merged-tauri.log` | `f9658f21641c1775bdb7a00682d1543265d60e48aedcb49c299e8d2695a85253` |
| `/tmp/loom-0303-merged-cli-smoke.log` | `5df8de2eb263b433030f6cfa0f5df7050d922c1fcc2d26556ca76e08341fcc20` |
| `/tmp/loom-0303-merged-roadmap.log` | `df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96` |
| `/tmp/loom-0303-merged-python.log` | `3abd2a402775222934c0517eec420f5456c385bce37f3211d74c7358ae5eb1bf` |
| `/tmp/loom-0303-merged-markdown.log` | `10c1246d61198634e4ff20fc3d1d4d5d0a7af4fc6b13e3a3db7a3e71c008e8fd` |
| `/tmp/loom-0303-roadmap-apply-final.log` | `dd9be20c85ddbba7616962d6eac2af119029b4bdc8f5e3d420af80aa011693b5` |
| `/tmp/loom-0303-live-audit.log` | `16ee5fe1476da2becd32174997eca119d6c2106ea76d36d39ce88380b5b573c3` |
| `/tmp/loom-0303-rulesets-final.json` | `09fc243d7c638969882e3c03f336844c623d61a710c90aff18159e42896c97ca` |

No screenshot artifact was needed for this parser/CLI change, so no full-screen screenshot was
captured. The UI behavior is covered by the focused React test; if visual evidence is added later,
it must be cropped to the relevant import panel or fixture rather than capturing the desktop.

## Closure decision

All three acceptance criteria map to retained fixtures, test names, CLI output, and this merged-main
record. The roadmap entry is promoted to `done`; the live issue may be closed only after this
evidence commit is merged and the final roadmap reconciliation reports zero delta.
