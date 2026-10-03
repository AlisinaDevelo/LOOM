#!/usr/bin/env bash
set -Eeuo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ ! -f "$ROOT/.cargo/audit.toml" ]]; then
  printf '%s\n' 'the tracked Cargo advisory policy is required; refusing user defaults' >&2
  exit 2
fi

if ! command -v gitleaks >/dev/null 2>&1; then
  printf '%s\n' 'gitleaks is required for the local secret scan' >&2
  exit 127
fi

# A metadata check proves lockfile consistency, not freedom from advisories.
# Allow a task-local audit binary/database without changing the checks or ignoring findings.
AUDIT_BINARY=${LOOM_CARGO_AUDIT_BIN:-cargo-audit}
if ! command -v "$AUDIT_BINARY" >/dev/null 2>&1; then
  printf '%s\n' 'cargo-audit is required for the local Rust advisory scan' >&2
  exit 127
fi

gitleaks detect --source "$ROOT" --no-banner --redact
npm audit --audit-level=high
cargo metadata --locked --no-deps --format-version 1 >/dev/null

AUDIT_ARGS=(audit --file Cargo.lock --deny warnings)
if [[ -n "${LOOM_CARGO_AUDIT_DB:-}" ]]; then
  AUDIT_ARGS+=(--db "$LOOM_CARGO_AUDIT_DB")
fi
"$AUDIT_BINARY" "${AUDIT_ARGS[@]}"
