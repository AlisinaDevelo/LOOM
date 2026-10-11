# Retention restore and evidence-state fixes

Issues: [#345](https://github.com/AlisinaDevelo/LOOM/issues/345),
[#346](https://github.com/AlisinaDevelo/LOOM/issues/346),
[#347](https://github.com/AlisinaDevelo/LOOM/issues/347).

Code revision: `00f68c21aee6fa5ad155f6d21c0ca1f9fe4ae0ea`, based on merged dependency
update `e102fd4c1074230da99d0a6e2548bafeaf5eaf0f`. Documentation-only receipt changes
do not alter the tested implementation. The issue closure receipts must additionally
record the protected-main merge and repeat these checks on that exact merged revision.

## Device and scope

2026-10-11, Mac17,3, 32 GiB, macOS 27.0.1 (26A434), arm64; rustc 1.99.0
(`b940084d7`), cargo 1.99.0 (`5f94df478`), Node 26.11.0, npm 11.20.0,
Chrome 155.0.8059.40. Fixtures contain synthetic text and the existing synthetic,
CC0 browser/OCR corpus. No user library, documents or capture permissions were used.

## Acceptance map

| Artifact ID | Criterion | Observed evidence |
| --- | --- | --- |
| LOOM-345-ROUNDTRIP | Disabled retention survives export and encrypted restore | The disabled-retention regressions reproduce the old `invalid setting retention_days=` refusal. At `00f68c21`, a clean target belonging only to this checkout passed all 13 portability tests, including both disabled-retention cases, preserving settings and every canonical row after restore/reopen. Earlier shared-target attempts cannot establish the identity of the executed implementation; see the boundary below. |
| LOOM-345-INVALID | Invalid retention values fail atomically | The hostile-export fixture rejects `0`, `36501`, `-1`, nonnumeric and whitespace values; existing corruption, wrong-password and tamper coverage remains. |
| LOOM-345-CLI | Actual CLI recovery | A temporary synthetic library enabled then disabled retention, exported/imported, backed up/restored with the default KDF, and compared stats plus exact result identity/hash/anchor after reopening. Wrong-password restore failed without creating a database. The retained JSON receipt is `2026-10-11-retention-cli.json`. |
| LOOM-346-POLICY | Persisted policy, denial and retry | Sixteen added UI regressions cover disabled startup, unknown/failed reads, retry, refresh races, denied writes and failed startup reconciliation. Successful writes followed by failed reads show unavailable rather than the old policy. |
| LOOM-346-PRIVACY | Removed evidence and late responses | Revoke/root purge/capture purge/OCR disable/OCR purge clear cached results and viewer/history/relationship state. Request generations fence delayed responses. A destructive attempt clears caches conservatively because a backend error may follow a partial commit; fresh search remains available. |
| LOOM-346-RENDER | Rendered user flow and layout | Chrome exercised persisted disabled OCR → synthetic search → evidence → revoke, and confirmed the excerpt and viewer disappeared. Desktop 1280×900 and narrow 390×844 views had no error overlay, console/page error or horizontal overflow. Tauri IPC was synthetic; this does not claim a native-window or permission session. |
| LOOM-347-UTF8 | Byte caps and exact retained data | Three added tests cover 2/3/4-byte characters, complete UTF-8 prefixes, snapshot/title/selection limits, raw collection limits, byte reconstruction, SHA-256 and independent HMAC. All 15 extension tests passed. |
| LOOM-347-PARTIAL | Sanitizer cannot restore completeness | A raw prefix with a removable leading script remains partial even when sanitized bytes fit the cap. Complete small, offline, denied, redirect, SPA, manifest and visible-action fixtures still pass. |
| LOOM-347-BROWSER | Serialized collection in a real browser | The self-contained collector executed against a synthetic multibyte DOM in Chrome: truncated=true, 8,388,607 retained bytes, no replacement characters. This is collector execution, not a toolbar, pairing or native-host permission session. |

Retained rendered receipt: `2026-10-11-rendered-ui.json`. Synthetic screenshots:
[before revocation](images/2026-10-11-before-revoke.png) and
[after revocation](images/2026-10-11-after-revoke.png).

## Verification

- `npm run check`: lint, 59 Vitest UI/evidence/accessibility tests, 15 extension
  tests, 6 markdown-tooling tests, typecheck and Vite 8.3.2 build passed.
- `npm audit --audit-level=high` passed; three preexisting low KaTeX-chain findings
  remain. No exception or forced downgrade was applied.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed.
- `cargo fmt --all --check`, `git diff --check`, roadmap validation, 9 accessibility
  contract tests, 14 CI contract tests and 10 browser protocol tests passed.
- Independent source/lock reviews found no remaining actionable finding after
  correcting the desktop post-commit error and OCR-refresh failure paths.

The retention regression command is:

```sh
cargo test --locked -p loom-core --test portable_backup
```

### Rust artifact identity

The first full `--no-fail-fast` attempt and a later focused portability attempt used a
Cargo target shared with another checkout. Their retained results are observations of
those attempts, not proof that every executable was built from `00f68c21`. The full
attempt ran only 11 portability tests, omitting the two new regressions; the focused
attempt ran 13 but returned the old disabled-retention refusal despite the fixed source.
Concurrent builds in the shared target prevent attributing those binaries to the current
implementation. A successful unrelated target does not resolve that ambiguity.

The target was then separated: `/tmp/loom-progress/target` was built from this checkout
without another checkout writing to it. The portability build completed in 42.13 seconds,
and all 13 tests passed in 2.19 seconds. The retained isolated log is
`loom-retention-isolated.log`. It is the authoritative Rust regression result for
these retention fixes. The isolated CLI and extractor were then rebuilt from the
same code, and the
actual export/import/backup/restore/wrong-password fixture passed again. Its JSON
receipt is identical to the retained CLI receipt above.
No process was terminated when the completed shared-target attempt was retired.

## Native OCR no-go and remaining gates

The untouched-main workspace test at `7d396ec` failed
`jobs::tests::persistent_retrieval_during_native_queued_ocr`: 176 core tests passed,
one failed, and all 24 OCR jobs reported the redacted provider refusal. An isolated
unchanged rerun failed the same test in 9.05 seconds. The later shared-target full
attempt observed the 120-second OCR cohort deadline and a separate
`versioned_v2_upgrade_is_explicit_preserves_records_and_can_be_repeated` failure with
`JobWorkerBusy`; its core-library result was 175 passed and two failed. Other targets
in that attempt reported success, subject to the artifact-identity limitation above.
Neither this attempt nor the scoped isolated portability pass is a green workspace
result. The checkpoint follow-up remains separate from these three fixes. Baseline
refusal and slow native calls are separate observations, not proof of one cause.

Two bounded direct provider probes on the same fixture exceeded 35 seconds. An
owned-process sample located Vision revision 3 waiting while an Espresso/ANE
compilation worker was active. Probes overlapped another native cohort; no
permanent platform-failure conclusion is supported. Diagnostic manifest SHA-256:
`edd8ae0fa83a2d9fc81bdc865ea0b4e5ee83e507e7f5851da4b44d9d8ba050d8`.
This evidence is recorded on [#338](https://github.com/AlisinaDevelo/LOOM/issues/338).
No OCR provider, timeout or quality gate was changed by these fixes.

Native window/permission sessions, participant usability, independent crypto
review, browser pairing/packaging and the full durable-engine resource gate remain
outside these scoped issues. External CLI retention changes have no new desktop
subscription. Broader #27/#28/#30/#34/#36/#338 remain open.

## Log digests

The compact receipts above retain outcomes; original target-device logs are kept
with the local verification evidence. SHA-256:

| Log | SHA-256 |
| --- | --- |
| Final npm check | `071192ae121c81b585c2ca1770966e732530268946430528119ec800e3315474` |
| Retention CLI | `54fd56f40d4d6356cc56c142e91c52704d8e60c775e9b06b34b1223b32b04c82` |
| Rendered UI | `17aab04de72e2b75c6ec5827ced1c746cac84c04ddefea8c762c042123abe0f5` |
| Synthetic before-revocation screenshot | `807ba0dabbaf2bf585e2c828c56fa0f9c578cbdd3a97f5e40b6dc8890fbe9307` |
| Synthetic after-revocation screenshot | `d0243a515ff2618b0c4d13c6b6bf81adc3a4f04c0818849a7d0efbb67e3ffc4b` |
| Browser regression failures | `a0b221a71d01b85a14400e94b34a6a716e2e725a090249ea4c497fc3d5028677` |
| Browser regression pass | `e50513419eeeb7e12aad6f7eff850efc00f6a2e89cd150ef609c6bc945411484` |
| UI regression failures | `cbbe5c95834243e5b5df80cb123e01d1f2e10194887e23060946117433f4e4f1` |
| Final UI test suite | `c71a57d591e41424be338e842ee285961c46c8a07113c2fe78ac7bfde1daf053` |
| Workspace Clippy | `9be59dfd961346f338adab764615dc5c4ab1de7d6a169dbe8f0eae125ca79f62` |
| Untouched-main Rust failure | `5b70e492f928ece1e3150fc3d0a38ce2554e6c59336b37acfa747df694ceb5d7` |
| Isolated native OCR failure | `8986b54ec9f89e402bdd54a9bfd40b57d76fd488fe87e312a3253d9688295da3` |
| Shared-target full Rust attempt (identity unverified) | `1e4c8d39a9799d4eb5d20449f0a85d51d54f2bbc51edea98be26f51c37f45456` |
| Shared-target portability refusal (identity unverified) | `afae02dd7594d53a4027a6911bab187198eba303ef34e38a0a8baa37c4271fcf` |
| Clean isolated portability target | `819e284bd358b64c43c9d0b26804a4ade2138fb25f7a8cbfa4b0e78ca1f594e7` |
