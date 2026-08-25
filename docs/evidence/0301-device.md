# Issue 0301 — explicit-save browser extension evidence

- Issue: [#30](https://github.com/AlisinaDevelo/LOOM/issues/30)
- Implementation PR: [#257](https://github.com/AlisinaDevelo/LOOM/pull/257)
- Implementation commit: `b76f0e8de2cf0f483e57514be333aacc2bbe3274`
- Merged-main reproduction SHA: `dbbd997221f93faeac6908b88657a3b25b3b1634`
- Roadmap status: `review`
- Verification status: the offline connector contract passes; the issue remains open for a
  consented interactive browser permission/native-host session.

## Target and retained evidence

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Python | 3.9.6 |
| Merged-main run | `/tmp/loom-0301-final-device.34Wyvw` |
| MVP demo run | `/tmp/loom-mvp-demo-proof` |
| Main protection | Ruleset `main-protection` (`21332347`), active on `refs/heads/main` |

The run directory retains the source SHA, environment, extension tests, syntax checks, protocol
tests, Rust workspace/MSRV tests, retrieval fixtures, adversarial PDF checks, semantic and
performance contracts, accessibility, security, native-host pipe, Tauri build, and the npm check
that now invokes the browser contract. Its summary SHA-256 is
`669ea8531a0e31feef2ecdd358244eaac5fd49bebda564be51eaa1a30f8d7b78`, the npm-check log SHA-256 is
`c04241fbcae4670f986d80fea04986de87244f94920745c4a547dfcf7410083f`, the native-host log SHA-256
is `f918857300bdf2ccf45fa7ed19cd738d77c3432b75d667c64ef9cc965d50fec1`, the Tauri build log
SHA-256 is `fbac0905bdbc0d212d224091ba52b0b16dfb5c6dd6c604d598cc9384d32bd1b8`, the log manifest
SHA-256 is `845ab4b52d694ae07339e580125589ed2c25a9e421e58e7c47d00e749d9ad123`, and the performance
report SHA-256 is `7f9d7938ed914f383c5a9c0a31e7ef44cd8dd58a6ef2fafdd61061f1032a3978`.

## Acceptance mapping

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0301-INTENT` | Extension requests minimum permissions and captures only after a visible user action | `browser-extension/manifest.chrome.json` and `manifest.firefox.json` request only `activeTab`, `commands`, `nativeMessaging`, `scripting`, and `storage`, with no host permissions. `background.js` has only the explicit action and `save-page` command entry points. The test named `background wiring exposes only explicit save actions` rejects history, web-request, timer, and background capture wiring. |
| `LOOM-0301-SANITIZE` | HTML is sanitized and normalized before indexing; scripts and remote execution never enter the viewer | `browser-extension/src/capture.js` strips executable/dangerous elements, all attributes, forms, remote URLs, comments, and event handlers before producing authenticated payload frames. The test named `sanitizes scripts, remote attributes, forms, and event handlers` asserts the forbidden content is absent. |
| `LOOM-0301-CONTRACT` | Cross-browser contract tests cover redirect, SPA, offline, and denied-capture cases | `browser-extension/test/capture.test.js` passes 11 tests covering complete authenticated chunking, cross-origin redirect partial state, same-origin single-page-app capture, bounded oversized payloads, offline metadata-only capture, denied page access before native messaging, missing pairing, explicit counter advancement, background wiring, and identical Chrome/Firefox permission manifests. The protocol fixture suite passes 10 tests and the existing Python suite passes 20 tests. |

The pre-close native-host boundary is separately retained as `LOOM-0301-HOST`: the Rust
`loom-browser-capture` crate passes 7 unit tests for framing, duplicate keys, authenticated
acceptance, atomic spooling, replay/recovery, hostile HTML, and forbidden attributes. The merged
device smoke test (`native-host-contract.log`) speaks the actual Node extension builder to the
Rust stdio host and reports one accepted sanitized capture plus four deterministic rejections
(malformed frame, replay, hostile HTML, and payload MAC failure). No rejected payload leaves a
snapshot or metadata file. The extension payload session shape was corrected to the protocol’s
`id`/`counter` form before the merged rerun.

Every displayed demo result resolved to an artifact ID, source locator, content hash, and evidence
anchor. The text result reported line/character offsets, the PDF result reported page/character
offsets, and the OCR result reported image coordinates and confidence. The MVP demo indexed five
rights-clean fixtures, recovered all three queries, reported no failures, and ended with five
artifacts and seven passages. Its output is retained in `mvp-demo.log`.

## Merged-main checks

The current merged-main device rerun at `dbbd997221f93faeac6908b88657a3b25b3b1634` returned
`status=PASS`. In addition to the checks below, `npm run check` now executes
`npm run test:browser-extension`: the frontend suite passed 23 tests and the browser contract
passed 11 tests. The native-host device contract passed after `cargo build --locked -p loom
--bin loom-native-host`, and `npm run tauri build -- --debug --no-bundle` built
`target/debug/loom`. This is a local target-device reproduction; hosted checks are not treated as
a substitute for it.

The following commands were run against the exact merged-main SHA above. Hosted status checks are
not used as a substitute for this target-device reproduction.

```text
node --test browser-extension/test/capture.test.js       10 passed (also run by `npm run check`)
node --check browser-extension/src/capture.js             passed
node --check browser-extension/src/background.js         passed
python3 scripts/test-browser-capture-protocol.py         10 passed
python3 scripts/test-native-host.py --host target/debug/loom-native-host  passed (Node → Rust pipe)
python3 -m unittest discover -s tests -p 'test_*.py'     20 passed
cargo test --workspace --locked                          passed
cargo test -p loom-browser-capture                       7 passed
python3 scripts/roadmap.py --validate-only               valid
markdownlint-cli2 ...                                     0 issues
cargo fmt --all -- --check                               passed
scripts/verify-device.sh                                  status=PASS
scripts/demo-mvp.sh                                       5 indexed; 3 anchored hits; 0 failures
```

The current merged-main workspace test and Tauri debug build both passed on the target device. The
interactive browser/native-host gate remains separate and is not inferred from these offline
contract tests.

## Privacy, failure, and recovery boundary

The connector has no history listener, network client, timer, or content telemetry. It refuses to
inspect a page before a local pairing record exists, emits no snapshot payload while offline, and
marks cross-origin redirects or bounded truncation as partial. Denied script access fails before
native messaging. The checked native host refuses to start without an explicit pairing secret and
spool root, never fetches URLs, rejects untrusted HTML/attributes, and writes only after complete
hash verification. The current implementation still does not claim Keychain pairing, a signed
host manifest, browser-store review, interactive permission onboarding, SPA execution in a real
browser, or a packaged connector release.

No desktop screenshot was used as evidence. The OCR fixture consumed by the demo is already cropped
to its evidence region. If a future handoff includes a desktop capture, it must show only the
relevant LOOM/result panel, with the rest of the desktop cropped away and no source paths,
credentials, or private documents.

The repository owner is the only available GitHub reviewer, so no independent approval is claimed.
Issue #30 remains open in `review` until the consented interactive browser/native-host session is
run on this Mac or an equivalent permitted target.
