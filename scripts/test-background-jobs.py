#!/usr/bin/env python3
"""Exercise the actual opt-in queue CLI against a synthetic temporary library."""
import argparse
import json
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

        def command(*arguments, rejected=False):
            result = subprocess.run(
                [str(binary), "--database", str(database), *arguments],
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
        assert command("enqueue-fts-repair", "device-repair", "--high") == queued
        assert "conflicting input" in command("enqueue-fts-repair", "device-repair", "--low", rejected=True)
        assert command("jobs")[0]["id"] == queued["id"]
        completed = command("run-next-job")
        assert completed["id"] == queued["id"] and completed["state"] == "completed"
        assert completed["attempts"] == 1 and completed["result"]["after"]["healthy"]
        assert command("enqueue-fts-repair", "device-repair", "--high") == completed
        assert command("run-next-job") is None
        cancelled = command("enqueue-fts-repair", "device-cancel")
        assert command("cancel-job", cancelled["id"])["state"] == "cancelled"
        assert command("run-next-job") is None
        assert command("fts-health")["healthy"]
        assert source.read_bytes() == before
        hits = command("search", "queue smoke marker")
        assert hits and hits[0]["source_uri"] == str(source)
        assert hits[0]["anchor"]["kind"] == "text"
        assert len(command("jobs")) == 2
        pending = command("enqueue-fts-repair", "cannot-forget-pending")
        assert "only terminal" in command("forget-job", pending["id"], rejected=True)
        command("cancel-job", pending["id"])
        command("forget-job", pending["id"])
        assert command("forget-job", completed["id"])["state"] == "completed"
        replacement = command("enqueue-fts-repair", "device-repair", "--high")
        assert replacement["id"] != completed["id"]
        assert command("run-next-job")["state"] == "completed"
        print("background job CLI: PASS (duplicate/conflict, real repair, durable result, cancellation, source recovery)")


if __name__ == "__main__":
    main()
