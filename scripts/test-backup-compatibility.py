#!/usr/bin/env python3
"""Verify real encrypted-backup compatibility between two CLI builds on synthetic data."""

import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization; compatibility checks must remain enabled")
    before, after = args.before.resolve(), args.after.resolve()
    before_hash = hashlib.sha256(before.read_bytes()).hexdigest()
    after_hash = hashlib.sha256(after.read_bytes()).hexdigest()
    if before.samefile(after) or before_hash == after_hash:
        parser.error("before and after must be distinct CLI builds, not the same binary")
    print("before_binary_sha256=" + before_hash)
    print("after_binary_sha256=" + after_hash)
    with tempfile.TemporaryDirectory(prefix="loom-backup-compatibility-") as temporary:
        root = Path(temporary).resolve()
        source = root / "source.md"
        source.write_text("# Evidence\n\nCross-build encrypted backup marker.\n", encoding="utf-8")
        password = root / "password.txt"
        password.write_text("synthetic-compatibility-password\n", encoding="utf-8")
        password.chmod(0o600)

        def command(binary, database, *arguments, rejected=False):
            result = subprocess.run(
                [str(binary), "--database", str(database), *map(str, arguments)],
                text=True, capture_output=True, timeout=30, check=False,
            )
            if rejected:
                assert result.returncode != 0, (arguments, result.stdout)
                assert not database.exists(), "rejected restore published a database"
                return
            assert result.returncode == 0, (arguments, result.stderr)
            return json.loads(result.stdout)

        original = root / "original.sqlite3"
        command(before, original, "index", source)
        revoked_source = root / "revoked.md"
        revoked_source.write_text("Persisted revoked backup marker.\n", encoding="utf-8")
        command(before, original, "index", revoked_source)
        # The CLI has no revoke command. Build only this negative fixture directly;
        # core tests separately exercise the public revocation API.
        with sqlite3.connect(original) as connection:
            changed = connection.execute(
                "UPDATE source_roots SET enabled=0, scope_generation=scope_generation+1 WHERE locator=?",
                [str(revoked_source)],
            ).rowcount
            assert changed == 1
        original_hits = command(before, original, "search", "encrypted backup marker")
        assert len(original_hits) == 1 and original_hits[0]["anchor"]["kind"] == "text"

        def identity(hit):
            return {key: hit[key] for key in (
                "artifact_id", "version_id", "passage_id", "content_hash", "anchor", "source_uri",
            )}

        old_backup = root / "old.loombak"
        command(before, original, "backup", old_backup, "--password-file", password)
        upgraded = root / "upgraded.sqlite3"
        command(after, upgraded, "restore", old_backup, "--password-file", password)
        # Restore preserves exported enabled state; it must not re-enable revoked roots.
        assert command(after, upgraded, "search", "revoked backup marker") == []
        new_hits = command(after, upgraded, "search", "encrypted backup marker")
        assert len(new_hits) == 1 and identity(new_hits[0]) == identity(original_hits[0])

        new_backup = root / "new.loombak"
        command(after, upgraded, "backup", new_backup, "--password-file", password)
        downgraded = root / "downgraded.sqlite3"
        command(before, downgraded, "restore", new_backup, "--password-file", password)
        assert command(before, downgraded, "search", "revoked backup marker") == []
        restored_hits = command(before, downgraded, "search", "encrypted backup marker")
        assert len(restored_hits) == 1 and identity(restored_hits[0]) == identity(original_hits[0])

        tampered = root / "tampered.loombak"
        ciphertext = bytearray(old_backup.read_bytes())
        ciphertext[-30] ^= 0x80
        tampered.write_bytes(ciphertext)
        for name, binary in (("before", before), ("after", after)):
            command(binary, root / (name + "-rejected.sqlite3"), "restore", tampered,
                    "--password-file", password, rejected=True)
        for database in (upgraded, downgraded):
            with sqlite3.connect(database) as connection:
                assert connection.execute(
                    "SELECT enabled,scope_generation FROM source_roots WHERE locator=?",
                    [str(revoked_source)],
                ).fetchone() == (0, 1)
        print("encrypted backup cross-build: PASS (both directions, exact evidence identity, revocation preserved, tamper refused)")


if __name__ == "__main__":
    main()
