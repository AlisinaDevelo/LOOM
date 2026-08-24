# 0212 merged-main device evidence

Roadmap task 0212 is evaluated against merged `main` at
`71bba194febb0c628bc48596c693cd3b4b3d55f9` (PR [#213](https://github.com/AlisinaDevelo/LOOM/pull/213),
merged 2026-08-24 20:51:39 UTC). The reproduction ran on the target Mac:

- macOS 26.6.2 (build 25G83), arm64;
- Rust/cargo 1.96.0, Node.js v26.7.0, npm 11.19.0; and
- evidence directory `/tmp/loom-0212-merged.4VmwMG`.

The semantic fixture is the checked-in rights-clean retrieval corpus (533 bytes, three passages).
No screen capture was made or retained; any image evidence used by LOOM remains a deliberately
cropped fixture rather than a full-screen screenshot.

## Acceptance mapping

| Criterion | Retained evidence | Result |
| --- | --- | --- |
| An index manifest binds model identity, tokenizer, dimensions, source-version digest, and build parameters | `semantic/summary.json`; `semantic/rebuild-first.json`; `semantic/rebuild-second.json`; `crates/loom-core/src/domain.rs`, `store.rs`; `semantic-index.log` | The merged manifest records provider `loom.hash-embedding`, model `hashed-tokens-v1`, tokenizer `unicode-alnum-lower-v1`, dimension `128`, normalization `l2`, build parameters `hash-token=1.0;hash-bigram=0.5;vector=float32-le-v1`, revision `semantic-v1`, canonical source digest `blake3:b297956c6fde80313b4db1ba6847b08e431df30a6a1365176bf63b1badddb475`, 3 canonical/indexed passages, and 1,536 vector bytes. |
| Rebuild and retirement leave canonical rows untouched and refuse incompatible manifests rather than mixing vectors | `semantic/drop.json`; `semantic/status-dropped.json`; `semantic-index.log`; `crates/loom-core/tests/semantic_index.rs` | `semantic-drop` removes only the derivative; the six focused tests cover tokenizer mismatch, build-parameter mismatch, stale passage hashes, source-version digest changes, and legacy-column upgrade. Incompatible status is unhealthy and semantic search fails closed; canonical stats remain unchanged. |
| A fixture deletes and rebuilds the semantic derivative and reproduces candidate IDs and evidence references | `semantic/summary.json`; `semantic/search-first.json`; `semantic/search-second.json`; `semantic/rebuild-first.json`; `semantic/rebuild-second.json`; `semantic-index.log` | The contract reports `rebuild_repeatable: true`, `drop_fails_closed: true`, and `evidence_bound_search: true`. First/second searches have the same passage ID, score, text anchor, and passage hash; the six-test suite also verifies repeatable candidate identity after drop/rebuild. |

## Reproduction

The merged-main commands were:

```text
git rev-parse HEAD
cargo fmt --all --check
CARGO_BUILD_JOBS=1 cargo build -p loom-cli --locked --offline
LOOM_BINARY=$PWD/target/debug/loom scripts/verify-semantic-contract.sh /tmp/loom-0212-merged.4VmwMG/semantic
CARGO_BUILD_JOBS=1 cargo test -p loom-core --test semantic_index --locked --offline -- --nocapture
```

The semantic contract passed on the merged SHA. It measured the three local provider candidates,
performed the first rebuild, timed rebuild, derivative drop, second rebuild, and evidence-bound
search. The focused Rust suite passed 6/6 tests. GitHub Actions were queued but unavailable in
this validation window; no pending workflow result is used as evidence.

A broader `cargo test -p loom-core --lib --tests` attempt was stopped by the device volume's
`ENOSPC` condition while linking unrelated test binaries. The focused semantic suite and the
CLI contract completed successfully after generated `target/` output was reclaimed. No packaged
Tauri artifact, Screen Recording permission session, or full-screen screenshot is claimed here.

## Evidence digests

Target/device metadata is retained in `environment.txt` with SHA-256
`8562ab083c1a6afe3da258a28d554f04bf87ba34a1420087157488a51d48ef64`.

```text
semantic/summary.json          9cad2a6bfd074907c2e5d81d6faf8d258593f245ee83ad04254188aee8d23b0d
semantic/rebuild-first.json    dc63eb65bed0061ef532b393934530debd4118a0ff6e14a1851a9fdafd623bce
semantic/rebuild-second.json   dc63eb65bed0061ef532b393934530debd4118a0ff6e14a1851a9fdafd623bce
semantic/search-first.json     fa04c720f9aa07237e2aa4fea016cd635d1de0b535fb9776afee330bba52b4e6
semantic/search-second.json    fa04c720f9aa07237e2aa4fea016cd635d1de0b535fb9776afee330bba52b4e6
semantic/drop.json             ac7da1eab0603083a3790ace2b4ecf662cc6e4a146753de6c7e67baa7a82b0f7
semantic-index.log             5935d6a066df453ecb97744121eb5394c3a8490d261344677e98a3dc41364dbc
cargo-fmt.log                  e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```
