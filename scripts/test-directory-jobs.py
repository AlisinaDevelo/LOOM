#!/usr/bin/env python3
"""Exercise resumable approved-directory quanta with synthetic/CC0 sources only."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization")
    binary = args.loom.resolve()
    measurements = []
    with tempfile.TemporaryDirectory(prefix="loom-directory-contract-") as temporary:
        base = Path(temporary).resolve()
        database = base / "library.sqlite3"
        root = base / "selected"
        root.mkdir()

        def command(*arguments, rejected=False):
            result = subprocess.run([str(binary), "--database", str(database), *map(str, arguments)],
                                    capture_output=True, text=True, timeout=40)
            if rejected:
                assert result.returncode != 0, result.stdout
                return
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)

        command("stats")
        command("ocr-disable")
        command("enqueue-index-directory", root, "unapproved", rejected=True)
        command("index", root)  # An explicit empty-folder selection, before queue admission.
        old = root / "old.md"
        old.write_text("Previous directory recovery marker\n", encoding="utf-8")
        command("index", root)
        old.unlink()
        for name in ("a.md", "b.md"):
            (root / name).write_text(f"Durable directory evidence marker {name}\n", encoding="utf-8")
        (root / "c.bin").write_bytes(b"Unsupported synthetic source; retained as a skipped unit")
        (root / "d.png").write_bytes(b"OCR-disabled source; never sent to a provider")
        pdf = Path(__file__).resolve().parents[1] / "benchmarks/retrieval/v1/corpus/pdf/research-page.pdf"
        (root / "e.pdf").write_bytes(pdf.read_bytes())
        command("enqueue-index-file", root / "a.md", "not-exact-file", rejected=True)
        job = command("enqueue-index-directory", root, "directory")
        assert job["directory_progress"]["total_units"] == 5
        assert command("enqueue-index-directory", root, "directory")["id"] == job["id"]
        maintenance = command("enqueue-fts-repair", "interleaved")
        for ordinal in range(1, 6):
            quantum = command("run-next-job")  # Each call is a fresh worker process.
            assert quantum["id"] == job["id"] and quantum["state"] == "queued", quantum
            assert quantum["attempts"] == 0
            progress = quantum["directory_progress"]
            assert progress["next_unit"] == ordinal
            if ordinal <= 2 or ordinal == 5:
                metrics = progress["last_extraction"]
                assert metrics["sample_interval_ms"] == 25
                assert metrics["address_space_limit_installed"] is True
                assert metrics["peak_resident_bytes"] > 0 and metrics["wall_ms"] > 0
                measurements.append({"ordinal": ordinal, **metrics})
            else:
                assert progress["last_extraction"] is None
            if ordinal == 1:
                assert command("run-next-job")["id"] == maintenance["id"]
        assert len(command("search", '"Previous directory recovery marker"')) == 1
        completed = command("run-next-job")
        assert completed["state"] == "completed" and completed["directory_progress"] is None
        assert completed["result"]["indexed"] == 3 and completed["result"]["skipped"] == 2
        assert completed["result"]["missing"] == 1
        assert command("search", '"Previous directory recovery marker"') == []
        assert len(command("search", '"Durable directory evidence marker"')) == 2
        hit = command("search", '"exact artifact recovery marker" type:pdf')[0]
        assert hit["anchor"]["kind"] == "pdf_page" and hit["anchor"]["page"] == 1
        assert (root / "e.pdf").read_bytes() == pdf.read_bytes()
        assert command("enqueue-index-directory", root, "directory") == completed
        assert command("run-next-job") is None
        command("purge-root", root)
        assert command("stats")["artifacts"] == 0
        assert not any(row["operation"] == "index_directory" for row in command("jobs"))
    if args.report:
        args.report.write_text(json.dumps({"fixture": "synthetic-approved-directory", "quanta": measurements}, indent=2) + "\n", encoding="utf-8")
    print("directory jobs: PASS (separate-process text/PDF quanta, fairness, skips, final reconciliation, purge)")


if __name__ == "__main__":
    main()
