# v0.1 activation and recovery gate

This is the decision contract for the exact-source recovery wedge. The numbers below are planning
hypotheses, not measured product claims. The gate stays `measurement_status: not_run` until the
rights-clean benchmark and a consented 12–20 participant study produce retained evidence.

The machine-readable source is [`benchmarks/retrieval/v0/gate.json`](../benchmarks/retrieval/v0/gate.json).
The fixture is synthetic CC0 text/Markdown; it never receives private participant content.

## Numeric thresholds

| Measure | Advance threshold | Evidence source |
| --- | ---: | --- |
| First-session activation | ≥ 0.70 | Activated participants / eligible setup participants (defined below) |
| Exact-source Recall@1 | ≥ 0.80 | Held-out rights-clean benchmark report |
| Exact-source Recall@5 | ≥ 0.95 | Held-out rights-clean benchmark report |
| Evidence-open success | ≥ 0.90 | Consent-safe task worksheet; source opens with the returned artifact/version/hash tuple |
| Query p95 latency | ≤ 1,000 ms | Target-device benchmark report |
| Index completeness | ≥ 0.98 | Benchmark/index health report; failures and skipped inputs are separate |
| No-result disclosure | 1.00 | Negative-query worksheet; no unsupported answer is shown as a result |
| Eligible cohort | ≥ 12 retained setup participants, 12–20 enrolled | Privacy-safe participant worksheet |
| Completed participants | ≥ 8 of 12–20 enrolled | Privacy-safe participant worksheet |
| Returning participants | ≥ 2 in a later week | Privacy-safe participant worksheet |

An **activated participant** completes consented corpus setup, independently recovers at least one
known item, and opens its matching source evidence during the first session. The denominator includes
every consented participant with retained study data who starts setup, including setup and retrieval
failures. An explicit consent withdrawal or study-data deletion request received before the recorded
report cutoff removes that participant from **both numerator and denominator**, with all their
measurements excluded and the withdrawn count disclosed separately. A later request invalidates an
earlier advance decision until the aggregates and cohort eligibility are recalculated and a
superseding decision is recorded. At least 12 participants must remain eligible from 12–20 enrolled;
withdrawals cannot manufacture a pass from an undersized cohort. Finishing the study or returning
later is not a substitute for activation.

No-result disclosure uses a separate negative-query task set: the numerator counts completed negative
queries with zero unsupported results and an explicit no-match disclosure; the denominator counts
**all completed negative queries**, including failed disclosures and unsupported-result failures.
Record issued and completed negative queries separately from positive known-item tasks. A zero
denominator for either rate is **not measured**, never 0% or 100%; the advance decision is ineligible.

The current v0 smoke fixture reports Recall@1/5 1.0, anchor precision 1.0, false-positive rate
0.0, completeness 1.0, and sub-second p95 on the target Mac. Those are reproducibility
observations for three synthetic local-text queries, not evidence that the activation gate has
passed.

## Decision rules

- Apply **stop first**, then advance if every condition passes, otherwise narrow. No participant
  decision or activation result is claimed while measurement status is `not_run`.
- **Advance:** no stop condition applies; 12–20 participants enrolled with at least 12 eligible
  setup participants remaining; every numeric threshold passes with nonzero denominators, including
  70% first-session activation; at least eight participants complete the known-item
  task set; at least two return unaided; and no critical privacy, source-integrity, unanchored-result,
  or data-loss issue remains open.
- **Narrow:** no stop condition applies, but any numeric threshold misses or is not measured,
  the cohort is ineligible, or capture friction prevents activation. Publish missing evidence or the
  failure, reduce scope, and rerun before adding formats, passive capture, sync, or synthesis.
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
