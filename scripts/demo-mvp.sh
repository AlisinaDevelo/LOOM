#!/usr/bin/env bash
set -Eeuo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ $# -gt 1 ]]; then
  printf 'usage: %s [demo-directory]\n' "$0" >&2
  exit 2
fi

if [[ $# -eq 1 ]]; then
  DEMO_DIR=$1
  mkdir -p "$DEMO_DIR"
else
  DEMO_DIR=$(mktemp -d /tmp/loom-mvp-demo.XXXXXX)
fi

CORPUS="$DEMO_DIR/sources"
DATABASE="$DEMO_DIR/library.sqlite3"
mkdir -p "$CORPUS"
cp benchmarks/retrieval/v1/corpus/local-text/engineering-notes.md "$CORPUS/engineering-notes.md"
cp benchmarks/retrieval/v1/corpus/saved-web/loom-research-snapshot.md "$CORPUS/loom-research-snapshot.md"
cp benchmarks/retrieval/v1/corpus/pdf/research-page.pdf "$CORPUS/research-page.pdf"
cp benchmarks/retrieval/v1/corpus/screenshot/ocr-cropped.png "$CORPUS/ocr-cropped.png"

printf 'LOOM MVP demo corpus\n\nThe exact source is the answer.\n' > "$CORPUS/demo-notes.md"

printf 'Demo directory: %s\n' "$DEMO_DIR"
printf 'Indexing selected demo sources…\n'
cargo run --locked -q -p loom-cli -- --database "$DATABASE" index "$CORPUS"

printf '\nRecovering a text passage…\n'
cargo run --locked -q -p loom-cli -- --database "$DATABASE" search '"retry anomalies"' --limit 5

printf '\nRecovering a PDF page…\n'
cargo run --locked -q -p loom-cli -- --database "$DATABASE" search '"exact artifact recovery marker"' --limit 5

if [[ "$(uname -s)" == "Darwin" ]]; then
  printf '\nRecovering an OCR region…\n'
  cargo run --locked -q -p loom-cli -- --database "$DATABASE" search '"LOOM OCR marker"' --limit 5
  printf '\nOCR policy…\n'
  cargo run --locked -q -p loom-cli -- --database "$DATABASE" ocr-status
fi

printf '\nLibrary counts…\n'
cargo run --locked -q -p loom-cli -- --database "$DATABASE" stats

printf '\nDesktop viewer:\n'
printf '  npm run tauri -- dev\n'
printf '  choose %s with Add a folder\n' "$CORPUS"
printf '  search “retry anomalies” or “LOOM OCR marker”\n'
printf '  click View evidence, then Open original when needed\n'
printf '\nThe demo directory is retained for the viewer run; remove it when finished.\n'
