#!/usr/bin/env python3
"""Exercise real queued approved-file refresh against synthetic local sources."""
import argparse
import fcntl
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, required=True)
    parser.add_argument("--previous-loom", type=Path)
    parser.add_argument("--native-ocr", action="store_true")
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization")
    if args.native_ocr and sys.platform != "darwin":
        parser.error("native OCR requires macOS")
    binary = args.loom.resolve()
    fixtures = Path(__file__).resolve().parents[1] / "benchmarks"
    with tempfile.TemporaryDirectory(prefix="loom-queued-source-") as temporary:
        root = Path(temporary).resolve()
        database = root / "library.sqlite3"
        source = root / "fixture.md"
        source.write_text("Original approved source marker.\n", encoding="utf-8")

        def command(*arguments, rejected=False, database_path=database, executable=binary):
            result = subprocess.run(
                [str(executable), "--database", str(database_path), *map(str, arguments)],
                text=True, capture_output=True, timeout=120 if args.native_ocr else 30, check=False,
            )
            if rejected:
                assert result.returncode != 0, (arguments, result.stdout)
                return result.stderr
            assert result.returncode == 0, (arguments, result.stderr)
            return json.loads(result.stdout)

        command("index", source)
        before = command("search", "approved source marker")[0]
        source.write_text("Refreshed queued source marker.\n", encoding="utf-8")
        queued = command("enqueue-index-file", source, "refresh-file")
        assert queued["operation"] == "index_file" and queued["state"] == "queued"
        assert command("enqueue-index-file", source, "refresh-file")["id"] == queued["id"]
        assert command("search", "queued source marker") == [], "admission indexed without a claim"
        with sqlite3.connect(database) as connection:
            checkpoints = connection.execute("SELECT COUNT(*) FROM index_jobs").fetchone()[0]
        completed = command("run-next-job")
        assert completed["id"] == queued["id"] and completed["state"] == "completed"
        after = command("search", "queued source marker")[0]
        assert after["artifact_id"] == before["artifact_id"]
        assert after["version_id"] != before["version_id"] and after["anchor"]["kind"] == "text"
        assert command("search", "Original approved source marker") == []
        with sqlite3.connect(database) as connection:
            assert connection.execute("SELECT COUNT(*) FROM index_jobs").fetchone()[0] == checkpoints
        unknown = root / "not-approved.md"
        unknown.write_text("must not be admitted", encoding="utf-8")
        command("enqueue-index-file", unknown, "unknown-scope", rejected=True)
        command("enqueue-index-file", root, "not-a-directory-job", rejected=True)
        cancelled = command("enqueue-index-file", source, "cancel-file")
        command("cancel-job", cancelled["id"])
        assert command("run-next-job") is None
        command("enqueue-index-file", source, "purge-file")
        command("purge-artifact", after["artifact_id"])
        assert command("jobs") == [], "artifact purge retained target locators/diagnostics"
        assert command("run-next-job") is None
        assert source.read_text(encoding="utf-8") == "Refreshed queued source marker.\n"

        pdf = root / "page.pdf"
        shutil.copyfile(fixtures / "retrieval/v1/corpus/pdf/research-page.pdf", pdf)
        pdf_bytes = pdf.read_bytes()
        command("index", pdf)
        command("purge-artifact", command("search", "exact artifact recovery marker")[0]["artifact_id"])
        command("enqueue-index-file", pdf, "pdf-recovery")
        assert command("run-next-job")["state"] == "completed"
        pdf_hit = command("search", "exact artifact recovery marker")[0]
        assert pdf_hit["anchor"]["kind"] == "pdf_page" and pdf_hit["anchor"]["page"] == 1
        assert pdf.read_bytes() == pdf_bytes
        malformed = root / "malformed.pdf"
        malformed.write_bytes(b"intentionally malformed PDF fixture")
        assert command("index", malformed)["failed"] == 1
        command("enqueue-index-file", malformed, "bad-pdf")
        failed = command("run-next-job")
        assert failed["state"] == "failed" and failed["attempts"] == 1
        assert command("search", "exact artifact recovery marker")[0]["artifact_id"] == pdf_hit["artifact_id"]

        image = root / "cropped-ocr.png"
        shutil.copyfile(fixtures / "retrieval/v1/corpus/screenshot/ocr-cropped.png", image)
        image_bytes = image.read_bytes()
        command("ocr-disable")
        command("index", image)
        command("enqueue-index-file", image, "ocr-off", rejected=True)
        if args.native_ocr:
            command("ocr-enable")
            command("enqueue-index-file", image, "native-ocr")
            assert command("run-next-job")["state"] == "completed"
            image_hit = command("search", "LOOM OCR marker")[0]
            assert image_hit["anchor"]["kind"] == "image_region"
            assert image_hit["anchor"]["width"] > 0 and image_hit["anchor"]["height"] > 0
            command("enqueue-index-file", image, "old-policy")
            command("ocr-enable")  # same enabled state still rotates the consent revision
            assert command("run-next-job")["state"] == "cancelled"
            command("enqueue-index-file", image, "ocr-purge")
            command("ocr-purge")
            assert not any(job["target_locator"] == str(image) for job in command("jobs"))
            assert command("search", "LOOM OCR marker") == []
            assert image.read_bytes() == image_bytes

        if args.previous_loom:
            previous = args.previous_loom.resolve()
            old_database = root / "version2.sqlite3"
            command("index", source, executable=previous, database_path=old_database)
            old_job = command("enqueue-fts-repair", "v2-preserve", executable=previous, database_path=old_database)
            command("stats", database_path=old_database)  # canonical opening does not migrate runtime
            with sqlite3.connect(old_database) as connection:
                assert connection.execute("SELECT value FROM schema_meta WHERE key='background_job_schema_version'").fetchone()[0] == "2"
            command("jobs", database_path=old_database, rejected=True)
            with old_database.with_name(old_database.name + ".worker.lock").open("a+b") as owner:
                fcntl.flock(owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
                command("upgrade-job-runtime", database_path=old_database, rejected=True)
            command("upgrade-job-runtime", database_path=old_database)
            command("upgrade-job-runtime", database_path=old_database)
            assert command("jobs", database_path=old_database)[0]["id"] == old_job["id"]
            command("jobs", executable=previous, database_path=old_database, rejected=True)
            assert command("search", "queued source marker", executable=previous, database_path=old_database)
            command("enqueue-index-file", source, "after-upgrade", database_path=old_database)
            assert command("run-next-job", database_path=old_database)["operation"] == "fts_repair"
            assert command("run-next-job", database_path=old_database)["operation"] == "index_file"
            assert command("run-next-job", database_path=old_database) is None
        print("queued source CLI: PASS (text/PDF, scoped admission, atomic parent, cancellation/purge, OCR-off)"
              + ("; native OCR PASS" if args.native_ocr else "; native OCR not run")
              + ("; v2/v3 binaries PASS" if args.previous_loom else "; previous binary not provided"))


if __name__ == "__main__":
    main()
