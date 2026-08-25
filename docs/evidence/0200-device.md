# LOOM 0200 device evidence

This artifact records the bounded PDF extraction and page-evidence contract for issue
[#19](https://github.com/AlisinaDevelo/LOOM/issues/19), roadmap ID `0200`. PDF source bytes remain
canonical; extraction output, page anchors, warnings, and FTS rows are inspectable derived
records. The renderer/open operation remains bound to the verified original path.
The implementation was merged through PR [#179](https://github.com/AlisinaDevelo/LOOM/pull/179).
The current-main focused rerun is recorded at the end of this artifact; the roadmap status is
advanced after that evidence is merged and reconciled.

## Device and toolchain

- Device: MacBook Pro 17,1, Apple M1, 8 GB
- OS: macOS 26.6.2 (25G83), `aarch64-apple-darwin` / arm64
- Native toolchain: `rustc 1.96.0` / Cargo 1.96.0, Node v26.7.0, npm 11.19.0
- Declared MSRV toolchain: `rustc 1.88.0` (`6b00bc3880198600130e1cf62b8f8a93494488cc`),
  Cargo 1.88.0
- Source under test: `7296b12bec2312f08f5a4ac5208363f8a8b570c4`
  (`feature/issue-19-pdf-extraction`)

## Acceptance-criterion evidence map

|Artifact ID|Acceptance criterion|Retained evidence|
|---|---|---|
|`LOOM-0200-ANCHOR`|Extractor records page number, text span, extractor version, and parse warnings|`golden_pdf_records_page_anchors_warnings_and_verified_navigation` indexes a deterministic three-page PDF with one empty text page, verifies `loom.pdf`/`0.1.0`, page count 3, the exact warning `page 2 contains no extractable text`, and page-local character/line anchors for pages 1 and 3. The CLI `inspect` log shows the same extractor identity and `pdf_page` anchors.|
|`LOOM-0200-BOUNDED`|Encrypted, malformed, image-only, and very large PDFs have explicit bounded outcomes|`malformed_encrypted_image_only_and_oversized_pdfs_fail_closed_then_recover` verifies named failures for malformed bytes, an empty/image-only text layer, an `/Encrypt` marker, an 8 MiB boundary violation, and a configured one-page limit. The test then overwrites the source with a valid PDF and verifies successful indexing and page-anchored recovery. No parser panic escapes the ingestion boundary.|
|`LOOM-0200-DETERMINISM`|Golden fixtures verify deterministic extraction and page navigation|The golden fixture compares the BLAKE3 source hash and full `ArtifactObservation` across repeated indexing, verifies page 3 search recovery and a local character span, and resolves the artifact through the hash/version-bound original-path opener. The direct CLI run indexes a two-page PDF, inspects it, and recovers the second-page result with `page: 2`.|
|`LOOM-0200-MIGRATION`|Existing libraries remain readable|The v2 and v3 compatibility fixtures pass. The v3→v4 migration adds `parse_warnings_json` and nullable `page_count` defaults without rewriting the canonical content hash, passages, or FTS-searchable evidence.|
|`LOOM-0200-UI`|The desktop result preserves PDF evidence semantics|The frontend test renders an `application/pdf` result as `PDF` and retains `page 2 · lines 1–1` instead of collapsing the anchor into a text-only location.|

## Target-device reproduction

The checked-in harness is [`scripts/verify-device.sh`](../../scripts/verify-device.sh):

```text
bash scripts/verify-device.sh /tmp/loom-0200-device-final.xRbH1f
```

The run completed with `status=PASS` and exit code 0 at source commit
`7296b12bec2312f08f5a4ac5208363f8a8b570c4`. Format, workspace clippy, the full Rust workspace,
Rust 1.88 MSRV check and tests, `npm ci`, frontend checks, retrieval benchmark, local security
check, Tauri debug build, and the mixed failure/recovery corpus all passed. No GitHub-hosted
Actions result was used.

The focused PDF suite reports 2 passing tests; the schema compatibility suite reports 4 passing
tests. The local Python roadmap/activation contract suite reports 19 passing tests, and roadmap
validation reports 154 active issues, 4 retired issues, 20 milestones, 13 phases, 141 parent
edges, and 314 dependency edges.

The direct CLI fixture is a rights-clean, deterministic 887-byte two-page PDF. `loom-cli index`
reports `indexed=1`, `bytes_read=887`, and no failures. `loom-cli inspect` reports
`extractor_id=loom.pdf`, `extractor_version=0.1.0`, `page_count=2`, no warnings, and page-local
anchors for pages 1 and 2. Searching the second-page marker returns one `application/pdf` hit with
`kind=pdf_page`, `page=2`, and the expected character/line span. `/usr/bin/time -lp` recorded
12.5 MiB maximum resident set size for the index command, 10.6 MiB for inspect, and 11.0 MiB for
search on this device; these are command-process observations, not product-wide budgets.

The mixed corpus retained the existing safety checks: an unsupported binary, an 8,388,609-byte
source, and an outside-root symlink produced an explicit failure/skip and remained unreachable;
replacing the oversized source recovered it on the next scan. The security check found no
gitleaks secrets and `npm audit --audit-level=high` passed. This is not a third-party audit or a
complete dependency advisory statement.

## Log and digest record

The retained run directory is `/tmp/loom-0200-device-final.xRbH1f`. Its `summary.txt`, command
record, individual logs, focused PDF/schema output, CLI output, Python contract output, roadmap
validation, and `log-sha256.txt` are the source evidence for this entry:

```text
cli-pdf-build.log       sha256:ebecb1f56b721f8217708efbf2792d4f729eccdeda0069b7dab236c9a6576814
cli-pdf-index.log       sha256:e13928c41212bebe8f7b9549e1fa4fb01458d2599824cb45afd130b6f536ab42
cli-pdf-inspect.log     sha256:6fc2e908d35630b784d56e279d76040a7a0e0638ecfe7f1dd38c4bea8fd0a334
cli-pdf-meta.log        sha256:a404bc81e3f798d3877cd21b01eca2252618183385e430728dbc2cea177a9afd
cli-pdf-search.log      sha256:9f2f842e28e603db32966e6b2562a499a19df671b7f17569af4c1f92101c95a7
clippy.log              sha256:d6d25aaf5581d3c9b129f7fb6ab9f8fb656ff548d6f99304f9dec704687d7bd9
fmt.log                 sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
mixed-corpus.log        sha256:70090702d239afb8ac0e8d1af3eb28855736dd7cfdb2fa4a477f5976c8348848
npm-check.log           sha256:3498af84de1f7cf622b80542800b8efc4e85d448f9244930a1028b9fbc77462b
npm-install.log         sha256:bbacb689fb0c829145033c5a0e39bff2b1fcb458b993473ecadaf839f978c9c0
pdf-extraction.log      sha256:2b6fc8573b7409734aa807d956008d5810798ee3be713ed0fb6b2716a7a169da
python-tests.log        sha256:95f02de7def0e9edf6e728afd22f6a4bab5d20d31693170c910fe0e34fc05f16
retrieval-benchmark.log sha256:79ceb6a9db5925dbb17e626f6416bafe6ec29aebbf663b38ec5cdd70d2e49427
roadmap-validation.log  sha256:df36f3da734b1902d0f0e6711aeb03645bc9f04afad31993b9275c55f1a82c96
rust-msrv-check.log     sha256:4e8fae4b3d3d3e14d148070c00d5c715fb1d38bb4febd3a959178e0cb132255e
rust-msrv-tests.log     sha256:92ac4a86c8ce7418e6b96faa3e1929630695afb17f3c549606e72235402f0f50
rust-workspace.log      sha256:2f805123244027e494650db9323153b6643fa26d35b0dc8db71e94e712c6dff0
schema-compatibility.log sha256:5da999512c9b3fb15c84340c67852659cc241d6d99e7a0eef1dcfdcff8ddaa9c
security-check.log      sha256:ac4d5de53d2b6e73a042f4ceb3da810ad85f37764d7198beebfdac7d0293146b
tauri-build.log         sha256:9ef47eb75d7916f5447adc32418b6945d0697194cf9747213533a7947dc29990
```

## Limitations and closure gate

This issue now has reproducible PDF extraction, inspectable page anchors, explicit failure
outcomes, and local original-path verification. The current slice does not embed a PDF renderer or
promise pixel-perfect in-app page scrolling; the verified source path remains the render authority
for the user's PDF viewer. It also does not claim OCR for image-only PDFs, another OS/architecture,
notarization, a third-party security audit, or a large-library performance benchmark. The
current focused rerun below is the code/device authority; no hosted Actions result or
protected-branch enforcement was used.

## Merged-main reproduction

The same target-device harness was run against runtime-tested merged `main` commit
`eee1236710b98375e86b12187d545ed451ee2b7c` on the Mac specified above. The current main tip
`e5bcf782e0c5ea3efce27c7b3625fde50f6e25b9` adds only documentation and roadmap metadata after
that runtime-tested commit; no PDF extraction source changed.

- Verification directory: `/tmp/loom-0110-main-device.QLkKl1`
- Harness summary SHA-256: `45bc997dcb26b8bc6cbd63a09fa17aed6c5d4ae968ef349be473b0b034e94e70`
- Commands SHA-256: `d840925fb008af9101dc3121870b79960c1b6924451df17a7851f2f6132bb209`
- Log manifest: `/tmp/loom-0110-main-device.QLkKl1/log-sha256.txt`
- Log manifest SHA-256: `ed539c48e8c9b648f0ae341ddf37f02107d5aacff5d5e2a19ae0d28612c57d64`

The full local pipe passed. Its workspace tests included the PDF extraction fixtures for page
anchors, warnings, malformed/encrypted/image-only/oversized fail-closed behavior, and recovery;
frontend checks preserved the PDF page label and anchor. Retrieval, security, Tauri, and mixed
failure/recovery checks also passed. No hosted Actions or unavailable hardware substituted for this
target-device evidence. Future desktop captures must be cropped to the relevant evidence panel.

## Current merged-main focused rerun

The focused PDF suite was run against source-equivalent current `main` commit
`56203c442faeb6a72d52702c125db64e823fa6a9` (implementation source remains equivalent to
`87d1e03ffe2a43fed33826df58360e456ca4c753`) on the target Mac. The low-footprint log
`/tmp/loom-0200-03-focused-current.xxCDT1/focused.log` (SHA-256
`dc8a8d5d21c063ef17bb39bc927c1dc98f40a1ce5f89411fd23682fb68352f83`) records 2 PDF tests passed,
5 image-OCR tests passed, and 6 semantic-index tests passed. The current frontend `npm run check`
also passed 23 tests, TypeScript, lint, and Vite build; its log SHA-256 is
`66dc7617e127acdc9f5a5fb533e8fa146a6a6faffac91def5ecec437595d42d3`.

## Q03 roadmap reconciliation

Roadmap ID `0200` was marked done only after the focused rerun and was reconciled with the other
Q03 gates through merged PR [#249](https://github.com/AlisinaDevelo/LOOM/pull/249), commit
`c653ac45006cbd22201b1a16ec45e61e5fbb33e6`. The single-writer reconciler changed issues
`0200`–`0203` and then verified 154 active managed issues, 4 retired issues, 20 milestones, 13
phases, 141 parent edges, 314 dependency edges, and 31 closed issues. It reported no warnings and
the second plan was zero-delta.

```text
preflight plan: /tmp/loom-0200-03-roadmap-plan-before.json
  sha256: d60a99e4a37ec87d3031fe9ef4266459f3442d1d044a2e845f2f19850ab96da0
apply log: /tmp/loom-0200-03-roadmap-apply.log
  sha256: bd4d6ceb8e19293745ff0214044694cec35c67023cc507ebad459d790ad45c6a
after plan: /tmp/loom-0200-03-roadmap-plan-after.json
  sha256: de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
second plan: /tmp/loom-0200-03-roadmap-plan-second.json
  sha256: de11779046e5cbcf4ea4e7bbe88d19c641676058a6cb88254f80357b752941f1
```

The live issue for `0200` (#19) is closed and its title, milestone, labels, body marker, and
dependencies match the canonical manifest. No hosted Actions result was used.
