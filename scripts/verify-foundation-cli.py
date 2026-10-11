#!/usr/bin/env python3
"""Source-backed foundation checks against an operator-built CLI; synthetic data only."""
import argparse
import contextlib
import datetime
import errno
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import time
import uuid


class VerificationFailure(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise VerificationFailure(message)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


@contextlib.contextmanager
def connection(database):
    db = sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=0.1)
    try:
        yield db
    finally:
        db.close()


def current_projection(database):
    with connection(database) as db:
        rows = db.execute("""
            SELECT l.locator, a.media_type, v.content_hash, v.byte_size,
                   v.extractor_id, v.extractor_version, p.ordinal,
                   p.text, p.text_hash, p.locator_json
            FROM artifacts a JOIN artifact_locators l ON l.artifact_id=a.id AND l.active=1
            JOIN artifact_versions v ON v.id=a.active_version_id
            JOIN passages p ON p.artifact_version_id=v.id
            WHERE a.state='active' ORDER BY l.locator, p.ordinal
        """).fetchall()
    return rows


def canonical_identity(database):
    with connection(database) as db:
        return {table: db.execute(f"SELECT * FROM {table} ORDER BY id").fetchall()
                for table in ("artifacts", "artifact_locators", "artifact_versions", "passages")}


def calibrate_network(profile, root):
    """The denial must reject a connection that succeeds without the same policy."""
    sandbox = Path("/usr/bin/sandbox-exec")
    require(sys.platform == "darwin" and sandbox.is_file(),
            "macOS sandbox-exec is unavailable; network denial is unverified")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        port = listener.getsockname()[1]
        probe = (
            "import json,socket,sys\n"
            "try:\n"
            f" s=socket.create_connection(('127.0.0.1',{port}),timeout=2); s.close()\n"
            " print(json.dumps({'connected':True})); sys.exit(0)\n"
            "except OSError as exc:\n"
            " print(json.dumps({'connected':False,'errno':exc.errno})); sys.exit(13)\n"
        )
        argv = [sys.executable, "-I", "-c", probe]
        allowed = subprocess.run(argv, capture_output=True, text=True, timeout=5)
        require(allowed.returncode == 0 and json.loads(allowed.stdout)["connected"],
                "network calibration's unconstrained loopback control failed")
        accepted, _ = listener.accept()
        accepted.close()
        denied = subprocess.run([str(sandbox), "-f", str(profile), *argv],
                                capture_output=True, text=True, timeout=5)
        try:
            denied_result = json.loads(denied.stdout)
        except json.JSONDecodeError as exc:
            raise VerificationFailure(f"sandbox did not execute denial probe: {denied.stderr}") from exc
        require(denied.returncode == 13 and not denied_result["connected"]
                and denied_result["errno"] in (errno.EPERM, errno.EACCES),
                f"network policy did not produce a permission denial: {denied_result}")
        result = {"allowed_control": json.loads(allowed.stdout), "denied_control": denied_result,
                  "profile_sha256": digest(profile),
                  "meaning": "CLI checks can run with network operations denied; attempted connections are not measured"}
        write_json(root / "network-calibration.json", result)
        return result


class Runner:
    def __init__(self, binary, root, profile, timeout):
        self.binary, self.root, self.profile, self.timeout = binary, root, profile, timeout
        self.steps = []
        (root / "steps").mkdir()

    def argv(self, database, arguments):
        result = [str(self.binary), "--database", str(database), *map(str, arguments)]
        if self.profile:
            result = ["/usr/bin/sandbox-exec", "-f", str(self.profile), *result]
        return result

    def run(self, name, database, *arguments, rejected=False):
        argv = self.argv(database, arguments)
        measured = sys.platform == "darwin" and Path("/usr/bin/time").is_file()
        if measured:
            argv = ["/usr/bin/time", "-lp", *argv]
        start = time.monotonic()
        process = subprocess.Popen(argv, cwd=self.root, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, start_new_session=True)
        timed_out = False
        try:
            stdout, stderr = process.communicate(timeout=self.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            # This session/group was created exclusively for this invocation. Kill the owned
            # time wrapper and its CLI descendant together so timeout cannot orphan a writer.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate(timeout=5)
        result = subprocess.CompletedProcess(argv, process.returncode, stdout, stderr)
        elapsed = (time.monotonic() - start) * 1000
        prefix = self.root / "steps" / f"{len(self.steps) + 1:03d}-{name}"
        prefix.with_suffix(".stdout").write_text(result.stdout, encoding="utf-8")
        prefix.with_suffix(".stderr").write_text(result.stderr, encoding="utf-8")
        peak = re.search(r"(\d+)\s+maximum resident set size", result.stderr) if measured else None
        user = re.search(r"^user\s+([0-9.]+)$", result.stderr, re.MULTILINE) if measured else None
        system = re.search(r"^sys\s+([0-9.]+)$", result.stderr, re.MULTILINE) if measured else None
        step = {"id": name, "arguments": [str(item).replace(str(self.root), "<fixture>") for item in arguments],
                "returncode": result.returncode, "timed_out": timed_out, "wall_ms": elapsed,
                "peak_resident_bytes": int(peak.group(1)) if peak else None,
                "user_cpu_seconds": float(user.group(1)) if user else None,
                "system_cpu_seconds": float(system.group(1)) if system else None,
                "stdout": str(prefix.with_suffix(".stdout").relative_to(self.root)),
                "stderr": str(prefix.with_suffix(".stderr").relative_to(self.root))}
        self.steps.append(step)
        require(not timed_out, f"{name}: invocation timed out; owned subprocess group killed and reaped")
        if rejected:
            require(result.returncode != 0, f"{name}: expected refusal, got {result.stdout}")
            return result.stderr
        require(result.returncode == 0, f"{name}: {result.stderr}")
        try:
            value = json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise VerificationFailure(f"{name}: output was not JSON") from exc
        if isinstance(value, dict) and "run_id" in value:
            step["index_report"] = value
        return value


def verify_hits(runner, database, root, fixture_manifest, queries):
    expected = {Path(fixture["path"]).name: fixture for fixture in fixture_manifest["fixtures"]}
    hits_by_query = {}
    for query in queries:
        hits = runner.run(f"search-{query['id']}", database, "search", query["query"])
        require(len(hits) == 1, f"{query['id']}: expected exactly one smoke result")
        hit = hits[0]
        source = root / query["expected_file"]
        require(hit["source_uri"] == str(source), f"{query['id']}: wrong original locator")
        require(hit["rank"] == 1 and hit["content_hash"] == expected[source.name]["content_hash"],
                f"{query['id']}: wrong rank/source hash")
        for field in ("artifact_id", "version_id", "passage_id"):
            uuid.UUID(hit[field])
        anchor = {key: value for key, value in query["expected_anchor"].items() if key != "contains"}
        require(hit["anchor"] == anchor, f"{query['id']}: wrong exact anchor")
        text = source.read_text(encoding="utf-8")
        require(text[anchor["char_start"]:anchor["char_end"]] == query["expected_anchor"]["contains"],
                f"{query['id']}: character anchor does not select original source text")
        require(text[:anchor["char_start"]].count("\n") + 1 == anchor["line_start"],
                f"{query['id']}: wrong original source line")
        segments = hit["excerpt"]["segments"]
        require(segments and all(isinstance(segment["text"], str)
                                and isinstance(segment["highlighted"], bool) for segment in segments),
                f"{query['id']}: malformed structured excerpt")
        require("".join(segment["text"] for segment in segments) == text,
                f"{query['id']}: displayed excerpt changed original passage text")
        cursor, ranges = 0, []
        for segment in segments:
            end = cursor + len(segment["text"])
            if segment["highlighted"]:
                ranges.append((cursor, end))
            cursor = end
        require(ranges == [(anchor["char_start"], anchor["char_end"])],
                f"{query['id']}: displayed highlight does not identify the exact quoted phrase")
        with connection(database) as db:
            identity = db.execute("""
                SELECT a.id, v.id, p.id, l.locator, v.content_hash, p.text
                FROM artifacts a JOIN artifact_versions v ON v.id=a.active_version_id
                JOIN artifact_locators l ON l.artifact_id=a.id AND l.active=1
                JOIN passages p ON p.artifact_version_id=v.id
                WHERE a.id=? AND v.id=? AND p.id=? AND a.state='active'
            """, (hit["artifact_id"], hit["version_id"], hit["passage_id"])).fetchone()
        require(identity is not None and identity[:5] ==
                (hit["artifact_id"], hit["version_id"], hit["passage_id"], str(source), hit["content_hash"])
                and identity[5] == text, f"{query['id']}: result does not join to exact canonical source")
        observation = runner.run(f"inspect-{query['id']}", database, "inspect", source)
        fixture = expected[source.name]
        for field in ("content_hash", "extractor_id", "extractor_version", "passages"):
            require(observation[field] == fixture[field], f"{query['id']}: {field} fixture mismatch")
        hits_by_query[query["id"]] = hits
    return hits_by_query


def source_evidence_projection(hits_by_query):
    """Retained historical passages can change BM25 corpus statistics after an edit."""
    return {
        query: [
            {**{field: value for field, value in hit.items() if field != "score"},
             "contributions": {field: value for field, value in hit["contributions"].items()
                               if field != "lexical"}}
            for hit in hits
        ]
        for query, hits in hits_by_query.items()
    }


def forced_termination(runner, root):
    selected = root / "kill-selected"
    selected.mkdir()
    for index in range(96):
        line = f"Synthetic CC0 termination unit {index:03d} complete source text.\n"
        (selected / f"{index:03d}.md").write_text(line * (32768 // len(line)), encoding="utf-8")
    originals = {path.name: digest(path) for path in selected.iterdir()}
    database, reference = root / "kill.sqlite3", root / "reference.sqlite3"
    stdout = (root / "kill.stdout").open("w", encoding="utf-8")
    stderr = (root / "kill.stderr").open("w", encoding="utf-8")
    started = time.monotonic()
    process = subprocess.Popen(runner.argv(database, ("index", selected)), cwd=root,
                               stdout=stdout, stderr=stderr, start_new_session=True)
    observed = None
    try:
        deadline = time.monotonic() + runner.timeout
        while time.monotonic() < deadline and process.poll() is None:
            if database.exists():
                try:
                    with connection(database) as db:
                        row = db.execute("SELECT id,state,next_unit,total_units FROM index_jobs").fetchone()
                    if row and 0 < row[2] < row[3]:
                        observed = row
                        process.kill()
                        break
                except sqlite3.OperationalError:
                    pass
            time.sleep(0.002)
        require(observed is not None,
                "forced termination did not observe a committed incomplete checkpoint; no kill evidence")
        require(process.wait(timeout=5) == -signal.SIGKILL, "owned writer did not terminate from SIGKILL")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        stdout.close()
        stderr.close()
    kill_wall_ms = (time.monotonic() - started) * 1000
    with connection(database) as db:
        checkpoint = db.execute("SELECT id,state,next_unit,total_units FROM index_jobs").fetchone()
        require(db.execute("PRAGMA integrity_check").fetchone() == ("ok",), "post-kill SQLite integrity failure")
        require(db.execute("PRAGMA foreign_key_check").fetchall() == [], "post-kill partial relationship")
        artifacts = db.execute("SELECT COUNT(*) FROM artifacts").fetchone()[0]
        ready = db.execute("SELECT COUNT(*) FROM artifact_versions WHERE status='ready'").fetchone()[0]
        without_passages = db.execute("""SELECT COUNT(*) FROM artifact_versions v
            WHERE NOT EXISTS(SELECT 1 FROM passages p WHERE p.artifact_version_id=v.id)""").fetchone()[0]
    require(checkpoint[1] == "running" and 0 < checkpoint[2] < checkpoint[3]
            and artifacts == ready == checkpoint[2] and without_passages == 0,
            f"post-kill checkpoint/canonical mismatch: {checkpoint}, artifacts={artifacts}, ready={ready}")
    resumed = runner.run("resume-killed-writer", database, "index", selected)
    require(resumed["run_id"] == checkpoint[0] and resumed["indexed"] == 96 - checkpoint[2]
            and resumed["attempted"] == 96 - checkpoint[2] and resumed["failed"] == 0
            and resumed["failures"] == [] and resumed["cancelled"] == 0,
            f"killed writer did not resume exact remaining units: {resumed}")
    runner.run("uninterrupted-reference", reference, "index", selected)
    require(current_projection(database) == current_projection(reference),
            "resumed canonical active projection differs from uninterrupted reference")
    require(runner.run("resumed-stats", database, "stats") == runner.run("reference-stats", reference, "stats"),
            "resumed canonical counts differ from reference")
    rerun = runner.run("resumed-idempotence", database, "index", selected)
    require(rerun["indexed"] == 0 and rerun["unchanged"] == 96 and rerun["failed"] == 0,
            "resumed sources duplicated on a later complete scan")
    require(originals == {path.name: digest(path) for path in selected.iterdir()}, "recovery modified originals")
    return {"method": "SIGKILL of the owned foreground CLI after an observed committed prefix",
            "observed_cursor_before_kill": observed[2], "cursor_after_kill": checkpoint[2],
            "total_units": checkpoint[3], "kill_wall_ms": kill_wall_ms,
            "run_id": checkpoint[0], "integrity_and_foreign_keys": "passed",
            "canonical_active_projection_equals_reference": True, "originals_unchanged": True,
            "peak_resident_bytes_for_killed_process": None,
            "limits": "active projection comparison ignores random IDs/timestamps; no disk-full/power-loss or GUI lifecycle claim"}


def verify(runner, repo, root, report):
    fixture_root = repo / "benchmarks/retrieval/v0"
    require(not fixture_root.is_symlink(), "repository fixture root is a symlink")
    selected = root / "v0" / "corpus"
    selected.parent.mkdir()
    shutil.copytree(fixture_root / "corpus", selected, symlinks=True)
    for name in ("manifest.json", "queries.jsonl"):
        shutil.copyfile(fixture_root / name, selected.parent / name)
    require(all(path.is_file() and not path.is_symlink() for path in selected.iterdir()),
            "v0 fixture contains non-regular or linked sources")
    originals = {path.name: digest(path) for path in selected.iterdir()}
    manifest = json.loads((selected.parent / "manifest.json").read_text())
    queries = [json.loads(line) for line in (selected.parent / "queries.jsonl").read_text().splitlines() if line]
    require(manifest["license"] == "CC0-1.0" and manifest["query_count"] == len(queries) == 3,
            "unexpected v0 fixture contract")
    database = root / "foundation.sqlite3"
    require(not database.exists(), "fresh-library database already exists")
    empty = runner.run("fresh-library", database, "stats")
    require(all(empty[field] == 0 for field in ("source_roots", "artifacts", "versions", "passages", "indexed_bytes")),
            "fresh library is not empty")
    indexed = runner.run("v0-index", database, "index", selected)
    for field, expected in {"discovered": 3, "attempted": 3, "indexed": 3, "unchanged": 0,
                            "skipped": 0, "failed": 0, "cancelled": 0}.items():
        require(indexed[field] == expected, f"v0-index {field} mismatch")
    require(indexed["bytes_read"] == sum(path.stat().st_size for path in selected.iterdir())
            and indexed["failures"] == [], "v0 source-byte accounting mismatch")
    uuid.UUID(indexed["run_id"])
    initial_hits = verify_hits(runner, database, selected, manifest, queries)
    no_match = "foundationnomatchd5192687"
    require(runner.run("no-match", database, "search", no_match) == [], "unsupported no-match evidence")
    with connection(database) as db:
        for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall():
            safe_table = '"' + table.replace('"', '""') + '"'
            rows = db.execute(f"SELECT * FROM {safe_table}").fetchall()
            require(not any(no_match in str(value) for row in rows for value in row),
                    f"query text persisted in {table}")
        require(db.execute("SELECT locator,kind,enabled FROM source_roots").fetchall()
                == [(str(selected), "directory", 1)], "selection silently widened")
    benchmark = runner.run("official-v0-smoke", database, "benchmark", "--corpus", selected,
                           "--queries", selected.parent / "queries.jsonl")
    require(benchmark["overall"]["queries"] == 3 and benchmark["failures"] == []
            and benchmark["index"]["completeness"] == 1.0, "official v0 smoke failed")
    report["v0_smoke"] = benchmark
    require(verify_hits(runner, database, selected, manifest, queries) == initial_hits,
            "separate process reopen changed ranked source identity")
    stats = runner.run("before-idempotence", database, "stats")
    repeated = runner.run("v0-idempotence", database, "index", selected)
    require(repeated["indexed"] == 0 and repeated["unchanged"] == 3 and repeated["failed"] == 0
            and runner.run("after-idempotence", database, "stats") == stats,
            "unchanged explicit rescan duplicated canonical versions")
    report["checks"].extend(["fresh library", "v0 exact identity/hash/character-line anchors",
                             "process reopen and unchanged idempotence", "explicit no-match", "query not stored in SQLite"])

    # Refuse unselected/out-of-root sources and do not follow nested or explicit aliases.
    outside = root / "unselected.md"
    outside.write_text("unselectedboundarye9194938 synthetic CC0\n", encoding="utf-8")
    link = selected / "linked.md"
    link.symlink_to(outside)
    runner.run("nested-symlink-rescan", database, "index", selected)
    require(runner.run("unselected-boundary", database, "search", "unselectedboundarye9194938") == [],
            "symlink imported an unselected source")
    before = canonical_identity(database)
    refusal = runner.run("explicit-symlink", database, "index", link, rejected=True)
    require("symbolic links are not followed" in refusal and canonical_identity(database) == before,
            "explicit symlink refusal changed canonical evidence")
    link.unlink()

    # Search may retain the indexed observation until a rescan; verified opening is a native API.
    edited = selected / "isolation.md"
    prior_hit = initial_hits["q001"][0]
    edited.write_text("# Synthetic revision\n\nstalerecoverysourcee5594791 evidence.\n", encoding="utf-8")
    stored = runner.run("stored-inspect-after-edit", database, "inspect", edited)
    require(stored["content_hash"] == prior_hit["content_hash"], "inspect unexpectedly promised current-byte resolution")
    rescan = runner.run("stale-source-explicit-rescan", database, "index", selected)
    recovered = runner.run("stale-source-new-marker", database, "search", "stalerecoverysourcee5594791")
    require(rescan["indexed"] == 1 and rescan["unchanged"] == 2 and len(recovered) == 1
            and recovered[0]["artifact_id"] == prior_hit["artifact_id"]
            and recovered[0]["version_id"] != prior_hit["version_id"]
            and recovered[0]["content_hash"] != prior_hit["content_hash"]
            and runner.run("stale-source-old-marker", database, "search", queries[0]["query"]) == [],
            "stale explicit rescan did not replace the active projection")
    edited_stats = runner.run("edited-version-counts", database, "stats")
    shutil.copyfile(fixture_root / "corpus/isolation.md", edited)
    runner.run("restore-synthetic-original", database, "index", selected)
    restored_hits = verify_hits(runner, database, selected, manifest, queries)
    # Returning bytes reactivates the exact original version/passages. The retained edited
    # historical passage changes BM25's corpus statistics, so score/lexical weight need not
    # equal the initial three-passage corpus. All identity, evidence and rank fields must match.
    require(source_evidence_projection(restored_hits) == source_evidence_projection(initial_hits),
            "returning to original bytes did not reuse original projection")
    require(runner.run("restored-version-counts", database, "stats") == edited_stats,
            "returning to original bytes duplicated a retained version or passage")
    report["checks"].extend(["nested and explicit symlink refusal", "unselected sibling excluded",
                             "stale-source explicit rescan and source-version reuse"])

    # Corruption is injected only into the disposable FTS derivative of this synthetic library.
    before = canonical_identity(database)
    health = runner.run("fts-baseline", database, "fts-health")
    runner.run("fts-repair-admission", database, "enqueue-fts-repair", "foundation-fts-repair")
    with contextlib.closing(sqlite3.connect(database)) as db:
        db.execute("DELETE FROM passages_fts")
        db.commit()
    with connection(database) as db:
        require(db.execute("SELECT COUNT(DISTINCT doc) FROM passages_fts_instances").fetchone()[0] == 0,
                "disposable projection was not damaged")
    repaired = runner.run("fts-claimed-repair", database, "run-next-job")
    require(repaired["state"] == "completed" and not repaired["result"]["before"]["healthy"]
            and repaired["result"]["after"]["healthy"] and canonical_identity(database) == before,
            "real derivative repair did not detect drift/preserve canonical rows")
    final_health = runner.run("fts-repeatable-digest", database, "fts-health")
    for field in ("canonical_digest", "expected_derivative_digest", "derivative_digest"):
        require(final_health[field] == health[field], f"FTS {field} not reproducible")
    require(verify_hits(runner, database, selected, manifest, queries) == restored_hits,
            "FTS repair changed ranked search identities")
    report["checks"].append("real damaged-FTS diagnosis, transactional repair, canonical/rank/digest preservation")

    negatives = root / "negative-selected"
    negatives.mkdir()
    (negatives / "oversize.md").write_bytes(b"oversizeboundaryd1fa9453\n")
    with (negatives / "oversize.md").open("ab") as stream:
        stream.truncate(8 * 1024 * 1024 + 1)
    invalid = negatives / "invalid.md"
    invalid.write_bytes(b"\xff invalid UTF-8 synthetic fixture")
    (negatives / "unsupported.bin").write_bytes(b"unsupportedboundaryd1fa9453")
    negative_database = root / "negatives.sqlite3"
    refused = runner.run("bounded-invalid-sources", negative_database, "index", negatives)
    require(refused["attempted"] == 3 and refused["failed"] == 2 and refused["skipped"] == 1
            and refused["indexed"] == 0 and refused["bytes_read"] == 0
            and any("8388608-byte limit" in item["reason"] for item in refused["failures"])
            and runner.run("negative-canonical-empty", negative_database, "stats")["artifacts"] == 0
            and runner.run("negative-no-evidence", negative_database, "search", "boundaryd1fa9453") == [],
            f"invalid/oversize sources did not fail with bounded outcomes: {refused}")
    invalid.write_text("recoveredinvalidsourceb2ee8901 synthetic CC0\n", encoding="utf-8")
    recovery = runner.run("recover-invalid-source", negative_database, "index", negatives)
    require(recovery["indexed"] == 1 and recovery["failed"] == 1
            and len(runner.run("recovered-invalid-hit", negative_database, "search", "recoveredinvalidsourceb2ee8901")) == 1,
            "failed synthetic input did not recover after explicit rescan")
    report["checks"].extend(["8 MiB bound, invalid UTF-8 and unsupported outcomes", "failed-source recovery"])
    report["forced_termination"] = forced_termination(runner, root)
    report["checks"].append("actual owned process termination, complete committed units and canonical-equivalent resume")
    report["storage"] = runner.run("storage-accounting", database, "storage-inspect")
    require(report["storage"]["source_bytes"] > 0 and report["storage"]["total_bytes"] > 0,
            "storage resource counters unavailable")
    runner.run("explicit-purge", database, "purge-root", selected)
    require(runner.run("purged-stats", database, "stats")["artifacts"] == 0
            and runner.run("purged-no-evidence", database, "search", queries[0]["query"]) == [],
            "explicit purge retained searchable evidence")
    require(originals == {path.name: digest(path) for path in selected.iterdir()},
            "read/index/repair/purge operations modified original synthetic bytes")
    report["checks"].append("explicit purge preserves all user-owned synthetic original files")
    report["fixture_input_sha256"] = originals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--loom", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True, help="fresh source checkout, read only")
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--out-dir", type=Path, required=True, help="new task-owned synthetic output directory")
    parser.add_argument("--network-mode", choices=("macos-deny", "unconstrained"), default="macos-deny")
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    binary, repo = args.loom.resolve(), args.repo.resolve()
    require(binary.is_file() and os.access(binary, os.X_OK), "operator CLI executable unavailable")
    head = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
    require(head == args.expected_commit, "checkout does not match expected source commit")
    for staged in (False, True):
        argv = ["git", "-C", str(repo), "diff", "--quiet"]
        if staged:
            argv.append("--cached")
        require(subprocess.run(argv, check=False).returncode == 0, "source checkout contains tracked changes")
    args.out_dir.mkdir(mode=0o700, parents=False, exist_ok=False)
    root = args.out_dir.resolve()
    os.chmod(root, 0o700)
    report = {"status": "RUNNING", "recorded_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "source_commit": head, "binary_sha256": digest(binary), "harness_sha256": digest(__file__),
              "binary_origin": "operator-supplied; fresh-checkout build provenance must be retained separately",
              "device": {"os": platform.system(), "os_release": platform.release(), "architecture": platform.machine()},
              "checks": [], "network_mode": args.network_mode,
              "limitations": ["synthetic/CC0 fixture, not a quality or activation study",
                              "CLI has no verified-evidence/open-original or revoke/reconcile command",
                              "native verified-open/stale-source refusal, external viewer and revocation remain separate",
                              "resource measurements cover this fixture, not whole-library responsiveness or power loss"]}
    if sys.platform == "darwin":
        report["device"]["macos_version"] = subprocess.check_output(["sw_vers", "-productVersion"], text=True).strip()
        report["device"]["macos_build"] = subprocess.check_output(["sw_vers", "-buildVersion"], text=True).strip()
        report["device"]["model"] = subprocess.check_output(["sysctl", "-n", "hw.model"], text=True).strip()
    profile = None
    runner = None
    try:
        if args.network_mode == "macos-deny":
            profile = root / "network-denied.sb"
            profile.write_text("(version 1)\n(allow default)\n(deny network*)\n", encoding="utf-8")
            report["network_calibration"] = calibrate_network(profile, root)
        runner = Runner(binary, root, profile, args.timeout)
        verify(runner, repo, root, report)
        report["status"] = "PASS"
    except Exception as exc:
        report["status"] = "FAIL"
        report["error"] = f"{type(exc).__name__}: {exc}"
    finally:
        report["steps"] = runner.steps if runner else []
        write_json(root / "report.json", report)
        receipts = {str(path.relative_to(root)): digest(path) for path in sorted(root.rglob("*"))
                    if path.is_file() and not path.is_symlink() and path.suffix not in (".sqlite3",)
                    and not path.name.endswith(("-wal", "-shm"))}
        write_json(root / "artifact-sha256.json", receipts)
    print(json.dumps({"status": report["status"], "report": str(root / "report.json"),
                      "error": report.get("error")}, indent=2))
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
