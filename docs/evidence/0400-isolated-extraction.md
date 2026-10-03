# Supervised queued extraction: local verification

## Scope and provenance

This is another prerequisite slice of [#36](https://github.com/AlisinaDevelo/LOOM/issues/36),
not completion of background indexing or approval of a desktop release. Queued approved-file
text/PDF/OCR extraction now runs in a one-shot helper; the parent alone retains source
authorization, original bytes/hash, canonical identity, claim/epoch and atomic publication.
See [the process contract](../EXTRACTION_PROCESS.md) for installation and exact limits.

The full target-device runner tested Rust, CLI, workflow and fixtures at
`7709cc759264c68db8d2d53234fbf84f2bba5d28`, based on main
`9c045bbc26bcaabe265c8ed2ef7bd8ab742f5a0f`. Follow-up commits through
`313af1021a68738359a4af4ca27581c60e3d814c` only changed the performance verifier and
its tests; a direct diff confirmed the tested Rust/CLI/workflow/queue fixtures unchanged.
The corrected verifier was separately tested and exercised with fresh default-size runs.
Post-merge receipts belong in the associated pull request; these are pre-merge measurements.

Date: 2026-10-03. Device: MacBookPro17,1, Apple M1 arm64, 8 GiB RAM,
macOS 26.6.2 build 25G83. Toolchains: Rust 1.96.0/1.88.0, Node 26.7.0,
npm 11.19.0 and Python 3.9.6. Rust used one build job, no incremental compilation
and no debug information. Evidence ID **isolated-extraction-LszLJI** retains raw
failed/passed logs, reports and both staged executables locally, outside temporary scratch.
Raw local paths and logs are not published.

## Actual checks

`bash scripts/verify-device.sh EVIDENCE_DIRECTORY` completed every step. Its exit was
**1**, solely from the existing strict Rust advisory gate; this is not a globally green
pipeline. No hosted Actions result or signing/notarization result is claimed.

- Full workspace: **295 passed** — 207 core, 32 extraction, 12 CLI,
  28 browser protocol and 16 desktop.
- Full-workspace/all-target Clippy with warnings denied, format and diff checks: PASS.
- Full-workspace/all-target Rust 1.88 check and **239** MSRV core/extraction tests: PASS.
  A separate MSRV CLI run passed **12** tests: **251** tested in those MSRV packages.
- Actual staged CLI: FTS ownership/repair/cancellation/recovery, approved-file text/PDF,
  native Vision OCR, duplicate/conflict, scope refusal and erasure: PASS.
- Retained real v2 and v3 executables: locked/live-owner refusal, explicit/repeated v4
  upgrade, row/policy/fairness preservation, old-worker refusal and canonical reads: PASS.
- Python: 33 repository, 10 browser, nine accessibility and nine CI contracts: PASS.
  The corrected performance verifier's **nine** tests passed separately.
- npm lint/typecheck/build, **39** UI, **12** extension and **six** Markdown-tooling tests: PASS.
- v0 recovery 3/3, all seven adversarial PDF outcomes, semantic rebuild/drop/rebuild,
  hybrid ablation and mixed-corpus recovery/symlink exclusion: PASS.
- Native messaging: accepted capture, deterministic malformed/unauthorized refusals and
  recovery: PASS. No actual browser profile or private document was used.
- Tauri debug `--no-bundle` build: PASS, not a distributed/signed application test.
- Five-year graph: 154 managed active/four retired, 20 quarters, 141 parents and
  314 prerequisites. Fresh read-only live reconciliation had zero mutations and no warnings.

The independent read-only review inspected the boundary and incremental verifier fixes;
it did not execute tests. Directory/semantic scheduling, desktop controllers/bundling,
manual permission/VoiceOver receipts and Linux process execution remain unproved here.

## Failure, privacy and recovery coverage

Actual subprocess fixtures exercise allocation above the memory policy, kernel CPU exhaustion,
hang/deadline, early crash, malformed/oversize output, forged metrics/wrong media, interrupted
and unwinding probes, and parent death before metadata even while stdin remains open.
Owned-child PID entry and kill/reap assertions prevent a mock or prelaunch refusal from being
mistaken for fault execution. Bootstrap/configuration preserve stricter inherited limits.

Live helper tests revoke/cancel scopes, rotate/purge OCR policy, delete canonical identities,
restore/replace databases and alter target/CAS while provider work runs. Invalid geometry,
metadata, text controls and resource/provider failures cannot publish canonical evidence.
Launch/crash retry budgets exhaust; provider unavailability is failed, not consent cancellation.
Original bytes and portable canonical digests remain authoritative; resource diagnostics are
not exported as canonical evidence.

Red tests retained before fixes include an inherited CPU limit raised from one to 120 seconds,
a success response accepted at the final wall deadline, and NUL-bearing text/image output
accepted as evidence. Their corrected suites passed. Early helper fixtures also needed
prewarming and correct parent-lifetime setup; setup errors are not counted as behavioral proof.

The first full runner trial found two Clippy errors and two post-drop ownership reacquisition
failures. The corrected tests allow a bounded 500 ms retry only after deliberate owner release;
production acquisition and live-owner contention remain immediate. The corrected full run passed.
An initial responsiveness trial reached its aggregate fixture deadline; its cause is not established.
Later repeats passed. Discarded CLI-timing experiments included startup/index rebuild work and
are not used below. All failed logs remain retained.

## Concurrent retrieval measurement

One already-open interactive `Library` searched 256 synthetic text files beside **24** explicitly
selected cropped-image OCR jobs. This excludes CLI startup, migrations and repeated FTS rebuilds.
Each counted query observed the same running job ID, epoch and immutable claim token before and
after search; exact artifact/version/passage/hash/anchor results were unchanged.

The final run completed all 24 jobs. Fifty baseline queries had p95 **2.165166 ms**;
**481** confirmed same-claim overlap queries had p95 **3.885292 ms**, below the fixture's
25 ms smoke ceiling with at least 20 samples. Earlier repeats recorded 542/498 overlap queries
with p95 4.352917/3.688417 ms. These are small synthetic-cohort results, not desktop or
100k background responsiveness, a hard peak-RSS guarantee or real-user recovery evidence.

The source is the existing rights-clean **878×191** cropped text region, not a full screen.
No new private screenshot was captured. Peak reported helper resident usage in the final
cohort was 56,213,504 bytes; Vision system-service memory is not included.

The separate actual CLI compatibility run recorded:

| Media | Wall ms | CPU ms | Peak resident bytes |
| --- | --- | --- | --- |
| Text/Markdown | 71 | 5 | 9,240,576 |
| PDF | 62 | 5 | 10,534,912 |
| Cropped PNG OCR | 1,999 | 383 | 51,183,616 |
| Text after v3→v4 upgrade | 54 | 3 | 9,240,576 |

Each successful result recorded the installed address-space guard and 25 ms sampling interval.
Sampled resident/footprint enforcement is not an exact physical-memory cap or an OS security sandbox.

## Performance-verifier correction and remaining gates

Inspection found the previous release gate selected only the first largest-scale repetition.
Additional red tests showed a bad second run could be hidden; missing CPU components and RSS
could also become fabricated zero measurements in report assembly. The corrected gate evaluates
all largest-scale repeats, chooses the worst value for each budget, and marks unavailable data
explicitly. Incomplete variance omits aggregate statistics. A genuine measured zero CPU value
remains distinct from missing data. Tests cover ordering, every budget, malformed/missing metrics,
measurement assembly and the actual final report assembly.

The corrected script then ran two fresh independent cohorts each at 10k/100k artifacts,
with 31 warm queries per run. Both 100k runs indexed every artifact; warm p95 was
**0.470250/0.931125 ms**. All six planning budgets passed across both repetitions;
largest observed process RSS was **108,101,632 bytes**, and no metric was unavailable.
OS page cache was not flushed and machine load was not controlled. This is foreground lexical
measurement, not concurrent OCR at 100k. The earlier 31.756833 ms outlier in the
[v3 report](0400-approved-file-queue.md) remains evidence; no retrieval optimization or
explanation of that outlier is claimed.

The v1 quality metrics and failure records exactly matched the retained v3 executable after
excluding timing: exact-source Recall@1/5 **0.9**, local-text recall **0.75**, anchor precision
**1.0**, `q008` no result and `q009` a false positive. These known weaknesses remain unresolved.

Secret scanning, npm audit and locked Cargo metadata passed. Cargo audit 0.22.2 fetched
RustSec revision `ef6173cbc5c50ec8166f9a5b28f07834144373ee` and denied **eight warnings**:
seven unmaintained dependencies and glib's unsound advisory. No finding was ignored, filtered or
converted to success. [#278](https://github.com/AlisinaDevelo/LOOM/issues/278) stays open.
No issue is closed by this partial adapter or by a report command's zero exit status.

## Selected SHA-256 digests

| Artifact | SHA-256 |
| --- | --- |
| Staged CLI | `1fc4bd6e3633923beb35f67b708d5f7fb667df7b93d7de12d1e935427a291dab` |
| Staged helper | `f7c7f6dd7c82b08d3095809dc15839a30f003406342cb1746f709dab4e9d6327` |
| Workspace log | `5f20d4771bf404ec428bcdc339c6912abd2373c57933f9bd1aec2fd138954cd4` |
| MSRV check log | `1f250291dc8ce810aefa98a96d36d9f985b889af97d12e6b38a881a9164c2236` |
| MSRV core/extraction log | `f523ca6846c74c6c05df1b5ed9f7a5c680f498d8be51975beec6e82829fb01d5` |
| v2/v3/v4 CLI compatibility log | `7dbc35920cc95fba4bc7406b5c1ad7b8ea640b06dbd88553931f2c0ce1ed2244` |
| Concurrent-query report | `ccd6e362bae56fb4b5ccc8997f9d00a75a0125c16f89741728f45a26df7f4306` |
| Corrected large-library report | `699ab8fe5bd65ec1f661d3c93d4a2e5c8032a575e9b2a9667786dcaf949ee54f` |
| Hidden-repeat regression, red | `ff280795b10116b4216dd674533e60d77fd58b7d137e575d4b748f7a246b3985` |
| Missing-resource assembly, red | `9723a60952faa730c86ad9db7f9bb76e577fe8d787f48b282c918fd33e80ae06` |
| Strict security log | `ce6bdc682bf349994baeb9eb5e663598325040a4cf68c3f204248506b3f10ca3` |
| Tauri build log | `e6249ae2caf63fc66c9b996af66fcc3a01b58a3d4c6f77515b0dbd26f7db23dc` |
