#!/usr/bin/env python3
"""Real CLI fail-closed discovery checks with only disposable synthetic sources."""
import argparse
import json
import os
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
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization")
    if os.name != "posix":
        parser.error("this real no-follow fixture requires Unix")
    binary = args.loom.resolve()
    reports = []
    for scenario, expected in [
        ("depth", "depth limit"), ("directories", "directory limit"),
        ("files", "20000-file request limit"), ("entries", "entry limit"),
    ]:
        with tempfile.TemporaryDirectory(prefix="loom-directory-") as temporary:
            base = Path(temporary).resolve()
            root = base / "selected"
            root.mkdir()
            database = base / "library.sqlite3"
            existing = root / "old.md"
            existing.write_text("Preserved synthetic source marker", encoding="utf-8")

            def command(*arguments):
                result = subprocess.run([str(binary), "--database", str(database), *map(str, arguments)],
                                        text=True, capture_output=True, timeout=30, check=False)
                assert result.returncode == 0, (arguments, result.stderr)
                return json.loads(result.stdout)

            def canonical():
                with sqlite3.connect(database) as connection:
                    return {table: connection.execute(f"SELECT * FROM {table} ORDER BY id").fetchall()
                            for table in ("source_roots", "artifacts", "artifact_versions", "artifact_locators", "passages", "index_jobs")}

            command("index", root)
            before = canonical()
            stats = command("stats")
            hits = command("search", '"Preserved synthetic source marker"')
            existing.unlink()
            (root / "new.md").write_text("Unpublished synthetic source marker", encoding="utf-8")
            if scenario == "depth":
                deep = root
                for _ in range(33):
                    deep = deep / "child"
                    deep.mkdir()
                generated = 33
            elif scenario == "directories":
                for index in range(4096):
                    (root / f"d{index:04}").mkdir()
                generated = 4096
            elif scenario == "files":
                for index in range(20000):
                    (root / f"f{index:05}.bin").touch()
                generated = 20000
            else:
                outside = base / "outside.md"
                outside.write_text("Outside synthetic source marker", encoding="utf-8")
                for index in range(65536):
                    (root / f"l{index:05}.md").symlink_to(outside)
                generated = 65536
            invocation = [str(binary), "--database", str(database), "index", str(root)]
            timed = sys.platform == "darwin"
            if timed:
                invocation = ["/usr/bin/time", "-lp", *invocation]
            started = time.monotonic()
            result = subprocess.run(invocation, text=True, capture_output=True, timeout=30, check=False)
            wall_ms = (time.monotonic() - started) * 1000
            assert result.returncode != 0, (scenario, result.stdout, result.stderr)
            # A slower filesystem may reach the earlier cooperative deadline.
            # Report which boundary actually fired; do not pretend it was entry/file exhaustion.
            observed = expected if expected in result.stderr else "cooperative time limit"
            assert observed in result.stderr, (scenario, result.stderr)
            assert canonical() == before, f"{scenario}: partial canonical/checkpoint publication"
            assert command("stats") == stats
            assert command("search", '"Preserved synthetic source marker"') == hits
            assert command("search", '"Unpublished synthetic source marker"') == []
            assert command("search", '"Outside synthetic source marker"') == []
            # The same refused tree must not become an enabled watch/reconcile scope in a
            # previously empty library. Do not exempt source-root consent from the receipt.
            first_database = base / "first-selection.sqlite3"
            first = subprocess.run([str(binary), "--database", str(first_database), "index", str(root)],
                                   text=True, capture_output=True, timeout=30, check=False)
            assert first.returncode != 0, (scenario, first.stdout, first.stderr)
            assert expected in first.stderr or "cooperative time limit" in first.stderr, first.stderr
            with sqlite3.connect(first_database) as connection:
                assert connection.execute("SELECT COUNT(*) FROM source_roots").fetchone()[0] == 0
                assert connection.execute("SELECT COUNT(*) FROM index_jobs").fetchone()[0] == 0
            peak = re.search(r"(\d+)\s+maximum resident set size", result.stderr) if timed else None
            reports.append({"scenario": scenario, "generated_entries": generated,
                            "observed_limit": observed, "wall_ms": wall_ms,
                            "process_peak_resident_bytes": int(peak.group(1)) if peak else None,
                            "artifact_and_checkpoint_tables_unchanged": True,
                            "source_roots_unchanged": True,
                            "failed_first_selection_created_no_root": True})
    report = {"scope": "synthetic default-limit foreground CLI refusal, not directory scheduling",
              "measurements": reports,
              "limitations": ["cooperative syscall deadline, not OS preemption", "CLI RSS includes SQLite/startup"]}
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
