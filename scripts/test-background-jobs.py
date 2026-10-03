#!/usr/bin/env python3
"""Exercise the actual opt-in queue CLI against a synthetic temporary library."""
import argparse
import fcntl
import json
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, required=True)
    args = parser.parse_args()
    binary = args.loom.resolve()
    with tempfile.TemporaryDirectory(prefix="loom-job-cli-") as temporary:
        root = Path(temporary).resolve()
        database = root / "library.sqlite3"
        source = root / "fixture.md"
        source.write_text("# Fixture\n\nSource-faithful queue smoke marker.\n", encoding="utf-8")

        def command(*arguments, rejected=False, database_path=database):
            result = subprocess.run(
                [str(binary), "--database", str(database_path), *arguments],
                capture_output=True, text=True, timeout=30, check=False,
            )
            if rejected:
                assert result.returncode != 0, (arguments, result.stdout, result.stderr)
                return result.stderr
            assert result.returncode == 0, (arguments, result.stderr)
            return json.loads(result.stdout)

        command("index", str(source))
        before = source.read_bytes()
        queued = command("enqueue-fts-repair", "device-repair", "--high")
        assert queued["state"] == "queued"

        def projection_count():
            connection = sqlite3.connect(database)
            try:
                return connection.execute("SELECT COUNT(DISTINCT doc) FROM passages_fts_instances").fetchone()[0]
            finally:
                connection.close()

        def damage_projection():
            connection = sqlite3.connect(database)
            try:
                connection.execute("DELETE FROM passages_fts")
                connection.commit()
            finally:
                connection.close()
            assert projection_count() == 0

        damage_projection()
        assert command("enqueue-fts-repair", "device-repair", "--high") == queued
        assert "conflicting input" in command("enqueue-fts-repair", "device-repair", "--low", rejected=True)
        assert command("jobs")[0]["id"] == queued["id"]
        assert projection_count() == 0, "queue inspection/admission repaired FTS without a claim"
        with database.with_name(database.name + ".worker.lock").open("a+b") as owner:
            fcntl.flock(owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
            refusal = command("run-next-job", rejected=True)
            assert "JobWorkerBusy" in refusal, refusal
            assert projection_count() == 0, "contending worker repaired FTS before ownership"
        completed = command("run-next-job")
        assert completed["id"] == queued["id"] and completed["state"] == "completed"
        assert completed["attempts"] == 1 and completed["result"]["after"]["healthy"]
        assert not completed["result"]["before"]["healthy"], "CLI repaired before claiming the job"
        assert command("enqueue-fts-repair", "device-repair", "--high") == completed
        assert command("run-next-job") is None
        damage_projection()
        assert command("run-next-job") is None
        assert projection_count() == 0, "empty worker invocation repaired FTS"
        cancelled = command("enqueue-fts-repair", "device-cancel")
        assert command("cancel-job", cancelled["id"])["state"] == "cancelled"
        assert command("run-next-job") is None
        assert projection_count() == 0, "cancelled work repaired FTS"
        assert command("fts-health")["healthy"]
        assert source.read_bytes() == before
        hits = command("search", "queue smoke marker")
        assert hits and hits[0]["source_uri"] == str(source)
        assert hits[0]["anchor"]["kind"] == "text"
        assert len(command("jobs")) == 2
        pending = command("enqueue-fts-repair", "cannot-forget-pending")
        assert "only terminal" in command("forget-job", pending["id"], rejected=True)
        command("cancel-job", pending["id"])
        damage_projection()
        command("forget-job", pending["id"])
        assert projection_count() == 0, "forgetting repaired FTS"
        assert command("forget-job", completed["id"])["state"] == "completed"
        replacement = command("enqueue-fts-repair", "device-repair", "--high")
        assert replacement["id"] != completed["id"]
        assert command("run-next-job")["state"] == "completed"
        hardlink = root / "hard-link.sqlite3"
        os.link(database, hardlink)
        for operation in [
            ("jobs",), ("enqueue-fts-repair", "hard-link-denied"),
            ("cancel-job", replacement["id"]), ("forget-job", replacement["id"]),
            ("run-next-job",),
        ]:
            assert "hard-linked" in command(*operation, rejected=True, database_path=hardlink)
        print("background job CLI: PASS (duplicate/conflict, real repair, durable result, cancellation, source recovery)")


if __name__ == "__main__":
    main()
