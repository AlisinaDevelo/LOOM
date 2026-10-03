#!/usr/bin/env python3
"""Small deterministic unit checks for the large-library harness."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("performance-budget.py")
SPEC = importlib.util.spec_from_file_location("loom_performance_budget", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PerformanceBudgetTests(unittest.TestCase):
    @classmethod
    def measurement_report(cls, corpus: Path, count: int) -> dict:
        report = cls.passing_report(count)
        report.update(schema_version=2, max_files=MODULE.SHARD_SIZE)
        report["index"]["completeness"] = 1.0
        report["stats"] = {"artifacts": count}
        report["query"].update(cold_has_evidence=True, cold_hit_count=1)
        report["fts_rebuild"]["report"] = {"after": {"healthy": True}}
        report["index_batches"] = [
            {
                "root": str((corpus / f"shard-{number:03d}").resolve()),
                "report": {
                    "run_id": f"batch-{number}",
                    "discovered": min(MODULE.SHARD_SIZE, count - number * MODULE.SHARD_SIZE),
                    "indexed": min(MODULE.SHARD_SIZE, count - number * MODULE.SHARD_SIZE),
                    "unchanged": 0, "skipped": 0, "failed": 0, "cancelled": 0, "failures": [],
                },
            }
            for number in range((count + MODULE.SHARD_SIZE - 1) // MODULE.SHARD_SIZE)
        ]
        report["index_batches"].sort(key=lambda batch: batch["root"])
        return report

    @staticmethod
    def passing_report(count: int = 100_000) -> dict:
        return {
            "artifact_count": count,
            "index": {"artifacts_per_second": 1_000.0},
            "query": {"warm_p95_latency_ms": 1.0},
            "resource_profile": {"max_rss": 100_000_000, "cpu_seconds": 1.0},
            "database_bytes_per_source_byte": 10.0,
            "fts_rebuild": {"elapsed_ms": 1_000.0},
        }

    def test_time_profile_parser_accepts_macos_shape(self) -> None:
        profile = MODULE.parse_time(
            "real 1.25\nuser 0.80\nsys 0.10\nmaximum resident set size 4096\n"
        )
        self.assertEqual(profile["real_seconds"], 1.25)
        self.assertEqual(profile["user_seconds"], 0.8)
        self.assertEqual(profile["system_seconds"], 0.1)
        self.assertEqual(profile["max_rss"], 4096)
        alternate = MODULE.parse_time("0.80 user time\n0.10 system time\n4096 maximum resident set size\n")
        self.assertEqual(alternate["user_seconds"], 0.8)
        self.assertEqual(alternate["system_seconds"], 0.1)
        self.assertEqual(alternate["max_rss"], 4096)

    def test_corpus_manifest_is_repeatable(self) -> None:
        with tempfile.TemporaryDirectory(prefix="loom-performance-test.") as directory:
            root = Path(directory)
            first = MODULE.generate_corpus(root / "first", 17)
            second = MODULE.generate_corpus(root / "second", 17)
            first.pop("path")
            second.pop("path")
            self.assertEqual(first, second)
            self.assertEqual(first["artifact_count"], 17)
            self.assertEqual(first["shards"], 1)
            self.assertEqual(first["composition"]["text_markdown"] + first["composition"]["text_plain"], 17)
            manifest = json.loads((root / "first" / "manifest-17.json").read_text())
            self.assertEqual(manifest, first)

    def test_exceeded_budget_has_disposition(self) -> None:
        report = {
            "artifact_count": 100_000,
            "index": {"artifacts_per_second": 1.0},
            "query": {"warm_p95_latency_ms": 100.0},
            "resource_profile": {"max_rss": 2_000_000_000, "cpu_seconds": 3_000.0},
            "database_bytes_per_source_byte": 1_000.0,
            "fts_rebuild": {"elapsed_ms": 200_000.0},
        }
        gate = MODULE.evaluate_budgets([report])
        self.assertEqual(gate["status"], "conditional")
        self.assertGreater(gate["exceeded_count"], 0)
        self.assertTrue(gate["all_exceedances_have_disposition"])

    def test_all_largest_scale_repeats_use_worst_metrics_without_order_dependence(self) -> None:
        passing = self.passing_report()
        failing = {
            "artifact_count": 100_000,
            "index": {"artifacts_per_second": 1.0},
            "query": {"warm_p95_latency_ms": 100.0},
            "resource_profile": {"max_rss": 2_000_000_000, "cpu_seconds": 3_000.0},
            "database_bytes_per_source_byte": 1_000.0,
            "fts_rebuild": {"elapsed_ms": 200_000.0},
        }
        gate = MODULE.evaluate_budgets([passing, failing])
        self.assertEqual(gate["status"], "conditional")
        self.assertEqual(gate["exceeded_count"], 6)
        self.assertEqual(gate["runs_evaluated"], 2)
        self.assertEqual(gate, MODULE.evaluate_budgets([failing, passing]))
        for check in gate["checks"]:
            self.assertEqual(check["measurements_available"], 2)
            self.assertEqual(check["status"], "exceeded")

    def test_gate_selects_all_largest_scale_runs_not_smaller_cohorts(self) -> None:
        smaller = self.passing_report(10_000)
        smaller["query"]["warm_p95_latency_ms"] = 100.0
        gate = MODULE.evaluate_budgets([smaller, self.passing_report(), self.passing_report()])
        self.assertEqual(gate["scale_used_for_gate"], 100_000)
        self.assertEqual(gate["runs_evaluated"], 2)
        self.assertEqual(gate["status"], "pass")

    def test_unavailable_resource_metrics_cannot_be_fabricated_as_zero(self) -> None:
        for field in ("max_rss", "cpu_seconds"):
            invalid = [None, -1, float("nan"), float("inf"), True, "1"]
            if field == "max_rss":
                invalid.append(0)
            for value in invalid:
                with self.subTest(field=field, value=value):
                    missing = self.passing_report()
                    missing["resource_profile"][field] = value
                    gate = MODULE.evaluate_budgets([self.passing_report(), missing])
                    self.assertEqual(gate["status"], "conditional")
                    self.assertEqual(gate["unavailable_count"], 1)
                    unavailable = [check for check in gate["checks"] if check["status"] == "unavailable"]
                    self.assertEqual(len(unavailable), 1)
                    self.assertIsNone(unavailable[0]["observed"])
                    self.assertEqual(unavailable[0]["measurements_available"], 1)
                    self.assertTrue(unavailable[0]["disposition"])
                    self.assertEqual(gate, MODULE.evaluate_budgets([missing, self.passing_report()]))

    def test_actual_zero_cpu_measurement_is_distinct_from_unavailable(self) -> None:
        report = self.passing_report()
        report["resource_profile"]["cpu_seconds"] = 0.0
        gate = MODULE.evaluate_budgets([report])
        self.assertEqual(gate["status"], "pass")
        cpu = next(check for check in gate["checks"] if check["name"] == "cpu_seconds_per_1000_artifacts_max")
        self.assertEqual(cpu["observed"], 0.0)
        self.assertEqual(cpu["measurements_available"], 1)

    def test_measurement_keeps_missing_cpu_components_unavailable(self) -> None:
        profiles = (
            ("user 0.50\n", None),
            ("sys 0.25\n", None),
            ("", None),
            ("user 0.00\nsys 0.00\n", 0.0),
            ("user 0.50\nsys 0.25\n", 0.75),
        )
        with tempfile.TemporaryDirectory(prefix="loom-resource-assembly-") as directory:
            root = Path(directory)
            report = self.measurement_report(root, 100_000)
            for profile, expected in profiles:
                with self.subTest(profile=profile):
                    completed = subprocess.CompletedProcess(
                        [], 0, stdout=json.dumps(report),
                        stderr="real 1.25\nmaximum resident set size 4096\n" + profile,
                    )
                    with patch.object(MODULE.subprocess, "run", return_value=completed):
                        measured = MODULE.run_measurement(root / "loom", root, root, root, root, 100_000, 1, 31)
                    self.assertEqual(measured["resource_profile"]["cpu_seconds"], expected)
                    gate = MODULE.evaluate_budgets([measured])
                    cpu = next(check for check in gate["checks"] if check["name"] == "cpu_seconds_per_1000_artifacts_max")
                    self.assertEqual(cpu["status"], "unavailable" if expected is None else "pass")

    def test_measurement_indexes_explicit_bounded_shards_in_one_process(self) -> None:
        with tempfile.TemporaryDirectory(prefix="loom-batched-performance-") as directory:
            root = Path(directory)
            report = self.measurement_report(root, 20_001)
            completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(report), stderr="")
            with patch.object(MODULE.subprocess, "run", return_value=completed) as run:
                measured = MODULE.run_measurement(root / "loom", root, root, root, root, 20_001, 1, 31)
            command = run.call_args.args[0]
            self.assertEqual(command[command.index("--max-files") + 1], "20000")
            selected = [command[index + 1] for index, argument in enumerate(command) if argument == "--index-root"]
            self.assertEqual(selected, [str(root / "shard-000"), str(root / "shard-001")])
            self.assertEqual(len(measured["index_batches"]), 2)

    def test_measurement_rejects_missing_reordered_or_incomplete_batches(self) -> None:
        with tempfile.TemporaryDirectory(prefix="loom-invalid-batch-") as directory:
            root = Path(directory)
            for mutation in ("missing", "reordered", "partial", "old-schema", "raised-limit"):
                with self.subTest(mutation=mutation):
                    report = self.measurement_report(root, 20_001)
                    if mutation == "missing":
                        report.pop("index_batches")
                    elif mutation == "reordered":
                        report["index_batches"].reverse()
                    elif mutation == "partial":
                        report["index_batches"][1]["report"]["indexed"] = 0
                    elif mutation == "old-schema":
                        report["schema_version"] = 1
                    else:
                        report["max_files"] = 100_000
                    completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(report), stderr="")
                    with patch.object(MODULE.subprocess, "run", return_value=completed):
                        with self.assertRaisesRegex(RuntimeError, "batch"):
                            MODULE.run_measurement(root / "loom", root, root, root, root, 20_001, 1, 31)

    def test_measurement_matches_native_root_order_above_three_digit_shards(self) -> None:
        # Mock only resource/report assembly; do not generate 20 million source files.
        with tempfile.TemporaryDirectory(prefix="loom-shard-order-") as directory:
            root = Path(directory)
            count = 20_000_001
            report = self.measurement_report(root, count)
            completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(report), stderr="")
            with patch.object(MODULE.subprocess, "run", return_value=completed) as run:
                measured = MODULE.run_measurement(root / "loom", root, root, root, root, count, 1, 31)
            command = run.call_args.args[0]
            selected = [command[index + 1] for index, argument in enumerate(command) if argument == "--index-root"]
            self.assertEqual(selected, sorted(selected))
            partial = next(batch for batch in measured["index_batches"] if batch["root"].endswith("shard-1000"))
            self.assertEqual(partial["report"]["indexed"], 1)

    def test_final_report_does_not_publish_zero_or_partial_rss_variance(self) -> None:
        first, second = self.passing_report(17), self.passing_report(17)
        for report in (first, second):
            report["index"]["elapsed_ms"] = 1_000.0
        second["resource_profile"]["max_rss"] = None
        with tempfile.TemporaryDirectory(prefix="loom-report-assembly-") as directory:
            root = Path(directory)
            binary = root / "fixture-binary"
            binary.write_bytes(b"not executed; report assembly fixture only")
            args = SimpleNamespace(evidence_dir=root, loom=binary, counts=[17], runs=2,
                                   warm_queries=31, keep_corpus=False)
            with patch.object(MODULE, "parse_args", return_value=args), \
                    patch.object(MODULE, "build_binary", return_value=binary), \
                    patch.object(MODULE, "run_measurement", side_effect=[first, second]), \
                    patch("builtins.print"):
                self.assertEqual(MODULE.main(), 0)
            assembled = json.loads((root / "report.json").read_text())
            self.assertEqual(assembled["release_gate"]["unavailable_count"], 1)
            rss = assembled["scales"]["17"]["variance"]["max_rss_bytes"]
            for aggregate in ("min", "median", "max", "population_stddev"):
                self.assertNotIn(aggregate, rss)
            self.assertEqual(rss["measurements_available"], 1)
            self.assertEqual(rss["measurements_expected"], 2)
            self.assertFalse(rss["complete"])


if __name__ == "__main__":
    unittest.main()
