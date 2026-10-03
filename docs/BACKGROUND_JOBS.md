# Durable background work: first adapter

Roadmap `0400` / [#36](https://github.com/AlisinaDevelo/LOOM/issues/36) is not complete.
This is an opt-in durable queue with a real FTS-repair adapter, not desktop background indexing.
The existing synchronous ingestion/OCR/semantic commands remain unchanged.

## Using the current adapter

```sh
cargo run --locked -p loom-cli -- enqueue-fts-repair repair-after-import --low
cargo run --locked -p loom-cli -- jobs
cargo run --locked -p loom-cli -- run-next-job
cargo run --locked -p loom-cli -- cancel-job JOB_ID
cargo run --locked -p loom-cli -- forget-job TERMINAL_JOB_ID
```

Commands use the same `--database` option as other CLI operations. `run-next-job` performs
at most one due repair, returning its durable result or JSON `null` when no job is due.
It does not wait for delayed retries or launch a daemon. Repair publishes its derived FTS5
projection and `completed` result in the same epoch/token/cancellation-fenced transaction.
Canonical originals, hashes, passages, and anchors are not rewritten.
Queue commands require an existing initialized library; initialize or migrate it explicitly
with an ordinary command such as `stats` first. Queue inspection/admission/cancellation/forgetting
never implicitly rebuild FTS. An empty or contending `run-next-job` cannot repair the projection.

## Operational contract

`background_jobs` and singleton `background_job_runtime` are operational schema-10 additions,
not portable canonical records. They contain no new source permissions or arbitrary executable
payloads; the only admitted operation is `fts_repair`. A key is 1–128 ASCII identifier bytes.
Repeating a key with identical operation/priority returns its original row, including a terminal
row. Conflicting input is rejected without changing it. Terminal keys are not silently evicted.
At the retained-record limit, explicitly forgetting a terminal record frees capacity and forgets
its key; a later request with that key becomes new work. Pending/running jobs cannot be forgotten.
The separate `background_job_schema_version` marker is runtime-only. Version 2 enforces
UTF-8 byte bounds and valid result JSON; ordinary opening transactionally upgrades the first
unversioned queue layout, preserving job rows, policy, epoch, sequence, and priority accounting.
Invalid legacy diagnostics roll back that upgrade: canonical evidence still opens, no jobs are
dropped, and queue commands refuse the unmigrated runtime. Future unknown versions are refused.

| Transition | Condition |
| --- | --- |
| queued/retryable → running | Due work; exclusive worker; current epoch; attempt budget remains |
| running → completed | Same claim token/epoch, cancellation absent, fenced repair publishes |
| running → retryable | SQLite busy/locked or temporary I/O; attempts remain; persisted delay |
| running → failed | Permanent failure or exhausted attempts |
| queued/retryable → cancelled | Explicit cancellation before claim |
| running → cancelled | Durable cancellation acknowledged at a safe transaction boundary |
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
The worker connection does not run migrations or implicitly rebuild FTS on acquisition.
Ordinary `Library::open` retains its existing migration/rebuild behavior.
The worker locks before opening SQLite. A library remembers its canonical absolute path;
Unix worker acquisition rejects a database file replaced since that library was opened.
Runtime table/index definitions and persisted policy are validated before worker acquisition.
Completion/failure captures its return value inside the settlement transaction, so concurrent
terminal forgetting cannot turn completed work into a misleading “job not found” error.

Running cancellation sets a durable flag. A transaction already publishing may complete
before a later cancellation obtains the SQLite writer lock; that completion is not rewritten.
Cancellation committed before publication prevents the repair. No source bytes are deleted.

Export ignores this runtime state. Successful portable restore clears jobs and advances the
worker epoch in the canonical import transaction; a still-live old worker becomes stale.
Invalid restore rolls back both runtime and canonical changes. A fresh worker must wait for
the old descriptor to close even after restore; restore does not force takeover.

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
Source indexing/OCR must carry both the queue claim and existing scope/OCR fences through
every canonical write. Semantic rebuild needs staged bounded work and fenced publication.
Desktop admission, progress, durable cancellation/relaunch, and measured resource budgets
remain required before #36 can close. No ambient capture or additional source access is enabled.
