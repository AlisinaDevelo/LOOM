# Queued extraction process boundary

This is a prerequisite slice of [#36](https://github.com/AlisinaDevelo/LOOM/issues/36),
not completion of desktop/directory/semantic background indexing or a signed release.
Only queued `index_file` extraction uses this boundary. Foreground ingestion, intentional
capture, native messaging and semantic commands retain their existing synchronous paths.

## Authority and installation

The parent verifies the admitted claim, source capability and artifact identity, snapshots
canonical state, and reads at most 8 MiB through its no-follow/identity-checked descriptor.
The helper receives only a supported media enum, finite resource policy and original bytes.
It never receives a source path, URL, database, consent token, artifact ID, hash or capture
context. PDF pages, OCR regions and normalized text share one implementation with foreground
ingestion. The parent validates those values and derives source hash, timestamp and canonical
anchors. Final publication rechecks consent, identity, CAS and original bytes after obtaining
the SQLite writer lock; evidence and the parent job completion commit together.

Build and install both binaries in the same directory:

```sh
cargo build --locked -p loom-cli -p loom-extraction --bins
```

Default resolution is the exact `loom-extractor` beside the current executable, never PATH,
an environment override, a job payload or a shell. The executable must be an absolute regular
non-symlink file; on Unix it must be executable, owned by the current user/root and not writable
by group/others. Embedding hosts may explicitly configure another validated executable via
`JobWorker::with_extractor_path`; this is trusted application wiring, not source authorization.
Missing installation fails the job without rewriting evidence. Runtime v4 fences older v3
workers before claims; `upgrade-job-runtime` is explicit and exclusively owned.

The desktop does not yet run this queue or bundle the helper. Enabling a desktop worker requires
verified bundle placement, signing/notarization and permission/controller tests; CLI adjacency
is not evidence of those distribution gates. The existing desktop foreground path is unchanged.

## Framing and lifetime

One request contains two 16-byte-header frames: bounded JSON metadata (4 KiB) and bounded original
bytes (8 MiB). One response contains bounded strict-schema JSON (16 MiB). Headers carry magic,
version, kind and length; oversize lengths are rejected before allocation/body reads. Unknown
fields, unsupported media/version, truncation, trailing frames and invalid output geometry fail
closed. PDF/image text has one wire representation, not a second normalized copy. Canonical
publication limits remain 2 MiB text, 2,048 pages, 8,192 regions/passages, 128/16 KiB warnings and
64 KiB metadata. Provider failure responses contain typed codes, not raw parser/source diagnostics.

Foreground ingestion now applies the same normalized-output checks before canonical conversion:
UTF-8 text is capped at 2 MiB and rejects controls except newline/tab; PDF/image evidence must
have valid page/region geometry and bounded warnings. The shared validator does not grant a
worker resource budget to synchronous providers. Foreground retains its configured PDF-page
and 100-million-pixel limits; queued validation retains the strict process policy above.

The parent clears the environment, sets a non-source working directory, discards stderr and owns
the child plus bounded pipe threads. Every return/unwind kills and waits for the owned child
before joining pipe threads. A monotonic loop checks cancellation, exact target/claim, persisted
scope/OCR revision, identity/CAS, wall deadline and process memory at 25 ms intervals. DB probes
hold only a short read transaction with no busy wait; they do not reopen source files. Probe
failure stops extraction rather than continuing with stale authority.

The helper also starts an independent 25 ms watchdog before reading input. Loss of its expected
parent PID, deadline, resource overrun or unavailable monitoring terminates the helper. This bounds
orphan lifetime after parent death. The shipped helper does not spawn provider subprocesses.
Before request metadata arrives, the startup wall ceiling is a global 180 seconds; media-specific
limits replace it only after validation. Parent-loss polling is active in both phases and does not
wait for that startup ceiling or input EOF.
This ownership design does not promise cleanup for arbitrary descendant-spawning replacements.
[Rust's child contract](https://doc.rust-lang.org/std/process/struct.Child.html) requires explicit
kill/wait; dropping `Child` alone does not terminate it. No `pre_exec` callback is used: its
[post-fork restrictions](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html)
are unsuitable for allocator/runtime/provider work in a multithreaded desktop.

## Resource policy and limits

| Media | Wall deadline | CPU limit | Sampled resident/footprint ceiling |
| --- | --- | --- | --- |
| Text/Markdown | 5 s | 5 s | 256 MiB |
| PDF | 30 s | 20 s | 512 MiB |
| Image OCR | 180 s | 120 s | 768 MiB |

After exec and before source input/provider calls, the helper lowers and verifies its core-dump,
CPU and address-space limits, then reduces scheduling priority. Images are capped at 16 million
pixels before Vision; foreground indexing retains its previous 100-million-pixel limit. Unsupported
platforms or unavailable guards fail closed. macOS and Linux monitoring are implemented; only
target-device results actually recorded in the evidence report establish execution on that OS.
The CPU hard limit has one second of slack above the soft limit so exhaustion can be classified
from `SIGXCPU`; stricter inherited CPU limits that cannot preserve this distinction fail closed.

Address space is capped at 512 GiB on macOS (large shared-cache reservations), 2 GiB on Linux.
Neither is a physical-RAM cap. macOS resident/physical-footprint values and Linux resident pages
are sampled, so transient peaks can exceed a ceiling between observations. CPU enforcement is
kernel-based; parent/helper wall enforcement depends on scheduling and probe progress. The OS
[resource-limit semantics](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/setrlimit.2.html)
and [process usage fields](https://developer.apple.com/documentation/kernel/rusage_info_v2)
do not justify claiming an exact hard peak-RSS or whole-device memory bound.

Successful job results expose measured wall time, CPU time and peak resident bytes, sampling
interval and installed-limit status as operational diagnostics. These are excluded from portable
canonical archives. Crashes/launch failures may retry within the admitted job's existing budget;
resource overruns, invalid output and provider unavailability fail. Consent/OCR drift cancels.

This is failure/resource isolation, **not an OS security sandbox**: the helper still runs as the
user. Clearing environment and withholding identity reduce accidental authority/diagnostic leaks;
they do not confine a malicious executable, guarantee no network at the OS level, or account for
system services used by Vision. Search responsiveness and realistic media budgets require measured
device evidence; limits in source code alone are not completion proof.

## Reproduction

`scripts/test-queued-indexing.py` runs actual text/PDF/native Vision jobs and can accept
retained v2/v3 CLI binaries to verify explicit v4 upgrade and older-worker refusal.
The macOS `jobs::tests::persistent_retrieval_during_native_queued_ocr` fixture runs 24 explicitly
selected cropped-image OCR jobs beside lexical searches over 256 synthetic text files. Search
uses one already-open interactive `Library`, without CLI startup, migration or FTS rebuilds.
A sample counts only when the same immutable job ID/epoch/claim token is running before and
after the query. Its small-cohort smoke gate requires 20 overlap samples and p95 below 25 ms;
the report also records baseline latency and each helper's resource metrics. This is not desktop responsiveness,
100k-corpus acceptance, a signed-build test or a claim about Vision's system-service memory.
The target-device runner executes this experiment alongside the existing larger benchmark,
frontend, native-host, MSRV, security and recovery checks; their failures remain separate gates.
See [the retained device evidence](evidence/0400-isolated-extraction.md) for actual results,
failed trials, compatibility checks and remaining limitations.
