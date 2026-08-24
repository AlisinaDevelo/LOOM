# LOOM adversarial PDF corpus

This corpus is synthetic and released under CC0-1.0 for reproducible extractor
testing. It contains no user documents, copied publications, external fonts, or
screen captures. Every byte is emitted by
[`scripts/generate-pdf-adversarial-fixtures.py`](../../scripts/generate-pdf-adversarial-fixtures.py)
using the checked-in `loom-pdf-adversarial-v1` generator.

The seven classes are deliberately small and isolated:

- `tagged_text`: marked-content text with a logical structure tree;
- `multi_column`: two reading lanes on one page;
- `ligature`: a synthetic `fi` ligature marker in the text layer;
- `rotated_page`: a page carrying `/Rotate 90` and a recoverable marker;
- `encrypted`: a deterministic `/Encrypt` marker that must be rejected before parsing;
- `malformed`: truncated PDF bytes that must fail closed; and
- `image_only`: a valid page containing pixels but no text layer.

The manifest records a SHA-256 byte hash, expected indexed/unsupported outcome,
extractor identity/version, page identity, warnings, and a marker used by the
runner to verify source-backed recovery. Regenerate the bytes only when changing
the generator version and update the manifest deliberately:

```bash
python3 scripts/generate-pdf-adversarial-fixtures.py
```

The corpus runner is intentionally separate from the ordinary retrieval
benchmark because three classes are expected to be unsupported. It indexes one
selected corpus root, validates every byte hash before interpreting output,
checks the expected page/warning/extractor contract for indexed fixtures, checks
the explicit failure class for unsupported fixtures, and emits per-class
completeness/failure counts.
