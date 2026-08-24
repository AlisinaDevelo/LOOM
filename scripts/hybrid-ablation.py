#!/usr/bin/env python3
"""Run the preregistered lexical/semantic/hybrid ablation on a rights-clean corpus."""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import tempfile
import time
from pathlib import Path
from statistics import median


P95_LATENCY_MAX_MS = 1000.0


def command(binary: Path, database: Path, *args: str) -> tuple[object, float]:
    started = time.perf_counter()
    completed = subprocess.run(
        [str(binary), "--database", str(database), *args],
        check=True,
        capture_output=True,
        text=True,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    return json.loads(completed.stdout), elapsed_ms


def expected_source(corpus: Path, source_uri: str, expected_file: str) -> bool:
    try:
        return Path(source_uri).resolve() == (corpus / expected_file).resolve()
    except OSError:
        return False


def candidate_text(candidate: dict[str, object], mode: str) -> str:
    passage_text = str(candidate.get("passage_text", ""))
    if passage_text:
        return passage_text
    excerpt = candidate.get("excerpt", {})
    if not isinstance(excerpt, dict):
        return ""
    return "".join(
        str(segment["text"])
        for segment in excerpt.get("segments", [])
        if isinstance(segment, dict)
    )


def anchor_matches(candidate: dict[str, object], query: dict[str, object], mode: str) -> bool:
    expected = query["expected_anchor"]
    actual = candidate.get("anchor", {})
    if not isinstance(expected, dict) or not isinstance(actual, dict):
        return False
    fields = ("kind", "char_start", "char_end", "line_start", "line_end")
    return all(actual.get(field) == expected.get(field) for field in fields) and str(
        expected["contains"]
    ) in candidate_text(candidate, mode)


def is_expected(
    corpus: Path, candidate: dict[str, object], query: dict[str, object], mode: str
) -> bool:
    source_uri = str(candidate.get("source_uri", ""))
    expected_file = str(query["expected_file"])
    if expected_source(corpus, source_uri, expected_file):
        return True
    for alternative in query.get("acceptable_alternatives", []):
        if isinstance(alternative, dict) and expected_source(
            corpus, source_uri, str(alternative["expected_file"])
        ):
            return True
    return False


def anchor_is_correct(
    corpus: Path, candidate: dict[str, object], query: dict[str, object], mode: str
) -> bool:
    if expected_source(corpus, str(candidate.get("source_uri", "")), str(query["expected_file"])):
        return anchor_matches(candidate, query, mode)
    for alternative in query.get("acceptable_alternatives", []):
        if isinstance(alternative, dict) and expected_source(
            corpus, str(candidate.get("source_uri", "")), str(alternative["expected_file"])
        ):
            expected = dict(query)
            expected["expected_anchor"] = alternative["expected_anchor"]
            return anchor_matches(candidate, expected, mode)
    return False


def metrics(
    corpus: Path,
    query_rows: list[dict[str, object]],
    results: dict[str, list[tuple[list[dict[str, object]], float]]],
) -> dict[str, object]:
    report: dict[str, object] = {}
    for mode, rows in results.items():
        top_one = 0
        top_five = 0
        anchor_candidates = 0
        anchor_correct = 0
        returned = 0
        false_positives = 0
        latencies = []
        failures = []
        if len(rows) != len(query_rows):
            raise RuntimeError(f"{mode} returned {len(rows)} rows for {len(query_rows)} queries")
        for query, (candidates, elapsed_ms) in zip(query_rows, rows):
            latencies.append(elapsed_ms)
            returned += len(candidates)
            matches = [is_expected(corpus, candidate, query, mode) for candidate in candidates]
            query_anchor_candidates = 0
            query_anchor_correct = False
            if matches and matches[0]:
                top_one += 1
            if any(matches):
                top_five += 1
            for candidate, match in zip(candidates, matches):
                if match:
                    query_anchor_candidates += 1
                    anchor_candidates += 1
                    if anchor_is_correct(corpus, candidate, query, mode):
                        query_anchor_correct = True
                        anchor_correct += 1
                else:
                    false_positives += 1
            if not candidates or not any(matches):
                failures.append({"id": query["id"], "kind": "wrong_source"})
            elif not matches[0]:
                failures.append({"id": query["id"], "kind": "wrong_source_at_rank_1"})
            elif query_anchor_candidates == 0 or not query_anchor_correct:
                failures.append({"id": query["id"], "kind": "wrong_anchor"})
        count = max(len(query_rows), 1)
        ordered = sorted(latencies)
        p95_index = max(0, min(len(ordered) - 1, math.ceil(0.95 * len(ordered)) - 1))
        report[mode] = {
            "queries": len(query_rows),
            "exact_source_recall_at_1": top_one / count,
            "exact_source_recall_at_5": top_five / count,
            "anchor_precision": anchor_correct / max(anchor_candidates, 1),
            "false_positive_rate": false_positives / max(returned, 1),
            "median_latency_ms": median(latencies) if latencies else 0.0,
            "p95_latency_ms": ordered[p95_index] if ordered else 0.0,
            "failures": failures,
        }
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, default=Path("benchmarks/retrieval/v0/corpus"))
    parser.add_argument("--queries", type=Path, default=Path("benchmarks/retrieval/v0/queries.jsonl"))
    parser.add_argument("--manifest", type=Path, default=Path("benchmarks/retrieval/v0/manifest.json"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    corpus = (root / args.corpus).resolve()
    queries_path = (root / args.queries).resolve()
    manifest = json.loads((root / args.manifest).read_text())
    query_rows = [json.loads(line) for line in queries_path.read_text().splitlines() if line.strip()]

    with tempfile.TemporaryDirectory(prefix="loom-hybrid-ablation-") as temporary:
        database = Path(temporary) / "library.sqlite3"
        subprocess.run(["cargo", "build", "--locked", "-q", "-p", "loom-cli"], cwd=root, check=True)
        binary = root / "target" / "debug" / "loom"
        index, _ = command(binary, database, "index", str(corpus))
        if index["failures"]:
            raise RuntimeError(f"benchmark index failed: {index['failures']}")
        supported = max(int(index["discovered"]) - int(index["skipped"]), 0)
        completeness = (
            (int(index["indexed"]) + int(index.get("unchanged", 0))) / supported
            if supported
            else 1.0
        )
        command(binary, database, "semantic-rebuild")
        results: dict[str, list[tuple[list[dict[str, object]], float]]] = {
            "lexical": [],
            "semantic": [],
            "hybrid": [],
        }
        commands = {
            "lexical": "search",
            "semantic": "semantic-search",
            "hybrid": "hybrid-search",
        }
        for query in query_rows:
            for mode, subcommand in commands.items():
                raw, elapsed_ms = command(binary, database, subcommand, str(query["query"]), "--limit", "5")
                if not isinstance(raw, list):
                    raise RuntimeError(f"{mode} returned a non-list response")
                results[mode].append((raw, elapsed_ms))

    report = metrics(corpus, query_rows, results)
    thresholds = manifest["thresholds"]
    hybrid = report["hybrid"]
    lexical = report["lexical"]
    accuracy_pass = all(
        hybrid[key] >= thresholds[key]
        for key in ("exact_source_recall_at_1", "exact_source_recall_at_5", "anchor_precision")
    ) and hybrid["false_positive_rate"] <= thresholds["false_positive_rate"] and completeness >= thresholds[
        "index_completeness"
    ]
    latency_pass = hybrid["p95_latency_ms"] <= P95_LATENCY_MAX_MS
    non_regression = hybrid["exact_source_recall_at_1"] >= lexical["exact_source_recall_at_1"]
    gate_pass = accuracy_pass and latency_pass and non_regression and not hybrid["failures"]
    output = {
        "schema_version": 1,
        "algorithm": {
            "id": "hybrid-rank-v1",
            "method": "weighted reciprocal-rank fusion plus exact/path/recency signals",
            "config": {
                "reciprocal_rank_constant": 60,
                "lexical_weight": 0.45,
                "semantic_weight": 0.35,
                "exact_match_weight": 0.10,
                "path_weight": 0.05,
                "recency_weight": 0.05,
                "semantic_only_admission": "at least half of distinct query tokens in passage, title, or source URI",
            },
        },
        "gate": {
            "status": "eligible" if gate_pass else "hold",
            "accuracy_pass": accuracy_pass,
            "latency_pass": latency_pass,
            "non_regression_pass": non_regression,
            "hybrid_p95_latency_ms_max": P95_LATENCY_MAX_MS,
            "manifest_thresholds": thresholds,
        },
        "index": {
            "discovered": index["discovered"],
            "indexed": index["indexed"],
            "skipped": index["skipped"],
            "completeness": completeness,
        },
        "modes": report,
    }
    print(json.dumps(output, indent=2, sort_keys=True))
    return 0 if gate_pass else 2


if __name__ == "__main__":
    raise SystemExit(main())
