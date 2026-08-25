#!/usr/bin/env bash
set -Eeuo pipefail

# Run the same checks that normally give the repository confidence, but retain
# target-device logs and hashes so an issue can cite the actual reproduction.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

# Keep the device runner reproducible on a small development disk. These defaults
# only change disposable build artifacts; callers can override them explicitly.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"

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
    loom_cli "$database" index "$corpus"
    loom_cli "$database" search '"retry anomalies"' --limit 5
    if loom_cli "$database" search '"must never enter selected root"' --limit 5 | grep -q 'must never enter selected root'; then
      printf 'outside-root symlink was indexed\n' >&2
      return 1
    fi
    printf '%s\n' 'outside-root symlink remained unreachable'
    printf '%s\n' 'recovered after bounded failure' > "$corpus/oversized.md"
    loom_cli "$database" index "$corpus"
    loom_cli "$database" search '"recovered after bounded failure"' --limit 5
    loom_cli "$database" stats
  } >"$log" 2>&1 || mixed_status=1
  if [[ "$mixed_status" -ne 0 ]]; then
    STATUS=1
  fi
  printf 'MIXED %-23s %s\n' "$([ "$mixed_status" -eq 0 ] && printf PASS || printf FAIL)" "$log"
}

loom_cli() {
  "$LOOM_BINARY" --database "$1" "${@:2}"
}

clear_rust_outputs() {
  # The target-device runner exercises both Rust and Tauri. Keep their debug
  # artifacts sequential so a small development disk does not turn a later
  # test into an ENOSPC false negative.
  if [[ -d "$ROOT/target" ]]; then
    find "$ROOT/target" -type f -delete
    find "$ROOT/target" -type l -delete
    find "$ROOT/target" -depth -type d -empty -delete
  fi
}

stage_cli_binary() {
  local staged_binary="$EVIDENCE_DIR/loom"
  cp "$ROOT/target/debug/loom" "$staged_binary"
  chmod +x "$staged_binary"
  export LOOM_BINARY="$staged_binary"
  printf 'staged=%s\n' "$staged_binary"
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
run_step clear-clippy-target clear_rust_outputs
run_step rust-workspace cargo test --workspace --locked
run_step clear-stable-target clear_rust_outputs
run_step rust-msrv-check cargo +1.88.0 check --workspace --all-targets --locked
run_step clear-msrv-check-target clear_rust_outputs
run_step rust-msrv-tests cargo +1.88.0 test -p loom-core --lib --tests -- --nocapture
run_step clear-msrv-test-target clear_rust_outputs
run_step performance-build cargo build --locked -q -p loom-cli
run_step stage-cli-binary stage_cli_binary
run_step clear-rust-target clear_rust_outputs
run_step retrieval-benchmark "$EVIDENCE_DIR/loom" benchmark --corpus benchmarks/retrieval/v0/corpus --queries benchmarks/retrieval/v0/queries.jsonl
run_step retrieval-benchmark-v1 "$EVIDENCE_DIR/loom" benchmark --corpus benchmarks/retrieval/v1/corpus --queries benchmarks/retrieval/v1/queries.jsonl
run_step pdf-adversarial python3 scripts/pdf-adversarial.py --loom "$EVIDENCE_DIR/loom"
run_step hybrid-ablation python3 scripts/hybrid-ablation.py
run_step semantic-contract bash scripts/verify-semantic-contract.sh "$EVIDENCE_DIR/semantic"
run_step performance-budget-tests python3 scripts/test-performance-budget.py
run_mixed_corpus
run_step performance-budget python3 scripts/performance-budget.py --evidence-dir "$EVIDENCE_DIR/performance" --loom "$EVIDENCE_DIR/loom"
run_step accessibility-contract python3 scripts/test-accessibility-contract.py
run_step npm-install npm ci
run_step npm-check npm run check
run_step security-check bash scripts/security-check.sh
run_step tauri-build npm run tauri build -- --debug --no-bundle

printf '\nstatus=%s\n' "$([ "$STATUS" -eq 0 ] && printf PASS || printf FAIL)" >> "$SUMMARY"
shasum -a 256 "$EVIDENCE_DIR"/*.log > "$EVIDENCE_DIR/log-sha256.txt" 2>/dev/null || true
printf 'EVIDENCE_DIR=%s\n' "$EVIDENCE_DIR"
exit "$STATUS"
