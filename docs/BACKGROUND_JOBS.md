# Durable background work: explicit maintenance, file and directory refresh

Roadmap `0400` / [#36](https://github.com/AlisinaDevelo/LOOM/issues/36) is not complete.
This is an opt-in durable queue with real FTS-repair, approved-file and directory refresh adapters,
not desktop background indexing. Synchronous ingestion/OCR/semantic commands remain available.

## Using the current adapter

```sh
cargo build --locked -p loom-extraction --bin loom-extractor
cargo run --locked -p loom-cli -- enqueue-fts-repair repair-after-import --low
cargo run --locked -p loom-cli -- index /absolute/selected-file.md
cargo run --locked -p loom-cli -- enqueue-index-file /absolute/selected-file.md refresh-selected-file
cargo run --locked -p loom-cli -- jobs
cargo run --locked -p loom-cli -- run-next-job
cargo run --locked -p loom-cli -- cancel-job JOB_ID
cargo run --locked -p loom-cli -- forget-job TERMINAL_JOB_ID
```

Commands use the same `--database` option as other CLI operations. `run-next-job` performs
at most one due typed unit, returning its durable result or JSON `null` when no job is due.
It does not wait for delayed retries or launch a daemon. Repair publishes its derived FTS5
projection and `completed` result in the same epoch/token/cancellation-fenced transaction.
Canonical originals, hashes, passages, and anchors are not rewritten.
Queue commands require an existing initialized library; initialize or migrate it explicitly
with an ordinary command such as `stats` first. Queue inspection/admission/cancellation/forgetting
never implicitly rebuild FTS. An empty or contending `run-next-job` cannot repair the projection.

## Operational contract

`background_jobs` and singleton `background_job_runtime` are operational schema-10 additions,
not portable canonical records. They contain no new source permissions or arbitrary executable
payloads; admitted operations are `fts_repair`, `index_file` and `index_directory`.
A key is 1–128 ASCII identifier bytes.
Repeating a key with identical operation/priority/target returns its original row, including a terminal
row. Conflicting input is rejected without changing it. Terminal keys are not silently evicted.
At the retained-record limit, explicitly forgetting a terminal record frees capacity and forgets
its key; a later request with that key becomes new work. Pending/running jobs cannot be forgotten.
The separate `background_job_schema_version` marker is runtime-only. Version 5 adds bounded
directory manifests, quanta, and canonical-locator deletion/update fences. Version 4 requires
supervised byte-only extraction; version 3 introduced the typed file target, bounded to 16 KiB.
Existing version-4, version-3, version-2 or recognized unversioned layouts require explicit
`upgrade-job-runtime`. Upgrade takes the kernel worker lock before opening SQLite; migration,
epoch rotation, and abandoned-work recovery share one transaction. A live worker prevents it.
Ordinary opening does not migrate an existing runtime. Records, policy, sequence, and priority
accounting survive upgrade; invalid legacy diagnostics roll back without dropping jobs.
Unknown layouts are refused. Old binaries can still read canonical schema-10 evidence but must
refuse v5 queue commands. Upgrade is operational, not a portable schema migration.
Artifact/root/OCR deletion applies the same recovery gate: validated v2 or unversioned legacy
runtimes contain no file targets and may continue, while a recognized v3 runtime must complete
the owned upgrade before deletion. An unknown future marker or malformed current layout/state
requires a compatible LOOM release or reviewed migration. The deletion transaction fails closed before
canonical changes and reports that no data was deleted. Future-compatible erasure is an unresolved
design gate; no binary may claim complete deletion until it has a reviewed mapping for that
runtime's operational locators and diagnostics.
The first shipped v3 target includes admission-time artifact identity; earlier development
prototypes were not released as a separate supported runtime. A missing identity is not
silently upgraded to the identity of existing evidence. Explicit `null` records absence at
admission; omitting that field is invalid and permanently fails dispatch, even after deletion.

## Approved-file contract

`enqueue-index-file` accepts one bounded regular file whose exact canonical locator already has
an enabled **file** root. Approval of a parent directory is insufficient for this adapter.
It rejects directories, symlinks, unsupported extensions, files over 8 MiB (or a smaller caller
limit), unapproved/revoked roots, and images when OCR is disabled. Admission never selects a
source or extracts bytes. The typed target retains root ID, consent generation, authorization
incarnation, admission-time artifact ID (or absence), source kind/media type and, for images only,
enabled OCR revision. Unknown fields
or invalid capabilities fail closed. The path is capped at 4 KiB.

This is **refresh latest authorized bytes at dispatch**, not a frozen admission-time snapshot.
Artifact identity must still match admission. An older binary's canonical-only purge or a
replacement artifact cancels old work; a retry cannot recreate deleted evidence. Explicit new
admission after deletion may create a new artifact under the still-valid selected file scope.
Each attempt reuses the original capability; retries never refresh consent. Revocation,
re-selection, restore, kind changes, and OCR revision changes cannot reauthorize old work.
Text/PDF work does not depend on OCR policy. Scope/OCR drift cancels the unit; content or
canonical-version drift may retry within its existing attempt budget under that same capability.

The worker snapshots canonical locator ownership/version/state before extraction. Text/PDF/OCR
preparation holds no SQLite transaction or interactive connection mutex. Publication takes an
IMMEDIATE transaction, verifies claim/target/scope/OCR, compares the canonical snapshot, then
re-reads and hashes bounded original bytes **after** acquiring the writer lock. An intervening
foreground update cannot be replaced by the older prepared result. Artifact/version/passages,
FTS triggers, and parent `completed` result commit together; no legacy `index_jobs` checkpoint
is created. Failures leave canonical evidence intact, with no missing-state cleanup or purge.

Foreground publication, missing-source reconciliation, and durable checkpoint updates also
reserve the SQLite writer with an IMMEDIATE transaction before reading consent/job state.
Extraction still runs outside that transaction. SQLite write failures and unstable source
reads stop the request without hiding the last good evidence or advancing the failing unit's
checkpoint; retry can resume that unit. Genuine unavailable/invalid sources retain the existing
missing-source behavior. This does not enroll foreground work in the queue or bound its providers.

Artifact purge removes jobs for its exact file locators, not other artifacts sharing its root.
Root purge also removes targeted jobs when no artifact exists. OCR purge removes image targets.
This includes pending, running and terminal diagnostics, preventing resurrection and retained
operational locators. Unsupported or malformed runtime layouts block the complete deletion
transaction before canonical changes commit; the actionable failure says that no data was deleted
and names the compatible-release/owned-migration recovery gate. Resolve the runtime before
claiming a successful purge; this is application-level deletion, not secure erasure.
Current-runtime deletion requires exactly one slot-1 runtime row, a nonnegative epoch,
a positive sequence and a 0–8 priority streak. Exhausted counters remain deletable;
malformed scheduling policy alone does not block a structurally valid runtime's deletion.
Runtime v5 validates every bounded directory target before deletion, including empty manifests,
and matches an independent relational root identity/locator. A malformed directory target blocks
the transaction before canonical deletion. Inspect `jobs`, `cancel-job` the diagnostic, then
`forget-job` once terminal and retry; neither command silently authorizes another source.
Known older runtimes also refuse unexpected directory objects rather than leave private paths behind.
File-target selectors retain their earlier diagnostic contract: an unrelated malformed file target
without a recognizable locator/identity remains inspectable and can be forgotten once terminal.

Mixed-version compatibility is canonical **read** compatibility, not a complete privacy-erasure
guarantee. Older canonical-only binaries cannot remove v3 operational locators/diagnostics.
Their artifact deletion still cancels previously admitted refreshes through the identity fence,
but use the current binary's purge (or terminal-record forgetting) to erase those queue records.
An older binary reporting a successful canonical purge does not prove v3 queue metadata is gone.

Queued preparation limits extracted UTF-8 text to 2 MiB, PDF pages to 2,048, regions/passages
to 8,192, warnings to 128 entries/16 KiB serialized JSON and extractor metadata to 64 KiB.
Passage settings require a minimum 256-character target-minus-overlap gap. JSON size checking
does not allocate an unbounded serialized copy. These are **post-provider publication limits**,
not proof of native
parser/OCR peak-memory guarantees by themselves. Queued providers now run in a one-shot
`loom-extractor` process with finite wall/CPU/input/output/address-space budgets and sampled
resident/physical-footprint enforcement. See [the process boundary](EXTRACTION_PROCESS.md)
for precise limits and limitations. The bounded final byte read holds the worker's
writer transaction briefly; resource measurements remain required. SQLite and user-owned
filesystem writes cannot commit atomically, so evidence opening must still validate originals.

| Transition | Condition |
| --- | --- |
| queued/retryable → running | Due work; exclusive worker; current epoch; attempt budget remains |
| running → completed | Same claim token/epoch/target, valid capability, cancellation absent; fenced publication |
| running → retryable | Content/CAS drift, SQLite busy/locked, temporary I/O, helper launch/crash; attempts remain; persisted delay |
| running → failed | Permanent failure or exhausted attempts |
| queued/retryable → cancelled | Explicit cancellation before claim |
| running → cancelled | Durable cancellation or superseded source/OCR capability at a safe boundary |
| abandoned running → retryable/failed/cancelled | New kernel-lock owner recovers using budget/flag |

The default policy allows 128 pending jobs, 4,096 total retained rows, three attempts, a
one-second retry delay, and four high-priority claims before forcing the oldest due job.
Transient failure moves a job to the end of the FIFO order.
Priority burst accounting persists across worker exits and separate `run-next-job` invocations.
Under continual high-priority admission,
an older eligible low-priority job cannot be repeatedly bypassed by newer jobs.
Policy changes require no pending work and cannot shrink the retained bound below current use.
Library policy validation caps pending jobs at 128, rows at 4,096, attempts/bursts at eight,
and retry delay at 3,600 seconds. Error text is capped at 4 KiB and results at 64 KiB.
Provider unavailability, invalid output and resource overruns are failures, not consent
cancellations. A successful operational result includes helper wall/CPU/peak-resident metrics;
these diagnostics are excluded from portable canonical exports.

## Ownership, cancellation, and restore

Only a persistent library can acquire a worker. The worker owns one exclusive advisory file
lock next to the canonical database path (`.worker.lock`) and a separate SQLite connection.
The descriptor is never cloned and the application never deletes the lock file. Acquisition
refuses symlinks/non-regular lock files; Unix creation uses no-follow and owner-only mode.
Canonical symlink database aliases share the lock; Unix hard-linked database aliases are refused.
[fs2's locking contract](https://docs.rs/fs2/0.4.3/fs2/trait.FileExt.html) is advisory:
this coordinates cooperating processes, not a sandbox against a malicious local administrator.

There is no clock/heartbeat expiry takeover. A live owner prevents another worker, even if
slow or paused; process exit releases the descriptor. Acquisition then increments a durable
epoch and recovers abandoned running jobs. An old claim cannot publish or complete after
recovery, even if the job ID is delivered again. Each delivery gets a fresh claim token.
Normal worker acquisition does not run migrations or implicitly rebuild FTS.
Ordinary `Library::open` retains its existing migration/rebuild behavior.
The worker locks before opening SQLite. A library remembers its canonical absolute path;
Unix worker acquisition rejects a database file replaced since that library was opened.
Runtime table/index definitions and persisted policy are validated before worker acquisition.
Completion/failure captures its return value inside the settlement transaction, so concurrent
terminal forgetting cannot turn completed work into a misleading “job not found” error.

Running cancellation sets a durable flag. A transaction already publishing may complete
before a later cancellation obtains the SQLite writer lock; that completion is not rewritten.
Cancellation committed before publication prevents it. Queued file extraction probes durable
claim/consent state and interrupts/reaps its one-shot helper; see [EXTRACTION_PROCESS.md](EXTRACTION_PROCESS.md).
Synchronous foreground providers do not use that helper controller. No source bytes are deleted.

Export ignores this runtime state. Successful portable restore clears jobs and advances the
worker epoch in the canonical import transaction; a still-live old worker becomes stale.
Invalid restore rolls back both runtime and canonical changes. A fresh worker must wait for
the old descriptor to close even after restore; restore does not force takeover.

## Approved-directory contract

`enqueue-index-directory /absolute/approved-folder KEY` requires an enabled exact **directory**
root. It never grants/reselects consent. Parent approval does not turn `index_file` into a directory
adapter. A caller may explicitly select an empty folder with `index` before admitting later work.

Admission completes bounded metadata discovery before writing any queue row. The v5 operational
manifest stores one checksummed ordered unit per regular file, including unsupported media and OCR-disabled
images as explicit skips. Paths must be exact UTF-8, control-free, component-contained locators;
non-UTF-8 queued paths are refused, never lossily converted. Unix device/inode plus filesystem
birth time pins the root and each child; the child's birth time comes from descriptor-bound
discovery, not a later admission recapture. The root also pins its change timestamp. Symlinks and
special files are not followed. Non-Unix systems and filesystems without birth-time identity
are refused rather than claiming equivalent replacement/identity-reuse fencing there.

The existing discovery limits apply: at most 20,000 files (or the smaller configured bound),
65,536 entries, 4,096 directories, depth 32, 4 KiB paths, an 8 MiB retained-path budget and a
separate 8 MiB namespace-observation budget, with a cooperative five-second walk limit.
Namespace fingerprints include native entry names/types/identity and directory timestamps;
ordinary content edits are read at dispatch, not frozen at admission. Images pin the current OCR
revision, including disabled state. Overlapping artifacts owned by an exact-file or nested-directory
scope and tombstoned artifacts are refused, not reparented or reactivated. A failed admission
leaves consent and canonical evidence unchanged. Across retained manifests, transactional
admission permits at most 65,536 units and 64 MiB of encoded header/root/unit payload. A capacity
refusal rolls back the entire admission; settle or cancel pending jobs to release capacity.
This payload budget excludes SQLite pages/index overhead and source/extraction buffers.

One `run-next-job` processes one file through the same supervised byte-only extractor and budgets
as `index_file`. The descriptor-pinned read checks the actual opened root and child identity
before allocating/reading bytes or invoking extraction, and again after reading. Root replacement
between the preparation check and open cannot supply bytes to the helper. Publication rechecks
the claim, root/child identity, consent/OCR, admission artifact
identity, the canonical preparation snapshot and stable bytes under an IMMEDIATE transaction.
Canonical writes, counters, cursor advancement and non-terminal queue yield commit together.
The manifest unit is compared again during probes/publication. Successful yields append to the
queue without consuming a retry attempt; crashes and actual extraction failures still consume the
parent's bounded retry budget. `directory_progress` reports total/next units, indexed/unchanged/skipped,
bytes read and the last unit's measured helper resources. No whole-folder provider loop runs in a claim.

After the last unit, a separate claim repeats complete finite discovery. A changed namespace refuses
completion without missing-state cleanup. Descriptor-pinned directory stamps are rechecked after
obtaining SQLite's writer slot and immediately before reconciliation. The final canonical candidate
query uses validated root/locator indexes and reads at most the configured file limit plus one;
oversized historical roots fail before any missing changes. Only a fully settled, intact manifest
can mark absent prior artifacts missing and complete the parent. Unsupported/OCR-disabled units
remain seen. This is bounded **observed stability**, not an atomic filesystem/SQLite snapshot;
existing source opening still verifies original bytes against the indexed hash.

Terminal completion/failure/cancellation removes manifest paths. Root/artifact/OCR purge removes
matching pending/running work; a purge of any manifested locator conservatively invalidates the
whole directory job. Indexed locator delete, relocation/reparenting or deactivation triggers provide
a backstop for canonical-only writers, including absent → created → purged → absent admissions.
Artifact reparenting/tombstoning also invalidates manifested locators, including already-processed
units whose admission artifact was absent. Final reconciliation rechecks canonical ownership.
This assumes LOOM-managed SQLite connections with foreign keys enabled, not arbitrary raw writers
that disable integrity checks. Empty-root purge also removes targeted jobs. Restore validates runtime
definitions, explicitly clears all directory tables and advances the epoch atomically. Manifests,
queue rows, their indexes/triggers and resource diagnostics are excluded from portable exports.
Current v5 rejects unknown private runtime objects before queue work, purge or restore. Purge
validates every bounded unit's digest, shape and canonical relation without requiring source files
to remain on disk. Bulk deletion reuses that validation only within the same writer transaction.
Unit hashes detect damaged operational records; they are not protection against a hostile DB writer.

## Verification and remaining work

Core fixtures cover cross-connection duplicate admission, conflicting input, pending/retained
backpressure, all states, retry deadlines/exhaustion, priority bursts, queued/running cancellation,
stale epoch/token writes, portable exclusion/restore rollback, malformed policy, bounded
diagnostics, symlink refusal, independent connection mutexes, and an actual killed subprocess
whose lock is released and running job recovered. The real FTS fixture starts with a damaged
derivative and verifies healthy publication plus an unchanged canonical export digest.
The actual CLI fixture damages FTS, refuses a contending worker, then verifies the claimed
repair observed unhealthy input. Empty/cancelled jobs and queue-only commands leave it damaged.
Deterministic completion/failure-versus-forgetting tests retain their terminal return values.

The first adapter still performs FTS repair as one SQLite transaction. This is not proof of
bounded rebuild CPU/memory, power behavior, or interactive-search p95 under a large corpus.
The lock coordinates queued maintenance only; existing foreground writers still use their
existing SQLite/source-consent contracts, not this scheduler. Dispatch fairness is not a
wall-clock starvation bound while one unsliced maintenance operation runs.
The file and directory adapters carry claim and scope/OCR fences through canonical publication.
Directory file work yields, but admission/final discovery and the capped final reconciliation are
still bounded synchronous phases. Single-file helper limits do not prove whole-folder resource bounds.
Semantic rebuild needs staged bounded work and fenced publication.
Desktop admission, progress, durable cancellation/relaunch, and measured resource budgets
remain required before #36 can close. No ambient capture or additional source access is enabled.

`scripts/test-queued-indexing.py` exercises actual CLI text/PDF publication, malformed PDFs,
OCR-off, cancellation and purge. `--native-ocr` additionally requires real macOS Vision OCR;
`--previous-loom` proves old/v3 binary interoperability and explicit owned upgrade. Neither
option silently reports unrun coverage as passing. Core fixtures also inject prepared-provider
results to deterministically test OCR revision/purge races separately from OCR quality.
`scripts/test-directory-jobs.py` exercises separate-process directory quanta, interleaved maintenance,
explicit skips, final missing reconciliation, metrics and purge. Unit fixtures additionally cover
publication/cursor rollback, writer-wait namespace drift, actual killed workers before/after publication,
live helper interruption, scope overlap, canonical-only purge ABA, migration and restore refusal.
