# Durable OCR policy: local verification

## Scope and environment

This is a prerequisite for roadmap `0400` / GitHub #36, not completion of the background
job engine. It prevents an old image extraction from inheriting a later OCR enable or
recreating purged derived evidence. It also removes unsafe post-commit capture cleanup.

Verified on 2026-10-03, MacBookPro17,1, Apple M1 arm64, 8 GiB RAM, macOS 26.6.2
build 25G83. Toolchains: Rust 1.96.0 and MSRV 1.88.0, Node 26.7.0, npm 11.19.0,
Python 3.9.6. Rust checks used one build job, no incremental compilation, and no debug info.

The tested code head is `bc96b12ccd5d0037912c50d07fece84d7f8d5e76`, based on merged
main `4ee54e6ae0c678fad6bbf9467f5b92f7dce27eda`. Core changes are in `94c4fd6`, capture
cleanup in `3cd89a8`, Markdown tooling in `1cc5038`, and purge copy in `bc96b12`.
The Rust tree was unchanged by the latter two commits; the final UI/build checks include them.
Raw logs, including failed runs, are retained locally in `/tmp/loom-jobs-evidence.INpfja`.

## Contract and regression evidence

- OCR policy is a strict SQLite snapshot: enabled boolean plus opaque operational revision.
  Open connections observe current policy rather than their former per-connection cache.
- Enable, disable, explicit purge, and successful portable restore rotate the revision in
  the same transaction. Failed restore rolls back both policy and canonical changes.
- Image-containing requests carry the captured revision through provider input, canonical
  writes, missing-source cleanup, checkpoints, and terminal diagnostics. Stale writes fail
  without changing source bytes, canonical tables, FTS state, or prior checkpoints.
- Prepared OCR must carry an enabled policy. Disable/re-enable, purge while enabled, and
  reasserting enable cannot authorize an old result. Fresh authorized work remains possible.
- Scan fingerprints include OCR policy; an interrupted image scan restarts after a change.
  Invalid policy fails closed for images without blocking a pure text/PDF request.
- Prior schema-10 databases receive the operational revision additively. Portable hashes,
  source/extractor identities, and canonical schema version remain unchanged.
- Disabled OCR rejects intentional capture before the native picker or storage creation.
  Once pixels are committed, indexing failure retains them and reports their path. It does
  not purge rows or unlink bytes that a newer attempt may own. Explicit **Purge captures**
  still removes committed PNGs; the UI distinguishes file removal from indexed-record count.

Against pre-fix main, the initial policy suite produced five failed assertions and the
native cross-connection disable test failed. Both logs remain available. Expanded tests
use a deterministic prepared-provider fixture to control the commit boundary; separate
macOS integration tests exercise Apple Vision and pixel-anchor metadata. This is not an
OCR quality benchmark or a human Screen Recording permission session.

Two intermediate assertions incorrectly expected a new diagnostic job ID and new canonical
rows for unchanged OCR; these were corrected to test restart position, revision fencing,
and stable unchanged identity. Failed logs were retained, not presented as passing runs.

## Local pipeline

| Check | Actual result |
| --- | --- |
| Formatting, full diff check, Clippy all workspace targets | PASS; warnings denied |
| `cargo test --workspace --locked` | 218 passed: 163 core, 11 CLI, 28 browser protocol, 16 desktop |
| Rust 1.88.0 workspace/all-target check | PASS |
| Rust 1.88.0 full core/CLI tests | 174 passed |
| Fresh npm install and `npm run check` | PASS; 39 UI, 12 extension, 6 Markdown-tooling tests |
| Python roadmap tests / CI / protocol / accessibility contracts | 24 / 8 / 10 / 9 passed |
| CLI v0 retrieval fixture | 3/3 exact originals recovered; anchor precision 1.0 |
| CLI v1 diagnostic fixture | Gate passes; 9/10 positives; q008 miss and q009 false positive retained |
| PDF adversarial fixture | 4 indexed, 3 expected unsupported; no unexpected failures across 7 fixtures |
| Real native-messaging host | Accepted request, 4 deterministic refusals, 3 unpaired refusals, recovery PASS |
| `tauri build --debug --no-bundle` | PASS; local development build, not signed/notarized distribution |
| Offline roadmap graph | 154 active, 4 retired, 20 quarters, 141 parents, 314 prerequisites valid |
| Local security script | PASS: secret scan, npm audit, locked Cargo metadata |

CLI and desktop currently share the binary name `loom`. A combined build emitted a filename
collision warning; they were rebuilt separately, and the CLI was staged before the desktop
build. Retrieval/PDF tests used that staged CLI, not the subsequently overwritten desktop path.
No GitHub Actions result was needed for these checks or the normal PR merge.

## Markdown dependency remediation

The final security gate initially reported five high npm findings in the dev-only
`markdownlint-cli2 -> globby -> fast-glob/micromatch -> braces` chain.
[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) describes
stack exhaustion from deeply nested brace patterns and lists no patched version at verification.

The wrapper was removed, retaining its exact Markdown engine `0.41.1` and equivalent rule
configuration. The runner uses Git's NUL-delimited literal tracked/non-ignored file inventory,
never glob interpretation. Six tests cover literal filenames, exclusions/deletions, actual rule
failures and diagnostics, rule parity, symlink refusal, and removal of the vulnerable chain.
Fresh installation and full/development-excluded npm audits reported zero vulnerabilities.
An intermediate Vitest discovery/config overlap and two runner/fixture mistakes were fixed
before the final passing pipeline. This does not claim a Rust advisory audit or resolve #278.

## Selected log hashes

| Log | SHA-256 |
| --- | --- |
| `ocr-policy-before.log` | `cddc4cccb25ad36847f9b41c7e6aeb4f11c66a11eb99ff5a9839d1c3cab62af0` |
| `ocr-policy-native-before.log` | `e7c001fb412b644101d42a99ca008d28e5b9cd1fed94cf29ba5b72292b24cd24` |
| `ocr-remediated-workspace.log` | `d34193501f0b4cd70ab81333cfabeadc00a37c53fc1ba030f4f9f6a88332009d` |
| `ocr-remediated-msrv-tests.log` | `7f8553cce319d498c5b8c1dfc2a778621ca5693bec9c971a44c9de6d92e2c227` |
| `ocr-remediated-clippy.log` | `9cc569e2e1bd190b547a5fa226ba57988f2237d70488be49f9b0f4de768ff08e` |
| `ocr-ui-final.log` | `e17eebafcea8435d9b1b7359a82a9814bd3a01a07ec016ce69a36b4958e3b6c3` |
| `ocr-packaged-final.log` | `59df40915a1f3c0ee82fea16c04c6338a46b289ac0878b2df7e6655e5b35fdf6` |
| `ocr-final-v0.log` | `9ed0f2657a7b423850b397ddd42c54743fc3af4ccf1cf7f57088034bc265d5b1` |
| `ocr-final-v1.log` | `3e73ee4917d59cb208bc313598e77d28c90f75398347d4128676fbeb49feeba4` |
| `ocr-final-pdf.log` | `d96056dc309a6114c4b99d3bc3f9e51ce452f664445d36a1b47dc82eeea874cf` |
| `ocr-final-native-host.log` | `babf157dc05792fdd7a07e6332690413dfceacc492c315157fa903dab97e59e5` |
| `ocr-toolchain-verified-security.log` | `2cba7df0dd117374e94e7f749329b9aaa208d0efc008a6597ece61e9824426fc` |

## Review and remaining gates

Read-only review of immutable `3cd89a8` against `4ee54e6` found no blocker after capture
cleanup was removed. It was static review, not a second independent device run. The later
lint and purge-copy changes were separately inspected and locally tested.

This is not the durable queue/worker epoch, fairness, resource-budget, cancellation, or
crash-recovery implementation required by #36. It adds no ambient capture, new source scope,
cloud transport, human study, external security audit, signing credentials, or automatic purge.
The issue remains open. After merge, record exact tree identity and reproduce on clean main.
