#!/usr/bin/env bash
set -Eeuo pipefail

# Run the same checks that normally give the repository confidence, but retain
# target-device logs and hashes so an issue can cite the actual reproduction.

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
  EVIDENCE_DIR=$(mktemp -d /tmp/loom-device-verify.XXXXXX)
fi

COMMANDS="$EVIDENCE_DIR/commands.txt"
SUMMARY="$EVIDENCE_DIR/summary.txt"
STATUS=0

record_command() {
  printf '%s\n' "$*" >> "$COMMANDS"
}

run_step() {
  local name=$1
  shift
  local log="$EVIDENCE_DIR/$name.log"
  record_command "$*"
  printf 'RUN %-24s %s\n' "$name" "$*"
  if "$@" >"$log" 2>&1; then
    printf 'PASS %-24s %s\n' "$name" "$log"
  else
    printf 'FAIL %-24s %s\n' "$name" "$log" >&2
    STATUS=1
  fi
}

run_mixed_corpus() {
  local log="$EVIDENCE_DIR/mixed-corpus.log"
  local run_dir
  local mixed_status=0
  run_dir=$(mktemp -d /tmp/loom-device-mixed.XXXXXX)
  local corpus="$run_dir/corpus"
  local database="$run_dir/library.sqlite3"
  mkdir -p "$corpus"
  cp benchmarks/retrieval/v0/corpus/isolation.md "$corpus/isolation.md"
  printf '%s\n' 'recoverable device marker' > "$corpus/recovery.md"
  printf '\377\000\001\002' > "$corpus/unsupported.bin"
  truncate -s 8388609 "$corpus/oversized.md"
  printf '%s\n' 'must never enter selected root' > "$run_dir/outside.md"
  ln -s "$run_dir/outside.md" "$corpus/outside-link.md"

  record_command "mixed corpus at $run_dir"
  {
    printf 'run_dir=%s\n' "$run_dir"
    cargo run --locked -q -p loom-cli -- --database "$database" index "$corpus"
    cargo run --locked -q -p loom-cli -- --database "$database" search '"retry anomalies"' --limit 5
    if cargo run --locked -q -p loom-cli -- --database "$database" search '"must never enter selected root"' --limit 5 | grep -q 'must never enter selected root'; then
      printf 'outside-root symlink was indexed\n' >&2
      return 1
    fi
    printf '%s\n' 'outside-root symlink remained unreachable'
    printf '%s\n' 'recovered after bounded failure' > "$corpus/oversized.md"
    cargo run --locked -q -p loom-cli -- --database "$database" index "$corpus"
    cargo run --locked -q -p loom-cli -- --database "$database" search '"recovered after bounded failure"' --limit 5
    cargo run --locked -q -p loom-cli -- --database "$database" stats
  } >"$log" 2>&1 || mixed_status=1
  if [[ "$mixed_status" -ne 0 ]]; then
    STATUS=1
  fi
  printf 'MIXED %-23s %s\n' "$([ "$mixed_status" -eq 0 ] && printf PASS || printf FAIL)" "$log"
}

mkdir -p "$EVIDENCE_DIR"
: > "$COMMANDS"
printf 'LOOM target-device verification\n' > "$SUMMARY"
printf 'root=%s\n' "$ROOT" >> "$SUMMARY"
sw_vers >> "$SUMMARY"
printf 'architecture=' >> "$SUMMARY"
uname -m >> "$SUMMARY"
rustc --version --verbose >> "$SUMMARY"
cargo --version >> "$SUMMARY"
rustc +1.88.0 --version --verbose >> "$SUMMARY"
cargo +1.88.0 --version >> "$SUMMARY"
node --version >> "$SUMMARY"
npm --version >> "$SUMMARY"
git rev-parse HEAD >> "$SUMMARY"

run_step fmt cargo fmt --all --check
run_step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
run_step rust-workspace cargo test --workspace --locked
run_step rust-msrv-check cargo +1.88.0 check --workspace --all-targets --locked
run_step rust-msrv-tests cargo +1.88.0 test -p loom-core --lib --tests -- --nocapture
run_step npm-install npm ci
run_step npm-check npm run check
run_step retrieval-benchmark cargo run --locked -q -p loom-cli -- benchmark --corpus benchmarks/retrieval/v0/corpus --queries benchmarks/retrieval/v0/queries.jsonl
run_step retrieval-benchmark-v1 cargo run --locked -q -p loom-cli -- benchmark --corpus benchmarks/retrieval/v1/corpus --queries benchmarks/retrieval/v1/queries.jsonl
run_step semantic-contract bash scripts/verify-semantic-contract.sh "$EVIDENCE_DIR/semantic"
run_step security-check bash scripts/security-check.sh
run_step tauri-build npm run tauri build -- --debug --no-bundle
run_mixed_corpus

printf '\nstatus=%s\n' "$([ "$STATUS" -eq 0 ] && printf PASS || printf FAIL)" >> "$SUMMARY"
shasum -a 256 "$EVIDENCE_DIR"/*.log > "$EVIDENCE_DIR/log-sha256.txt" 2>/dev/null || true
printf 'EVIDENCE_DIR=%s\n' "$EVIDENCE_DIR"
exit "$STATUS"
