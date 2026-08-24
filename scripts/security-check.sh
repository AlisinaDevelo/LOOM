#!/usr/bin/env bash
set -Eeuo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if ! command -v gitleaks >/dev/null 2>&1; then
  printf '%s\n' 'gitleaks is required for the local secret scan' >&2
  exit 127
fi

gitleaks detect --source "$ROOT" --no-banner --redact
npm audit --audit-level=high
cargo metadata --locked --no-deps --format-version 1 >/dev/null
