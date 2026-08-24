# LOOM Roadmap

Program window: **2026-08-23 through 2031-08-23**.

LOOM is planned as an evidence-first, local-first recovery system: recover the original source
object, show why it matched, and make uncertainty visible. The current implementation is a
pre-alpha text/Markdown/PDF-text/image-OCR slice on macOS. Browser capture, semantic retrieval,
sync, mobile, cross-platform, and professional workflows below are plans, not supported behavior.

The managed program contains **20 rolling quarterly milestones**, **154 active outcome-oriented
issues**, **13 product-phase epics**, **141 native parent/sub-issue links**, and **314 prerequisite
edges**. Four overlapping planning issues were consolidated before implementation and remain as
sanitized, closed historical records. The tracked [roadmap manifest](../roadmap/roadmap.json) is the
public machine-readable contract; GitHub Issues and milestones are the execution state.

The validator rejects missing acceptance criteria, ambiguous placement, unknown prerequisites,
cycles, later-quarter blockers, generic outcomes, missing labels, and private routing metadata.

## Execution contract

Every active managed issue has:

- a stable four-digit roadmap ID that survives GitHub renumbering;
- one concrete outcome and two to four measurable acceptance criteria;
- exactly one quarterly milestone, product phase, type, priority, and horizon;
- at least one product area;
- a phase parent and explicit prerequisites where applicable;
- required closure evidence: the tests, fixtures, measurements, review artifacts, or consented
  study evidence actually produced.

The v0.1 activation and recovery decision contract is [ACTIVATION_GATE.md](ACTIVATION_GATE.md).
Its numeric thresholds are hypotheses until the rights-clean fixture and privacy-safe participant
worksheet produce measurements; a narrow or stop decision is valid.

An issue is not complete because work was attempted. It is complete when its acceptance evidence
exists. A quarter may continue, narrow, reorder, or stop later work; long-horizon items are options,
not promises.

## Twenty-quarter program

| Quarter | Window | Milestone | Outcome and exit gate |
| --- | --- | --- | --- |
| **Q01** | 2026-08-23 – 2026-11-22 | Thread — Local contract | Make exact local-text recovery operational. Public repository, canonical schema, explicit ingest, exact lexical evidence, desktop flow, benchmark, and CI work end to end. |
| **Q02** | 2026-11-23 – 2027-02-22 | Hem — Durable indexing | Make the index recoverable and revocable. Crash recovery, reconciliation, migrations, selected-root access, cancellation, and FTS repair converge without stale searchable sources. |
| **Q03** | 2027-02-23 – 2027-05-22 | Needle — Multimodal extraction | Add deterministic PDF and image extraction. Page/region anchors, evidence viewing, and the rebuildable semantic-index contract are reproducible. |
| **Q04** | 2027-05-23 – 2027-08-22 | Lens — Evaluated retrieval | Measure hybrid retrieval, accessibility, and resources. Multimodal fixtures, confidence states, filters, screenshot capture, and the 100k-artifact budget are reported. |
| **Q05** | 2027-08-23 – 2027-11-22 | Pattern — Intentional capture | Define least-privilege browser capture. The proposed [browser protocol](protocol/browser-capture-v1.md) binds explicit saves, provenance fields, authenticated envelopes, and best-effort snapshot states before extension implementation. |
| **Q06** | 2027-11-23 – 2028-02-22 | Shuttle — Provenance alpha | Prove portable provenance with design partners. Export/backup round trips, graph bounds, connector replay, retention controls, and privacy-safe alpha evidence pass. |
| **Q07** | 2028-02-23 – 2028-05-22 | Weave — Daily-driver core | Build a durable release candidate. Jobs, versioning, deduplication, recovery, SDK boundaries, and private ranking meet integrity and resource gates. |
| **Q08** | 2028-05-23 – 2028-08-22 | Loom — Release readiness | Earn a supported macOS release. Independent review, v1 quality and value, API, integrations, restore drills, signed artifacts, adoption, and pricing gates pass. |
| **Q09** | 2028-08-23 – 2028-11-22 | Tapestry — Private continuity | Specify optional continuity. E2EE, device identity, conflict behavior, iOS scope, and local-model isolation are reviewed while local-only use remains complete. |
| **Q10** | 2028-11-23 – 2029-02-22 | Bobbin — Gated experiments | Test connectors, bounded capture, and obligation workflows. Each publishes a continue, narrow, or stop decision with privacy, support, and cost evidence. |
| **Q11** | 2029-02-23 – 2029-05-22 | Fabric — Platform boundaries | Define portable platform, collection, and extension contracts. Compatibility, consent, capability, revocation, deletion, and unsupported states are explicit. |
| **Q12** | 2029-05-23 – 2029-08-22 | Selvage — Public evidence | Publish synthesis, provenance, professional-workflow, benchmark, HCI, adoption, market, and operating-economics evidence for the next strategy decision. |
| **Q13** | 2029-08-23 – 2029-11-22 | Atlas — Portability | Make libraries portable. Interchange, identity, encrypted backup, sync observability, language coverage, private ranking, migration, and offline packaging pass. |
| **Q14** | 2029-11-23 – 2030-02-22 | Observatory — Temporal evidence | Make time and uncertainty inspectable. Temporal provenance, snapshots, explanation, multilingual ranking, calibration, correction, and privacy-safe diagnostics pass. |
| **Q15** | 2030-02-23 – 2030-05-22 | Commons — Interoperability | Open a constrained ecosystem. Public interchange, extension containment, connector conformance, benchmark v2, red-team governance, and provenance portability pass. |
| **Q16** | 2030-05-23 – 2030-08-22 | Relay — Optional transport | Admit transport only if demand justifies it. Authority, rendezvous, metering, keys, conflicts, restore, endpoint security, and lifecycle tests pass; stopping remains valid. |
| **Q17** | 2030-08-23 – 2030-11-22 | Stewardship — Professional assurance | Bound retention, audit, integrity, and incident controls. Independent review passes without legal, forensic, or archival completeness claims. |
| **Q18** | 2030-11-23 – 2031-02-22 | Horizon — Robust retrieval research | Measure distribution shift, cross-language retrieval, uncertainty UX, citation graphs, model budgets, privacy-safe learning, and longitudinal human factors reproducibly. |
| **Q19** | 2031-02-23 – 2031-05-22 | Field Study — Five-year retrospective | Audit architecture, support, sustainability, maintainership, and standards. Publish an evidence-backed draft future for every workstream. |
| **Q20** | 2031-05-23 – 2031-08-23 | Five-Year Review — Sustainability | Revalidate market and research value, willingness to adopt, funding, stewardship, and benchmark evidence; choose continue, narrow, archive, or fork. |

Dates are planning windows, not delivery promises. A quarterly gate records thresholds before
measurement, the fixture or participants, environment, result, uncertainty, and decision.

## Product-phase layer

Quarterly milestones are the execution clock. The 13 named phases remain parent epics and preserve
the product narrative across quarters.

| Phase | Quarters | Product boundary |
| --- | --- | --- |
| v0.1 Thread | Q01–Q02 | Exact local-text recovery and index reliability |
| v0.2 Needle | Q03–Q04 | Page/region evidence and evaluated hybrid retrieval |
| v0.3 Pattern | Q05–Q06 | Intentional capture, provenance, privacy, and alpha evidence |
| v1.0 Weave | Q07–Q08 | Recoverable, reviewed, signed macOS daily driver |
| v1.x Tapestry | Q09–Q10 | Optional continuity and measured experiments |
| v2.0 Fabric | Q11–Q12 | Portable platform boundaries and public evidence |
| v2.1 Atlas | Q13 | Interchange, backup, migration, and private personalization |
| v2.2 Observatory | Q14 | Temporal provenance, explanation, and calibration |
| v2.3 Commons | Q15 | Open interchange and constrained extension ecosystem |
| v3.0 Relay | Q16 | Optional continuity transport behind a demand gate |
| v3.1 Stewardship | Q17 | Professional policy and assurance boundaries |
| v3.2 Horizon | Q18 | Robust retrieval and longitudinal HCI research |
| v3.3 Five-Year Review | Q19–Q20 | Retrospective, sustainability, and program decision |

Each phase epic is blocked by the preceding phase gate. Every non-epic issue belongs to exactly one
phase parent. Native GitHub blocked-by and sub-issue relationships—not this prose—carry the managed
execution graph.

## Program outcomes

| Outcome | Evidence required before expansion |
| --- | --- |
| Exact-source recovery | Held-out Recall@1/5/10, anchor precision, evidence-open success, index completeness, false-positive rate, and explicit no-result behavior by source class. |
| Local reliability | Restart, cancellation, migration, disk-full, permission-revocation, watcher-overflow, WAL/journal, backup/restore, and derivative-repair drills preserve canonical identity or fail closed. |
| Privacy and security | Capability, retention, deletion, cache/log/WAL cleanup, connector, extension, model, sync, and incident tests have no unowned critical/high findings at a release gate. |
| Sustainable performance | Index, query, restore, model, energy proxy, disk, network, and support costs are reported by corpus scale, platform, source class, and derivative choice. |
| Human value | Consented studies separate capture friction, missing-index failures, wrong-source failures, and evidence-viewer failures; they measure time-to-source and retention by segment. |
| Adoption and economics | Setup completion, first recovery, repeated use, alternatives, willingness to pay, support burden, connector upkeep, and optional-service cost produce investment decisions. |
| Interoperability | Versioned exports, provenance, extensions, and benchmarks round-trip with explicit loss/unsupported states and never fabricate an original. |

Planning thresholds such as median time-to-source under 30 seconds or Recall@1 above 0.80 remain
hypotheses until preregistered against a corpus or cohort. Results must be reported even when a gate
fails.

## Dependency and decision rules

- Canonical source bytes, identity, versions, locations, timestamps, transforms, and anchors come
  before OCR, embeddings, synthesis, transport, or collaboration.
- Derived FTS, vector, thumbnail, model, and transport state must remain rebuildable and disposable.
- Web snapshots are best-effort. Live URL and saved snapshot are distinct, with visible status,
  capture time, hash, parser version, and failure reason.
- Generated explanations must link every claim to an evidence anchor or abstain.
- Sync, mobile, passive capture, relay, teams, and professional modes require separate value,
  privacy, security, recovery, and operating-cost gates. A stop decision is valid.
- Market and platform assumptions are re-baselined before major expansion; comparative quality
  claims require an apples-to-apples benchmark.
- Telemetry stays off by default. Any study export is opt-in and excludes raw queries, source text,
  screenshots, credentials, and private documents.
- Every stopped workstream has an export, migration, support, and archive plan so users retain their
  library.

## Global non-goals

LOOM will not make cloud access mandatory, default to continuous screen/audio/clipboard capture,
ship a generic chat-first interface, silently rename or delete sources, pretend a snapshot is an
original, or claim legal, forensic, archival, or evidentiary completeness. Broad format coverage is
not a goal by itself; a format enters only when LOOM can preserve identity, show a trustworthy
anchor, bound failure, and measure recovery.

## Reproduce the roadmap checks

```text
python3 scripts/roadmap.py --validate-only
python3 -m unittest discover -s tests -v
python3 scripts/roadmap.py --repository AlisinaDevelo/LOOM
```

The first two commands are offline. The third compares the manifest with live GitHub and prints the
mutation plan without writing. `--apply` is reserved for maintainers reconciling managed state.
