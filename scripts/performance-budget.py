#!/usr/bin/env python3
"""Run the reproducible large-library performance gate on the current device.

The corpus is synthetic and rights-clean.  It is deliberately generated instead of checked in:
the manifest records the generator version, seed, composition, byte count, and aggregate digest so
another run can recreate exactly the same shape without publishing 100,000 disposable files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


GENERATOR_VERSION = "loom-performance-corpus-v1"
SEED = 20260824
SHARD_SIZE = 20_000

# These are pre-optimization planning budgets, not a claim about a population or every future
# device.  The report records every exceeded budget with a disposition rather than hiding it.
BUDGETS = {
    "index_throughput_artifacts_per_second_min": 100.0,
    "warm_query_p95_ms_max": 25.0,
    "max_rss_bytes_max": 1_073_741_824,
    "database_amplification_max": 128.0,
    "cpu_seconds_per_1000_artifacts_max": 20.0,
    "fts_rebuild_seconds_max": 120.0,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--evidence-dir",
        type=Path,
        required=True,
        help="directory for manifests, reports, and resource logs",
    )
    parser.add_argument(
        "--loom",
        type=Path,
        help="path to a built loom binary (defaults to target/debug/loom)",
    )
    parser.add_argument(
        "--counts",
        default="10000,100000",
        help="comma-separated artifact counts (default: 10000,100000)",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=2,
        help="independent runs per scale for variance (default: 2)",
    )
    parser.add_argument(
        "--warm-queries",
        type=int,
        default=31,
        help="repeated warm queries per measurement (default: 31)",
    )
    parser.add_argument(
        "--keep-corpus",
        action="store_true",
        help="retain generated corpora under the evidence directory",
    )
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be at least 1")
    if args.warm_queries < 1:
        parser.error("--warm-queries must be at least 1")
    try:
        args.counts = [int(value) for value in args.counts.split(",") if value]
    except ValueError as error:
        parser.error(f"invalid --counts: {error}")
    if not args.counts or any(count < 1 for count in args.counts):
        parser.error("--counts must contain positive integers")
    return args


def build_binary(repo: Path, requested: Path | None) -> Path:
    binary = requested or repo / "target" / "debug" / "loom"
    if binary.exists() and os.access(binary, os.X_OK):
        return binary
    subprocess.run(
        ["cargo", "build", "--locked", "-q", "-p", "loom-cli"],
        cwd=repo,
        check=True,
    )
    if not binary.exists() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"loom binary was not produced at {binary}")
    return binary


def generate_corpus(root: Path, count: int) -> dict[str, Any]:
    corpus = root / f"corpus-{count}"
    corpus.mkdir(parents=True)
    digest = hashlib.sha256()
    extension_counts = {"md": 0, "txt": 0}
    source_bytes = 0
    for index in range(count):
        shard = index // SHARD_SIZE
        extension = "md" if (index + SEED) % 5 else "txt"
        relative = Path(f"shard-{shard:03d}") / f"artifact-{index:06d}.{extension}"
        path = corpus / relative
        path.parent.mkdir(exist_ok=True)
        body = (
            f"# Synthetic LOOM artifact {index:06d}\n"
            f"Performance budget corpus seed {SEED}; shard {shard:03d}.\n"
            f"Selective recovery marker performance-{index:06d}.\n"
            "This rights-clean record exists only for deterministic local scale measurement.\n"
        ).encode("utf-8")
        path.write_bytes(body)
        digest.update(str(relative).encode("utf-8"))
        digest.update(b"\0")
        digest.update(body)
        digest.update(b"\0")
        extension_counts[extension] += 1
        source_bytes += len(body)

    query_marker = f"performance-{count - 1:06d}"
    manifest = {
        "generator_version": GENERATOR_VERSION,
        "seed": SEED,
        "artifact_count": count,
        "shard_size": SHARD_SIZE,
        "shards": (count + SHARD_SIZE - 1) // SHARD_SIZE,
        "composition": {
            "text_markdown": extension_counts["md"],
            "text_plain": extension_counts["txt"],
        },
        "source_bytes": source_bytes,
        "content_sha256": digest.hexdigest(),
        "query": query_marker,
        "selection": "one explicitly selected corpus root; no home-directory or passive capture",
    }
    (root / f"manifest-{count}.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return {"path": corpus, **manifest}


def parse_time(stderr: str) -> dict[str, float | int | None]:
    patterns = {
        "real_seconds": r"^real\s+([0-9.]+)",
        "user_seconds": r"^user\s+([0-9.]+)",
        "system_seconds": r"^sys\s+([0-9.]+)",
        "max_rss": r"maximum resident set size\s+(\d+)",
    }
    values: dict[str, float | int | None] = {
        "real_seconds": None,
        "user_seconds": None,
        "system_seconds": None,
        "max_rss": None,
    }
    for line in stderr.splitlines():
        for key, pattern in patterns.items():
            match = re.search(pattern, line)
            if not match and key == "user_seconds":
                match = re.search(r"([0-9.]+)\s+user time", line)
            if not match and key == "system_seconds":
                match = re.search(r"([0-9.]+)\s+system time", line)
            if not match and key == "max_rss":
                match = re.search(r"(\d+)\s+maximum resident set size", line)
            if match:
                values[key] = int(match.group(1)) if key == "max_rss" else float(match.group(1))
    return values


def run_measurement(
    binary: Path,
    repo: Path,
    work: Path,
    evidence: Path,
    corpus: Path,
    count: int,
    run_number: int,
    warm_queries: int,
) -> dict[str, Any]:
    database = work / f"library-{count}-{run_number}.sqlite3"
    command = [
        str(binary),
        "--database",
        str(database),
        "performance",
        "--corpus",
        str(corpus),
        "--query",
        f"performance-{count - 1:06d}",
        "--warm-queries",
        str(warm_queries),
        "--max-files",
        str(count),
    ]
    started = time.perf_counter()
    completed = subprocess.run(
        ["/usr/bin/time", "-lp", *command],
        cwd=repo,
        text=True,
        capture_output=True,
        check=False,
    )
    elapsed_seconds = time.perf_counter() - started
    stdout_path = evidence / f"run-{count}-{run_number}.stdout.json"
    stderr_path = evidence / f"run-{count}-{run_number}.time.txt"
    stdout_path.write_text(completed.stdout, encoding="utf-8")
    stderr_path.write_text(completed.stderr, encoding="utf-8")
    if completed.returncode != 0:
        raise RuntimeError(
            f"performance run failed for {count}/{run_number}; see {stdout_path} and {stderr_path}"
        )
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"performance output was not JSON: {stdout_path}") from error
    resources = parse_time(completed.stderr)
    report["resource_profile"] = {
        **resources,
        "wall_seconds_python": elapsed_seconds,
        "cpu_seconds": (resources["user_seconds"] or 0.0)
        + (resources["system_seconds"] or 0.0),
        "time_command": "/usr/bin/time -lp",
    }
    report["run_number"] = run_number
    report["artifact_count"] = count
    report["raw_output"] = {
        "stdout": str(stdout_path),
        "stderr": str(stderr_path),
    }
    if report["index"]["completeness"] != 1.0:
        raise RuntimeError(f"incomplete performance index: {stdout_path}")
    if report["stats"]["artifacts"] != count:
        raise RuntimeError(f"artifact count mismatch in performance run: {stdout_path}")
    if not report["query"]["cold_has_evidence"] or report["query"]["cold_hit_count"] == 0:
        raise RuntimeError(f"performance query lost evidence: {stdout_path}")
    if not report["fts_rebuild"]["report"]["after"]["healthy"]:
        raise RuntimeError(f"FTS derivative rebuild did not recover health: {stdout_path}")
    # The JSON report and resource logs are the retained evidence. Remove the large SQLite
    # derivative immediately so two 100k runs do not consume the developer disk between samples.
    for suffix in ("", "-wal", "-shm"):
        database_variant = Path(f"{database}{suffix}")
        if database_variant.exists():
            database_variant.unlink()
    return report


def median(values: list[float]) -> float:
    return statistics.median(values) if values else 0.0


def metric_summary(reports: list[dict[str, Any]], getter: Any) -> dict[str, float]:
    values = [float(getter(report)) for report in reports]
    return {
        "min": min(values),
        "median": median(values),
        "max": max(values),
        "population_stddev": statistics.pstdev(values) if len(values) > 1 else 0.0,
    }


def evaluate_budgets(all_reports: list[dict[str, Any]]) -> dict[str, Any]:
    largest = max(all_reports, key=lambda report: report["artifact_count"])
    resources = largest["resource_profile"]
    count = largest["artifact_count"]
    observed = {
        "index_throughput_artifacts_per_second_min": largest["index"]["artifacts_per_second"],
        "warm_query_p95_ms_max": largest["query"]["warm_p95_latency_ms"],
        "max_rss_bytes_max": resources["max_rss"] or 0,
        "database_amplification_max": largest["database_bytes_per_source_byte"],
        "cpu_seconds_per_1000_artifacts_max": ((resources["cpu_seconds"] or 0.0) / count) * 1000,
        "fts_rebuild_seconds_max": largest["fts_rebuild"]["elapsed_ms"] / 1000,
    }
    checks: list[dict[str, Any]] = []
    for name, budget in BUDGETS.items():
        value = observed[name]
        passed = value >= budget if name.endswith("_min") else value <= budget
        checks.append(
            {
                "name": name,
                "budget": budget,
                "observed": value,
                "status": "pass" if passed else "exceeded",
                "disposition": "within pre-optimization budget"
                if passed
                else "remediation required before the v0.2 release gate; no exception is silently accepted",
            }
        )
    exceeded = [check for check in checks if check["status"] == "exceeded"]
    return {
        "scale_used_for_gate": count,
        "checks": checks,
        "status": "pass" if not exceeded else "conditional",
        "exceeded_count": len(exceeded),
        "all_exceedances_have_disposition": all(bool(item["disposition"]) for item in exceeded),
    }


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parent.parent
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    binary = build_binary(repo, args.loom.resolve() if args.loom else None)
    work = Path(tempfile.mkdtemp(prefix="loom-performance-work."))
    reports: list[dict[str, Any]] = []
    corpora: list[dict[str, Any]] = []
    try:
        for count in args.counts:
            corpus = generate_corpus(work, count)
            corpora.append({key: value for key, value in corpus.items() if key != "path"})
            for run_number in range(1, args.runs + 1):
                report = run_measurement(
                    binary,
                    repo,
                    work,
                    evidence,
                    corpus["path"],
                    count,
                    run_number,
                    args.warm_queries,
                )
                reports.append(report)
            if not args.keep_corpus:
                shutil.rmtree(corpus["path"])

        per_scale: dict[str, Any] = {}
        for count in args.counts:
            scale_reports = [report for report in reports if report["artifact_count"] == count]
            per_scale[str(count)] = {
                "runs": scale_reports,
                "variance": {
                    "index_elapsed_ms": metric_summary(scale_reports, lambda item: item["index"]["elapsed_ms"]),
                    "index_throughput_artifacts_per_second": metric_summary(
                        scale_reports, lambda item: item["index"]["artifacts_per_second"]
                    ),
                    "warm_query_p95_latency_ms": metric_summary(
                        scale_reports, lambda item: item["query"]["warm_p95_latency_ms"]
                    ),
                    "max_rss_bytes": metric_summary(
                        scale_reports, lambda item: item["resource_profile"]["max_rss"] or 0
                    ),
                    "database_bytes_per_source_byte": metric_summary(
                        scale_reports, lambda item: item["database_bytes_per_source_byte"]
                    ),
                    "fts_rebuild_seconds": metric_summary(
                        scale_reports, lambda item: item["fts_rebuild"]["elapsed_ms"] / 1000
                    ),
                },
            }

        report = {
            "schema_version": 1,
            "generator": {
                "version": GENERATOR_VERSION,
                "seed": SEED,
                "shard_size": SHARD_SIZE,
                "corpora": corpora,
            },
            "device": {
                "os": platform.platform(),
                "machine": platform.machine(),
                "python": platform.python_version(),
                "binary": str(binary),
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            },
            "conditions": {
                "cold": "first query after opening/indexing a new SQLite connection; OS page cache not dropped",
                "warm": "repeated query in the same process after the cold observation",
                "runs_per_scale": args.runs,
                "warm_queries": args.warm_queries,
            },
            "pre_optimization_budgets": BUDGETS,
            "scales": per_scale,
            "release_gate": evaluate_budgets(reports),
            "limitations": [
                "Synthetic local Markdown/plain-text artifacts; no user content is read or uploaded.",
                "The 100k corpus is split into deterministic 20k shards only for fixture generation; one corpus root is selected and max-files is explicit.",
                "Cold means a new process/SQLite connection, not a privileged OS page-cache flush.",
                "Maximum RSS and user/system CPU are a process-level proxy; battery energy is not measured.",
                "This gate measures lexical FTS5 and canonical ingestion. OCR, PDFs, semantic vectors, and passive capture require separate corpus gates.",
            ],
            "profiling_decision": "Resource profiles are retained for review. No index or concurrency optimization is claimed until a measured bottleneck is reproduced; any exceeded budget carries an explicit remediation disposition.",
        }
        (evidence / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        (evidence / "report.sha256").write_text(
            hashlib.sha256((evidence / "report.json").read_bytes()).hexdigest() + "  report.json\n",
            encoding="utf-8",
        )
        print(json.dumps({"report": str(evidence / "report.json"), "release_gate": report["release_gate"]}, indent=2))
        return 0
    finally:
        if args.keep_corpus:
            destination = evidence / "generated-corpus"
            if destination.exists():
                shutil.rmtree(destination)
            shutil.copytree(work, destination)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"performance gate failed: {error}", file=sys.stderr)
        raise SystemExit(1)
