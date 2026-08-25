# Issue 0300 — browser capture protocol evidence

- Issue: [#29](https://github.com/AlisinaDevelo/LOOM/issues/29)
- Implementation PR: [#217](https://github.com/AlisinaDevelo/LOOM/pull/217)
- Reconciler-fix PR: [#219](https://github.com/AlisinaDevelo/LOOM/pull/219)
- Merged-main SHA: `193ee538a62ce04d64dea13c690287c35166379d`
- Roadmap status: `done`
- Verification status: PASS for the protocol specification and offline contract; prerequisite
  `0202` is now closed. Browser/permission implementation remains in `0301` and is not claimed by
  this record.

## Target and retained evidence

| Field | Recorded value |
| --- | --- |
| Hardware | MacBook Pro 17,1; Apple M1; 8 GB |
| Operating system | macOS 26.6.2 (25G83), arm64 |
| Rust | rustc 1.96.0 / cargo 1.96.0 |
| JavaScript | Node v26.7.0 / npm 11.19.0 |
| Python | 3.9.6 |
| Merged-main run | `/tmp/loom-0300-final.BSyWZq` |
| Merged PRs | `#217` (`7de9dff2a5633efc149a8575fa234ae446443c4a`), `#219` (`193ee538a62ce04d64dea13c690287c35166379d`) |
| Main protection | Ruleset `main-protection` (`21332347`), active on `refs/heads/main` |

The run directory retains the exact source SHA, environment, protocol-test output, existing Python
test output, roadmap validation, markdown lint result, effective main rules, PR merge records, clean
diff check, clean status, the reconciler plan, and a SHA-256 manifest. The manifest is
`/tmp/loom-0300-final.BSyWZq/log-sha256.txt` (SHA-256
`7777977107df93b236c633d62d0dd7ef29b3735433dd9fd574bfb617661e73dc`).

## Acceptance mapping

| Artifact ID | Acceptance criterion | Retained evidence |
| --- | --- | --- |
| `LOOM-0300-THREAT` | Threat model covers malicious pages, extension compromise, native messaging, sensitive fields, redirects, and oversized payloads | `docs/protocol/browser-capture-v1.md` threat table and `docs/adr/0009-browser-capture-protocol.md`; the document maps every threat to a control and observable rejection/state. |
| `LOOM-0300-AUTH` | The local peer is authenticated and replay, downgrade, and unapproved capture are rejected | Versioned pairing/session flow, Keychain-backed capability, HMAC canonical-envelope rule, duplicate-key rejection, monotonic counter, single-use IDs/tokens, and explicit user gesture are specified. `scripts/test-browser-capture-protocol.py` runs the HMAC vector, duplicate-key, replay, downgrade, and missing-consent tests. |
| `LOOM-0300-FIELDS` | Captured fields and exclusions are documented before implementation | The protocol lists URL, final URL, title, selected text, capture time, redirects, scope, content hash, parser ID/version, byte limits, and the complete forbidden-field set. The fixture at `tests/fixtures/browser-capture/protocol-v1.json` is synthetic and contains no private source. |
| `LOOM-0300-SNAPSHOT` | Snapshot capture is best-effort with visible complete/partial/failed states | The protocol defines `complete`, `partial`, `failed`, and `not_requested`, typed failure reasons, live-versus-snapshot distinction, parser metadata, redirect rules, paywall/JS/CSP limitations, and user-facing messages. Fixture tests validate all four states and oversized rejection. |

## Merged-main checks

The following commands were run against the exact merged-main SHA above, without hosted Actions or
unavailable hardware as substitutes:

```text
python3 scripts/test-browser-capture-protocol.py       10 passed
python3 -m unittest discover -s tests -p 'test_*.py'   20 passed
python3 scripts/roadmap.py --validate-only             valid
markdownlint-cli2 ...                                  0 issues
git diff --check                                        clean
python3 scripts/roadmap.py --apply                     verified; zero mutations
```

Roadmap validation reported 154 active issues, 4 retired issues, 20 milestones, 13 phases, 141
parent edges, and 314 dependency edges. The reconciler's initial and second plans both reported
zero label, milestone, issue, retirement, parent, and dependency mutations; its verified result
reported 158 managed issues and 13 closed issues. The standalone reconciler output is retained at
`/tmp/loom-0300-final.BSyWZq/roadmap-apply.json` with SHA-256
`43f96b0c5a024c5917da3b0fd895b2c761cb3bff458355ed088d7b0fc753e3e7`. The main rules endpoint
returned deletion, non-fast-forward, and pull-request rules for the active protection ruleset. No
screenshot was used as evidence; any future desktop capture must be cropped to the relevant
browser/LOOM panel.

## Review boundary and limitations

This is a protocol specification and fixture contract. It does not claim an implemented browser
extension, native host, HTML sanitizer, browser permission onboarding, cross-browser behavior, or
web archive. Those are roadmap `0301` and later acceptance gates. The protocol does not defend
against a compromised macOS account, Keychain, browser process, or signed native host.

The PRs were locally diff-reviewed and merged to the active protected `main` ruleset. The repository
owner is the only available collaborator, so no independent GitHub approval is claimed. Issue #29
is ready for closure because its declared protocol acceptance and prerequisite are satisfied. The
interactive browser and native-host gates remain explicitly assigned to issue #30.

## Current merged-main focused rerun

The protocol and extension-boundary contracts were rerun against current merged `main`
`6b140986d36de41da3c7005c808a46c46a90f8e0` on the target Mac. The retained focused artifacts are
under `/tmp/loom-0300-current-focused`:

```text
python3 scripts/test-browser-capture-protocol.py: 10 passed
  sha256: cf8d3a2572a7898fcf32a4c2dd1445509b3d59e66061853243c4627f80695068
node --test browser-extension/test/capture.test.js: 10 passed
  sha256: f167a1ddf9d4a3470b68c52d361e46c8fea87e20bf205bc874ade4bdbb3b32a6
python3 -m unittest discover -s tests -p 'test_*.py': 20 passed
  sha256: 2ffd7985716b2e09b1f0ff6ece102745b256c3dfebc0891c03dbb315dd164b39
roadmap validation: 154 active, 4 retired, 20 milestones, 13 phases, 141 parent edges, 314 dependencies
  sha256: df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96
```

No hosted Actions result or interactive browser session is used for this protocol-only closure;
those implementation and permission boundaries remain visible in the dependent issues.
