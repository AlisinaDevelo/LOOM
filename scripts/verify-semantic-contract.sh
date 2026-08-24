#!/usr/bin/env bash
set -Eeuo pipefail

# Exercise the disposable semantic derivative against the rights-clean retrieval corpus. The
# script intentionally keeps the corpus and JSON/log outputs in the caller-provided evidence
# directory so a device run can be reproduced without uploading user data.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ $# -gt 1 ]]; then
  printf 'usage: %s [evidence-directory]\n' "$0" >&2
  exit 2
fi

if [[ $# -eq 1 ]]; then
  EVIDENCE_DIR=$1
  mkdir -p "$EVIDENCE_DIR"
else
  EVIDENCE_DIR=$(mktemp -d /tmp/loom-semantic-contract.XXXXXX)
fi

CORPUS="$EVIDENCE_DIR/corpus"
DATABASE="$EVIDENCE_DIR/library.sqlite3"
mkdir -p "$CORPUS"
cp benchmarks/retrieval/v0/corpus/* "$CORPUS/"

if stat -f %z "$CORPUS/field-notes.md" >/dev/null 2>&1; then
  corpus_bytes=$(find "$CORPUS" -maxdepth 1 -type f -print0 | xargs -0 stat -f %z | awk '{sum += $1} END {print sum + 0}')
else
  corpus_bytes=$(find "$CORPUS" -maxdepth 1 -type f -print0 | xargs -0 wc -c | awk 'END {print $1 + 0}')
fi

run_cli() {
  cargo run --locked -q -p loom-cli -- --database "$DATABASE" "$@"
}

run_cli index "$CORPUS" > "$EVIDENCE_DIR/index.json"
run_cli semantic-status > "$EVIDENCE_DIR/status-before.json"
run_cli semantic-rebuild > "$EVIDENCE_DIR/rebuild-first.json"
run_cli semantic-status > "$EVIDENCE_DIR/status-after.json"
run_cli semantic-benchmark > "$EVIDENCE_DIR/provider-benchmark.json"
run_cli semantic-search "retry anomalies" --limit 5 > "$EVIDENCE_DIR/search-first.json"

if [[ -x target/debug/loom ]]; then
  binary_bytes=$(stat -f %z target/debug/loom 2>/dev/null || stat -c %s target/debug/loom)
else
  binary_bytes=0
fi

/usr/bin/time -lp cargo run --locked -q -p loom-cli -- --database "$DATABASE" semantic-rebuild > "$EVIDENCE_DIR/rebuild-timed.json" 2> "$EVIDENCE_DIR/rebuild-time.txt"
run_cli semantic-drop > "$EVIDENCE_DIR/drop.json"
run_cli semantic-status > "$EVIDENCE_DIR/status-dropped.json"
run_cli semantic-rebuild > "$EVIDENCE_DIR/rebuild-second.json"
run_cli semantic-search "retry anomalies" --limit 5 > "$EVIDENCE_DIR/search-second.json"

python3 - "$EVIDENCE_DIR" "$corpus_bytes" "$binary_bytes" <<'PY'
import json
import pathlib
import sys

evidence = pathlib.Path(sys.argv[1])
corpus_bytes = int(sys.argv[2])
binary_bytes = int(sys.argv[3])

def load(name):
    return json.loads((evidence / name).read_text())

before = load("status-before.json")
after = load("status-after.json")
dropped = load("status-dropped.json")
first = load("search-first.json")
second = load("search-second.json")
benchmark = load("provider-benchmark.json")
first_manifest = load("rebuild-first.json")["manifest"]
second_manifest = load("rebuild-second.json")["manifest"]

assert before["healthy"] is False
assert after["healthy"] is True
assert dropped["healthy"] is False
assert len(first) == len(second) > 0
assert first[0]["passage_id"] == second[0]["passage_id"]
assert first[0]["score"] == second[0]["score"]
assert first[0]["anchor"]["kind"] == "text"
assert first[0]["passage_hash"]
assert first_manifest == second_manifest
assert len(benchmark) == 3
assert all(item["sample_count"] == after["canonical_passages"] for item in benchmark)
assert all(item["vector_bytes"] > 0 for item in benchmark)

summary = {
    "corpus_bytes": corpus_bytes,
    "corpus_passages": after["canonical_passages"],
    "binary_bytes": binary_bytes,
    "provider_benchmark": benchmark,
    "first_manifest": first_manifest,
    "rebuild_repeatable": True,
    "drop_fails_closed": True,
    "evidence_bound_search": True,
}
(evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
PY

shasum -a 256 "$EVIDENCE_DIR"/*.json "$EVIDENCE_DIR"/*.txt > "$EVIDENCE_DIR/sha256.txt"
printf 'semantic evidence=%s\n' "$EVIDENCE_DIR"
