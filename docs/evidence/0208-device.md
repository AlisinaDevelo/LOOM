# LOOM 0208 merged-main device evidence

This record documents the accessible evidence-first workflow change for
[issue #27](https://github.com/AlisinaDevelo/LOOM/issues/27). The implementation
was reviewed and merged through [PR #205](https://github.com/AlisinaDevelo/LOOM/pull/205).
The issue remains open because its final criterion requires consented usability
sessions; no participant study is claimed here.

## Target and source

| Field | Value |
| --- | --- |
| Merged source commit | `882919a6bf376781f36f082c12ec337e3cf2185e` |
| Device | Apple Silicon Mac, `arm64` |
| OS | macOS 26.6.2 (`25G83`) |
| Rust | `rustc 1.96.0`; MSRV `rustc 1.88.0` |
| Node/npm | Node `v26.7.0`; npm `11.19.0` |
| Pipe | `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols CARGO_PROFILE_TEST_STRIP=symbols CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 bash scripts/verify-device.sh /tmp/loom-0208-merged-device.413TYF` |
| Pipe result | `PASS` |

The merged-main verification directory is `/tmp/loom-0208-merged-device.413TYF`.
Its commands SHA-256 is
`2f72eb5b0bcd16da859fd42e2e035aa79ff38f56ac29a37fc176087dac77c39e`, summary
SHA-256 is `b3f4bcc0a08473816da12364dc92ed3b4be8f5140239495de40343a47c48e792`,
performance report SHA-256 is
`26f04d60e57ff8513ab05d351b86b72e34bf21408c03c644749e65ed4e727c76`, and the
log manifest SHA-256 is
`117abf14b4fbe5bd9d4cd0c07ae983d130e32f4280e61ae5e2770fe3e8597ec9`.

## Acceptance mapping

| Acceptance criterion | Evidence and boundary |
| --- | --- |
| Primary flows meet applicable WCAG 2.2 AA checks | `src/App.tsx` adds a skip link, named search landmark, semantic result/evidence regions, keyboard-readable image evidence, and explicit live-region semantics. `src/App.css` adds visible focus indicators, readable placeholder/rank/faint text colors, and reduced-motion behavior. `scripts/test-accessibility-contract.py` runs four deterministic checks, including ≥4.5:1 contrast for the primary dark-theme text tokens. The full `npm run check` passed with 2 test files and 17 tests. This is not a manual VoiceOver certification. |
| Focus order, announcements, shortcuts, contrast, zoom, and reduced motion are tested | `src/App.test.tsx` covers Control/Command-K focus and announcement, async result/no-result focus restoration, evidence heading focus, image evidence keyboard naming, zoom state, and rotation. The static contract checks `:focus-visible`, `prefers-reduced-motion`, and the selected contrast tokens. The target-device pipe passed `accessibility-contract`, frontend tests, typecheck, build, security, and Tauri debug build. |
| Usability sessions confirm users understand why a result matched | **Not complete.** Results now expose the source-provided `match_reason` as visible “Why this matched” text, and the DOM test asserts that explanation is present. That is an automated comprehension proxy, not a consented usability session with recruited users; issue #27 stays open until that study is run and recorded. |

## Target-device checks

The same pipe also passed rustfmt, warnings-denied workspace Clippy, the Rust
workspace, MSRV check/tests, v0/v1 retrieval, hybrid ablation, semantic
drop/rebuild recovery, the mixed-corpus failure/recovery path, the 10k/100k
performance gate, the local security scan, and the Tauri debug build. Hosted
GitHub Actions were pending and were not used as evidence.

The 100k performance release gate passed all six budgets. Across two runs the
observed ranges were 1,040.93–1,048.44 artifacts/s indexing, 0.421542–0.424542
ms warm p95, 109,527,040–126,074,880 bytes maximum RSS, 10.0543–10.1493×
database amplification, 0.6766–0.6894 CPU seconds per 1,000 artifacts, and
7.0775–8.5941 seconds for FTS rebuild. The synthetic fixture is rights-clean;
no user content was read or uploaded. Cold means a new process/SQLite
connection without flushing the macOS page cache.

The v0 retrieval fixture remained exact (Recall@1/5 `1.0`, anchor precision
`1.0`, false-positive rate `0.0`, completeness `1.0`). The v1 fixture retained
its documented q008 paraphrase miss and q009 duplicate hard-negative; they are
reported rather than hidden. Hybrid ablation remained eligible with Recall@1/5
`1.0`, anchor precision `1.0`, and false-positive rate `0.0`; the semantic-only
diagnostic remains intentionally non-gating.

No new desktop screenshot was needed for this acceptance record. Existing OCR
fixtures remain deliberately cropped to the relevant evidence region rather
than full-screen captures.

## Reproduction limits

This record does not claim VoiceOver certification, recruited-user comprehension,
screen-reader behavior outside the tested DOM tree, signing/notarization, or
unavailable hardware. The next closure action for #27 is a consented local
usability session using the evidence-first task worksheet; until then the live
issue remains open and the implementation is still safe to use locally.
