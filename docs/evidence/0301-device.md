# Issue 0301 — explicit-save browser extension evidence

- Issue: [#30](https://github.com/AlisinaDevelo/LOOM/issues/30)
- Implementation PR: [#221](https://github.com/AlisinaDevelo/LOOM/pull/221)
- Implementation commit: `cbd015d351f729a45ef0f8b828be701eb0678f4`
- Merged-main reproduction SHA: `84dff076c0b397ea703c96905279a70d3ac82b25`
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
| Merged-main run | `/tmp/loom-browser-contract-merged-device.EAg5Iz` |
| MVP demo run | `/tmp/loom-mvp-demo-proof` |
| Main protection | Ruleset `main-protection` (`21332347`), active on `refs/heads/main` |

The run directory retains the source SHA, environment, extension tests, syntax checks, protocol
tests, Rust workspace/MSRV tests, retrieval fixtures, adversarial PDF checks, semantic and
performance contracts, accessibility, security, Tauri build, and the npm check that now invokes
the browser contract. Its summary SHA-256 is
`d69b7c440244370d489b9cc853853148a6cb3667a591156871b4b20f94c0026f`, the npm-check log SHA-256 is
`1df3da73093047a1e222f9a2d05818b04c2fd308946d03f8d2dce00c05ac4f9d`, the log manifest SHA-256 is
`ccffb72c8c64f4570fc3dd6d201f7b8d9b4b47d6f8780f5116ae5489b616541d`, and the performance report
SHA-256 is `adffa915103e31eb7d63262f162d2aac94bf05de9cf544e6cc33dca592f5bf4f`.

## Acceptance mapping

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0301-INTENT` | Extension requests minimum permissions and captures only after a visible user action | `browser-extension/manifest.chrome.json` and `manifest.firefox.json` request only `activeTab`, `commands`, `nativeMessaging`, `scripting`, and `storage`, with no host permissions. `background.js` has only the explicit action and `save-page` command entry points. The test named `background wiring exposes only explicit save actions` rejects history, web-request, timer, and background capture wiring. |
| `LOOM-0301-SANITIZE` | HTML is sanitized and normalized before indexing; scripts and remote execution never enter the viewer | `browser-extension/src/capture.js` strips executable/dangerous elements, all attributes, forms, remote URLs, comments, and event handlers before producing authenticated payload frames. The test named `sanitizes scripts, remote attributes, forms, and event handlers` asserts the forbidden content is absent. |
| `LOOM-0301-CONTRACT` | Cross-browser contract tests cover redirect, SPA, offline, and denied-capture cases | `browser-extension/test/capture.test.js` passes 10 tests covering complete authenticated chunking, cross-origin redirect partial state, bounded oversized payloads, offline metadata-only capture, denied page access before native messaging, missing pairing, explicit counter advancement, background wiring, and identical Chrome/Firefox permission manifests. The protocol fixture suite passes 10 tests and the existing Python suite passes 20 tests. |

Every displayed demo result resolved to an artifact ID, source locator, content hash, and evidence
anchor. The text result reported line/character offsets, the PDF result reported page/character
offsets, and the OCR result reported image coordinates and confidence. The MVP demo indexed five
rights-clean fixtures, recovered all three queries, reported no failures, and ended with five
artifacts and seven passages. Its output is retained in `mvp-demo.log`.

## Merged-main checks

The current merged-main device rerun at `84dff076c0b397ea703c96905279a70d3ac82b25` returned
`status=PASS`. In addition to the checks below, `npm run check` now executes
`npm run test:browser-extension`: the frontend suite passed 23 tests and the browser contract
passed 10 tests. This is a local target-device reproduction; hosted checks are not treated as a
substitute for it.

The following commands were run against the exact merged-main SHA above. Hosted status checks are
not used as a substitute for this target-device reproduction.

```text
node --test browser-extension/test/capture.test.js       10 passed (also run by `npm run check`)
node --check browser-extension/src/capture.js             passed
node --check browser-extension/src/background.js         passed
python3 scripts/test-browser-capture-protocol.py         10 passed
python3 -m unittest discover -s tests -p 'test_*.py'     20 passed
cargo test --locked -p loom-core -p loom-cli              86 passed
python3 scripts/roadmap.py --validate-only               valid
markdownlint-cli2 ...                                     0 issues
cargo fmt --all -- --check                               passed
scripts/demo-mvp.sh                                       5 indexed; 3 anchored hits; 0 failures
```

The current merged-main workspace test and Tauri debug build both passed on the target device. The
interactive browser/native-host gate remains separate and is not inferred from these offline
contract tests.

## Privacy, failure, and recovery boundary

The connector has no history listener, network client, timer, or content telemetry. It refuses to
inspect a page before a local pairing record exists, emits no snapshot payload while offline, and
marks cross-origin redirects or bounded truncation as partial. Denied script access fails before
native messaging, and the native host remains an explicit integration prerequisite. The current
implementation therefore does not claim a signed native host, browser-store review, interactive
permission onboarding, SPA execution, or a packaged desktop release.

No desktop screenshot was used as evidence. The OCR fixture consumed by the demo is already cropped
to its evidence region. If a future handoff includes a desktop capture, it must show only the
relevant LOOM/result panel, with the rest of the desktop cropped away and no source paths,
credentials, or private documents.

The repository owner is the only available GitHub reviewer, so no independent approval is claimed.
Issue #30 remains open in `review` until the consented interactive browser/native-host session is
run on this Mac or an equivalent permitted target.
