# Local Rust advisory enforcement and crypto compatibility

## Tested snapshot

Code head: `f6fec1afcf264e6bebc9d92b285b8f7f1fd5d63f`, based on merged main
`39b8d2f7ed4bed297b48b723eecd195254fc2d62`. Verified on 2026-10-03 on
MacBookPro17,1, Apple M1 arm64 / 8 GiB, macOS 26.6.2 build 25G83;
Rust 1.96.0 and MSRV 1.88.0, Node 26.7.0, npm 11.19.0, Python 3.9.6.
Rust used one build job, no incremental compilation, and no debug info.

This updates only `chacha20 0.10.1` to `0.10.2` in Cargo.lock and makes the local
security script enforce real Rust advisory checks. It does not resolve the upstream
GTK/glib dependency gate [#278](https://github.com/AlisinaDevelo/LOOM/issues/278),
complete the job engine [#36](https://github.com/AlisinaDevelo/LOOM/issues/36), or
claim a signed release, independent security assessment, or hosted Actions result.

## Actual audit, including the failing gate

The task-local auditor was built from `cargo-audit 0.22.2` with its locked dependencies.
The fresh official RustSec database contained 1,288 advisories at revision
`f8dee89e1b2f2f1eaf548312df7655fe5202a302`, updated 2026-10-02 at 22:27:46 +02:00.
The whole lockfile, including target-specific dependencies, contained 540 packages.
No architecture/OS filter or advisory ignore was applied.

Before this change, an ordinary audit returned zero despite seven unmaintained warnings,
one unsoundness warning, and the yanked `chacha20 0.10.1`. The strict audit returned one
with nine denied warnings. After the compatible patch, no yanked package was reported;
the remaining seven unmaintained warnings and one glib unsoundness warning still cause
the real local security script to return one. This is a FAIL, not an all-green pipeline.
Gitleaks reported no leaks, npm reported zero vulnerabilities, and locked metadata passed.

The remaining warnings are:

| Package | Advisory | Classification |
| --- | --- | --- |
| `glib 0.18.5` | [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) | Unsound iterator implementation |
| `proc-macro-error 1.0.4` | [RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370.html) | Unmaintained |
| `ttf-parser 0.25.1` | [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192.html) | Unmaintained |
| `unic-char-property 0.9.0` | [RUSTSEC-2025-0081](https://rustsec.org/advisories/RUSTSEC-2025-0081.html) | Unmaintained |
| `unic-char-range 0.9.0` | [RUSTSEC-2025-0075](https://rustsec.org/advisories/RUSTSEC-2025-0075.html) | Unmaintained |
| `unic-common 0.9.0` | [RUSTSEC-2025-0080](https://rustsec.org/advisories/RUSTSEC-2025-0080.html) | Unmaintained |
| `unic-ucd-version 0.9.0` | [RUSTSEC-2025-0098](https://rustsec.org/advisories/RUSTSEC-2025-0098.html) | Unmaintained |

The JSON vulnerability list itself is empty: the glib finding is classified as an
informational unsoundness warning. That distinction does not make it safe to ignore.
The current `tauri 2.11.6` GTK path still resolves `glib 0.18.5`; this finding was
confirmed against the live open Dependabot alert. No unsupported override was used.
The Apple Silicon run does not reproduce or certify any affected x86 SIMD backend.

The tracked `.cargo/audit.toml` selects all informational categories, no ignores,
no severity cutoff, no target filters, fresh official data, and yanked checks.
The script refuses a missing policy rather than falling back to a user's file.
Project configuration precedence was checked in the installed auditor's source and
[official example](https://github.com/rustsec/rustsec/blob/cargo-audit/v0.22.2/cargo-audit/audit.toml.example).
The optional binary/database paths are trusted operator inputs, not proof by themselves.

| Input | SHA-256 |
| --- | --- |
| Auditor binary | `ba4743742a5b2509f4adb541522a1890d874f2c1c7ba6982589d0aacdd98daeb` |
| `.cargo/audit.toml` | `d29e42b9762bfb44ef2772fe72052079bce097993c9e11e45ffc8d369411df74` |
| Updated Cargo.lock | `b60a2f4af5f64cfec3a235372e4c9e3d02418ca2c0e91930df42625b282506bc` |
| Previous main CLI | `e932790fc1cfc7289a38272319ac01c5e6b5ff78d4f134c4513aa17adb135fde` |
| Updated CLI | `4965b2bb84cc9738638644cbb372b06b2e01756f49f6a9e40663037a124cb669` |

## Functional verification

- 240 full workspace tests passed: 184 core, 12 CLI, 28 browser protocol, 16 desktop.
- 196 core/CLI tests and full workspace/all-target check passed on Rust 1.88.0.
- Workspace/all-target Clippy with warnings denied, rustfmt, ShellCheck, and diff hygiene passed.
- npm lint/typecheck/build and 39 UI / 12 extension / 6 tooling tests passed.
- Python suite: 33 passed, including nine isolated security-script contracts. These stubs
  verify invocation, strict flags, quoted paths, missing tools/policy, nonzero findings,
  operational errors, and preceding-check failure propagation. They are not real audits.
- CI / browser protocol / accessibility contracts: 8 / 10 / 9 passed.
- Real queue CLI corruption/ownership/cancellation/forgetting fixture passed.
- v0 retrieval fixture recovered 3/3 exact sources. The PDF adversarial fixture had seven
  expected outcomes: four indexed and three expected unsupported, with no unexpected failure.
- Real native host accepted one capture, refused four deterministic and three unpaired cases,
  and recovered safely. Tauri `--debug --no-bundle` build passed.
- Offline roadmap validation passed; no public roadmap content was changed.

The real cross-build test used the retained CLI from merged main and a CLI rebuilt with
the updated lock. Old encrypted backups restored in the new binary, and new backups
restored in the old binary. Artifact/version/passage IDs, BLAKE3 hash, source locator, and
exact anchor were unchanged. A disabled source fixture stayed disabled with its consent
generation preserved in both directions; tampered ciphertext failed without creating a
database. The public revocation API is covered separately by the core tests. The harness
refuses identical binaries/hashes, preventing a same-build run from claiming compatibility.
It also refuses Python optimization so its assertions cannot be stripped; both refusal
cases were executed and returned two before running a fixture.

Two initial harness failures were corrected and retained: restore preserves exported
enabled state rather than disabling all roots, and macOS temporary locators need canonical
resolution. These are test-assumption errors, not newly fixed product vulnerabilities.

The original compatibility command (historical scratch binaries are no longer available):

```text
python3 scripts/test-backup-compatibility.py \
  --before /tmp/loom-jobs-evidence.INpfja/loom-jobs-merged \
  --after /tmp/loom-jobs-evidence.INpfja/loom-audit-cli
```

Raw reports/logs/binaries were recorded under `/tmp/loom-jobs-evidence.INpfja`.
That scratch directory was observed missing during later approved-file verification on
2026-10-03. The digests below record historical runs, not currently accessible raw reports.
The auditor and database were recorded under `/tmp/loom-audit-tools.cJAejt`; that scratch
directory was also observed missing. The original unused 618 MiB compile cache was moved
to task-owned Trash, then cleaned during the later approved-file verification. No source,
user library, evidence, or shared Cargo cache was deleted by these narrow build-cache cleanups.

## Selected log hashes

| Log | SHA-256 |
| --- | --- |
| `audit-workspace.log` | `1c61fcd1715c34bdf6ddba02c7330c1dbc780994666ac45132c3e194a70a5f6f` |
| `audit-msrv-tests.log` | `2b26e9e367b5abccf7b998c2b5c80fa61288a20bdafb5a646ee64a5c44e5c8cc` |
| `audit-hardened-contract-tests.log` | `3245817c6567a0f954257222872ddcbaecf0b96c2b84aacf74cbfa3950e0d0df` |
| `audit-backup-cross-build.log` | `b460d45164c9c4339d256e3c9c6268e0e1c10b5377d827c6440119e0cd6404e7` |
| `audit-frontend.log` | `e0235c0a3ee329c2e197af2e6ee9245a4f06ab4bee9d3bdd4a59e41f27e1af6a` |
| `audit-policy-security.log` | `ae4f10c467413f157c38d757b039c5b64124f0ff268d5371a2f8b76089709f50` |
| `audit-native-host.log` | `babf157dc05792fdd7a07e6332690413dfceacc492c315157fa903dab97e59e5` |
| `audit-tauri-build.log` | `d3a3dd801298f023b33d6c0e55d7ca770e091e36682d9be0601b033e840ab9b4` |
