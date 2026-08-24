#!/usr/bin/env python3
"""Run the checked-in adversarial PDF extraction contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path
from typing import Any


def command(binary: Path, database: Path, *args: str) -> tuple[int, Any, str]:
    result = subprocess.run(
        [str(binary), "--database", str(database), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    try:
        payload: Any = json.loads(result.stdout)
    except json.JSONDecodeError:
        payload = None
    return result.returncode, payload, result.stderr.strip()


def failure(result: list[dict[str, Any]], fixture: dict[str, Any], message: str) -> None:
    result.append(
        {
            "id": fixture["id"],
            "class": fixture["class"],
            "message": message,
        }
    )


def verify_fixture(
    binary: Path,
    database: Path,
    corpus: Path,
    fixture: dict[str, Any],
    index_report: dict[str, Any],
    failures: list[dict[str, Any]],
) -> None:
    path = (corpus / fixture["path"]).resolve()
    if not path.is_file() or not path.is_relative_to(corpus.resolve()):
        failure(failures, fixture, f"fixture path is missing or escapes corpus: {path}")
        return
    actual_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual_hash != fixture["byte_hash"]:
        failure(
            failures,
            fixture,
            f"byte hash changed: expected {fixture['byte_hash']}, observed {actual_hash}",
        )
        return

    source = str(path)
    matching_failures = [item for item in index_report.get("failures", []) if item.get("source") == source]
    if fixture["outcome"] == "unsupported":
        if len(matching_failures) != 1:
            failure(failures, fixture, f"expected one unsupported failure, got {matching_failures}")
            return
        reason = matching_failures[0].get("reason", "")
        if fixture["expected_error_contains"] not in reason:
            failure(
                failures,
                fixture,
                f"failure class drifted: expected {fixture['expected_error_contains']!r}, observed {reason!r}",
            )
        return

    if matching_failures:
        failure(failures, fixture, f"indexed fixture failed: {matching_failures}")
        return
    return_code, observation, stderr = command(binary, database, "inspect", str(path))
    if return_code != 0 or not isinstance(observation, dict):
        failure(failures, fixture, f"inspect failed ({return_code}): {stderr}")
        return
    expected = {
        "source_uri": source,
        "content_hash": None,
        "extractor_id": fixture["extractor_id"],
        "extractor_version": fixture["extractor_version"],
        "page_count": fixture["expected_page_count"],
        "parse_warnings": fixture["expected_warnings"],
    }
    if observation.get("source_uri") != expected["source_uri"]:
        failure(failures, fixture, "source URI is not canonical")
    for key in ("extractor_id", "extractor_version", "page_count", "parse_warnings"):
        if observation.get(key) != expected[key]:
            failure(
                failures,
                fixture,
                f"{key} drifted: expected {expected[key]!r}, observed {observation.get(key)!r}",
            )
    pages = sorted(
        {
            anchor["page"]
            for passage in observation.get("passages", [])
            for anchor in [passage.get("anchor", {})]
            if anchor.get("kind") == "pdf_page"
        }
    )
    if pages != fixture["expected_pages"]:
        failure(failures, fixture, f"page identity drifted: expected {fixture['expected_pages']}, observed {pages}")
    return_code, hits, stderr = command(binary, database, "search", fixture["expected_contains"], "--limit", "10")
    if return_code != 0 or not isinstance(hits, list):
        failure(failures, fixture, f"marker search failed ({return_code}): {stderr}")
        return
    if not any(
        hit.get("source_uri") == source
        and hit.get("anchor", {}).get("kind") == "pdf_page"
        and hit.get("anchor", {}).get("page") in fixture["expected_pages"]
        for hit in hits
    ):
        failure(failures, fixture, "expected marker did not recover its source page anchor")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--loom", type=Path, default=Path("target/debug/loom"))
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path("benchmarks/pdf-adversarial/corpus/manifest.json"),
    )
    args = parser.parse_args()
    manifest_path = args.manifest.resolve()
    corpus = manifest_path.parent
    manifest = json.loads(manifest_path.read_text())
    failures: list[dict[str, Any]] = []
    if manifest.get("schema_version") != 1 or manifest.get("license") != "CC0-1.0":
        failures.append({"id": "manifest", "class": "manifest", "message": "schema/license contract failed"})
    fixtures = manifest.get("fixtures", [])
    ids = [fixture.get("id") for fixture in fixtures]
    if len(ids) != len(set(ids)):
        failures.append({"id": "manifest", "class": "manifest", "message": "fixture IDs are not unique"})
    expected_paths = {fixture.get("path") for fixture in fixtures}
    actual_paths = {path.name for path in corpus.glob("*.pdf")}
    if expected_paths != actual_paths:
        failures.append(
            {
                "id": "manifest",
                "class": "manifest",
                "message": f"manifest/file set drifted: expected {sorted(expected_paths)}, observed {sorted(actual_paths)}",
            }
        )
    extractor = manifest.get("extractor", {})
    for fixture in fixtures:
        if fixture.get("extractor_id") != extractor.get("id") or fixture.get("extractor_version") != extractor.get("version"):
            failures.append(
                {
                    "id": fixture.get("id", "unknown"),
                    "class": fixture.get("class", "manifest"),
                    "message": "fixture extractor identity does not match manifest extractor",
                }
            )
        if fixture.get("outcome") not in {"indexed", "unsupported"}:
            failures.append(
                {
                    "id": fixture.get("id", "unknown"),
                    "class": fixture.get("class", "manifest"),
                    "message": f"unknown fixture outcome: {fixture.get('outcome')!r}",
                }
            )
    if not args.loom.is_file():
        failures.append({"id": "runner", "class": "runner", "message": f"loom binary is missing: {args.loom}"})
        print(json.dumps({"status": "fail", "failures": failures}, indent=2))
        return 2

    with tempfile.TemporaryDirectory(prefix="loom-pdf-adversarial-") as temporary:
        database = Path(temporary) / "library.sqlite3"
        aggregate = {"discovered": 0, "indexed": 0, "failed": 0, "skipped": 0}
        for fixture in fixtures:
            path = (corpus / fixture["path"]).resolve()
            return_code, index_report, stderr = command(args.loom.resolve(), database, "index", str(path))
            if return_code != 0 or not isinstance(index_report, dict):
                failures.append({"id": fixture["id"], "class": fixture["class"], "message": f"index failed ({return_code}): {stderr}"})
                index_report = {}
            for key in aggregate:
                aggregate[key] += int(index_report.get(key, 0))
            verify_fixture(args.loom.resolve(), database, corpus, fixture, index_report, failures)

    classes: dict[str, dict[str, int]] = defaultdict(lambda: {"expected": 0, "indexed": 0, "unsupported": 0, "failures": 0})
    for fixture in fixtures:
        values = classes[fixture["class"]]
        values["expected"] += 1
        values[fixture["outcome"]] += 1
    for item in failures:
        classes[item["class"]]["failures"] += 1
    expected = len(fixtures)
    observed = sum(values["indexed"] + values["unsupported"] for values in classes.values())
    report = {
        "schema_version": 1,
        "status": "pass" if not failures and observed == expected else "fail",
        "corpus": str(corpus),
        "fixtures": expected,
        "completeness": observed / expected if expected else 1.0,
        "by_class": dict(sorted(classes.items())),
        "index": {
            "discovered": aggregate["discovered"],
            "indexed": aggregate["indexed"],
            "failed": aggregate["failed"],
            "skipped": aggregate["skipped"],
        },
        "failures": failures,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["status"] == "pass" else 2


if __name__ == "__main__":
    raise SystemExit(main())
