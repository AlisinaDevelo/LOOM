# Issue 0300 — browser capture protocol evidence

- Issue: [#29](https://github.com/AlisinaDevelo/LOOM/issues/29)
- Implementation PR: [#217](https://github.com/AlisinaDevelo/LOOM/pull/217)
- Reconciler-fix PR: [#219](https://github.com/AlisinaDevelo/LOOM/pull/219)
- Merged-main SHA: `193ee538a62ce04d64dea13c690287c35166379d`
- Roadmap status: `review`
- Verification status: PASS for the protocol specification and offline contract; the issue remains
  open while prerequisite `0202` is still in review.

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
therefore remains in `review` until its declared prerequisite/review governance is satisfied.
