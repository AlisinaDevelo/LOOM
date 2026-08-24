# v0.1 activation and recovery gate

This is the decision contract for the exact-source recovery wedge. The numbers below are planning
hypotheses, not measured product claims. The gate stays `measurement_status: not_run` until the
rights-clean benchmark and a consented 12–20 participant study produce retained evidence.

The machine-readable source is [`benchmarks/retrieval/v0/gate.json`](../benchmarks/retrieval/v0/gate.json).
The fixture is synthetic CC0 text/Markdown; it never receives private participant content.

## Numeric thresholds

| Measure | Advance threshold | Evidence source |
| --- | ---: | --- |
| Exact-source Recall@1 | ≥ 0.80 | Held-out rights-clean benchmark report |
| Exact-source Recall@5 | ≥ 0.95 | Held-out rights-clean benchmark report |
| Evidence-open success | ≥ 0.90 | Consent-safe task worksheet; source opens with the returned artifact/version/hash tuple |
| Query p95 latency | ≤ 1,000 ms | Target-device benchmark report |
| Index completeness | ≥ 0.98 | Benchmark/index health report; failures and skipped inputs are separate |
| No-result disclosure | 1.00 | Negative-query worksheet; no unsupported answer is shown as a result |
| Completed participants | ≥ 8 of 12–20 | Privacy-safe participant worksheet |
| Returning participants | ≥ 2 in a later week | Privacy-safe participant worksheet |

The current v0 smoke fixture reports Recall@1/5 1.0, anchor precision 1.0, false-positive rate
0.0, completeness 1.0, and sub-second p95 on the target Mac. Those are reproducibility
observations for three synthetic local-text queries, not evidence that the activation gate has
passed.

## Decision rules

- **Advance:** every numeric threshold passes; at least eight participants complete the known-item
  task set; at least two return unaided; and no critical privacy, source-integrity, unanchored-result,
  or data-loss issue remains open.
- **Narrow:** source integrity and the rights-clean benchmark remain safe, but activation, latency,
  capture friction, or a non-critical cohort slice misses. Publish the failure, reduce scope, and
  rerun before adding formats, passive capture, sync, or synthesis.
- **Stop:** any critical privacy/source-integrity defect, fabricated or unanchored result,
  unrecoverable data loss, or rights-clean benchmark failure blocks the next expansion. Preserve the
  export and decision record; stopping is a valid product outcome.

## Privacy-safe study worksheet

Use [`docs/studies/v0.1-participant-worksheet.md`](studies/v0.1-participant-worksheet.md) for
12–20 Mac design partners. Record only pseudonymous participant IDs, aggregate task metrics, and
failure classes. Never paste source text, screenshots, URLs, credentials, raw queries, or private
documents into the repository or study export. Participants may withdraw their row and any local
study artifacts at any time.

## Claim traceability

- Current supported behavior is documented in [`README.md`](../README.md),
  [`docs/EVALUATION.md`](EVALUATION.md), and the checked-in device evidence artifacts.
- Product hypotheses and future formats are labeled as plans in [`docs/PRODUCT.md`](PRODUCT.md)
  and [`docs/ROADMAP.md`](ROADMAP.md).
- Comparative market or quality claims require a held-out benchmark and are not inferred from the
  three-query smoke fixture.
