#!/usr/bin/env python3
"""Exercise queued local semantic rebuild with temporary synthetic sources only."""
import argparse
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, required=True)
    parser.add_argument("--previous-loom", type=Path, help="retained released runtime-v5 CLI")
    parser.add_argument("--report", type=Path, help="retain per-process resource measurements (macOS)")
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization")
    if args.report and sys.platform != "darwin":
        parser.error("resource report currently requires macOS time -lp")
    binary = args.loom.resolve()
    measurements = []
    with tempfile.TemporaryDirectory(prefix="loom-semantic-contract-") as temporary:
        base = Path(temporary).resolve()
        database = base / "library.sqlite3"
        root = base / "selected"
        root.mkdir()
        for name in ("a", "b", "c"):
            (root / f"{name}.md").write_text(
                f"Synthetic semantic recovery marker {name}.\n", encoding="utf-8")

        def command(*arguments, executable=binary, rejected=False):
            argv = [str(executable), "--database", str(database), *map(str, arguments)]
            if args.report:
                argv = ["/usr/bin/time", "-lp", *argv]
            started = time.monotonic()
            result = subprocess.run(
                argv, capture_output=True, text=True, timeout=40)
            if rejected:
                assert result.returncode != 0, result.stdout
                return result.stderr
            assert result.returncode == 0, result.stderr
            payload = json.loads(result.stdout)
            if args.report:
                match = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", result.stderr, re.MULTILINE)
                assert match, result.stderr
                measurements.append({"operation": str(arguments[0]),
                                     "adapter": payload.get("operation") if isinstance(payload, dict) else None,
                                     "state": payload.get("state") if isinstance(payload, dict) else None,
                                     "elapsed_ms": (time.monotonic() - started) * 1000,
                                     "peak_resident_bytes": int(match[1])})
            return payload

        if args.previous_loom:
            previous = args.previous_loom.resolve()
            command("index", root, executable=previous)
            directory = command("enqueue-index-directory", root, "released-v5-directory", executable=previous)
            with sqlite3.connect(database) as connection:
                assert connection.execute("SELECT value FROM schema_meta WHERE key='background_job_schema_version'").fetchone()[0] == "5"
                units_before = connection.execute("SELECT * FROM background_directory_units ORDER BY ordinal").fetchall()
            command("stats")  # Canonical reads must not silently migrate the old queue.
            command("jobs", rejected=True)
            command("upgrade-job-runtime")
            command("upgrade-job-runtime")
            adopted = command("jobs")[0]
            assert all(adopted[key] == value for key, value in directory.items())
            with sqlite3.connect(database) as connection:
                assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
                assert connection.execute("SELECT * FROM background_directory_units ORDER BY ordinal").fetchall() == units_before
            command("jobs", executable=previous, rejected=True)
            assert len(command("search", '"Synthetic semantic recovery marker"', executable=previous)) == 3
            for _ in range(4):
                resumed = command("run-next-job")
                assert resumed["id"] == directory["id"]
            assert resumed["state"] == "completed"
        else:
            command("index", root)
        expected = command("semantic-rebuild")
        job = command("enqueue-semantic-rebuild", "semantic")
        assert job["semantic_progress"]["total_units"] == 3
        assert command("enqueue-semantic-rebuild", "semantic")["id"] == job["id"]
        maintenance = command("enqueue-fts-repair", "interleaved")
        for ordinal in range(1, 4):
            quantum = command("run-next-job")
            assert quantum["id"] == job["id"] and quantum["state"] == "queued", quantum
            assert quantum["attempts"] == 0
            assert quantum["semantic_progress"]["next_unit"] == ordinal
            assert command("semantic-status")["healthy"] is True
            if ordinal == 1:
                assert command("run-next-job")["id"] == maintenance["id"]
        completed = command("run-next-job")
        assert completed["state"] == "completed" and completed["semantic_progress"] is None
        assert completed["result"] == expected
        assert command("semantic-status")["healthy"] is True
        assert command("enqueue-semantic-rebuild", "semantic") == completed
        assert command("run-next-job") is None
        interrupted = command("enqueue-semantic-rebuild", "retired")
        command("run-next-job")
        command("semantic-drop")
        assert not any(row["id"] == interrupted["id"] for row in command("jobs"))
        assert command("run-next-job") is None
        assert command("semantic-status")["manifest"] is None
        assert len(command("search", '"Synthetic semantic recovery marker"')) == 3
        command("purge-root", root)
        assert command("stats")["artifacts"] == 0

        # A larger synthetic corpus measures the real processes and keeps lexical searches
        # usable between quanta. These are receipts, not a whole-workload SLO claim.
        if args.report:
            for ordinal in range(128):
                (root / f"resource-{ordinal:03}.md").write_text(
                    f"Synthetic resource evidence marker {ordinal:03}.\n", encoding="utf-8")
            command("index", root)
            command("semantic-rebuild")
            measured = command("enqueue-semantic-rebuild", "resource")
            total = measured["semantic_progress"]["total_units"]
            for _ in range(total):
                quantum = command("run-next-job")
                assert quantum["state"] == "queued"
                assert command("search", '"resource evidence marker 000"')
            assert command("run-next-job")["state"] == "completed"
            assert command("semantic-status")["healthy"]
            assert command("semantic-search", "resource evidence marker 000")
            command("purge-root", root)
    if args.report:
        args.report.write_text(json.dumps({"platform": sys.platform, "measurements": measurements}, indent=2) + "\n", encoding="utf-8")
    print("semantic jobs: PASS (single-passage quanta, fairness, atomic publication, drop)"
          + ("; released v5 migration PASS" if args.previous_loom else "; released v5 binary not provided"))


if __name__ == "__main__":
    main()
