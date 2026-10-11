#!/usr/bin/env python3
"""Verify explicit first-folder admission and worker restart with synthetic sources only."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization")
    binary = args.loom.resolve()
    with tempfile.TemporaryDirectory(prefix="loom-directory-selection-") as temporary:
        base = Path(temporary).resolve()
        database = base / "library.sqlite3"
        root = base / "selected"
        root.mkdir()
        for name in ("a.md", "b.md"):
            (root / name).write_text(f"Synthetic first-selection evidence marker {name}\n", encoding="utf-8")
        (root / "c.bin").write_bytes(b"Unsupported synthetic source")

        def command(*arguments, rejected=False):
            result = subprocess.run([str(binary), "--database", str(database), *map(str, arguments)],
                                    capture_output=True, text=True, timeout=40)
            if rejected:
                assert result.returncode != 0, result.stdout
                return
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)

        command("stats")
        command("enqueue-index-directory", root, "unapproved", rejected=True)
        admitted = command("select-and-enqueue-directory", root, "first-selection")
        assert admitted["state"] == "queued" and admitted["directory_progress"]["total_units"] == 3
        assert command("stats")["artifacts"] == 0
        with sqlite3.connect(database) as connection:
            assert connection.execute("SELECT COUNT(*) FROM index_jobs").fetchone()[0] == 0
            consent = connection.execute("SELECT * FROM source_roots ORDER BY id").fetchall()
            assert len(consent) == 1
            assert connection.execute("SELECT kind,enabled FROM source_roots").fetchone() == ("directory", 1)
            assert command("select-and-enqueue-directory", root, "first-selection") == admitted
            assert connection.execute("SELECT * FROM source_roots ORDER BY id").fetchall() == consent

        # Every worker invocation is a new process, including the final reconciliation claim.
        first_quantum = command("run-next-job")
        assert first_quantum["directory_progress"]["next_unit"] == 1
        assert first_quantum["state"] == "queued" and command("stats")["versions"] == 1
        assert command("select-and-enqueue-directory", root, "first-selection") == first_quantum
        assert command("cancel-job", admitted["id"])["state"] == "cancelled"
        fresh = command("select-and-enqueue-directory", root, "fresh-selection")
        for ordinal in range(1, 4):
            quantum = command("run-next-job")
            assert quantum["id"] == fresh["id"] and quantum["state"] == "queued", quantum
            assert quantum["directory_progress"]["next_unit"] == ordinal
        completed = command("run-next-job")
        assert completed["state"] == "completed" and completed["directory_progress"] is None
        assert completed["result"]["indexed"] == 1 and completed["result"]["unchanged"] == 1
        assert completed["result"]["skipped"] == 1 and completed["result"]["missing"] == 0
        assert command("stats")["versions"] == 2
        assert len(command("search", '"first-selection evidence marker"')) == 2
        with sqlite3.connect(database) as connection:
            consent = connection.execute("SELECT * FROM source_roots ORDER BY id").fetchall()
            assert command("select-and-enqueue-directory", root, "fresh-selection") == completed
            assert connection.execute("SELECT * FROM source_roots ORDER BY id").fetchall() == consent
        assert command("run-next-job") is None
        command("purge-root", root)
        assert command("stats")["artifacts"] == 0 and command("jobs") == []
        with sqlite3.connect(database) as connection:
            for table in ("source_roots", "background_directory_units", "background_directory_manifests"):
                assert connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 0
        assert (root / "a.md").read_text(encoding="utf-8").startswith("Synthetic first-selection")
    print("directory selection: PASS (atomic first admission, replay, restart, cancellation, no duplicate versions, purge)")


if __name__ == "__main__":
    main()
