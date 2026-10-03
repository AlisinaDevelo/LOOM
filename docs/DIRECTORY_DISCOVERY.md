# Bounded selected-folder discovery

Foreground `index`/folder-picker ingestion now enumerates without buffering and sorting an
unbounded directory. It still collects a finite regular-file list before extraction and still
uses the existing synchronous `index_jobs` checkpoints. Directory jobs, resumable manifests,
semantic scheduling and desktop background controllers are not implemented by this change.
It is one prerequisite of [#36](https://github.com/AlisinaDevelo/LOOM/issues/36), not its completion.

## Bounds and failure behavior

| Boundary | Default |
| --- | --- |
| Regular files, including unsupported extensions | 20,000; existing LibraryLimits may lower/change this |
| All entries, including symlinks and special files | 65,536; `.` and `..` excluded |
| Directories, including the selected root | 4,096 |
| Directory depth | 32 below the root |
| One path, measured as platform-encoded bytes | 4,096 bytes |
| Retained file and visited-directory path payload | 8 MiB |
| Cooperative elapsed discovery time | 5 seconds |

Every bound is checked before adding another retained record. Component/path length is checked
before constructing a child pathname. Directory count and depth bound the explicit DFS stack
and its open streams; path payload bounds are not a hard whole-process RSS cap. Native directory
stream buffers, vector/record overhead, sorting, SQLite and extraction have separate costs.

A limit, I/O error, probe refusal or detected directory mutation returns an error, not a
truncated successful list. Discovery finishes before `start_index_job`, so failure cannot create
or advance a checkpoint, publish any discovered artifact or reconcile unseen sources as missing.
Discovery observes the prior exact scope and library incarnation without creating or re-enabling
a root. Only successful enumeration may persist selection, in a writer transaction that verifies
the observation is unchanged. A failed first selection creates no approved root; a failed
reselection leaves prior consent and last-seen metadata unchanged. Revocation, purge, restore or
scope replacement during the walk refuses the old selection. Root purge also rotates a
selection-only revision to detect a select-and-purge cycle that begins and ends without a root;
this conservatively rejects any selection staged before a root purge, but does not invalidate
unrelated already-admitted jobs. Approved watcher scans never grant consent.
File extraction failures after a complete walk retain the existing per-file outcome behavior.
Foreground discovery currently passes a no-op probe: `IndexCancellationToken` is still checked
between extraction units, not during discovery. No folder-cancellation responsiveness or exact
cancelled-unit count before membership is known is claimed.

Time is checked between filesystem calls and bounded sorting. A blocked filesystem call cannot
be interrupted by this walker; five seconds is a cooperative budget, not an OS-preempted deadline.
There is no claim of search responsiveness, battery behavior or hard peak memory for a folder job.

## Unix traversal and portability

macOS/Unix use the safe `rustix` directory API. The selected root is opened with directory,
no-follow and close-on-exec flags. Each child directory is opened relative to its pinned parent
descriptor by one filename component, never by an unchecked multi-component path. Symlinks and
special files count as entries but are never opened or followed. Device/inode plus directory
mtime/ctime are compared before/after enumeration, at child open, and again for visited directories
reopened component-by-component from the original root. The root path and canonical binding are
checked at return. Descriptor ownership is bounded by depth and released on all exits.
Unix visited-directory records retain relative path payload; the path budget charges the bytes
actually retained, not a fully qualified pathname for every directory.

The initial root open still resolves ancestor path components. `O_NOFOLLOW` protects its final
component, not a hostile ancestor swap before the root is pinned. Identity and canonical-binding
checks detect some rebinding but do not make the current path-based foreground authorization a
filesystem sandbox. The descriptor boundary applies to traversal after root pinning.

These checks abort **detected** changes. Filesystem metadata semantics/granularity and changes
after a final check can escape detection: there is no atomic filesystem snapshot. Regular-file
contents are not read or frozen during discovery. The returned `Vec<PathBuf>` does not carry
directory descriptors into extraction; later consent, no-follow byte reads, original/hash,
canonical identity and publication fences remain necessary.

Non-Unix core builds use bounded `ReadDir` with path/timestamp checks. That fallback is explicitly
weaker: it is not descriptor-pinned and not a sandbox against an adversarial ancestor replacement.
The macOS app uses the Unix implementation. Compiling/exercising the fallback on macOS does not
establish Windows ABI or platform behavior.

Directory iteration order is filesystem-defined; it is not persisted as an ordinal/cookie.
Only after a complete bounded walk are file paths sorted by native components, reproducing
the former filename-sorted DFS ordering. No lossy string conversion is used for ordering or
fingerprinting. Checkpoints hash length-prefixed native path bytes, the discovery version and
all bounds, plus OCR policy where relevant. An older fingerprint or changed bound resets the
existing checkpoint rather than resuming past possibly different work. Same-contract interrupted
checkpoints retain their existing idempotent resume behavior.

## Verification

Unit fixtures cover ordering against the prior DFS, all bound classes, unsupported/link/socket
entry counting, callback refusal, root/child replacement and mutation after child EOF. A real
subprocess measures live descriptors and repeats error paths. Native non-UTF-8 path hashing and
component sorting are tested; APFS refuses physically creating those invalid-byte filenames, so
that physical-name fixture is enabled only on other Unix targets, not claimed as a macOS test.

Integration fixtures retain canonical evidence/checkpoints across an incomplete discovery and
exercise legacy-fingerprint/bound changes. `scripts/test-directory-discovery.py` drives actual
CLI refusals with 33 nested directories, 4,096 child directories, 20,000 unsupported files and
65,536 links; it records the limit that actually fires, unchanged artifact/version/locator/passage/
checkpoint rows and
macOS process peak RSS. All content is synthetic. The target-device runner includes this fixture.
No private screenshot is captured; the existing OCR runner uses its rights-clean cropped region.

The large-library performance harness indexes the existing 20,000-file generation shards as
explicit disjoint roots into one library. Its version 2 report retains every batch receipt and
aggregate full-library counts; discovery bounds remain unchanged. This measures a 100k library,
not one 100k folder admission or durable background directory scheduling.
